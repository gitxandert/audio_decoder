use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::{HashMap, hash_map::Entry};

use crate::file_parsing::decode_helpers::AudioFile;
use crate::audio_processing::{
    blast_time::blast_time::{TempoUnit, TempoMode},
    blast_rand::{X128P, fast_seed},
};

pub struct CmdQueue {
    buf: Vec<UnsafeCell<Option<Command>>>,
    cap: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

unsafe impl Send for CmdQueue {}
unsafe impl Sync for CmdQueue {}

impl CmdQueue {
    pub fn new(cap: usize) -> Self {
        let mut buf = Vec::<UnsafeCell<Option<Command>>>::with_capacity(cap);

        for _ in {0..cap} {
            buf.push(UnsafeCell::new(None));
        }

        Self {
            buf,
            cap,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn try_push(&self, cmd: Command) -> Result<(), String> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        if (head + 1) % self.cap == tail {
            return Err(String::from("Command queue full"));
        }

        unsafe {
            *self.buf[head].get() = Some(cmd);
        }

        self.head.store((head + 1) % self.cap, Ordering::Release);
        Ok(())
    }

    pub fn try_pop(&self) -> Option<Command> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);

        if head == tail {
            return None;
        }

        let cmd = unsafe {
            (*self.buf[tail].get()).take()
        };

        self.tail.store((tail + 1) % self.cap, Ordering::Release);
        
        cmd
    }
}

use blast_macros::var_args;

macro_rules! commands {
    ( $( $var:ident ),* $(,)? ) => {
        pub enum Command {
            $(
                $var(var_args!($var)), // formats as {CmdType}Args
            )*
        }

        unsafe impl Send for Command {}
        unsafe impl Sync for Command {}
    }
}

commands! {
    // Voices
    Load,
    Start,
    Pause,
    Resume,
    Stop,
    Unload,
    Velocity,
    // Groups
    Group,
    Tc,
    // Processes
    Seq,
    // Program
    Quit,
}

// specialized args for commands
// (need definition because they're declared in the commands! macro)

pub struct LoadArgs {
    pub track_idx: usize,
    pub tempo_repr: TempoRepr,
}

pub struct StartArgs {
    pub idx: Idx,
}

pub struct PauseArgs {
    pub idx: Idx,
}

pub struct ResumeArgs {
    pub idx: Idx,
}

pub struct StopArgs {
    pub idx: Idx,
}

pub struct UnloadArgs {
    pub idx: usize,
}

pub struct VelocityArgs {
    pub idx: usize,
    pub val: f32,
}

pub struct GroupArgs {
    pub tempo: TempoRepr,
    pub vs_fs_ps: Vec<(usize, bool, Vec<usize>)>, 
    // store the ids Voice
    // with whether or not its TempoState refers to the Group's
    // and with the ids of all of the Processes 
    // whose TempoStates refer to the Group's
}

pub struct TcArgs {
    pub tempo: TempoRepr,
}

pub struct SeqArgs {
    pub idx: Idx,
    pub tempo: TempoRepr,
    pub period: usize,
    pub steps: Vec<f32>,
    pub chance: Vec<f32>,
    pub jit: Vec<f32>,
    pub rng: X128P,
}

// doesn't need any members, just triggers raise(SIGTERM)
pub struct QuitArgs {}

// structs to represent engine/object state

// use for terse, ambiguous Commands like Start;
// prefer Reprs when more info is required
pub enum Idx {
    Tempo(usize),
    Voice(usize),
    Process(usize),
    Group(usize),
    // don't need one for Track because TrackRepr is already
    // just an index, and there are few Commands that operate on
    // Tracks, so it'll never be ambiguous
}

pub struct TrackRepr {
    idx: usize,
}

impl TrackRepr {
    fn new(idx: usize) -> Self {
        Self { idx }
    }
}

// owned bool determines whether a TempoState is initialized
// or cloned inside of the engine
pub struct TempoRepr {
    pub idx: usize,
    pub owned: bool,
    pub mode: TempoMode,
    pub unit: TempoUnit,
    pub interval: f32,
}

impl TempoRepr {
    fn new(idx: usize) -> Self {
        Self {
            idx,
            owned: true, // default owned, until clone_owner
            mode: TempoMode::TBD,
            unit: TempoUnit::Samples,
            interval: 0f32,
        }
    }

    pub fn clone(other: &TempoRepr) -> Self {
        Self {
            idx: other.idx,
            owned: other.owned,
            mode: other.mode,
            unit: other.unit,
            interval: other.interval,
        }
    }

    // this is used when referring to another object's TempoState
    fn clone_owner(other: &TempoRepr) -> Self {
        Self {
            idx: other.idx,
            owned: false,
            mode: other.mode,
            unit: other.unit,
            interval: other.interval,
        }
    }

