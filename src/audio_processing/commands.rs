use std::collections::HashMap;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use proc_macro::{TokenStream, TokenTree, Ident, Span};

use crate::file_parsing::decode_helpers::AudioFile;

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

#[proc_macro]
pub fn var_args(var: TokenStream) -> TokenStream {
    let var = var.to_string().trim().to_string();
    let var_args = Ident::new(&format!("{}Args", var), Span::call_site());

    TokenStream::from(TokenTree::Ident(var_args))
}

macro_rules! commands {
    ( $( $var:ident ),* $(,)? ) => {
        #[derive(Copy, Clone, Debug)]
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
    TempoContext,
    // Processes
    Seq,
    // Program
    Quit,
}

pub struct LoadArgs {
    track_idx: usize,
    tempo_repr: TempoRepr,
}

pub struct StartArgs {
    idx: Idx,
}

pub struct PauseArgs {
    idx: Idx,
}

pub struct ResumeArgs {
    idx: Idx,
}

pub struct StopArgs {
    idx: Idx,
}

pub struct UnloadArgs {
    idx: usize,
}

pub struct VelocityArgs {
    idx: usize,
    val: f32,
}

pub struct GroupArgs {
    tempo: TempoRepr,
    vs_fs_ps: Vec<(usize, bool, Vec<usize>)>, 
    // store the ids Voice
    // with whether or not its TempoState refers to the Group's
    // and with the ids of all of the Processes 
    // whose TempoStates refer to the Group's
}

// doesn't need any members, just triggers raise(SIGTERM)
pub struct QuitArgs {}

// process commands outside of the audio thread

use crate::audio_processing::{
    blast_time::blast_time::{TempoMode, TempoUnit, TempoState},
};

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
    fn new(idx: usize, af: AudioFile) -> Self {
        Self {
            idx,
            format: af.format,
            sample_rate: af.sample_rate,
            num_channels: af.num_channels,
            bits_per_sample: af.bits_per_sample,
        }
    }
}

pub struct TempoRepr { 
    idx: usize,
    // The function of idx varies depending on the mode and
    // the Command that instantiates it.
    // e.g. if this is instantiated with Load
    // (i.e. if it belongs to a new Voice), then idx represents
    // the position of the Voice in the engine's Vec<Voice>
    mode: TempoMode,
    unit: TempoUnit,
    interval: f32,
}

impl TempoRepr {
    fn new(idx: usize) -> Self {
        Self {
            idx,
            mode: TempoMode::TBD,
            unit: TempoUnit::Samples,
            interval: 0f32,
        }
    }

