use alsa_sys::*;
use std::os::unix::io::AsRawFd;
use libc::{
    self, 
    c_int, EAGAIN, EPIPE,
    termios, tcgetattr, tcsetattr, cfmakeraw, TCSANOW};
use std::{
    ptr,
    thread,
    ffi::CString,
    io::{self, Read, Write},
    time::{Duration, Instant},
    collections::{HashMap, hash_map::Entry},
    sync::{Arc, Mutex, 
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering}
    },
};

use crate::decode_helpers::AudioFile;

pub fn run_gart(tracks: HashMap<String, AudioFile>, sample_rate: u32, num_channels: u32) {
    // initialize audio engine and tracks
    
    let conductor = Arc::new(Mutex::new(Conductor::prepare(num_channels as usize, tracks)));
    let cond_for_repl = Arc::clone(&conductor);

    sample_rate::set(sample_rate);

    // take over STDIN
    let marker = Arc::new(Mutex::new(0usize));
    let buffer = Arc::new(Mutex::new(String::new()));
    let repl_chars = ['^', 'X', 'v', '>', 'X', '<', 'Z'];

    {
        let marker = marker.clone();
        let buffer = buffer.clone();
        let sr = sample_rate.clone();
        thread::spawn(move || {
            let mut last_len = 0;
            loop {
                {
                    // every 100 ms, change the REPL marker
                    let mut m = marker.lock().unwrap();
                    let tenth = (clock::current() / (sr as u64 / 10u64));
                    *m = tenth as usize % repl_chars.len();
                }

                {
                    // redraw marker + input_text
                    let m = *marker.lock().unwrap();
                    let buf = buffer.lock().unwrap();
                    let curr_len = buf.len();
                    print!("\r{} {}", repl_chars[m], *buf);

                    if last_len > curr_len {
                        let diff = last_len - curr_len;
                        for _ in 0..diff {
                            print!(" ");
                        }

                        print!("\x1b[{}D", diff);
                    }
                    last_len = curr_len;

                    std::io::stdout().flush().unwrap();
                }
            }
        });
    }
 
    raw_mode("on");

    // REPL
    println!("");
    {
        let buffer = buffer.clone();
        thread::spawn(move || {
            loop {
                let c = read_char();
               
                match c {
                    b'\n' | b'\r' => {
                        // enter
                        print!("\n");
                        let mut buf = buffer.lock().unwrap();
                        let mut cmd = buf.clone();
                        let mut parts = cmd.splitn(2, ' ');
                        let cmd = parts.next().unwrap();
                        let args = parts.next().unwrap_or_else(|| "");

                        let mut con = cond_for_repl.lock().unwrap();
                        match cmd {
                            "load" => con.load_voice(args),
                            "start" => con.start_voice(args),
                            "pause" => con.pause_voice(args),
                            "stop" => con.stop_voice(args),
                            "unload" => con.unload_voice(args),
                            "velocity" => con.set_velocity(args),
                            "seq" => con.seq(args),
                            "q" | "quit" => {
                                unsafe {
                                    libc::raise(libc::SIGTERM);
                                }
                                break;
                            }
                            _ => {
                                buf.clear();
                                println!("\nUnknown command '{}'", cmd);
                            }
                        }
                        buf.clear();
                    }
                    127 => {
                        // backspace
                        let mut buf = buffer.lock().unwrap();
                        if !buf.is_empty() {
                            buf.pop();
                        }
                    }
                    3 => {
                        // CTL + C
                        raw_mode("off");
                        let mut buf = buffer.lock().unwrap();
                        buf.clear();
                        println!("\nInterrupted.");
                        std::process::exit(130);
                    }
                    _ => {
                        let mut buf = buffer.lock().unwrap();
                        buf.push(c as char);
                    }
                }
            }
        });
    }

    // install signal catchers and panic callbacks 
    // to break main loop and turn off raw_mode
    install_sigterm_handler();
    install_panic_hook();

    // audio setup and main loop
    unsafe {
        // open pcm
        let mut handle: *mut snd_pcm_t = ptr::null_mut();
        let dev = CString::new("hw:0,0").unwrap();

        check_code(
            snd_pcm_open(
                &mut handle,
                dev.as_ptr(),
                SND_PCM_STREAM_PLAYBACK,
                SND_PCM_NONBLOCK,
            ),
            "snd_pcm_open",
        );

        // config hardware
        let mut hw: *mut snd_pcm_hw_params_t = ptr::null_mut();
        snd_pcm_hw_params_malloc(&mut hw);
        snd_pcm_hw_params_any(handle, hw);

        check_code(
            snd_pcm_hw_params_set_access(handle, hw, SND_PCM_ACCESS_MMAP_INTERLEAVED),
            "set_access",
        );
        check_code(
            snd_pcm_hw_params_set_format(handle, hw, SND_PCM_FORMAT_S16_LE),
            "set_format",
        );
        check_code(snd_pcm_hw_params_set_channels(handle, hw, num_channels), "set_ channels");
        check_code(snd_pcm_hw_params_set_rate(handle, hw, sample_rate, 0), "set_rate");

        let mut period_size: snd_pcm_uframes_t = 128;
        check_code(
            snd_pcm_hw_params_set_period_size_near(handle, hw, &mut period_size, 0 as *mut i32),
            "set_period_size",
        );

        let mut buffer_size: snd_pcm_uframes_t = period_size * 4;
        check_code(
            snd_pcm_hw_params_set_buffer_size_near(handle, hw, &mut buffer_size),
            "set_buffer_size",
        );

        check_code(snd_pcm_hw_params(handle, hw), "snd_pcm_hw_params");
        snd_pcm_hw_params_free(hw);

        // config software params
        let mut sw: *mut snd_pcm_sw_params_t = ptr::null_mut();
        snd_pcm_sw_params_malloc(&mut sw);
        snd_pcm_sw_params_current(handle, sw);

        let mut boundary: snd_pcm_uframes_t = 0;
        snd_pcm_sw_params_get_boundary(sw, &mut boundary);
        snd_pcm_sw_params_set_stop_threshold(handle, sw, boundary);
        // start immediately upon write
        check_code(snd_pcm_sw_params_set_start_threshold(handle, sw, period_size), "set_start_threshold");

        // wake when period is available
        check_code(
            snd_pcm_sw_params_set_avail_min(handle, sw, period_size),
            "set_avail_min",
        );

        check_code(snd_pcm_sw_params(handle, sw), "snd_pcm_sw_params");
        snd_pcm_sw_params_free(sw);

        // prepare device
        check_code(snd_pcm_prepare(handle), "snd_pcm_prepare");
       
        loop {
            if TERM_RECEIVED.load(Ordering::Relaxed) {
                break;
            }

            let mut avail = snd_pcm_avail_update(handle) as i32;
            if avail == -EPIPE {
                // underrun
                snd_pcm_recover(handle, avail, 1);
                continue;
            }
            if avail < 0 {
                snd_pcm_recover(handle, avail, 1);
                continue;
            }
            if avail == 0 {
                continue;
            }

            let mut remaining = avail as snd_pcm_uframes_t;

            while remaining > 0 {
                let mut areas_ptr: *const snd_pcm_channel_area_t = ptr::null();
                let mut offset: snd_pcm_uframes_t = 0;
                let mut frames: snd_pcm_uframes_t = remaining;

                // mmap begin
                let r = snd_pcm_mmap_begin(handle, &mut areas_ptr, &mut offset, &mut frames);

                if r == -EAGAIN {
                    break; // hardware not ready
                }
                if r < 0 {
                    snd_pcm_recover(handle, r, 1);
                    break;
                }

                // write to DMA buffer
                let mut con = conductor.lock().unwrap();
                con.coordinate(areas_ptr, offset, frames);

                let committed = snd_pcm_mmap_commit(handle, offset, frames) as i32;
                if committed < 0 {
                    snd_pcm_recover(handle, committed, 1);
                    break;
                }

                remaining -= committed as snd_pcm_uframes_t;
            }
            if snd_pcm_state(handle) != SND_PCM_STATE_RUNNING {
                snd_pcm_start(handle);
            }
        }
    }

    buffer.lock().unwrap().clear();
    raw_mode("off");
}