    fn init(&mut self, mode: TempoMode, unit: TempoUnit, interval: f32) {
        self.mode = mode;
        self.unit = unit;
        self.interval = interval;
    }
}

pub struct VoiceRepr {
    idx: usize,
    tempo: TempoRepr,
    processes: HashMap<String, ProcRepr>,
    proc_tempi: HashMap<usize, TempoRepr>,
}

impl VoiceRepr {
    fn new(idx: usize, tempo: TempoRepr) -> Self {
        Self {
            idx,
            tempo,
            processes: HashMap::<String, ProcRepr>::new(),
            proc_tempi: HashMap::<usize, TempoRepr>::new(),
        }
    }
}

pub struct ProcRepr {
    // Processes are difficult to represent because they all
    // differ, so can only represent info that applies
    // to all Processes
    //
    idx: usize, // index of the Process in its owner's
                // Vec<Process>

    owner_idx: Idx, // index of the Process's $owner
                      // in the engine's Vec<$owner>
    tempo: Option<TempoRepr>,
    // maybe create ProcArgs enum, one for each Process
}

impl ProcRepr {
    fn new(idx: usize, owner_idx: Idx, tempo: Option<TempoRepr>) -> Self {
        Self { idx, owner_idx, tempo }
    }
}

pub struct GroupRepr {
    idx: usize,
    tempo: TempoRepr,
    voices: HashMap<String, VoiceRepr>,
}

impl GroupRepr {
    fn new(idx: usize, tempo: TempoRepr, voices: HashMap<String, VoiceRepr>) -> Self {
        Self { idx, tempo, voices }
    }
}

// keeps track of all entities' states
pub struct EngineState {
    tracks: HashMap<String, TrackRepr>,
    voices: HashMap<String, VoiceRepr>,
    groups: HashMap<String, GroupRepr>,
    tempo_cons: HashMap<String, TempoRepr>,
    out_channels: usize,
}

impl EngineState {
    pub fn new(files: Vec<AudioFile>, out_channels: usize) -> Self {
        let mut tracks: HashMap<String, TrackRepr> = HashMap::new();
        for (idx, af) in files.iter().enumerate() {
            tracks.insert(af.file_name.clone(), TrackRepr::new(idx));
        }

        Self {
            tracks,
            out_channels,
            voices: HashMap::<String, VoiceRepr>::new(),
            groups: HashMap::<String, GroupRepr>::new(),
            tempo_cons: HashMap::<String, TempoRepr>::new(),
        }
    }
}

// validates and formats Commands for the engine
// (handles string allocations, integer/float parsing, etc)
pub struct CmdProcessor {
    pub engine_state: EngineState,
}

impl CmdProcessor {
    pub fn new(engine_state: EngineState) -> Self {
        Self { engine_state }
    }
    
    pub fn parse(&mut self, cmd: String) -> CmdResult<Command> {
        let mut parts = cmd.splitn(2, ' ');
        let cmd = parts.next().unwrap();
        let args = parts.next().unwrap_or_else(|| "").to_string();
        
        match cmd {
            "load" => self.try_load(args),
            "start" => self.try_start(args),
            "pause" => self.try_pause(args),
            "resume" => self.try_resume(args),
            "stop" => self.try_stop(args),
            "unload" => self.try_unload(args),
            "velocity" => self.try_velocity(args),
            "group" => self.try_group(args),
            "tc" | "tempocon" => self.try_tc(args),
            "seq" => self.try_seq(args),
            "q" | "quit" => Ok(Command::Quit(QuitArgs{})),
            _ => return Err(CmdErr::NoCmd { cmd: cmd.to_owned() }),
        }
    }

