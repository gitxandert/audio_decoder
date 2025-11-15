use std::{
    io::{self, Write},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread,
    time::Duration,
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
    let pcm = PCM::new("default", Direction::Playback, false).unwrap();

    // Set hardware parameters
    let hwp = HwParams::any(&pcm).unwrap();
    hwp.set_channels(af.num_channels).unwrap();
    hwp.set_rate(af.sample_rate, ValueOr::Nearest).unwrap();
    hwp.set_format(Format::S16LE).unwrap();
    hwp.set_access(Access::RWInterleaved).unwrap();
    pcm.hw_params(&hwp).unwrap();
    let io = pcm.io_i16().unwrap();
    
    let period_size = hwp.get_period_size().unwrap() as usize;
    let num_channels = hwp.get_channels().unwrap() as usize;

    let mut tracks: Vec<AudioFile> = Vec::new();
    println!("Adding {} to tracks", af.file_name);
    tracks.push(af);
    let conductor = Arc::new(Mutex::new(Conductor::prepare(num_channels, period_size, tracks)));
    let cond_for_repl = Arc::clone(&conductor);

    let running = Arc::new(AtomicBool::new(true));
    let running_for_repl = running.clone();

    println!("");
    thread::spawn(move || {
        loop {
            print!("> ");
            io::stdout().flush().unwrap();
            let mut cmd = String::new();
            io::stdin()
                .read_line(&mut cmd)
                .expect("Failed to read command");
            let cmd = cmd.trim();
            let mut parts = cmd.splitn(2, ' ');
            let cmd = parts.next().unwrap();
            let args = parts.next().unwrap_or_else(|| "");

            let mut con = cond_for_repl.lock().unwrap();
            match cmd {
                "start" => con.add_voice(args),
                "pause" => con.pause_voice(args),
                "resume" => con.resume_voice(args),
                "stop" => con.stop_voice(args),
                "q" | "quit" => {
                    running_for_repl.store(false, Ordering::SeqCst);
                    break;
                }
                _ => println!("Unknown command '{cmd}'"),
            }
        }
    });

    if pcm.state() != State::Running {
        pcm.start().unwrap();
    }
    let mut out_buf = vec![0i16; period_size * num_channels];
    while running.load(Ordering::SeqCst) {
        let mut con = conductor.lock().unwrap();
        con.coordinate(&mut out_buf);
        drop(con);
    
        // for some reason, this only plays when I stop the
        // track that is being written to the buffer; the track
        // also plays back twice as fast
        io.writei(&out_buf).unwrap();
    }

    pcm.drop().unwrap();
    pcm.drain().unwrap();
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

    fn pause_voice(&self, name: &str) {
        for voice in &self.voices {
            if voice.name == name {
                voice.active == false;
                return;
            }
        }
        eprintln!("Err: Could not find voice '{name}'");
    }

    fn resume_voice(&self, name: &str) {
        for voice in &self.voices {
            if voice.name == name {
                voice.active == true;
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
    speed: f32,  
    direction: f32,
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
            speed: 1.0,
            direction: 1.0, 
            gain: 1.0, 
            active: true,
        }
    }

    fn process(&mut self, acc: &mut f32, frame: usize, mut ch: usize) {
        if !self.active { return; }

        let idx = self.position as usize;
        if idx + 1 >= self.samples.len() / self.channels {
            self.active = false;
            return;
        }

        // if there are more output channels than the track has
        // recorded into, then skip putting info into the extra
        // channels, unless the track is mono and there are two 
        // output channels, in which case, output the same samples 
        // through both channels
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
        self.position += self.direction * self.speed;
    }
}


