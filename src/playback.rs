use std::os::unix::io::AsRawFd;
use libc::{termios, tcgetattr, tcsetattr, cfmakeraw, TCSANOW};
use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};
use alsa::{
    Direction, ValueOr,
    pcm::{PCM, HwParams, Format, Access, State},
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
    // Open default playback device
    let pcm = PCM::new("hw:0,0", Direction::Playback, false).unwrap();

    // Set hardware parameters
    let hwp = HwParams::any(&pcm).unwrap();
    hwp.set_channels(af.num_channels).unwrap();
    hwp.set_rate(af.sample_rate, ValueOr::Nearest).unwrap();
    hwp.set_format(Format::S16LE).unwrap();
    hwp.set_access(Access::RWInterleaved).unwrap();
    pcm.hw_params(&hwp).unwrap();
    let io = pcm.io_i16().unwrap();
    
    let period_size: usize = 1024;
    let buffer_size = 1024 * 16;
    println!("Min period_size: {:?} Min buffer_size: {:?}", hwp.get_period_size_min().unwrap(), hwp.get_buffer_size_min().unwrap());
    println!("Accepted rate: {:?}", hwp.get_rate().unwrap());
    hwp.set_period_size(period_size as i64, ValueOr::Nearest).unwrap();
    hwp.set_buffer_size(buffer_size).unwrap();

    let swp = pcm.sw_params_current().unwrap();
    swp.set_start_threshold(1).unwrap();
    swp.set_avail_min(period_size as i64).unwrap();
    pcm.sw_params(&swp).unwrap();

    let num_channels = hwp.get_channels().unwrap() as usize;

    let mut tracks: Vec<AudioFile> = Vec::new();
    println!("Adding {} to tracks", af.file_name);
    tracks.push(af);
    let conductor = Arc::new(Mutex::new(Conductor::prepare(num_channels, period_size, tracks)));
    let cond_for_repl = Arc::clone(&conductor);

    let running = Arc::new(AtomicBool::new(true));
    let running_for_repl = running.clone();

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
                    if now.elapsed().as_millis() % 100 == 0 {
                        let mut m = marker.lock().unwrap();
                        *m = (*m + 1) % repl_chars.len();
                    }
                }

                {
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

    println!("");
    {
        let buffer = buffer.clone();
        thread::spawn(move || {
            loop {
                let c = read_char();
               
                match c {
                    b'\n' | b'\r' => {
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
                                running_for_repl.store(false, Ordering::SeqCst);
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

    let mut out_buf = vec![0i16; period_size * num_channels];
    while running.load(Ordering::SeqCst) {
        let mut con = conductor.lock().unwrap();
        con.coordinate(&mut out_buf);
        drop(con);
    
        io.writei(&out_buf).unwrap();
    }

    raw_mode("off");
    print!("\n");
    pcm.drop().unwrap();
    pcm.drain().unwrap();  
}

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