    // CmdResults (returned directly to command thread)
    //
    fn try_load(&mut self, args: String) -> CmdResult<Command> {
        // parse args to:
        // - validate that the Track exists
        // - get the Track's idx
        // - format TempoRepr
        // - format VoiceRepr for engine
        //
        // engine then parses LoadArgs to:
        // - get the Track
        // - create a TempoState based on the TempoRepr
        //      - this involves checking TempoRepr.mode
        //        to see if the Voice's TempoState refers to
        //        an existing TempoState
        // - call Voice::new(track, tempo_state)
        //
        let mut args = args.split_whitespace();
        let name = args
            .next()
            .ok_or(CmdErr::MissingArg { 
                arg: "name".to_string(), 
                cmd: "load".to_string() 
            })?;
        let name = name.to_string();

        let track = self.find_track(name.clone())?;
        let track_idx = track.idx;
        
        // initialize tempo_repr with an idx of 0 because
        // a Voice will only ever have one personal TempoState
        let mut tempo_repr = TempoRepr::new(0usize);

        // if a Voice by this name (currently the track name)
        // already exists, then return error
        match self.find_voice(name.clone()) {
            Ok(voice) => return Err(CmdErr::AlreadyIs { 
                ty: "Voice".to_string(), 
                name: name 
            }),
            Err(_) => (),
        }
        
        while let Some(arg) = args.next() {
            match arg {
                "-t" | "--tempo" => {
                    let t_arg = args
                        .next()
                        .ok_or(CmdErr::MissingArg { 
                            arg: "unit".to_string(), 
                            cmd: "load -t/--tempo".to_string() 
                        })?;

                    let mut t_args = t_arg.split(':');

                    let u = t_args.next().unwrap();
                    if u == "c" {
                        // find TempoContext
                        let tc_name = t_args
                            .next()
                            .ok_or(CmdErr::MissingArg { 
                                arg: "name".to_string(), 
                                cmd: "load -t c:???".to_string() 
                            })?;
                        
                        tempo_repr = match self.find_tc(tc_name.to_string()) {
                            Ok(tc) => TempoRepr::clone_owner(&tc),
                            Err(error) => return Err(error.into()),
                        };
                        continue;
                    }
                    if u == "g" {
                        // find Group
                        let g_name = t_args
                            .next()
                            .ok_or(CmdErr::MissingArg { 
                                arg: "name".to_string(), 
                                cmd: "load -t g:???".to_string() 
                            })?;

                        tempo_repr = match self.find_group(g_name.to_string()) {
                            Ok(g) => TempoRepr::clone_owner(&g.tempo),
                            Err(error) => return Err(error.into()),
                        };
                        continue;
                    }

                    // make new TempoState from matched arguments
                                    
                    let unit = match u {
                        "s" => TempoUnit::Samples,
                        "m" => TempoUnit::Millis,
                        "b" => TempoUnit::Bpm,
                        _ => return Err(CmdErr::InvalidArg { 
                            arg: u.to_owned(), 
                            cmd: "load -t".to_string() 
                        }),
                    };

                    let interval = t_args
                        .next()
                        .ok_or(CmdErr::MissingArg { 
                            arg: "interval".to_string(), 
                            cmd: "load -t".to_string() 
                        })
                        .and_then(|raw| {
                            raw.parse::<f32>()
                               .map_err(|_| CmdErr::InvalidArg { 
                                    arg: raw.to_owned(), 
                                    cmd: "load -t".to_string() 
                               })
                        })?;

                    tempo_repr.init(TempoMode::Voice, unit, interval);
                }
                // no argument matched
                _ => return Err(CmdErr::InvalidArg { 
                    arg: arg.to_owned(), 
                    cmd: "load".to_string() 
                }),
            }
        }
        // if this is the first Voice,
        // it will be indexed at 0
        let idx = self.engine_state.voices.len();
        self.engine_state.voices.insert(
            name,
            VoiceRepr::new(idx, TempoRepr::clone(&tempo_repr))
        );
        
        Ok(Command::Load(LoadArgs{track_idx, tempo_repr}))
    }

