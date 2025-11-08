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

#[derive(Clone, Copy, PartialEq)]
enum PlayState { Stopped, Running, Paused, Quit }

pub fn play_file(af: AudioFile) {
    /*
     * struct AudioFile {
     *     format: String,      // file type (WAV, AIFF)
     *     sample_rate: u32,
     *     num_channels: u32,
     *     bits_per_sample: u32,
     *     samples: Vec<u8>,
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
    let chunk_size = period_size * af.num_channels as usize;
 
    /*
    not sure yet if the following is necessary
    // Make sure we don't start the stream too early
    let hwp = pcm.hw_params_current().unwrap();
    let swp = pcm.sw_params_current().unwrap();
    swp.set_start_threshold(100).unwrap();
    pcm.sw_params(&swp).unwrap();
    */

    let state = Arc::new(Mutex::new(PlayState::Stopped));
    let state_for_repl = Arc::clone(&state);

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
            let mut s = state_for_repl.lock().unwrap();
            match cmd {
                "start" => *s = PlayState::Running,
                "pause" => *s = PlayState::Paused,
                "stop" => *s = PlayState::Stopped,
                "q" | "quit" => {
                    *s = PlayState::Quit;
                    break;
                }
                _ => println!("Unknown command '{cmd}'"),
            }
        }
    });

    let mut idx = 0;
    loop {
        let s = *state.lock().unwrap();
        match s {
            PlayState::Quit => break,
            PlayState::Stopped => {
                idx = 0;
                pcm.reset();
                thread::sleep(Duration::from_millis(100));
            }
            PlayState::Paused => {
                if pcm.state() == State::Running {
                    pcm.pause(true).ok();
                }
                thread::sleep(Duration::from_millis(100));
            }
            PlayState::Running => {
                if pcm.state() == State::Paused {
                    pcm.pause(false).ok();
                } else if pcm.state() != State::Running {
                    pcm.prepare().unwrap();
                }
                if idx >= af.samples.len() {
                    *state.lock().unwrap() = PlayState::Stopped;
                    continue;
                }
                let end = (idx + chunk_size).min(af.samples.len());
                let chunk = &af.samples[idx..end];
                match io.writei(chunk) {
                    Ok(frames) => idx += frames as usize * af.num_channels as usize,
                    Err(error) => {
                        if error.errno() == 32 { 
                            pcm.prepare().unwrap(); 
                        } else {
                            println!("Error: {:?}", error);
                        }
                    }
                }
            }
        }
    }

    pcm.drop().unwrap();
    pcm.drain().unwrap();
}