// check error codes for alsa
//
unsafe fn check_code(code: c_int, ctx: &str) {
    if code < 0 {
        let msg = std::ffi::CStr::from_ptr(snd_strerror(code));
        panic!("{ctx}: {}", msg.to_string_lossy());
    }
}

// signal and panic handlers
//
static TERM_RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_sig: libc::c_int) {
    TERM_RECEIVED.store(true, Ordering::Relaxed);
    raw_mode("off");
}

fn install_sigterm_handler() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_sigterm as usize;
        sa.sa_flags = 0;

        // non-blocking
        libc::sigemptyset(&mut sa.sa_mask);

        // register
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        raw_mode("off");
        eprintln!("\nPanic: {info}");
    }));
}

// terminal takeover funcs
//
static mut ORIG_TERM: Option<termios> = None;

fn raw_mode(switch: &str) {
    unsafe {
        let fd = libc::STDIN_FILENO;

        let mut term: termios = std::mem::zeroed();
        tcgetattr(fd, &mut term);

        match switch {
            "on" => {
                ORIG_TERM = Some(term);
                let mut raw = term;
                cfmakeraw(&mut raw);
                tcsetattr(fd, TCSANOW, &raw);
            }
            _ => {
                if let Some(orig) = ORIG_TERM {
                    tcsetattr(fd, TCSANOW, &orig);
                }
            }
        };
    }
}