    // the following could start multiple things at the same time
    // (e.g. *Args could hold a Vec<Idx>);
    // maybe implement "all" as a reserved word
    //
    fn try_start(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(
            args, "start".to_string()
        )?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Start(StartArgs{ idx }))
    }

    fn try_pause(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(
            args, "pause".to_string()
        )?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Pause(PauseArgs{ idx }))
    } 

    fn try_resume(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(
            args, "resume".to_string()
        )?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Resume(ResumeArgs{ idx }))
    }  

    fn try_stop(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(
            args, "stop".to_string()
        )?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Stop(StopArgs{ idx }))
    } 

    fn try_unload(&mut self, name: String) -> CmdResult<Command> {
        // gets idx and removes VoiceRepr from self.engine_state.voices
        let idx = match self.engine_state.voices.entry(name.clone()) {
            Entry::Occupied(e) => {
                let e_idx = e.get().idx;
                e.remove();
                e_idx
            }
            Entry::Vacant(_) => {
                return Err(CmdErr::NoVoice { 
                    name: name.to_owned(), 
                    group: None 
                });
            }
        };

        // since all Voices after the removed Voice will be 
        // shifted to the left, decrease all VoiceReprs with
        // an idx greater than the removed Voice's
        for (_, mut voice) in &mut self.engine_state.voices {
            if voice.idx > idx {
                voice.idx -= 1;
            }
        }

        Ok(Command::Unload(UnloadArgs{ idx }))
    }

    fn try_velocity(&mut self, args: String) -> CmdResult<Command> {
        let mut args = args.splitn(2, ' ');
        
        let name = args
            .next()
            .ok_or(CmdErr::MissingArg{ 
                arg: "name".to_string(), 
                cmd: "velocity".to_string() 
            })?;
        
        let vidx = self.get_idx("-v".to_string(), name.to_string())?;
        let idx = match vidx {
            Idx::Voice(i) => i,
            _ => 0,
        }; // this will match

        let val = args
            .next()
            .ok_or(CmdErr::MissingArg{ 
                arg: "value".to_string(), 
                cmd: "velocity".to_string() 
            })
            .and_then(|raw| {
                raw.parse::<f32>()
                   .map_err(|_| CmdErr::InvalidArg { 
                        arg: raw.to_owned(), 
                        cmd: "velocity".to_string() 
                   })
            })?;

        Ok(Command::Velocity(VelocityArgs{ idx, val }))
    }

    fn try_group(&mut self, args: String) -> CmdResult<Command> {
        let mut args = args.split_whitespace();
        let name = args
            .next()
            .ok_or(CmdErr::MissingArg { 
                arg: "name".to_string(), 
                cmd: "group".to_string() 
            })?;

        // -t tempo -v voices

        let mut tempo = TempoRepr::new(0);
        tempo.init(TempoMode::Group, TempoUnit::Bpm, 240.0);
        let mut voices = HashMap::<String, VoiceRepr>::new();
        // save Voice indices as Voices are collected,
        // since these indices will change when added to voices
        let mut v_ids = Vec::<usize>::new();

        while let Some(arg) = args.next() {
            match arg {
                "-t" | "--tempo" => {
                    match args.next() {
                        Some(t) => {
                            let mut t_args = t.split(':');
                            let u_str = t_args
                                .next()
                                .ok_or(CmdErr::MissingArg { 
                                    arg: "unit".to_string(), 
                                    cmd: "group -t".to_string() 
                                })?;

                            if u_str == "c" {
                                let tc_name = t_args
                                    .next()
                                    .ok_or(CmdErr::MissingArg { 
                                        arg: "TempoContext name".to_string(), 
                                        cmd: "group -t".to_string() 
                                    })?;

                                match self.find_tc(name.to_string()) {
                                    Ok(tc) => tempo = TempoRepr::clone(&tc),
                                    Err(error) => return Err(error.into()),
                                }
                            } else {
                                let unit = match u_str {
                                    "s" => TempoUnit::Samples,
                                    "m" => TempoUnit::Millis,
                                    "b" => TempoUnit::Bpm,
                                    _ => return Err(CmdErr::InvalidArg { 
                                            arg: u_str.to_string(), 
                                            cmd: "group -t".to_string() 
                                        }),
                                };
                                let interval = t_args
                                    .next()
                                    .ok_or(CmdErr::MissingArg { 
                                        arg: "interval".to_string(), 
                                        cmd: "group -t".to_string() 
                                    })
                                    .and_then(|raw| {
                                        raw.parse::<f32>()
                                           .map_err(|_| CmdErr::InvalidArg { 
                                                arg: raw.to_string(), 
                                                cmd: "group -t".to_string() 
                                            })
                                    })?;
    
                                let mut new_tempo = TempoRepr::new(0);
                                new_tempo.init(TempoMode::Group, unit, interval);
                                tempo = new_tempo;
                            }
                        }
                        None => return Err(CmdErr::MissingArg { 
                            arg: "arguments".to_string(), 
                            cmd: "group -t".to_string() 
                        }),
                    }
                }
                "-v" | "--voices" => {
                    match args.next() {
                        Some(v) => {
                            let names: Vec<_> = v.split(',').collect();

                            // need to collect all indices of the Voices that
                            // are being removed; then sort high to low
                            // and decrement all other indices -ge
                            for name in names {
                                let name = name.to_string();
                                match self.engine_state.voices.remove(&name) {
                                    Some(mut voice) => {
                                        v_ids.push(voice.idx);
                                        voice.idx = voices.len(); // assign new index
                                                                  // in Group's Vec
                                        voices.insert(name, voice);
                                    }
                                    None => return Err(CmdErr::NoVoice { 
                                        name: name, 
                                        group: None 
                                    }),
                                }
                            }

                            // sort removed voices in reverse
                            // so that the remaining voice.idx
                            // are decremented correctly
                            let mut sorted = v_ids.clone();
                            sorted.sort_by(|a, b| b.cmp(a));

                            for removed_id in sorted {
                                for (_, v) in &mut self.engine_state.voices {
                                    if v.idx > removed_id {
                                        v.idx -= 1;
                                    }
                                }
                            }
                        }
                        None => return Err(CmdErr::MissingArg { 
                            arg: "arguments".to_string(), 
                            cmd: "group -v".to_string() 
                        }),
                    }
                }
                _ => return Err(CmdErr::InvalidArg { 
                        arg: arg.to_owned(), 
                        cmd: "group".to_string() 
                    }),
            }
        }
       
        // assign flags to each Voice depending on whether its
        // TempoState will refer to the Group's
        let mut v_flags: Vec<bool> = Vec::new();

        // collect indices of Processes whose TempoStates are being
        // assigned to the Group's TempoState
        let mut p_ids: Vec<Vec<usize>> = Vec::new();

        for (_, voice) in &mut voices {
            // if the Voice wasn't assigned a TempoState at birth,
            // it takes on the TempoState of the Group
            // (this is how a Voice's Process is synced with a Group's TempoState
            // [by proxy, if the Process refers to its Voice's TempoState])
            let mut p_i: Vec<usize> = Vec::new();

            if voice.tempo.mode == TempoMode::TBD {
                voice.tempo = TempoRepr::clone_owner(&tempo);
                v_flags.push(true);
                for (_, mut process) in &mut voice.processes {
                    // checks if any Process tempo has TempoMode::TBD
                    // (i.e. it was assigned to its Voice's
                    // uninitialized tempo, in anticipation of the
                    // Voice being added to a Group later)
                    match &process.tempo {
                        Some(t) => {
                            if t.mode == TempoMode::TBD {
                                process.tempo = Some(TempoRepr::clone_owner(&tempo));
                            }
                            p_i.push(process.idx);
                        }
                        None => (),
                    }
                }
            } else {
                v_flags.push(false);
            }

            p_ids.push(p_i);
        }

        let group = GroupRepr::new(self.engine_state.groups.len(), TempoRepr::clone(&tempo), voices);

        self.engine_state.groups.insert(name.to_string(), group);

        let mut vs_fs_ps: Vec<(usize, bool, Vec<usize>)> = 
            v_ids.into_iter()
                 .zip(v_flags)
                 .zip(p_ids)
                 .map(|((a, b), c) | (a, b, c))
                 .collect();
        // sort in reverse so that Voices are removed from
        // Conductor.voices in reverse
        vs_fs_ps.sort_by(|a, b| {
            let (va, _, _) = a;
            let (vb, _, _) = b;
            vb.cmp(va)
        });

        Ok(Command::Group(GroupArgs { tempo, vs_fs_ps }))
    }

    fn try_tc(&mut self, args: String) -> CmdResult<Command> {
        let mut args = args.split_whitespace();
        let name = args
            .next()
            .ok_or(CmdErr::MissingArg { 
                arg: "name".to_string(), 
                cmd: "tempocon".to_string() 
            })?;        

        let tempo = args
            .next()
            .ok_or(CmdErr::MissingArg {
                arg: "tempo".to_string(),
                cmd: "tempocon".to_string()
            })?;

        let tempo: Vec<_> = tempo.split(':').collect();

        if tempo.len() != 2 {
            return Err(CmdErr::TempoFormatting{});
        }

        let u = tempo.get(0).unwrap();
        let unit = match *u {
            "b" => TempoUnit::Bpm,
            "m" => TempoUnit::Millis,
            "s" => TempoUnit::Samples,
            _ => return Err(CmdErr::InvalidArg {
                               arg: u.to_string(),
                               cmd: "-t/--tempo".to_string(),
                            }),
        };

        let int_str = tempo.get(1).unwrap();
        let interval = int_str
                       .parse::<f32>()
                       .map_err(|_| CmdErr::InvalidArg {
                                    arg: tempo[1].to_owned(),
                                    cmd: "-t/--tempo".to_string(),
                       })?;

        let mut tempo_state = TempoRepr::new(self.engine_state.tempo_cons.len());
        tempo_state.init(TempoMode::Context, unit, interval);
        let ts_clone = TempoRepr::clone(&tempo_state);
        self.engine_state.tempo_cons.insert(name.to_string(), tempo_state);

        Ok(Command::Tc(TcArgs { tempo: ts_clone }))
    }

    // TODO: make able to apply to Group
    // TODO: implement naming Processes
    //       and replace insert("seq".to_string(), ...) with
    //       insert(name, ...)
    fn try_seq(&mut self, args: String) -> CmdResult<Command> {
        let mut args = args.split_whitespace();
        let name = args
            .next()
            .ok_or(CmdErr::MissingArg { 
                arg: "name".to_string(), 
                cmd: "seq".to_string() 
            })?;
        let name = name.to_string();

        // default assign to Process
        let mut tempo: TempoRepr = {
            // TODO: find object? needs to be more generic
            let voice = self.find_voice(name.clone())?;
            TempoRepr::new(voice.proc_tempi.len())
        };
        let mut period: usize = 4;
        let mut steps: Vec<f32> = Vec::new();
        let mut chance: Vec<f32> = Vec::new();
        let mut jit: Vec<f32> = Vec::new();
        // implement user-defined seed l8r
        let mut rng = X128P::new(fast_seed());

        while let Some(arg) = args.next() {
            match arg {
                "-t" | "--tempo" => {
                    let t_arg = args
                        .next()
                        .ok_or(CmdErr::MissingArg {
                            arg: "unit:interval".to_string(),
                            cmd: "seq -t".to_string(),
                        })?;

                    let t_args: Vec<_> = t_arg.split(':').collect();

                    if t_args.len() != 2 {
                        return Err(CmdErr::TempoFormatting{});
                    }

                    let u = t_args.get(0).unwrap();

                    if *u == "c" {
                        // find TempoContext
                        let tc_name = t_args.get(1).unwrap();
                        let tc_name = tc_name.to_string();
                        let tc = self.find_tc(tc_name)?;
                        tempo = TempoRepr::clone_owner(&tc);
                        continue;
                    }

                    if *u == "g" {
                        // find Group
                        let g_name = t_args.get(1).unwrap();
                        let g_name = g_name.to_string();
                        let g = self.find_group(g_name)?;
                        tempo = TempoRepr::clone_owner(&g.tempo);
                        continue;
                    }

                    if *u == "v" {
                        // refer to Voice's TempoState
                        tempo = {
                            let voice = self.find_voice(name.clone())?;
                            TempoRepr::clone_owner(&voice.tempo)
                        };
                        continue;
                    }

                    // if not referring, then init new TempoState
                    //
                    let unit = match *u {
                        "s" => TempoUnit::Samples,
                        "m" => TempoUnit::Millis,
                        "b" => TempoUnit::Bpm,
                        _ => return Err(CmdErr::InvalidArg {
                            arg: u.to_string(), 
                            cmd: "seq -t".to_string()
                        }),
                    };

                    let int_str = t_args.get(1).unwrap();
                    let interval = int_str
                                .parse::<f32>()
                                .map_err(|_| CmdErr::InvalidArg { 
                                    arg: int_str.to_string(), 
                                    cmd: "seq -t".to_string() 
                                })?;

                    tempo.init(TempoMode::Process, unit, interval);
                }
                "-p" | "--period" => {
                    period = args
                        .next()
                        .ok_or(CmdErr::MissingArg { 
                            arg: "value".to_string(), 
                            cmd: "seq -p".to_string() 
                        })
                        .and_then(|raw| 
                            raw.parse::<usize>()
                               .map_err(|_| CmdErr::InvalidArg { 
                                   arg: raw.to_owned(), 
                                   cmd: "seq -p".to_string() 
                               })
                        )?;
                }
                "-s" | "--steps" => {
                    let s_arg = args
                        .next()
                        .ok_or(CmdErr::MissingArg {
                            arg: "value".to_string(),
                            cmd: "seq -s".to_string(),
                        })?;

                    let step_strs: Vec<&str> = s_arg.split(',').collect();

                    for step in step_strs {
                        match step.parse::<f32>() {
                            Ok(val) => steps.push(val),
                            Err(_) => return Err(CmdErr::InvalidArg {
                                    arg: step.to_owned(),
                                    cmd: "seq -s".to_string(),
                                }),
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
                        return Err(CmdErr::Formatting {
                            err: "Must provide arguments to -s/--steps before -c/--chance or -j/--jitter".to_string()
                        });
                    }

                    let c_arg = args.next().ok_or(CmdErr::MissingArg {
                        arg: "value".to_string(),
                        cmd: "seq -c".to_string(),
                    })?;

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
                                        if at_index.len() != 2 {
                                            return Err(
                                                CmdErr::Formatting {
                                                    err: "Indexed chance arguments must be formatted beat:chance".to_string(),
                                                }
                                            );
                                        }

                                        // get chance first in case index = 'a'
                                        let chance_str = at_index.get(1).unwrap();
                                        let chance_val = chance_str
                                            .parse::<f32>()
                                            .map_err(|_| CmdErr::InvalidArg {
                                                arg: chance_str.to_string(),
                                                cmd: "seq -c".to_string(),
                                            })?;

                                        let index_str = at_index.get(0).unwrap();

                                        // if index = 'a', set all chance vals to chance_val and continue
                                        if *index_str == "a" {
                                            for i in {0..chance.len()} {
                                                chance[i] = chance_val;
                                            }
                                            continue;
                                        }

                                        let index = index_str
                                                    .parse::<f32>()
                                                    .map_err(|_| CmdErr::InvalidArg {
                                                        arg: index_str.to_string(),
                                                        cmd: "seq -c".to_string(),
                                                    })?;
                                        
                                        for i in {0..steps.len()} {
                                            let step = *steps.get(i).unwrap();
                                            if index == step {
                                                chance[i] = chance_val;
                                                break;
                                            }
                                            // only reaches here if index isn't found
                                            return Err(CmdErr::Formatting {
                                                err: "Invalid index for seq -c".to_string()
                                            });
                                        }
                                    }
                                    '-' => {
                                        let at_indices: Vec<&str> = string.split(':').collect();
                                        if at_indices.len() != 2 {
                                            return Err(
                                                CmdErr::Formatting {
                                                    err: "Ranged chance arguments must be formatted range:chance".to_string(),
                                                }
                                            );
                                        }
                                        
                                        let chance_str = at_indices.get(1).unwrap();
                                        let chance_val = chance_str
                                                         .parse::<f32>()
                                                         .map_err(|_| CmdErr::InvalidArg {
                                                            arg: chance_str.to_string(),
                                                            cmd: "seq -c".to_string(),
                                                         })?;

                                        let indices = at_indices.get(0).unwrap();
                                        let indices: Vec<&str> = indices.split('-').collect();
                                        if indices.len() != 2 {
                                            return Err(
                                                CmdErr::Formatting {
                                                    err: "Ranges must be formatted lower-upper".to_string(),
                                                }
                                            );
                                        }

                                        let i1_str = indices.get(0).unwrap();
                                        let idx1 = i1_str
                                                   .parse::<f32>()
                                                   .map_err(|_| CmdErr::InvalidArg {
                                                        arg: i1_str.to_string(),
                                                        cmd: "seq -c".to_string(),
                                                   })?;
                                        let i2_str = indices.get(0).unwrap();
                                        let idx2 = i2_str
                                                   .parse::<f32>()
                                                   .map_err(|_| CmdErr::InvalidArg {
                                                        arg: i2_str.to_string(),
                                                        cmd: "seq -c".to_string(),
                                                   })?;

                                        let mut lower = idx1;
                                        let mut upper = idx2;

                                        if lower > upper {
                                            lower = idx2;
                                            upper = idx1;
                                        }
                        
                                        // only check against lower because who cares if upper is too high
                                        if lower > *steps.get(steps.len() - 1).unwrap() {
                                            return Err(CmdErr::Formatting {
                                                err: "seq -c range applies to nothing".to_string()
                                            });
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
                                let chance_val = string
                                                 .parse::<f32>()
                                                 .map_err(|_| CmdErr::InvalidArg { 
                                                     arg: string.to_string(), 
                                                     cmd: "seq -c".to_string() 
                                                 })?;
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
                _ => return Err(CmdErr::InvalidArg { arg: arg.to_owned(), cmd: "seq".to_string() }),
            }
        }

        // TODO: allow for Idx::Group
        let voice = self.find_voice(name.clone())?;
        let repr = ProcRepr::new(
            voice.processes.len(), 
            Idx::Voice(voice.idx), 
            Some(TempoRepr::clone(&tempo))
        );
        voice.processes.insert("seq".to_string(), repr);
        // push tempo to proc_tempi only if owned by the Process
        if tempo.mode == TempoMode::Process {
            voice.proc_tempi.insert(
                voice.proc_tempi.len(), 
                TempoRepr::clone(&tempo)
            );
        }

        let args = SeqArgs {
            idx: Idx::Voice(voice.idx),
            tempo,
            period,
            steps,
            chance,
            jit,
            rng,
        };

        Ok(Command::Seq(args))
    }

    // StateResults (returned to a CmdResult fn)
    //
    fn parse_type_and_name(&self, args: String, cmd: String) -> StateResult<(String, String)> {
        let mut args = args.split_whitespace();
        let first = args
            .next()
            .ok_or(StateErr::MissingArg { 
                arg: "type and name".to_string(), 
                cmd: cmd.clone() 
            })?;
        let second = args
            .next()
            .ok_or(StateErr::MissingArg { 
                arg: "type or name".to_string(), 
                cmd: cmd
            })?;

        Ok((first.to_string(), second.to_string()))
    }

    fn get_idx(&mut self, ty: String, name: String) -> StateResult<Idx> {
        match ty.as_str() {
            "-v" | "--voice" => {
                let v = self.find_voice(name)?;
                Ok(Idx::Voice(v.idx))
            }
            "-g" | "--group" => {
                let g = self.find_group(name)?;
                Ok(Idx::Group(g.idx))
            }
            "-t" | "--tempocontext" => {
                let t = self.find_tc(name)?;
                Ok(Idx::Tempo(t.idx))
            }
            _ => return Err(StateErr::MissingArg { 
                arg: "type".to_string(), 
                cmd: "-v/-g/-t".to_string() 
            }),
        }
    }

    fn find_track(&mut self, name: String) -> StateResult<&mut TrackRepr> {
        self.engine_state.tracks
            .get_mut(&name)
            .ok_or(StateErr::NoItem { 
                ty: "track".to_string(), 
                name: name
            })
    }

    fn find_voice(&mut self, args: String) -> StateResult<&mut VoiceRepr> {      
        let mut args: Vec<&str> = args.split('.').collect();
        if args.len() > 2 {
            return Err(StateErr::Formatting { 
                err: "Too many delimiters for format group.voice".to_string() 
            });
        }

        // args will never be 0
        if args.len() == 1 {
            let v_name = args.get(0).unwrap();
            let v_name = v_name.to_string();
            self.engine_state.voices
                .get_mut(&v_name)
                .ok_or(StateErr::NoVoice { 
                    name: v_name, 
                    group: None 
                })
        } else {
            let group = args.get(0).unwrap();
            let group = group.to_string();
            let voice = args.get(1).unwrap();
            let voice = voice.to_string();

            match self.engine_state.groups.get_mut(&group) {
                Some(g) => {
                    g.voices.
                        get_mut(&voice)
                        .ok_or(StateErr::NoVoice { 
                            name: voice, 
                            group: Some(group)
                        })
                }
                None => {
                    return Err(StateErr::NoItem { 
                        ty: "Group".to_string(), 
                        name: group 
                    });
                }
            }
        }
    }

    fn find_group(&mut self, name: String) -> StateResult<&mut GroupRepr> {
        self.engine_state.groups.get_mut(&name)
            .ok_or(StateErr::NoItem { 
                ty: "Group".to_string(), 
                name: name
            })
    }

    fn find_tc(&mut self, name: String) -> StateResult<&mut TempoRepr> {
        self.engine_state.tempo_cons.get_mut(&name)
            .ok_or(StateErr::NoItem { 
                ty: "TempoContext".to_string(), 
                name: name 
            })
    }
}

// results and error handling
//
// ...for Commands
// (user-facing)
//
pub type CmdResult<Command> = Result<Command, CmdErr>;

// ...for states (*Reprs, Idx, args parsed internally, etc.)
// (private, but map directly to CmdErrs)
//
type StateResult<T> = Result<T, StateErr>;

// generate identical enums for CmdErr and StateErr
// and impl conversion from StateErr (internal) 
// to CmdErr (user-facing)
//
macro_rules! cmd_errors {
    ( $( $var:ident { $( $arg:ident : $type:ty ),* } ),* $(,)? ) => {
        #[derive(Debug)]
        pub enum CmdErr {
            $(
                $var { $( $arg: $type, )* },
            )*
        }

        #[derive(Debug)]
        enum StateErr {
            $(
                $var { $( $arg: $type, )* },
            )*
        }
        
        impl From<StateErr> for CmdErr {
            fn from(err: StateErr) -> Self {
                match err {
                    $(
                        StateErr::$var { $( $arg, )* } => {
                            CmdErr::$var { $( $arg, )* }
                        },
                    )*
                }
            }
        }
    }
}

cmd_errors! {
    TempoFormatting {},
    Formatting { err: String }, // generic
    MissingArg { arg: String, cmd: String },
    InvalidArg { arg: String, cmd: String },
    AlreadyIs { ty: String, name: String },
    NoCmd { cmd: String },
    NoItem { ty: String, name: String },
    NoVoice { name: String, group: Option<String> },
}

// display different messages based on error
//
use std::fmt;

impl fmt::Display for CmdErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmdErr::TempoFormatting {} => {
                write!(f, "-t/--tempo must be formatted as unit:interval")
            }
            CmdErr::Formatting { err } => {
                // verbatim, must explain in context
                write!(f, "{}", err)
            }
            CmdErr::MissingArg { arg, cmd } => {
                write!(f, "Missing {} for '{}'", arg, cmd)
            }
            CmdErr::InvalidArg { arg, cmd } => {
                write!(f, "Invalid argument '{}' for '{}'", arg, cmd)
            }
            CmdErr::AlreadyIs { ty, name } => {
                write!(f, "Already a {} called '{}'", ty, name)
            }
            CmdErr::NoCmd { cmd } => {
                write!(f, "Invalid command '{}'", cmd)
            }
            CmdErr::NoItem { ty, name } => {
                write!(f, "Couldn't find {} '{}'", ty, name)
            }
            CmdErr::NoVoice { name, group } => {
                match group {
                    Some(g_name) => write!(f, "Couldn't find Voice '{}' in Group '{}'", name, g_name),
                    None => write!(f, "Couldn't find Voice '{}'", name),
                }
            }
        }
    }
}
