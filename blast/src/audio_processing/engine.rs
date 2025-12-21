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

                    for voice in &mut self.voices {
                        if voice.state.active {
                            voice.process(sample_ptr, f, ch);
                        }
                    }

                    for group in &mut self.groups {
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
            Command::Tc(args) => self.tempo_context(args),
            Command::Seq(args) => self.seq(args),
            Command::Quit(_) => {
                unsafe {
                    libc::raise(libc::SIGTERM);
                }
            }
        }
    }

    fn load(&mut self, args: LoadArgs) {
        let track = self.tracks.get(args.track_idx).unwrap();
        let tempo_state = self.tempo_from_repr(args.tempo_repr);
        self.voices.push(Voice::new(track, tempo_state));
    }

    
    fn start(&mut self, args: StartArgs) {
        match args.idx {
            Idx::Voice(idx) => {
                let voice: &mut Voice = self.voices.get_mut(idx).unwrap();
                voice.start();
            }
            Idx::Group(idx) => {
                let group: &mut Group = self.groups.get_mut(idx).unwrap();
                group.start();
            }
            Idx::Tempo(idx) => {
                let mut tc = self.tempo_cons.get(idx).unwrap().borrow_mut();
                tc.start();
            }
            _ => (),
        }
    }

    fn pause(&mut self, args: PauseArgs) {
        match args.idx {
            Idx::Voice(idx) => {
                let voice: &mut Voice = self.voices.get_mut(idx).unwrap();
                voice.pause();
            }
            Idx::Group(idx) => {
                let group: &mut Group = self.groups.get_mut(idx).unwrap();
                group.pause();
            }
            Idx::Tempo(idx) => {
                let mut tc = self.tempo_cons.get(idx).unwrap().borrow_mut();
                tc.pause();
            }
            _ => (),
        }
    }

    fn resume(&mut self, args: ResumeArgs) {
        match args.idx {
            Idx::Voice(idx) => {
                let voice: &mut Voice = self.voices.get_mut(idx).unwrap();
                voice.resume();
            }
            Idx::Group(idx) => {
                let group: &mut Group = self.groups.get_mut(idx).unwrap();
                group.resume();
            }
            Idx::Tempo(idx) => {
                let mut tc = self.tempo_cons.get(idx).unwrap().borrow_mut();
                tc.resume();
            }
            _ => (),
        }
    }

    fn stop(&mut self, args: StopArgs) {
        match args.idx {
            Idx::Voice(idx) => {
                let voice: &mut Voice = self.voices.get_mut(idx).unwrap();
                voice.stop();
            }
            Idx::Group(idx) => {
                let group: &mut Group = self.groups.get_mut(idx).unwrap();
                group.stop();
            }
            Idx::Tempo(idx) => {
                let mut tc = self.tempo_cons.get(idx).unwrap().borrow_mut();
                tc.stop();
            }
            _ => (),
        }
    }

    fn unload(&mut self, args: UnloadArgs) {
        self.voices.remove(args.idx);
    }

    fn velocity(&mut self, args: VelocityArgs) {
        let voice: &mut Voice = self.voices.get_mut(args.idx).unwrap();
        voice.state.velocity = args.val;
    }

    fn group(&mut self, args: GroupArgs) {
       let tempo = self.tempo_from_repr(args.tempo);
       let mut voices: Vec<Voice> = Vec::new();
       for (idx, update_tempo, p_ids) in args.vs_fs_ps {
           // move Voices out of conductor.voices into group.voices
           let mut voice = self.voices.remove(idx);
           if update_tempo {
               // refer to Group TempoState
               voice.state.tempo = Rc::clone(&tempo);
               for p in p_ids {
                   // these Processes also refer to the 
                   // Group TempoState
                   let mut process = &mut voice.processes[p];
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

    // Processes
    //
    fn seq(&mut self, args: SeqArgs) {
        let tempo = self.tempo_from_repr(TempoRepr::clone(&args.tempo));
        let state = SeqState {
            active: true,
            tempo: Rc::clone(&tempo),
            period: args.period,
            steps: args.steps,
            chance: args.chance,
            jit: args.jit,
            rng: args.rng,
            idx: 0,
        };
        
        match args.idx {
            Idx::Voice(v) => {
                let voice: &mut Voice = self.voices.get_mut(v).unwrap();
                voice.processes.push(Process::Seq(Seq { state }));
                if args.tempo.mode == TempoMode::Process {
                    voice.proc_tempi.push(tempo);
                }
            }
            Idx::Group(g) => {
                let group: &mut Group = self.groups.get_mut(g).unwrap();
                group.processes.push(Process::Seq(Seq { state }));
            }
            _ => (), // will only be Voice or Group
        }
    }

    // helpers
    //
    fn tempo_from_repr(&self, tr: TempoRepr) -> Rc<RefCell<TempoState>> {
        // either create (and init) a new TempoState,
        // or find the referenced one within Groups or Contexts
        let mut tempo = Rc::new(RefCell::new(TempoState::new(None)));
        if tr.owned {
            tempo.borrow_mut().init(tr.mode, tr.unit, tr.interval);
        } else {
            match tr.mode {
                TempoMode::Voice => {
                    tempo = Rc::clone(&self.voices[tr.idx].state.tempo);
                }
                TempoMode::Group => {
                    tempo = Rc::clone(&self.groups[tr.idx].state.tempo);
                }
                TempoMode::Context => {
                    tempo = Rc::clone(&self.tempo_cons[tr.idx]);
                }
                // Process will never borrow from another Process
                TempoMode::Process | TempoMode::TBD => (),
            }
        }

        tempo
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
            processes: Vec::<Process>::new(),
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
        if ts.mode == TempoMode::Voice || ts.mode == TempoMode::TBD {
            ts.start();
        } else {
            if ts.active == false {
                println!("\nWarn: Tempo not active for Voice");
            }
        }
                
        for tempo_state in &mut self.proc_tempi {
            let mut ts = tempo_state.borrow_mut();
            ts.start();
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
            ts.stop();
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
    pub processes: Vec<Process>,
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
            processes: Vec::<Process>::new(),
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