fn read_char() -> u8 {
    let mut buf = [0u8; 1];
    std::io::stdin().read_exact(&mut buf).unwrap();
    buf[0]
}



// sample_rate
// (mainly used by TempoState and TempoGroup)
//
mod sample_rate {
    use super::*;

    pub static SAMPLE_RATE: AtomicU32 = AtomicU32::new(0);

    pub fn set(sample_rate: u32) {
        SAMPLE_RATE.store(sample_rate, Ordering::Relaxed);
    }

    pub fn get() -> u32 {
        SAMPLE_RATE.load(Ordering::Relaxed)
    }
}

mod gart_time {
    use super::*;

    // global clock
    pub mod clock {
        use super::*;

        pub static SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

        pub fn advance(n: u64) {
            SAMPLE_COUNTER.fetch_add(n, Ordering::Relaxed);
        }

        pub fn current() -> u64 {
            SAMPLE_COUNTER.load(Ordering::Relaxed)
        }
    }
    // tempo control
    // 
    // processes that rely on temporal parameters
    // can be assigned to a TempoGroup to synchronize with others
    // or to a TempoSolo to be in their own little time world
    //
    // a TempoGroups is created by a special command (TBD);
    // a TempoSolo is created along with the Process that requires it
    //
    // a TempoGroup has a name that can be assigned to a Process
    //
    // all TempoStates are updated by the Conductor
    //
    // interval is stored as samples, but converted from
    // samples, milliseconds, or BPM, depending on initialization
    //
    pub struct TempoState {
        pub mode: TempoMode,
        pub unit: TempoUnit,
        pub interval: f32,
        pub active: AtomicBool,
        pub current: AtomicU32,
    }

    pub enum TempoMode {
        Solo,
        Group,
    }

    pub enum TempoUnit {
        Samples,
        Millis,
        Bpm,
    }

    impl TempoState {
        pub fn new() -> Self {
            Self {
                mode: TempoMode::Solo,
                unit: TempoUnit::Samples,
                interval: sample_rate::get() as f32,
                active: AtomicBool::new(false),
                current: AtomicU32::new(0),
            }
        }

        pub fn init(&mut self, mode: TempoMode, unit: TempoUnit, interval: f32) {
            let interval_in_samps = convert_interval(&unit, interval);
            self.mode = mode;
            self.unit = unit; 
            self.interval = interval_in_samps;
        }

