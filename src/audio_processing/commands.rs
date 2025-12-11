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
        #[derive(Copy, Clone)]
        pub enum Command {
            $(
                $var(var_args!($var)), // formats as {CmdType}Args)
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
    voice_repr: VoiceRepr,
}

impl LoadArgs {
    fn new(track_idx: usize, voice_repr: VoiceRepr) -> Self {
        Self { track_idx, voice_repr }
    }
}

// doesn't need any members, just triggers raise(SIGTERM)
pub struct QuitArgs {}

// process commands outside of the audio thread

use crate::audio_processing::{
    blast_time::blast_time::{TempoMode, TempoUnit, TempoState},
};

struct TrackRepr {
    idx: usize,
    format: String,
    sample_rate: u32,
    num_channels: u32,
    bits_per_sample: u32,
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

struct TempoRepr { 
    idx: usize,
    mode: TempoMode,
    unit: TempoUnit,
    interval: f32,
    active: bool,
    current: u32,
}

struct VoiceRepr {
    idx: usize,
    active: bool,
    pos: f32,
    end: usize,
    vel: f32,
    gain: f32,
    tempo: TempoRepr,
    sr: u32,
    channels: usize,
    processes: HashMap<String, ProcRepr>,
    // don't need a representation of the proc_tempi
    // because those are in a Vec and only referenced by
    // the Processes themselves, and since Processes can't be
    // parsed outside of the engine (currently), figuring out
    // the proc_tempi state is just something the engine will handle
}

struct ProcRepr {
    // Processes are difficult to represent because they all
    // differ, so can only represent info that applies
    // to all Processes
    //
    idx: usize, // index of the Process in its owner's
                // Vec<Process>

    owner_idx: usize, // index of the Process's $owner
                      // in the engine's Vec<$owner>
    
    // this is all I can think of rn
}

struct GroupRepr {
    idx: usize,
    active: bool,
    gain: f32,
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

// and formats commands for the engine
// (handles string allocations, integer/float parsing, etc)
pub struct CmdProcessor {
    pub engine_state: EngineState,
}

impl CmdProcessor {
    pub fn new(engine_state: EngineState) -> Self {
        Self { engine_state }
    }
    
    pub fn parse(&mut self, cmd: String) -> Result<Command, String> {
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
            _ => {
                let err = "Invalid command ".to_owned() + &cmd;
                Err(err)
            }
        }
    }

    fn try_load(&mut self, args: String) -> Result<Command, String> {
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
        Ok(Command::Load { LoadArgs::new(track_idx, voice_repr) })
    }
}
