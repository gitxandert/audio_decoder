use std::collections::HashMap;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

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

    pub fn match_cmd(&self, cmd: &str) -> Option<CmdArg> {
        match cmd {
            "load" => Some(CmdArg::Load),
            "start" => Some(CmdArg::Start),
            "pause" => Some(CmdArg::Resume),
            "stop" => Some(CmdArg::Stop),
            "unload" => Some(CmdArg::Unload),
            "velocity" => Some(CmdArg::Velocity),
            "seq" => Some(CmdArg::Seq),
            "q" | "quit" => Some(CmdArg::Quit),
            _ => None,
        }
    }
}

#[derive(Copy, Clone)]
pub enum CmdArg {
    Load,
    Start,
    Pause,
    Resume,
    Stop,
    Unload,
    Velocity,
    Seq,
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
