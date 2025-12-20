use std::{
    rc::Rc, cell::RefCell,
    collections::{HashMap, hash_map::Entry},
};

use alsa_sys::*;

use crate::file_parsing::decode_helpers::{
    DecodeResult, DecodeError, AudioFile,
};
use crate::audio_processing::{
    commands::*, // too many to list
    processes::*, // this will be ditto
    blast_rand::{
        X128P, fast_seed
    },
    blast_time::{
        sample_rate,
        blast_time::{
            clock, TempoMode, TempoUnit, TempoState
        }
    },
};

// audio engine
//
pub struct Conductor {
    voices: Vec<Voice>,
    groups: Vec<Group>,
    tempo_cons: Vec<Rc<RefCell<TempoState>>>,
    out_channels: usize,
    tracks: Vec<AudioFile>,
}

impl Conductor {
    pub fn prepare(out_channels: usize, tracks: HashMap<String, AudioFile>) -> Self {
        Self { 
            voices: Vec::<Voice>::new(), 
            groups: Vec::<Group>::new(),
            tempo_cons: Vec::<Rc<RefCell<TempoState>>>::new(),
            out_channels, 
            tracks: tracks.into_values().collect(),
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

    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Load(args) => self.load(args),
            Command::Start(args) => self.start(args),
            Command::Pause(args) => self.pause(args),
            Command::Resume(args) => self.resume(args),
            Command::Stop(args) => self.stop(args),
            Command::Unload(args) => self.unload(args),
            Command::Velocity(args) => self.velocity(args),
            Command::Group(args) => self.group(args),
            Command::TempoContext(args) => self.tempo_context(args),
            Command::Seq(args) => self.seq(args),
            Command::Quit(_) => {
                unsafe {
                    libc::raise(libc::SIGTERM);
                }
            }
        }
    }

    fn load(&mut self, args: LoadArgs) {
        let track = &self.tracks[args.track_idx];
        let tempo_state = self.tempo_from_repr(args.tempo_repr);
        self.voices.push(Voice::new(track, tempo_state);
    }

    fn tempo_from_repr(&mut self, tr: TempoRepr) -> Rc<RefCell<TempoState>> {
        // either create (and init) a new TempoState,
        // or find the referenced one within Groups or Contexts
        let mut tempo = Rc::new(RefCell::new(TempoState::new(None)));
        match tr.mode {
            TempoMode::Voice => {
                tempo.borrow_mut().init(tr.mode, tr.unit, tr.interval);
            },
            TempoMode::Group => {
                tempo = Rc::clone(&self.groups[tr.idx].state.tempo);
            }
            TempoMode::Context => {
                tempo = Rc::clone(&self.tempo_cons[tr.idx]);
            }
            TempoMode::TBD => (),
        }

        tempo
    }

    fn start(&mut self, args: StartArgs) {
        match args.idx {
            Idx::Voice(idx) => self.voices[idx].start(),
            Idx::Group(idx) => self.groups[idx].start(),
            Idx::Tempo(idx) => self.tempo_cons[idx].start(),
            _ => (),
        }
    }

    fn pause(&mut self, args: PauseArgs) {
        match args.idx {
            Idx::Voice(idx) => self.voices[idx].pause(),
            Idx::Group(idx) => self.groups[idx].pause(),
            Idx::Tempo(idx) => self.tempo_cons[idx].pause(),
            _ => (),
        }
    }

    fn resume(&mut self, args: ResumeArgs) {
        match args.idx {
            Idx::Voice(idx) => self.voices[idx].resume(),
            Idx::Group(idx) => self.groups[idx].resume(),
            Idx::Tempo(idx) => self.tempo_cons[idx].resume(),
            _ => (),
        }
    }

    fn stop(&mut self, args: StopArgs) {
        match args.idx {
            Idx::Voice(idx) => self.voices[idx].stop(),
            Idx::Group(idx) => self.groups[idx].stop(),
            Idx::Tempo(idx) => self.tempo_cons[idx].stop(),
            _ => (),
        }
    }

    fn unload(&mut self, args: UnloadArgs) {
        self.voices.remove(args.idx);
    }

    fn velocity(&mut self, args: VelocityArgs) {
        self.voices[args.idx].state.velocity = args.val;
    }

    fn group(&mut self, args: GroupArgs) {
       let tempo = self.tempo_from_repr(args.tempo);
       let voices: Vec<Voice> = Vec::new();
       for (idx, update_tempo, p_ids) in args.v_ids_and_flags {
           // move Voices out of conductor.voices into group.voices
           let mut voice = self.voices.remove(idx);
           if update_tempo {
               // refer to Group TempoState
               voice.state.tempo = Rc::clone(&tempo);
               for p in p_ids {
                   // these Processes also refer to the 
                   // Group TempoState
                   let mut process = &voice.processes[p];
                   process.update_tempo(Rc::clone(&tempo));
               }
           }
           voices.push(voice);
       }

       let group = Group::new(voices, tempo);
       self.groups.push(group);
    }

    fn tempo_context(&mut self, args: TcArgs) {
        let tempo_state = self.tempo_from_repr(args.tempo);
        self.tempo_cons.push(tempo_state);
    }

    fn seq(&mut self, args: String) {
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
                println!("\nErr: Couldn't find voice '{name}'");
                return;
            }
        };

