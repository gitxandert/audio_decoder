use std::collections::{HashMap, hash_map::Entry};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering}
};

use alsa_sys::*;

use crate::file_parsing::decode_helpers::{
    DecodeResult, DecodeError, AudioFile,
};
use crate::audio_processing::{
    commands::{
        CmdArg, Command
    },
    processes::{
        Process, Seq, SeqState
    },
    gart_time::{
        sample_rate,
        gart_time::{
            clock, TempoMode, TempoUnit, TempoState
        }
    },
};

// audio engine
//
pub struct Conductor {
    voices: HashMap<String, Voice>,
    out_channels: usize,
    tracks: HashMap<String, AudioFile>,
    tempo_groups: HashMap<String, Arc<Mutex<TempoState>>>,
}

impl Conductor {
    pub fn prepare(out_channels: usize, tracks: HashMap<String, AudioFile>) -> Self {
        Self { 
            voices: HashMap::<String, Voice>::new(), 
            out_channels, 
            tracks,
            tempo_groups: HashMap::<String, Arc<Mutex<TempoState>>>::new(),
        }
    }

    pub fn apply(&mut self, command: Command) {
        let (cmd, args) = command.unwrap();
        match cmd {
            CmdArg::Load => self.load_voice(args),
            CmdArg::Start => self.start_voice(args),
            CmdArg::Pause => self.pause_voice(args),
            CmdArg::Resume => self.resume_voice(args),
            CmdArg::Stop => self.stop_voice(args),
            CmdArg::Unload => self.unload_voice(args),
            CmdArg::Velocity => self.velocity(args),
            CmdArg::Seq => self.seq(args),
            CmdArg::Quit => {
                unsafe {
                    libc::raise(libc::SIGTERM);
                }
            }
        }
    }

    pub fn coordinate(&mut self, areas_ptr: *const snd_pcm_channel_area_t, offset: snd_pcm_uframes_t, frames: snd_pcm_uframes_t) {
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

                    for (_, tempo_group) in &self.tempo_groups {
                        let mut tg = tempo_group.lock().unwrap();
                        if tg.active.load(Ordering::Relaxed) {
                            tg.update(1.0);
                        }
                    }
                    
                    for (_, voice) in &mut self.voices {
                        voice.process(sample_ptr, f, ch);
                    }
                }

                clock::advance(1);
            }
        }
    }

    pub fn load_voice(&mut self, name: String) {
        match self.tracks.get(&name) {
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

    pub fn start_voice(&mut self, name: String) {
        match self.voices.get_mut(&name) {
            Some(voice) => {
                let state = &mut voice.state;
                state.active.store(true, Ordering::Relaxed);
                
                for tempo_solo in &voice.tempo_solos {
                    let mut ts = tempo_solo.lock().unwrap();
                    ts.active.store(true, Ordering::Relaxed);
                    ts.reset();
                }

                state.position = match state.velocity >= 0.0 {
                    true => 0.0,
                    false => state.end as f32,
                };
            }
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    pub fn pause_voice(&mut self, name: String) {
        match self.voices.get_mut(&name) {
            Some(voice) => {
                voice.state.active.store(false, Ordering::Relaxed);;
                for tempo_solo in &voice.tempo_solos {
                    let ts = tempo_solo.lock().unwrap();
                    ts.active.store(false, Ordering::Relaxed);
                }
            }
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    pub fn resume_voice(&mut self, name: String) {
        match self.voices.get_mut(&name) {
            Some(voice) => {
                let state = &mut voice.state;
                state.active.store(true, Ordering::Relaxed);
                for tempo_solo in &voice.tempo_solos {
                    let ts = tempo_solo.lock().unwrap();
                    ts.active.store(true, Ordering::Relaxed);
                }
            }
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

    pub fn stop_voice(&mut self, name: String) {
        match self.voices.get_mut(&name) {
            Some(voice) => {
                let state = &mut voice.state;
                state.active.store(false, Ordering::Relaxed);

                for (_, proc) in &voice.processes {
                    let mut p = proc.lock().unwrap();
                    p.reset();
                }

                for tempo_solo in &voice.tempo_solos {
                    let ts = tempo_solo.lock().unwrap();
                    ts.active.store(false, Ordering::Relaxed);
                }

                state.position = match state.velocity >= 0.0 {
                    true => 0.0,
                    false => state.end as f32,
                };
            }
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    pub fn unload_voice(&mut self, name: String) {
        match self.voices.entry(name) {
            Entry::Vacant(_) => {
                println!("\nErr: Could not find voice");
                return;
            }
            Entry::Occupied(e) => { e.remove(); }
        }
    }

    pub fn velocity(&mut self, args: String) {
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

    pub fn seq(&mut self, args: String) {
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

                    let unit = match u {
                        's' => TempoUnit::Samples,
                        'm' => TempoUnit::Millis,
                        'b' => TempoUnit::Bpm,
                        _ => {
                            println!("\nErr: unrecognized time unit for tempo");
                            return;
                        }
                    };

                    let interval = match &t_arg[1..].parse::<f32>() {
                        Ok(val) => *val,
                        Err(_) => {
                            println!("\nErr: invalid tempo interval");
                            return;
                        }
                    };

                    t.init(TempoMode::Solo, unit, interval);

                    drop(t);

                    voice.tempo_solos.push(Arc::clone(&tempo));
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
                    while let Some(val) = &args.clone().peekable().peek() {
                        match val.parse::<f32>() {
                            Ok(valid) => {
                                let num = args.next().unwrap();
                                let num = num.parse::<f32>().unwrap();
                                steps.push(num);
                            }
                            Err(_) => {
                                continue;
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
            active: AtomicBool::new(true),
            period,
            tempo,
            steps,
            chance,
            jit,
            seq_idx: 0,
        };

        voice.processes.insert(
            "seq".to_string(), 
            Arc::new(Mutex::new(Seq { state }))
        );
    }
}

pub struct VoiceState {
    pub active: AtomicBool,
    pub position: f32,
    pub end: usize,
    pub velocity: f32,
    pub gain: f32,
}

pub struct Voice {
    samples: Arc<Vec<i16>>,
    sample_rate: u32,
    channels: usize,
    pub state: VoiceState,  
    processes: HashMap<String, Arc<Mutex<dyn Process>>>,
    tempo_solos: Vec<Arc<Mutex<TempoState>>>,
}

impl Voice {
    fn new(af: &AudioFile) -> Self {
        let end = af.samples.len() / af.num_channels as usize - 1;
        let state = VoiceState {
            active: AtomicBool::new(false),
            position: 0.0,
            end,
            velocity: 1.0,
            gain: 1.0,
        };

        Self {
            samples: Arc::new(af.samples.clone()),
            sample_rate: af.sample_rate, 
            channels: af.num_channels as usize, 
            state,
            processes: HashMap::<String, Arc<Mutex<dyn Process>>>::new(),
            tempo_solos: Vec::<Arc<Mutex<TempoState>>>::new(),
        }
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active.load(Ordering::Relaxed) { return; }

        let state = &mut self.state;

        // processing
        for (_, p) in &self.processes {
            let mut proc = p.lock().unwrap();
            proc.process(state);
        }

        for tempo_solo in &self.tempo_solos {
            let mut ts = tempo_solo.lock().unwrap();
            ts.update(1.0);
        }

        let idx = state.position as usize;
        if idx >= state.end || idx < 0 {
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
