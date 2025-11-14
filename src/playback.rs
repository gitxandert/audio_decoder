use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
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
 
    let conductor = Arc::new(Mutex::new(Conductor::prepare(num_channels, period_size)));
    let cond_for_repl = Arc::clone(&conductor);

    let running = Arc::new(Mutex::new(true));
    let running_for_repl = Arc::clone(&running);

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
            let con = cond_for_repl.lock().unwrap();
            let mut r = running_for_repl.lock().unwrap();
            match cmd {
                "start" => con.add_voice(&af),
                "pause" => con.voices.get(0).active = false,
                "stop" => con.voices.pop(),
                "q" | "quit" => {
                    *r = false;
                    break;
                }
                _ => println!("Unknown command '{cmd}'"),
            }
        }
    });

    let mut out_buf = vec![0i16; period_size * num_channels];

    while running {
        let con = conductor.lock().unwrap();
        con.coordinate(&mut out_buf);
        io.writei(&out_buf).unwrap();
    }

    pcm.drop().unwrap();
    pcm.drain().unwrap();
}

struct Conductor {
    voices: Vec<Voice>,
    out_channels: usize,
    period_size: usize,
}

impl Conductor {
    fn prepare(out_channels: usize, period_size: usize) -> Self {
        let voices: Vec<Voice> = Vec::new();

        Self { voices, out_channels, period_size }
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
}

struct Voice {
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
            samples: Arc::new(af.samples.clone()),
            sample_rate: af.sample_rate, 
            channels: af.num_channels, 
            position: 0.0,
            speed: 1.0,
            direction: 1.0, 
            gain: 1.0, 
            active: false
        }
    }

    fn process(&mut self, acc: &mut f32; frame: usize, ch: usize) {
        if !self.active { continue; }

        let idx = self.position as usize;
        if idx + 1 >= self.samples.len() / self.channels {
            self.active = false;
            continue;
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
                continue;
            }
        } else if ch >= self.channels {
            continue;
        }

        // linear interpolation
        let frac = self.position.fract();
        let s0 = self.samples[(idx * self.channels) + (ch % self.channels)] as f32;
        let s1 = self.samples[((idx + 1) * self.channels) + (ch % self.channels)] as f32;
        let sample = s0 * (1.0 - frac) + s1 * frac;

        *acc += sample * voice.gain;

        // advance
        self.position += self.direction * self.speed;
    }
}