        let mut period: usize = sample_rate::get() as usize;
        let mut tempo: Rc<RefCell<TempoState>> = Rc::new(RefCell::new(TempoState::new(Some(TempoMode::Process))));
        let mut steps: Vec<f32> = Vec::new();
        let mut chance: Vec<f32> = Vec::new();
        let mut jit: Vec<f32> = Vec::new();
        // implement user-defined seed l8r
        let mut rng = X128P::new(fast_seed());

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

                    let mut t_args = t_arg.split(':');

                    let u = t_args.next().unwrap();

                    if u == "c" {
                        // find TempoContext
                        let tc_name = match t_args.next() {
                            Some(tc) => tc,
                            None => {
                                println!("\nErr: not enough arguments to find TempoContext");
                                return;
                            }
                        };
                        let tc_name = tc_name.to_string();
                        tempo = match self.tempo_cons.get(&tc_name) {
                            Some(tc) => {
                                Rc::clone(tc);
                                continue;
                            }
                            None => {
                                println!("\nErr: no TempoContext with the name {tc_name}");
                                return;
                            }
                        };
                        continue;
                    }

                    if u == "v" {
                        // refer to Voice's TempoState;
                        // if Voice's TempoState is a Group's, Seq will run when this Group does
                        tempo = Rc::clone(&voice.state.tempo);
                        continue;
                    }

                    let unit = match u {
                        "s" => TempoUnit::Samples,
                        "m" => TempoUnit::Millis,
                        "b" => TempoUnit::Bpm,
                        _ => {
                            println!("\nErr: unrecognized time unit for tempo");
                            return;
                        }
                    };

                    let mut interval = 0.0;

                    if let Some(int) = t_args.next() {
                        match int.parse::<f32>() {
                            Ok(val) => interval = val,
                            Err(_) => {
                                println!("\nErr: invalid tempo interval");
                                return;
                            }
                        }
                    } else {
                        println!("\nErr: missing interval argument for tempo");
                        return;
                    }

                    let tempo_ref = Rc::new(RefCell::new(TempoState::new(None)));
                    tempo_ref.borrow_mut().init(TempoMode::Process, unit, interval);

                    tempo = Rc::clone(&tempo_ref);

                    voice.proc_tempi.push(tempo_ref);
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
                    let s_arg = match args.next() {
                        Some(arg) => arg,
                        None => {
                            println!("\nErr: not enough arguments for steps");
                            return;
                        }
                    };
                    let step_strs: Vec<&str> = s_arg.split(',').collect();