        // store current as AtomicU32, preserving three degrees
        // of float data by multiplying by 1000.0
        pub fn update(&mut self, delta_in_samples: f64) {
            let step_f = delta_in_samples as f32 / self.interval;
            let step_u = (step_f * 1000.0) as u32;
            self.current.fetch_add(step_u, Ordering::Relaxed);
        }

        // return current as f32, restoring three degrees
        // of float precision by dividing by 1000.0
        pub fn current(&self) -> f32 {
            let step_u = self.current.load(Ordering::Relaxed);
            let step_f = step_u as f32 / 1000.0;
            step_f
        }

        pub fn reset(&mut self) {
            self.current.store(0, Ordering::Relaxed);
        }

        pub fn set_interval(&mut self, new_interval: f32) {
            let new_interval_in_samps = convert_interval(&self.unit, new_interval);
            self.interval = new_interval_in_samps;
        }
    }

    fn convert_interval(unit: &TempoUnit, interval: f32) -> f32 {
        let frac = match unit {
            TempoUnit::Samples => return interval,
            TempoUnit::Millis => interval / 1000.0,
            TempoUnit::Bpm => 60.0 / interval,
        };
        
        let interval_in_samples = sample_rate::get() as f32 * frac;
       
        interval_in_samples
    }
}

use gart_time::{clock, TempoState, TempoMode, TempoUnit};

// audio engine
//
struct Conductor {
    voices: HashMap<String, Voice>,
    out_channels: usize,
    tracks: HashMap<String, AudioFile>,
    tempo_groups: HashMap<String, Arc<Mutex<TempoState>>>,
    tempo_solos: Vec<Arc<Mutex<TempoState>>>,
}

impl Conductor {
    fn prepare(out_channels: usize, tracks: HashMap<String, AudioFile>) -> Self {
        Self { 
            voices: HashMap::<String, Voice>::new(), 
            out_channels, 
            tracks,
            tempo_groups: HashMap::<String, Arc<Mutex<TempoState>>>::new(),
            tempo_solos: Vec::<Arc<Mutex<TempoState>>>::new(),
        }
    }

    fn coordinate(&mut self, areas_ptr: *const snd_pcm_channel_area_t, offset: snd_pcm_uframes_t, frames: snd_pcm_uframes_t) {
        unsafe {
            let areas = std::slice::from_raw_parts(areas_ptr, self.out_channels);

            for f in 0..frames {
                for ch in 0..self.out_channels {
                    let a = &areas[ch];
                    let base = a.addr as *mut u8;

                    // ALSA channel area addressing
                    let bit_offset = a.first as isize + (offset + f) as isize * a.step as isize;
                    let byte_offset = bit_offset / 8;

                    let sample_ptr = base.offset(byte_offset) as *mut i16;
            
                    unsafe {
                        *sample_ptr = 0;
                    }

                    for (_, voice) in &mut self.voices {
                        voice.process(sample_ptr, f, ch);
                    }
                }

                clock::advance(1);
            }
        }
    }

    fn load_voice(&mut self, name: &str) {
        match self.tracks.get(&name.to_string()) {
            Some(track) => {
                match self.voices.entry(name.to_string()) {
                    Entry::Vacant(e) => { e.insert(Voice::new(track)); }
                    Entry::Occupied(_) => {
                        println!("\nErr: a Voice called {name} already exists");
                        return;
                    }
                }
            }
            None => println!("\nErr: Could not find track '{name}'"),
        }
    }

