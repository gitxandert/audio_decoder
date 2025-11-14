use std::os::unix::io::AsRawFd;
use libc::{termios, tcgetattr, tcsetattr, cfmakeraw, TCSANOW};
use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
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

    let marker = Arc::new(Mutex::new(0usize));
    let buffer = Arc::new(Mutex::new(String::new()));
    let repl_chars = ['^', 'X', 'v', '>', 'X', '<', 'Z'];

    {
        let marker = marker.clone();
        let buffer = buffer.clone();
        thread::spawn(move || { 
            loop {
                {
                    let mut m = marker.lock().unwrap();
                    *m = (*m + 1) % repl_chars.len();
                }

                {
                    let m = *marker.lock().unwrap();
                    let input_text = buffer.lock().unwrap().clone();
                    print!("\r{} {}", repl_chars[m], input_text);
                    std::io::stdout().flush().unwrap();
                }

                thread::sleep(Duration::from_millis(100));
            }
        });
    }
    
    let state = Arc::new(Mutex::new(PlayState::Stopped));
    let state_for_repl = Arc::clone(&state);

    println!("");

    raw_mode("on");

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

                        let mut s = state_for_repl.lock().unwrap();
                        match buf.as_str() {
                            "start" => *s = PlayState::Running,
                            "pause" => *s = PlayState::Paused,
                            "stop" => *s = PlayState::Stopped,
                            "q" | "quit" => {
                                *s = PlayState::Quit;
                                break;
                            }
                            _ => println!("Unknown command '{}'", *buf),
                        }
                        buf.clear();
                    }
                    127 => {
                        // backspace
                        let mut buf = buffer.lock().unwrap();
                        buf.pop();
                    }
                    _ => {
                        let mut buf = buffer.lock().unwrap();
                        buf.push(c as char);
                    }
                }
            }
        });
    }
    
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
    raw_mode("off");
    print!("\n");

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