                    for step in step_strs {
                        match step.parse::<f32>() {
                            Ok(val) => steps.push(val),
                            Err(_) => {
                                println!("\nErr: invalid argument {step} for steps");
                                return;
                            }
                        }
                    }

                    // set chance and jit Vecs to same len as steps
                    // to avoid panics
                    chance.resize(steps.len(), 100f32);
                    jit.resize(steps.len(), 100f32);
                }
                "-c" | "--chance" => {
                    // a value specifies chance for the step
                    //// at the same index as the value
                    // _ is shorthand for 100
                    // n:val specifies chance=val for step=n
                    // a:val sets the same chance=val for all steps
                    // n1-n2:val specifies a chance=val for
                    //// n1-n2 contiguous steps

                    if steps.len() < 1 {
                        println!("\nErr: provide arguments to -s/--steps before -c/--chance or -j/--jitter");
                        return;
                    }

                    let c_arg = match args.next() {
                        Some(arg) => arg,
                        None => {
                            println!("\nErr: not enough arguments for chance");
                            return;
                        }
                    };
                    let c_strs: Vec<&str> = c_arg.split(',').collect();

                    let mut spec_char = |s: &str| -> Option<char> {
                        for c in s.chars() {
                            match c {
                                '_' => return Some('_'),
                                ':' => return Some(':'),
                                '-' => return Some('-'),
                                _ => continue,
                            }
                        }
                        None
                    };
                    
                    // use chance.len() if too many arguments were provided
                    let len = {
                        if c_strs.len() > chance.len() {
                            chance.len()
                        } else {
                            c_strs.len()
                        }
                    };

                    for i in {0..len} {
                        let string = c_strs.get(i).unwrap();
                        match spec_char(string) {
                            Some(c) => {
                                match c {
                                    '_' => chance[i] = 100.0,
                                    ':' => {
                                        let at_index: Vec<&str> = string.split(':').collect();
                                        if at_index.len() < 2 {
                                            println!("\nErr: not enough arguments for :");
                                            return;
                                        } else if at_index.len() > 2 {
                                            println!("\nErr: too many arguments for :");
                                            return;
                                        }

                                        // get chance first in case index = 'a'
                                        let chance_str = at_index.get(1).unwrap();
                                        let chance_val = match chance_str.parse::<f32>() {
                                            Ok(val) => val,
                                            Err(_) => {
                                                println!("\nErr: invalid argument {chance_str} for change");
                                                return;
                                            }
                                        };

                                        let index_str = at_index.get(0).unwrap();

                                        // if index = 'a', set all chance vals to chance_val and continue
                                        if *index_str == "a" {
                                            for i in {0..chance.len()} {
                                                chance[i] = chance_val;
                                            }
                                            continue;
                                        }

                                        let index = match index_str.parse::<f32>() {
                                            Ok(val) => val,
                                            Err(_) => {
                                                println!("\nErr: invalid argument {index_str} for chance");
                                                return;
                                            }
                                        };
                                        
                                        let mut found = false;
                                        for i in {0..steps.len()} {
                                            let step = *steps.get(i).unwrap();
                                            if index == step {
                                                chance[i] = chance_val;
                                                found = true;
                                                break;
                                            }
                                        }

                                        if !found {
                                            println!("\nErr: {index} not in steps");
                                            return;
                                        }
                                    }
                                    '-' => {
                                        let at_indices: Vec<&str> = string.split(':').collect();
                                        if at_indices.len() < 2 {
                                            println!("\nErr: not enough arguments for :");
                                            return;
                                        } else if at_indices.len() > 2 {
                                            println!("\nErr: too many arguments for :");
                                            return;
                                        }
                                        
                                        let chance_str = at_indices.get(1).unwrap();
                                        let chance_val = match chance_str.parse::<f32>() {
                                            Ok(val) => val,
                                            Err(_) => {
                                                println!("\nErr: invalid argument {chance_str} for change");
                                                return;
                                            }
                                        };

                                        let indices: Vec<&str> = at_indices[0].split('-').collect();
                                        if indices.len() < 2 {
                                            println!("\nErr: not enough arguments for -");
                                            return;
                                        } else if indices.len() > 2 {
                                            println!("\nErr: too many arguments for -");
                                            return;
                                        }

                                        let idx1 = match indices[0].parse::<f32>() {
                                            Ok(val) => val,
                                            Err(_) => {
                                                println!("\nErr: invalid argument {} for -", indices[0]);
                                                return;
                                            }
                                        };
                                        let idx2 = match indices[1].parse::<f32>() {
                                            Ok(val) => val,
                                            Err(_) => {
                                                println!("\nErr: invalid argument {} for -", indices[1]);
                                                return;
                                            }
                                        };

                                        let mut lower = idx1;
                                        let mut upper = idx2;

                                        if lower > upper {
                                            lower = idx2;
                                            upper = idx1;
                                        }
                        
                                        // only check against lower because who cares if upper is too high
                                        if lower > *steps.get(steps.len() - 1).unwrap() {
                                            println!("\nErr: range {lower}-{upper} applies to nothing");
                                            return;
                                        }

                                        for idx in {0..steps.len()} {
                                            let step = *steps.get(idx).unwrap();
                                            if step >= lower && step <= upper {
                                                chance[idx] = chance_val;
                                            }
                                        }
                                    }
                                    _ => (),
                                }
                            }
                            // no special chars; just assign value at current index
                            None => {
                                let chance_val = match string.parse::<f32>() {
                                    Ok(val) => val,
                                    Err(_) => {
                                        println!("\nErr: invalid argument {string} for chance");
                                        return;
                                    }
                                };
                                chance[i] = chance_val;
                            }
                        }
                    }                   
                }
                "-j" | "--jitter" => {
                    // a value specifies jitter for the step
                    //// at the same index as the value
                    // _ means no jitter
                    // e|l indicates jitter before=e and after=l the beat
                    //// (of ranges e-0.0 and 0.0-l)
                    // e1-e2|l1-l2 indicate jitter ranges
                    // n:e|l specifies jitter=e|l for step=n
                    // a:e|l specifies jitter=e|l for all steps
                    // n1-n2,e1-2|l1-l2 specifies jitter ranges for
                    //// n1-n2 contiguous steps
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
            rng,
            idx: 0,
        };

        voice.processes.insert(
            "seq".to_string(), 
            Process::Seq(Seq { state })
        );
    }
}

