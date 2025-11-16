use alsa_sys::*;
use std::os::unix::io::AsRawFd;
use libc::{
    self, 
    c_int, EAGAIN,
    termios, tcgetattr, tcsetattr, cfmakeraw, TCSANOW};
use std::{
    ptr,
    ffi::CString,
    io::{self, Read, Write},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};

use crate::decode_helpers::AudioFile;

pub fn play_file(af: AudioFile) {
    /*
     * struct AudioFile {
     *     file_name: String,
     *     format: String,      // file type (WAV, AIFF)
     *     sample_rate: u32,
     *     num_channels: u32,
     *     bits_per_sample: u32,
     *     samples: Vec<i16>,
     * }
    */
    
    // initialize audio engine and tracks
    let sample_rate = af.sample_rate;
    let num_channels = af.num_channels;
    
    let mut tracks: Vec<AudioFile> = Vec::new();
    println!("Adding {} to tracks", af.file_name);
    tracks.push(af);
    let conductor = Arc::new(Mutex::new(Conductor::prepare(num_channels, period_size, tracks)));
    let cond_for_repl = Arc::clone(&conductor);

    // take over STDIN
    let marker = Arc::new(Mutex::new(0usize));
    let buffer = Arc::new(Mutex::new(String::new()));
    let repl_chars = ['^', 'X', 'v', '>', 'X', '<', 'Z'];

    {
        let marker = marker.clone();
        let buffer = buffer.clone();
        thread::spawn(move || {
            let mut last_len = 0;
            let now = Instant::now();
            loop {
                {
                    // every 100 ms, change the REPL marker
                    if now.elapsed().as_millis() % 100 == 0 {
                        let mut m = marker.lock().unwrap();
                        *m = (*m + 1) % repl_chars.len();
                    }
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
                            "start" => con.add_voice(args),
                            "pause" => con.pause_voice(args),
                            "resume" => con.resume_voice(args),
                            "velocity" => con.set_velocity(args),
                            "stop" => con.stop_voice(args),
                            "q" | "quit" => {
                                unsafe {
                                    libc::raise(libc::SIGTERM);
                                }
                                break;
                            }
                            _ => println!("Unknown command '{cmd}'"),
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

        check(
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

        check(
            snd_pcm_hw_params_set_access(handle, hw, SND_PCM_ACCESS_MMAP_INTERLEAVED),
            "set_access",
        );
        check(
            snd_pcm_hw_params_set_format(handle, hw, SND_PCM_FORMAT_S16_LE),
            "set_format",
        );
        check(snd_pcm_hw_params_set_channels(handle, hw, num_channels) "set channels");
        check(snd_pcm_hw_params_set_rate(handle, hw, sample_rate, 0), "set_rate");

        let mut period_size: snd_pcm_uframes_t = 128;
        check(
            snd_pcm_hw_params_set_period_size_near(handle, hw, &mut period_size, 0),
            "set_period_size",
        );

        let mut buffer_size: snd_pcm_uframes_t = period_size * 4;
        check(
            snd_pcm_hw_params_set_buffer_size_near(handle, hw, &mut buffer_size),
            "set_buffer_size",
        );

        check(snd_pcm_hw_params(handle, hw), "snd_pcm_hw_params");
        snd_pcm_hw_params_free(hw);

        // config software params
        let mut sw: *mut snd_pcm_sw_params_t = ptr::null_mut();
        snd_pcm_sw_params_malloc(&mut sw);
        snd_pcm_sw_params_current(handle, sw);

        // start immediately upon write
        check(snd_pcm_sw_params_set_start_threshold(handle, sw, 0), "set_start_threshold");

        // wake when period is available
        check(
            snd_pcm_sw_params_set_avail_min(handle, sw, period_size),
            "set_avail_min",
        );

        check(snd_pcm_sw_params(handle, sw), "snd_pcm_sw_params");
        snd_pcm_sw_params_free(sw);

        // prepare device
        check(snd_pcm_prepare(handle), "snd_pcm_prepare");

        loop {
            if TERM_RECEIVED.load(Ordering::Relaxed) {
                break;
            }

            let mut avail = snd_pcm_avail_update(handle);
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

                let areas = std::slice::from_raw_parts(areas_ptr, 2);

                // write to DMA buffer
                // TODO: update coordinate logic
                let con = conductor.lock().unwrap();
                conductor.coordinate(&mut areas_ptr, &mut offset, &mut frames);

                let committed = snd_pcm_mmap_commit(handle, offset, frames);
                if committed < 0 {
                    snd_pcm_recover(handle, committed, 1);
                    break;
                }

                remaining -= committed as snd_pcm_uframes_t;
            }
        }
    }

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

static TERM_RECEIVED: AtomicBool = AtomicBool::new(false);
//
extern "C" fn handle_sigterm(_sig: libc::c_int) {
    TERM_RECEIVED.store(true, Ordering::Relaxed);
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

    raw_mode("off");
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

// audio engine

struct Conductor {
    voices: Vec<Voice>,
    out_channels: usize,
    period_size: usize,
    tracks: Vec<AudioFile>
}

impl Conductor {
    fn prepare(out_channels: usize, period_size: usize, tracks: Vec<AudioFile>) -> Self {
        Self { 
            voices: Vec::<Voice>::new(), 
            out_channels, 
            period_size,
            tracks
        }
    }

    fn coordinate(&mut self, out_buf: &mut [i16]) {
        for s in out_buf.iter_mut() {
            *s = 0;
        }

        for frame in 0..self.period_size {
            for ch in 0..self.out_channels {
                let out_idx = frame * self.out_channels + ch;
                let mut acc = 0f32;
            
                for voice in &mut self.voices {
                    voice.process(&mut acc, frame, ch);
                }

                out_buf[out_idx] = acc.clamp(-32767.0, 32767.0) as i16;
            }
        }
    }

    fn add_voice(&mut self, name: &str) {
        for track in &self.tracks {
            if track.file_name.as_str() == name {
                self.voices.push(Voice::new(&track));
                return;
            }
        }
        eprintln!("Err: Could not find track '{name}'");
    }

    fn pause_voice(&mut self, name: &str) {
        for voice in &mut self.voices {
            if voice.name == name {
                voice.active = false;
                return;
            }
        }
        eprintln!("Err: Could not find voice '{name}'");
    }

    fn resume_voice(&mut self, name: &str) {
        for voice in &mut self.voices {
            if voice.name == name {
                voice.active = true;
                return;
            }
        }
        eprintln!("Err: Could not find voice '{name}'");
    }

    fn set_velocity(&mut self, args: &str) {
        let mut args = args.splitn(2, ' ');
        let name = match args.next() {
            Some(string) => string,
            None => {
                eprintln!("Err: not enough arguments for velocity");
                return;
            }
        };
        let velocity = match args.next() {
            Some(num) => num.parse::<f32>().unwrap(),
            None => {
                eprintln!("Err: not enough arguments for velocity");
                return;
            }
        };
        match args.next() {
            Some(extra) => {
                eprintln!("Err: too many args for velocity");
                return;
            }
            None => (),
        }        
        for voice in &mut self.voices {
            if voice.name == name {
                voice.velocity = velocity;
                return;
            }
        }
        eprintln!("Err: Could not find voice '{name}'");
    }


    fn stop_voice(&mut self, name: &str) {
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].name == name {
                self.voices.remove(i);
                return;
            }
            i += 1;
        }
        eprintln!("Err: Could not find voice '{name}'");
    }
}

struct Voice {
    name: String,
    samples: Arc<Vec<i16>>,
    sample_rate: u32,
    channels: usize,
    position: f32,
    velocity: f32,  
    gain: f32,
    active: bool,
}

impl Voice {
    fn new(af: &AudioFile) -> Self {
        Self {
            name: af.file_name.clone(),
            samples: Arc::new(af.samples.clone()),
            sample_rate: af.sample_rate, 
            channels: af.num_channels as usize, 
            position: 0.0,
            velocity: 1.0,
            gain: 1.0, 
            active: true,
        }
    }

    fn process(&mut self, acc: &mut f32, frame: usize, mut ch: usize) {
        if !self.active { return; }

        let idx = self.position as usize;
        if idx + 1 >= self.samples.len() / self.channels || self.position < 0.0 {
            self.active = false;
            if self.velocity > 0.0 {
                self.position = 0.0;
            } else {
                self.position = (self.samples.len() as f32 / self.channels as f32) - 2.0;
            }
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

        // linear interpolation
        let frac = self.position.fract();
        let s0 = self.samples[(idx * self.channels) + (ch % self.channels)] as f32;
        let s1 = self.samples[((idx + 1) * self.channels) + (ch % self.channels)] as f32;
        let sample = s0 * (1.0 - frac) + s1 * frac;

        *acc += sample * self.gain;

        // advance
        if ch == self.channels - 1 && self.channels != 1 {
            self.position += self.velocity;
        }
    }
}
