use std::collections::{HashMap, hash_map::Entry};
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::{
    Arc,
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
    groups: HashMap<String, Group>,
    out_channels: usize,
    tracks: HashMap<String, AudioFile>,
}

impl Conductor {
    pub fn prepare(out_channels: usize, tracks: HashMap<String, AudioFile>) -> Self {
        Self { 
            voices: HashMap::<String, Voice>::new(), 
            groups: HashMap::<String, Group>::new(),
            out_channels, 
            tracks,
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
            CmdArg::Group => self.group(args),
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

                    for (_, voice) in &mut self.voices {
                        if voice.state.active {
                            voice.process(sample_ptr, f, ch);
                        }
                    }

                    for (_, group) in &mut self.groups {
                        if group.state.active {
                            group.process(sample_ptr, f, ch);
                        }
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
                state.active = true;
                
                for tempo_state in &mut voice.tempo_solos {
                    let mut ts = tempo_state.borrow_mut();
                    ts.active = true;
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
                voice.state.active = false;
                for tempo_state in &voice.tempo_solos {
                    let mut ts = tempo_state.borrow_mut();
                    ts.active = false;
                }
            }
            None => println!("\nErr: Could not find voice '{name}'"),
        }
    }

    pub fn resume_voice(&mut self, name: String) {
        match self.voices.get_mut(&name) {
            Some(voice) => {
                let state = &mut voice.state;
                state.active = true;
                for tempo_state in &voice.tempo_solos {
                    let mut ts = tempo_state.borrow_mut();
                    ts.active = true;
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
                state.active = false;

                for (_, p) in &mut voice.processes {
                    p.reset();
                }

                for tempo_state in &voice.tempo_solos {
                    let mut ts = tempo_state.borrow_mut();
                    ts.active = false;
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

    pub fn group(&mut self, args: String) {
        let mut args = args.split_whitespace();
        let name = match args.next() {
            Some(string) => string,
            None => {
                println!("\nErr: not enough arguments for group");
                return;
            }
        };

        // -t tempo -v voices

        let tempo_state = Rc::new(RefCell::new(TempoState::new()));
        let mut voices = HashMap::<String, Voice>::new();

        let mut t_arg = |t: &str| {
            let u_str = &t[0..=1];
            let unit = match u_str {
                "s:" => TempoUnit::Samples,
                "m:" => TempoUnit::Millis,
                "b:" => TempoUnit::Bpm,
                _ => {
                    println!("\nErr: invalid tempo unit {}", u_str);
                    return;
                }
            };
            let interval = match &t[2..].parse::<f32>() {
                Ok(val) => *val,
                Err(_) => {
                    println!("\nErr: invalid interval {}", &t[2..]);
                    return;
                }
            };
            
            tempo_state.borrow_mut().init(TempoMode::Group, unit, interval);
        };

        let mut v_arg = |v: &str| {
            let names: Vec<_> = v.split(',').collect();
            
            for name in names {
                let name = name.to_string();
                match self.voices.remove(&name) {
                    Some(voice) => voices.insert(name, voice),
                    None => {
                        println!("\nErr: could not find voice '{name}'");
                        return;
                    }
                };
            }
        };

        while let Some(arg) = args.next() {
            match arg {
                "-t" => {
                    match args.next() {
                        Some(t) => t_arg(t),
                        None => {
                            println!("\nErr: not enough arguments for -t");
                            return;
                        }
                    };
                }
                "-v" => {
                    match args.next() {
                        Some(v) => v_arg(v),
                        None => {
                            println!("\nErr: enough arguments for -v");
                            return;
                        }
                    };
                }
                _ => {
                    println!("\nErr: invalid arg '{arg}' for group");
                    return;
                }
            }
        }

        let group = Group::new(voices, tempo_state);

        self.groups.insert(name.to_string(), group);        
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
        let mut tempo: Rc<RefCell<TempoState>> = Rc::new(RefCell::new(TempoState::new()));
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
                        tempo = match self.groups.get(&tg_name) {
                            Some(group) => {
                                Rc::clone(&group.state.tempo_state);
                                continue;
                            }
                            None => {
                                println!("\nErr: no TempoGroup with the provided name");
                                return;
                            }
                        };
                    }
                    
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

                    let tempo_ref = Rc::new(RefCell::new(TempoState::new()));
                    tempo_ref.borrow_mut().init(TempoMode::Solo, unit, interval);

                    tempo = Rc::clone(&tempo_ref);

                    voice.tempo_solos.push(tempo_ref);
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
            active: true,
            period,
            tempo,
            steps,
            chance,
            jit,
            seq_idx: 0,
        };

        voice.processes.insert(
            "seq".to_string(), 
            Box::new(Seq { state })
        );
    }
}

pub struct VoiceState {
    pub active: bool,
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
    processes: HashMap<String, Box<dyn Process>>,
    tempo_solos: Vec<Rc<RefCell<TempoState>>>,
}

impl Voice {
    fn new(af: &AudioFile) -> Self {
        let end = af.samples.len() / af.num_channels as usize - 1;
        let state = VoiceState {
            active: false,
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
            processes: HashMap::<String, Box<dyn Process>>::new(),
            tempo_solos: Vec::<Rc<RefCell<TempoState>>>::new(),
        }
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active { return; }

        let state = &mut self.state;

        // processing
        for (_, p) in &mut self.processes {
            p.process(state);
        }

        for tempo_state in &mut self.tempo_solos {
            let mut ts = tempo_state.borrow_mut();
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
        let mut sample = 0f32;
        let s0 = self.samples[(idx * self.channels) + (ch % self.channels)] as f32;
        if state.velocity != 1.0 {
            let frac = state.position.fract();
            let s1 = self.samples[((idx + 1) * self.channels) + (ch % self.channels)] as f32;
            sample = s0 * (1.0 - frac) + s1 * frac;
        } else {
            sample = s0;
        }

        unsafe {
            *acc += (sample * state.gain) as i16;
        }

        // advance
        if ch == self.channels - 1 {
            state.position += state.velocity;
        }
    }
}

pub struct GroupState {
    pub active: bool,
    pub gain: f32,
    pub tempo_state: Rc<RefCell<TempoState>>,
}

pub struct Group {
    pub state: GroupState, 
    pub voices: HashMap<String, Voice>,
    // pub processes: HashMap<String, Box<dyn Process>>,
}

impl Group {
    fn new(voices: HashMap<String, Voice>, tempo_state: Rc<RefCell<TempoState>>) -> Self {
        let state = GroupState {
            active: false,
            gain: 1.0,
            tempo_state,
        };

        Self {
            state,
            voices,
            // processes: HashMap::<String, Box<dyn Process>>::new(),
        }
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active { return; }

        let state = &mut self.state;

        // processing
        for (_, v) in &mut self.voices {
            v.process(acc, frame, ch);
        }

        state.tempo_state.borrow_mut().update(1.0);
    }
}