pub struct VoiceState {
    pub active: bool,
    pub position: f32,
    pub end: usize,
    pub velocity: f32,
    pub gain: f32,
    pub tempo: Rc<RefCell<TempoState>>,
}

pub struct Voice {
    samples: Vec<i16>,
    sample_rate: u32,
    channels: usize,
    pub state: VoiceState,  
    processes: Vec<Process>,
    proc_tempi: Vec<Rc<RefCell<TempoState>>>, // TempoMode::Process
}

impl Voice {
    fn new(af: &AudioFile, tempo_state: Rc<RefCell<TempoState>>) -> Self {
        let voice_state = VoiceState {
            active: false,
            position: 0.0,
            end: af.samples.len() / af.num_channels as usize - 1,
            velocity: 1.0,
            gain: 1.0,
            tempo: tempo_state
        };

        Self {
            samples: af.samples.clone(),
            sample_rate: af.sample_rate, 
            channels: af.num_channels as usize, 
            state: voice_state,
            processes: HashMap::<String, Process>::new(),
            proc_tempi: Vec::<Rc<RefCell<TempoState>>>::new(),
        }
    }

    fn start(&mut self) {
        let state = &mut self.state;
        state.active = true;

        for p in &mut self.processes {
            p.reset();
        }

        let mut ts = state.tempo.borrow_mut();
        if ts.mode == TempoMode::Voice {
            ts.active = true;
            ts.reset();
        } else {
            if ts.active == false {
                println!("\nWarn: Tempo not active for Voice");
            }
        }
                
        for tempo_state in &mut self.proc_tempi {
            let mut ts = tempo_state.borrow_mut();
            ts.active = true;
            ts.reset();
        }

        state.position = match state.velocity >= 0.0 {
            true => 0.0,
            false => state.end as f32,
        };
    }