    fn clone(other: &TempoRepr) -> Self {
        Self {
            idx: other.idx,
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

    owner_idx: usize, // index of the Process's $owner
                      // in the engine's Vec<$owner>
    
    tempo: Option<TempoRepr>,
    // maybe create ProcArgs enum, one for each Process
}

pub struct GroupRepr {
    idx: usize,
    tempo: TempoRepr,
    voices: HashMap<String, VoiceRepr>,
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
            tracks.insert(af.file_name.clone(), TrackRepr::new(idx, af.clone()));
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
            "q" | "quit" => Ok(Command::Quit { QuitArgs }),
            _ => return Err(CmdErr::NoCmd { name: cmd.to_owned() }),
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

        let mut track_idx = self.find_track(name.clone())?;
        
        // initialize tempo_repr with an idx of 0 because
        // a Voice will only ever have one personal TempoState
        let mut tempo_repr = TempoRepr::new(0usize);

        // if a Voice by this name (currently the track name)
        // already exists, then return error
        match self.find_voice(name.clone()) {
            Ok(voice) => return Err(CmdErr::AlreadyIs { 
                ty: "Voice".to_string(), 
                name: name.to_owned() 
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
                        
                        tempo_repr = match self.find_tc(tc_name) {
                            Ok(tc) => TempoRepr::clone(&tc),
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

                        tempo_repr = match self.find_group(g_name) {
                            Ok(g) => TempoRepr::clone(&g.tempo),
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
        let idx = self.voices.len();
        self.voices.insert(VoiceRepr::new(idx, TempoRepr::clone(&tempo_repr));
        
        Ok(Command::Load(LoadArgs{track_idx, tempo_repr}))
    }

    // the following could start multiple things at the same time
    // (e.g. *Args could hold a Vec<Idx>);
    // maybe implement "all" as a reserved word
    //
    fn try_start(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(args)?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Start(StartArgs{ idx }))
    }

    fn try_pause(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(args)?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Pause(PauseArgs{ idx }))
    } 

    fn try_resume(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(args)?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Resume(ResumeArgs{ idx }))
    }  

    fn try_stop(&mut self, args: String) -> CmdResult<Command> {
        let (ty, name) = self.parse_type_and_name(args)?;
        let idx = self.get_idx(ty, name)?;
        Ok(Command::Stop(StopArgs{ idx }))
    } 

    fn try_unload(&mut self, args: Name) -> CmdResult<Command> {
        // gets idx and removes VoiceRepr from self.voices
        let idx = match self.voices.entry(name.clone().to_string()) {
            Entry::Occupied(e) => {
                let e_idx = e.idx;
                e.remove();
                e
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
        for (_, voice) in self.voices {
            if voice.idx > idx {
                voice.idx -= 1;
            }
        }

        Ok(Command::Unload(UnloadArgs{ idx }))
    }

    fn try_velocity(&mut self, args: &str) -> CmdResult<Command> {
        let mut args = args.splitn(2, ' ');
        
        let name = args
            .next()
            .ok_or(CmdErr::MissingArg{ 
                arg: "name".to_string(), 
                cmd: "velocity".to_string() 
            })?;
        
        let idx = self.get_idx("-v", name)?;

        let val = args
            .next()
            .ok_or(CmdErr::MissingArg{ 
                arg: "value".to_string(), 
                cmd: "velocity".to_string() 
            })
            .and_then(|raw| {
                raw.parse::<f32>()
                    .map_err(|_| CmdErr::InvalidArg{ 
                        arg: raw.to_owned(), 
                        cmd: "velocity".to_string() 
                    })
            })?;

        Ok(Command::Velocity(VelocityArgs{ idx, val }))
    }

    fn try_group(&mut self, args: &str) -> CmdResult<Command> {
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

        let mut t_arg = |t: &str| -> StateResult<TempoRepr> {
            let mut t_args = t.split(':');
            let u_str = t_args
                .next()
                .ok_or(StateErr::MissingArg { 
                    arg: "unit".to_string(), 
                    cmd: "group -t".to_string() 
                })?;
            
            if u_str == "c" {
                let tc_name = t_args
                    .next()
                    .ok_or(StateErr::MissingArg { 
                        arg: "TempoContext name".to_string(), 
                        cmd: "group -t".to_string() 
                    })?;

                match self.find_tc(name) {
                    Ok(tc) => return Ok(TempoRepr::clone(&tc)),
                    Err(error) => return Err(error.into()),
                }
            } else {
                let unit = match u_str {
                    "s" => TempoUnit::Samples,
                    "m" => TempoUnit::Millis,
                    "b" => TempoUnit::Bpm,
                    _ => return Err(StateErr::InvalidArg { 
                        arg: u_str.to_owned(), 
                        cmd: "group -t".to_string() 
                    }),
                };
                let interval = t_args.next()
                    .ok_or(StateErr::MissingArg { arg: "interval".to_string(), cmd: "group -t".to_string() })
                    .and_then(|raw| {
                        raw.parse::<f32>()
                            .map_err(|_| StateErr::InvalidArg { 
                                arg: raw.to_owned(), 
                                cmd: "group -t".to_string() 
                            })
                    })?;
                
                let mut new_tempo = TempoRepr::new(0);
                new_tempo.init(TempoMode::Group, unit, interval);
                Ok(new_tempo)
            }
        };

        let mut v_arg = |v: &str| -> StateResult<()> {
            let names: Vec<_> = v.split(',').collect();

            // need to collect all indices of the Voices that
            // are being removed; then sort high to low
            // and decrement all other indices -ge
            for name in names {
                let name = name.to_string();
                match self.voices.remove(&name) {
                    Some(mut voice) => {
                        v_ids.push(voice.idx);
                        voice.idx = voices.len(); // assign new index
                                                  // in Group's Vec
                        voices.insert(name, voice);
                    }
                    None => return Err(StateErr::NoVoice { 
                        name: name.to_owned(), 
                        group: None 
                    }),
                }
            }

            let sorted = v_ids.sort_by(|a, b| b.cmp(a));

            for removed in sorted {
                for (_, v) in &mut self.voices {
                    if v.idx > removed {
                        v.idx -= 1;
                    }
                }
            }

            Ok(());
        };

        while let Some(arg) = args.next() {
            match arg {
                "-t" | "--tempo" => {
                    match args.next() {
                        Some(t) => tempo = t_arg(t, tempo_state)?, 
                        None => return Err(CmdErr::MissingArg { 
                            arg: "arguments".to_string(), 
                            cmd: "group -t".to_string() 
                        }),
                    }
                }
                "-v" | "--voices" => {
                    match args.next() {
                        Some(v) => let _ = v_arg(v)?,
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
        let v_flags: Vec<bool> = Vec::new();
        // collect indices of Processes whose TempoStates are being
        // assigned to the Group's TempoState
        let p_ids: Vec<Vec<usize> = Vec::new();

        for (_, voice) in &mut voices {
            // if the Voice wasn't assigned a TempoState at birth,
            // it takes on the TempoState of the Group
            // (this is how a Voice's Process is synced with a Group's TempoState
            // [by proxy, if the Process refers to its Voice's TempoState])
            let p_i: Vec<usize> = Vec::new();

            if voice.tempo.mode == TempoMode::TBD {
                voice.tempo = TempoRepr::clone(&tempo);
                v_flags.push(true);
                for (_, mut process) in &mut voice.processes {
                    // checks if any Process tempo has TempoMode::TBD
                    // (i.e. it was assigned to its Voice's
                    // uninitialized tempo, in anticipation of the
                    // Voice being added to a Group later)
                    match process.tempo {
                        Some(t) => {
                            if t.mode == TempoMode::TBD {
                                t = TempoRepr::clone(&tempo));
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

        let group = GroupRepr::new(self.groups.len(), voices, TempoRepr::clone(&tempo));

        self.groups.insert(name.to_string(), group);

        let vs_ps_fs: Vec<(usize, bool, Vec<usize>)> = 
            v_ids.into_iter()
                 .zip(v_flags)
                 .zip(p_ids)
                 .map(|((a, b), c) | (a, b, c))
                 .collect();

        Ok(Command::Group(GroupArgs { tempo, vs_fs_ps }))
    }

    // StateResults (returned to a CmdResult fn)
    //
    fn parse_type_and_name(args: &str) -> StateResult<(&str, &str)> {
        let mut args = args.split_whitespace();
        let first = args
            .next()
            .ok_or(StateErr::MissingArg { 
                arg: "type".to_string(), 
                cmd: "resume".to_string() 
            })?;
        let second = args
            .next()
            .ok_or(StateErr::MissingArg { 
                arg: "name".to_string(), 
                cmd: "resume".to_string() 
            })?;

        Ok((first, second))
    }

    fn get_idx(&mut self, ty: &str, name: &str) -> StateResult<Idx> {
        match ty {
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

    fn find_track(&mut self, name: &str) -> StateResult<&mut TrackRepr> {
        self.tracks
            .get_mut(&name.clone().to_string())
            .ok_or(StateErr::NoItem { 
                ty: "track".to_string(), 
                name: name.to_owned() 
            })
    }

    fn find_voice(&mut self, args: &str) -> StateResult<&mut VoiceRepr> {      
        let mut args: Vec<&str> = args.split('.').collect();
        if args.len() > 2 {
            return Err(StateErr::Formatting { 
                err: "Too many delimiters for format group.voice".to_string() 
            });
        }

        // args will never be 0
        if args.len() == 1 {
            let voice = args.get(0).unwrap();
            self.voices
                .get_mut(&voice.to_string())
                .ok_or(StateErr::NoVoice { 
                    name: voice.to_owned(), 
                    group: None 
                })
        } else {
            let group = args.get(0).unwrap();
            let group = group.to_string();
            let voice = args.get(1).unwrap();

            match self.groups.get_mut(&group) {
                Some(g) => {
                    g.voices.
                        get_mut(&voice.clone().to_string())
                        .ok_or(StateErr::NoVoice { 
                            name: voice.to_owned(), 
                            group: group.to_owned() 
                        })
                }
                None => {
                    return Err(StateErr::NoItem { 
                        ty: "Group".to_string(), 
                        name: group.to_owned() 
                    });
                }
            }
        }
    }

    fn find_group(&mut self, name: &str) -> StateResult<&mut GroupRepr> {
        self.groups.get_mut(&name.clone().to_string())
            .ok_or(StateErr::NoItem { 
                ty: "Group".to_string(), 
                name: name.to_owned() 
            })
    }

    fn find_tc(&mut self, name: &str) -> StateResult<&mut TempoRepr> {
        self.tempo_cons.get_mut(&name.clone().to_string())
            .ok_or(StateErr::NoItem { 
                ty: "TempoContext".to_string(), 
                name: name.to_owned() 
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
    ( $( $var:ident { $( $arg:ident : $type:ty ),* ),* $(,)? ) => {
        #[derive(Debug)]
        pub enum CmdErr {
            $(
                $var { $( $arg: $type, )* },
            )
        }

        #[derive(Debug)]
        enum StateErr {
            $(
                $var { $( $arg: $type, )* },
            )
        }
        
        impl From<StateErr> for CmdErr {
            fn from(err: StateErr) -> Self {
                match err {
                    $(
                        StateErr::$var { $( $arg, )* } => {
                            CmdErr::$var { $( $arg, )* }
                        },
                    )
                }
            }
        }
    }
}

cmd_errors! {
    Formatting { err: String },
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
            CmdErr::Formatting { err } => {
                // verbatim, must explain in context
                write!(f, "{}", err)
            }
            CmdErr::MissingArg { arg, cmd } => {
                write!(f, "Missing {} for '{}'", cmd)
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