    fn start_voice(&mut self, name: &str) {
        let name = name.to_string();
        match self.voices.get_mut(&name) {
            Some(voice) => {
                let state = &mut voice.state;
                state.active = true;
                if state.position > voice.end as f32 {
                    state.position = 0.0;
                } else if state.position < 0.0 {
                    state.position = voice.end as f32;
                }
            }
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    fn pause_voice(&mut self, name: &str) {
        let name = name.to_string();
        match self.voices.get_mut(&name) {
            Some(voice) => voice.state.active = false,
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    /* TODO: turn loop into a Process
    fn loop_voice(&mut self, name: &str) {
        for voice in &mut self.voices {
            if voice.name == name {
                voice._loop = true;
                return;
            }
        }
        println!("\nErr: Could not find voice '{name}'");
    }
    */

    fn stop_voice(&mut self, name: &str) {
        let name = name.to_string();
        match self.voices.get_mut(&name) {
            Some(voice) => {
                let state = &mut voice.state;
                state.active = false;
                state.position = match state.velocity >= 0.0 {
                    true => 0.0,
                    false => voice.end as f32,
                };
            }
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    fn unload_voice(&mut self, name: &str) {
        let name = name.to_string();
        match self.voices.entry(name) {
            Entry::Vacant(_) => {
                println!("\nErr: Could not find voice");
                return;
            }
            Entry::Occupied(e) => { e.remove(); }
        }
    }

    fn set_velocity(&mut self, args: &str) {
        let mut args = args.splitn(2, ' ');
        let name = match args.next() {
            Some(string) => string,
            None => {
                println!("\nErr: not enough arguments for velocity");
                return;
            }
        };
        let name = name.to_string();
        
        let voice = match self.voices.get_mut(&name) {
            Some(v) => v,
            None => {
                println!("\nErr: Could not find voice '{name}'");
                return;
            }
        };

        let velocity = match args.next() {
            Some(num) => {
                match num.parse::<f32>() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("\nErr: {num} is not a valid argument for velocity");
                        return;
                    }
                }
            }
            None => {
                println!("\nErr: not enough arguments for velocity");
                return;
            }
        };

        match args.next() {
            Some(extra) => {
                println!("\nErr: too many args for velocity");
                return;
            }
            None => voice.state.velocity = velocity,
        }        
    }

    fn seq(&mut self, args: &str) {
        let mut args = args.split_whitespace();
        let name = match args.next() {
            Some(string) => string,
            None => {
                println!("\nErr: not enough arguments for velocity");
                return;
            }
        };
        let name = name.to_string();

        let voice = match self.voices.get_mut(&name) {
            Some(v) => v,
            None => {
                println!("\nErr: Could not find voice '{name}'");
                return;
            }
        };

        let mut period: usize = sample_rate::get() as usize;
        let mut tempo: Arc<Mutex<TempoState>> = Arc::new(Mutex::new(TempoState::new()));
        let mut steps: Vec<f32> = Vec::new();
        let mut chance: Vec<f32> = Vec::new();
        let mut jit: Vec<f32> = Vec::new();
        
        while let Some(arg) = args.next() {
            match arg {
                "-t" | "--tempo" => {
                    let t_arg = match args.next() {
                        Some(arg) => arg,
                        None => {
                            println!("\nErr: not enough arguments for seq");
                            return;
                        }
                    };

                    let u = t_arg.chars().next().unwrap();

                    if u == 'g' {
                        let tg_name = String::from(&t_arg[1..]);
                        tempo = match self.tempo_groups.get(&tg_name) {
                            Some(group) => {
                                Arc::clone(group);
                                continue;
                            }
                            None => {
                                println!("\nErr: no TempoGroup with the provided name");
                                return;
                            }
                        };
                    }
                    
                    let mut t = tempo.lock().unwrap();

                    t.interval = match &t_arg[1..].parse::<f32>() {
                        Ok(val) => *val,
                        Err(_) => {
                            println!("\nErr: invalid tempo interval");
                            return;
                        }
                    };

                    t.unit = match u {
                        's' => TempoUnit::Samples,
                        'm' => TempoUnit::Millis,
                        'b' => TempoUnit::Bpm,
                        _ => {
                            println!("\nErr: unrecognized time unit for tempo");
                            return;
                        }
                    };

                    drop(t);

                    self.tempo_solos.push(Arc::clone(&tempo));
                }
                "-p" | "--period" => {
                    period = match args.next() {
                        Some(arg) => match arg.parse::<f32>() {
                            Ok(val) => val as usize,
                            Err(_) => {
                                println!("\nErr: invalid argument for period");
                                return;
                            }
                        }
                        None => {
                            println!("\nErr: not enough arguments for seq");
                            return;
                        }
                    };
                }
                "-s" | "--steps" => {
                    // need to figure out how to parse numbers
                    // until next char
                    while let Some(val) = args.next() {
                        match val.parse::<f32>() {
                            Ok(valid) => steps.push(valid),
                            Err(_) => {
                                println!("\nErr: invalid step argument");
                                return;
                            }
                        }
                    }
                }
                "-c" | "--chance" => {
                    // a value specifies chance for the step
                    //+ at the same index as the value
                    // _ is shorthand for 100
                    // n,val specifies chance=val for step=n
                    // a,val sets the same chance=val for all steps
                    // n1-n2,val specifies a chance=val for
                    //+ n1-n2 contiguous steps
                }
                "-j" | "--jitter" => {
                    // a value specifies jitter for the step
                    //+ at the same index as the value
                    // _ means no jitter
                    // e|l indicates jitter before=e and after=l the beat
                    //+ (of ranges e-0.0 and 0.0-l)
                    // e1-e2|l1-l2 indicate jitter ranges
                    // n,e|l specifies jitter=e|l for step=n
                    // a,e|l specifies jitter=e|l for all steps
                    // n1-n2,e1-2|l1-l2 specifies jitter ranges for
                    //+ n1-n2 contiguous steps
                }
                _ => break,
            }
        } 

        let state = SeqState {
            active: false,
            period,
            tempo,
            steps,
            chance,
            jit,
        };

        voice.processes.insert("seq".to_string(), 
            Arc::new(Mutex::new(Seq { state })));
    }
}

struct VoiceState {
    active: bool,
    position: f32,
    velocity: f32,
    gain: f32,
}

struct Voice {
    samples: Arc<Vec<i16>>,
    end: usize,
    sample_rate: u32,
    channels: usize,
    state: VoiceState,  
    processes: HashMap<String, Arc<Mutex<dyn Process>>>,
}

impl Voice {
    fn new(af: &AudioFile) -> Self {
        let end = af.samples.len() / af.num_channels as usize - 1;
        let state = VoiceState {
            active: false,
            position: 0.0,
            velocity: 1.0,
            gain: 1.0,
        };

        Self {
            samples: Arc::new(af.samples.clone()),
            end,
            sample_rate: af.sample_rate, 
            channels: af.num_channels as usize, 
            state,
            processes: HashMap::<String, Arc<Mutex<dyn Process>>>::new(),
        }
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active { return; }

        let idx = self.state.position as usize;
        if idx >= self.end || idx < 0 {
            return;
        }

        // if there are more output channels than the track has
        // recorded into, then skip putting info into the extra
        // channels, unless the track is mono and there are two 
        // output channels, in which case, output the same samples 
        // through both channels
        //
        // this is a hack; def need a better routing system later
        if self.channels == 1 {
            if ch < 2 {
                ch = 0;
            } else {
                return;
            }
        } else if ch >= self.channels {
            return;
        }

        let state = &mut self.state;

        // processing
        for (_, p) in &self.processes {
            let mut proc = p.lock().unwrap();
            proc.process(state);
        }

        // linear interpolation
        let frac = state.position.fract();
        let s0 = self.samples[(idx * self.channels) + (ch % self.channels)] as f32;
        let s1 = self.samples[((idx + 1) * self.channels) + (ch % self.channels)] as f32;
        let sample = s0 * (1.0 - frac) + s1 * frac;

        unsafe {
            *acc += (sample * state.gain) as i16;
        }

        // advance
        if ch == self.channels - 1 {
            state.position += state.velocity;
        }
    }
}

// Processes 
//
trait Process: Send {
    fn process(&mut self, voice: &mut VoiceState);
}

struct Seq {
    state: SeqState,
}

struct SeqState {
    active: bool,
    period: usize,
    tempo: Arc<Mutex<TempoState>>,
    steps: Vec<f32>,
    chance: Vec<f32>,
    jit: Vec<f32>,
}

impl Process for Seq {
    fn process(&mut self, voice: &mut VoiceState) {
        return;
    }
}