    fn pause(&mut self) {
        self.state.active = false;
    }

    fn resume(&mut self) {
        self.state.active = true;

        let ts = self.state.tempo.borrow();
        if ts.mode != TempoMode::Voice {
            if ts.active == false {
                println!("\nWarn: Tempo not active for Voice");
            }
        }
    }

    fn stop(&mut self) {
        let state = &mut self.state;
        state.active = false;

        for p in &mut self.processes {
            p.reset();
        }

        let mut ts = state.tempo.borrow_mut();
        if ts.mode == TempoMode::Voice {
            ts.active = false;
            ts.reset();
        }

        for tempo_state in &self.proc_tempi {
            let mut ts = tempo_state.borrow_mut();
            ts.active = false;
            ts.reset();
        }

        state.position = match state.velocity >= 0.0 {
            true => 0.0,
            false => state.end as f32,
        };
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active { return; }

        let state = &mut self.state;

        // processing
        for p in &mut self.processes {
            p.process(state);
        }

        let mut own_tempo = state.tempo.borrow_mut();
        if own_tempo.mode == TempoMode::Voice || own_tempo.mode == TempoMode::TBD {
            // only update own TempoState if it belongs to this Voice
            own_tempo.update(1.0);
        }

        for tempo_state in &mut self.proc_tempi {
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
    pub tempo: Rc<RefCell<TempoState>>,
}

pub struct Group {
    pub state: GroupState, 
    pub voices: Vec<Voice>,
    // pub processes: HashMap<String, Process>,
}

impl Group {
    fn new(voices: Vec<Voice>, tempo: Rc<RefCell<TempoState>>) -> Self {
        let state = GroupState {
            active: false,
            gain: 1.0,
            tempo,
        };

        Self {
            state,
            voices,
            // processes: HashMap::<String, Process>::new(),
        }
    }

    fn start(&mut self) {
        let state = &mut self.state;
        state.active = true;

        {   
            let mut ts = state.tempo.borrow_mut();
            if ts.mode == TempoMode::Group {
                ts.active = true;
                ts.reset();
            } else {
                if ts.active == false {
                    println!("\nWarn: Tempo not active for Group");
                }
            }
        }

        for voice in &mut self.voices {
            voice.start();
        }
    }

    fn pause(&mut self) {
        self.state.active = false; // Voices still active, but won't
                                   // be doing anything since their
                                   // process() isn't being called
    }

    fn resume(&mut self) {
        self.state.active = true;
                
        let ts = self.state.tempo.borrow();
        if ts.mode == TempoMode::Context {
            if ts.active == false {
                println!("\nWarn: Tempo not active for Group");
            }
        }
    }

    fn stop(&mut self) {
        self.state.active = false;

        for mut voice in &mut self.voices {
            voice.state.active = false;
        }
                
        let mut ts = self.state.tempo.borrow_mut();
        if ts.mode == TempoMode::Group {
            ts.active = false;
            ts.reset();
        }
    }

    fn process(&mut self, acc: *mut i16, frame: u64, mut ch: usize) {
        if !self.state.active { return; }

        // processing
        for v in &mut self.voices {
            v.process(acc, frame, ch);
        }

        let mut ts = self.state.tempo.borrow_mut();
        if ts.mode == TempoMode::Group {
            ts.update(1.0);
        }
    }
}
