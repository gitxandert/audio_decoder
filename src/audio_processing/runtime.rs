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
    collections::{HashMap, hash_map::Entry},
    sync::{Arc, Mutex, 
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering}
    },
};

use crate::file_parsing::decode_helpers::AudioFile;
use crate::audio_processing::{
    engine::{Conductor, Voice},
    commands::{CmdArg, Command, CmdQueue},
    gart_time::{gart_time::clock, sample_rate},
};

pub fn run_gart(tracks: HashMap<String, AudioFile>, sample_rate: u32, num_channels: u32) {
    // initialize audio engine and tracks
    
    let mut conductor = Conductor::prepare(num_channels as usize, tracks);

    sample_rate::set(sample_rate);

    // take over STDIN
    let marker = Arc::new(Mutex::new(0usize));
    let buffer = Arc::new(Mutex::new(String::new()));
    let cursor = Arc::new(Mutex::new(0usize));
    let repl_chars = ['^', 'X', 'v', '>', 'X', '<', 'Z'];

    {
        let marker = marker.clone();
        let buffer = buffer.clone();
        let cursor = cursor.clone();
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
 
                    let cur = *cursor.lock().unwrap();
                    let diff = curr_len - cur;
                    print!("\x1b[{}D", diff);
                   
                    std::io::stdout().flush().unwrap();
                }
            }
        });
    }
 
    raw_mode("on");

    let queue = Arc::new(CmdQueue::new(256));
    // REPL
    println!("");
    {
        let buffer = buffer.clone();
        let cursor = cursor.clone();
        let q = queue.clone();

        let prev_cmds = Vec::<String>::new();

        thread::spawn(move || {
            loop {
                let c = read_char();
               
                match c {
                    b'\n' | b'\r' => {
                        // enter
                        print!("\n");
                        let mut buf = buffer.lock().unwrap();

                        let mut cur = cursor.lock().unwrap();
                        *cur = 0;

                        let mut cmd = buf.clone();
                        prev_cmd.push(cmd.clone());

                        let mut parts = cmd.splitn(2, ' ');
                        let cmd = parts.next().unwrap();
                        let args = parts.next().unwrap_or_else(|| "").to_string();

                        if let Some(matched) = q.match_cmd(cmd) {
                            let command = Command::new(matched, args);
                            match q.try_push(command) {
                                Ok(()) => (),
                                Err(error) => {
                                    buf.clear();
                                    println!("\nErr: {error}");
                                }
                            }
                        } else {
                            buf.clear();
                            println!("\nUnknown command '{}'", cmd);
                        }

                        buf.clear();
                    }
                    127 => {
                        // backspace
                        let mut buf = buffer.lock().unwrap();
                        let mut cur = cursor.lock().unwrap();

                        if !buf.is_empty() {
                            if *cur > 0 {
                                buf.remove(*cur - 1);
                                *cur -= 1;
                            }
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
                    27 => {
                        // ESC
                        let c2 = read_char();
                        if c2 == b'[' {
                            let c3 = read_char();
                            match c3 {
                                b'D' => { // left arrow
                                    let mut cur = cursor.lock().unwrap();
                                    if *cur > 0 { *cur -= 1; }
                                }
                                b'C' => { // right arrow
                                    let buf = buffer.lock().unwrap();
                                    let mut cur = cursor.lock().unwrap();
                                    if *cur < buf.len() { *cur += 1; }
                                }
                                b'A' => { // up arrow
                                }
                                b'B' => { // down arrow
                                }
                                _ => (),
                            }
                            continue;
                        }
                    }
                    _ => {
                        let mut buf = buffer.lock().unwrap();
                        let mut cur = cursor.lock().unwrap();
                        buf.push(c as char);
                        *cur += 1;
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

            // apply commands from queue
            while let Some(cmd) = queue.try_pop() {
                conductor.apply(cmd);
            }

            // get remaining frames to write
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
                conductor.coordinate(areas_ptr, offset, frames);

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
        std::process::exit(130);
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
