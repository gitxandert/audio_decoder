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
    sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}},
};

use crate::decode_helpers::AudioFile;

pub fn run_gart(tracks: Vec<AudioFile>, sample_rate: u32, num_channels: u32) {
    // initialize audio engine and tracks
    
    let conductor = Arc::new(Mutex::new(Conductor::prepare(num_channels as usize, tracks)));
    let cond_for_repl = Arc::clone(&conductor);

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

        // config hardward
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

// global clock
mod clock {
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
// can be assigned to a TempoGroup or given their own TempoState
//
trait TempoGroup: Send {
    // required for inheritance-like shared fields
    fn state(&self) -> &TempoState;
    fn state_mut(&mut self) -> &mut TempoState;

    // impl defined
    fn update(&mut self, delta_samples: u64);

    // same for all
    fn current(&self) -> f32 {
        self.state().current
    }

    fn change_state(&mut self, current: Option<f32>, interval: Option<f32>) {
        if let Some(cur) = current {
            self.state_mut().current = cur;
        }

        if let Some(int) = interval {
            self.state_mut().interval = int;
        }
    }
}

struct TempoState {
    current: f32,
    interval: f32,
    sample_rate: f32,
}

struct Samples {
    state: TempoState,
}
struct MilliSeconds {
    state: TempoState,
}
struct Bpm {
    state: TempoState,
}

impl TempoGroup for Samples {
    fn state(&self) -> &TempoState { &self.state }
    fn state_mut(&mut self) -> &mut TempoState { &mut self.state }

    fn update(&mut self, delta_samples: u64) {
        self.state_mut().current += delta_samples as f32;
    }
}

impl TempoGroup for MilliSeconds {
    fn state(&self) -> &TempoState { &self.state }
    fn state_mut(&mut self) -> &mut TempoState { &mut self.state }

    fn update(&mut self, delta_samples: u64) {
        self.state_mut().current += (delta_samples as f32) * (1000.0 / self.state().sample_rate);
    }
}

impl TempoGroup for Bpm {
    fn state(&self) -> &TempoState { &self.state }
    fn state_mut(&mut self) -> &mut TempoState { &mut self.state }

    fn update(&mut self, delta_samples: u64) {
        let sec = delta_samples as f32 / self.state().sample_rate;
        self.state_mut().current += sec * (self.state().interval / 60.0);
    }
}

// audio engine
//
struct Conductor {
    voices: Vec<Voice>,
    out_channels: usize,
    tracks: Vec<AudioFile>,
    tempo_groups: Vec<Arc<Mutex<dyn TempoGroup + Send>>>,
}

impl Conductor {
    fn prepare(out_channels: usize, tracks: Vec<AudioFile>) -> Self {
        Self { 
            voices: Vec::<Voice>::new(), 
            out_channels, 
            tracks,
            tempo_groups: Vec::<Arc<Mutex<dyn TempoGroup + Send>>>::new(),
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

                    for voice in &mut self.voices {
                        voice.process(sample_ptr, f, ch);
                    }
                }

                clock::advance(1);
            }
        }
    }

    fn load_voice(&mut self, name: &str) {
        for track in &self.tracks {
            if track.file_name.as_str() == name {
                self.voices.push(Voice::new(&track));
                return;
            }
        }
        println!("\nErr: Could not find track '{name}'");
    }

    fn start_voice(&mut self, name: &str) {
        for voice in &mut self.voices {
            if voice.name == name {
                let state = &mut voice.state;
                state.active = true;
                if state.position > voice.end as f32 {
                    state.position = 0.0;
                } else if state.position < 0.0 {
                    state.position = voice.end as f32;
                }
                return;
            }
        }
        println!("\nErr: Could not find voice '{name}'");
    }

    fn pause_voice(&mut self, name: &str) {
        for voice in &mut self.voices {
            if voice.name == name {
                voice.state.active = false;
                return;
            }
        }
        println!("\nErr: Could not find voice '{name}'");
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
        for voice in &mut self.voices {
            if voice.name == name {
                let state = &mut voice.state;
                state.active = false;
                state.position = match state.velocity >= 0.0 {
                    true => 0.0,
                    false => voice.end as f32,
                };
                return;
            }
        }
        println!("\nErr: Could not find voice '{name}'");
    }

    fn unload_voice(&mut self, name: &str) {
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].name == name {
                self.voices.remove(i);
                return;
            }
            i += 1;
        }
        println!("\nErr: Could not find voice '{name}'");
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
        let velocity = match args.next() {
            Some(num) => {
                match num.parse::<f32>() {
                    Some(val) => val,
                    None {
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
            None => (),
        }        
        for voice in &mut self.voices {
            if voice.name == name {
                voice.state.velocity = velocity;
                return;
            }
        }
        println!("\nErr: Could not find voice '{name}'");
    }
}

struct VoiceState {
    active: bool,
    position: f32,
    velocity: f32,
    gain: f32,
}

struct Voice {
    name: String,
    samples: Arc<Vec<i16>>,
    end: usize,
    sample_rate: u32,
    channels: usize,
    state: VoiceState,  
    processes: Vec<Arc<Mutex<dyn Process + Send>>>,
}

impl Voice {
    fn new(af: &AudioFile) -> Self {
        let end = af.samples.len() / af.num_channels as usize;
        let state = VoiceState {
            active: false,
            position: 0.0,
            velocity: 1.0,
            gain: 1.0,
        };

        Self {
            name: af.file_name.clone(),
            samples: Arc::new(af.samples.clone()),
            end,
            sample_rate: af.sample_rate, 
            channels: af.num_channels as usize, 
            state,
            processes: Vec::<Arc<Mutex<dyn Process + Send>>>::new(),
        }
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active { return; }

        let idx = self.state.position as usize;
        if idx > self.end || idx < 0 {
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
        for p in &self.processes {
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
// processes have to know exactly what to process;
// they can't just be sent a Voice because they're called from
// inside of Voice's own functions
//
// I need to restructure the Voice's process() function
// to more adequately accommodate this pipeline
//
trait Process: Send {
    fn process(&mut self, voice: &mut VoiceState);
}

struct Seq {
    len: usize,
    beatmask: u64, // could make larger eventually; meh
    offset: isize, // can be negative for indexing from end of len
    rand: f32,
}

impl Process for Seq {
    /*
    fn new(args: &str) -> Self {
       let mut args = args.split_whitespace();
       let mut len: usize = 0;
       let mut beatmask: u64 = 0;
       let mut offset: isize = 0;

       let first: usize = match args.next() {
           Some(val) => match val {
               "%" => 0,
                _ => val.parse::<usize>().unwrap(),
           }
           None => {
               println!("\nErr: not enough args for 'seq'");
               return;
           } 
       };

       match first {
           "" => {
               beatmask = 
               len = args.next();
       while let Some(args) = args.next() {
    */

    fn process(&mut self, voice: &mut VoiceState) {
        return;
    }
}
