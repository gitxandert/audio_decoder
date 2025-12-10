use std::collections::HashMap;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

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

#[derive(Copy, Clone)]
pub enum CmdArg {
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

unsafe impl Send for CmdArg {}
unsafe impl Sync for CmdArg {}

#[derive(Clone)]
pub struct Command {
    cmd: CmdArg,
    args: String,
}

unsafe impl Send for Command {}
unsafe impl Sync for Command {}

impl Command {
    pub fn new(cmd: CmdArg, args: String) -> Self {
        Self { cmd, args }
    }

    pub fn unwrap(&self) -> (CmdArg, String) {
        (self.cmd, self.args.clone())
    }
}

// need to process commands outside of the audio thread

use crate::audio_processing::{
    blast_time::blast_time::{TempoMode, TempoUnit, TempoState},
};

struct TrackRepr {
    idx: usize,
    file_name: String,
    format: String,
    sample_rate: u32,
    num_channels: u32,
    bits_per_sample: u32,
    // don't need the samples
}

impl TrackRepr {
    fn new(idx: usize, af: AudioFile) -> Self {
        Self {
            idx,
            file_name: af.file_name,
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
        
        match self.match_cmd(cmd) {
            Some(matched) => Ok(Command::new(matched, args)),
            None => Err("No command by that name".to_string()),
        }
    }

    fn match_cmd(&self, cmd: &str) -> Option<CmdArg> {
        match cmd {
            "load" => Some(CmdArg::Load),
            "start" => Some(CmdArg::Start),
            "pause" => Some(CmdArg::Pause),
            "resume" => Some(CmdArg::Resume),
            "stop" => Some(CmdArg::Stop),
            "unload" => Some(CmdArg::Unload),
            "velocity" => Some(CmdArg::Velocity),
            "group" => Some(CmdArg::Group),
            "tc" | "tempocon" => Some(CmdArg::TempoContext),
            "seq" => Some(CmdArg::Seq),
            "q" | "quit" => Some(CmdArg::Quit),
            _ => None,
        }
    }
}
