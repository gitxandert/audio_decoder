use std::cell::RefCell;
use std::sync::{
    Arc, Mutex, 
    atomic::{AtomicBool, Ordering},
};

use crate::audio_processing::{
    engine::VoiceState,
    gart_time::gart_time::TempoState,
};

// Processes 
//
pub trait Process: Send {
    fn process(&mut self, voice: &mut VoiceState);
    fn reset(&mut self);
}

pub struct Seq {
    pub state: SeqState,
}

pub struct SeqState {
    pub active: AtomicBool,
    pub period: usize,
    pub tempo: RefCell<TempoState>,
    pub steps: Vec<f32>,
    pub chance: Vec<f32>,
    pub jit: Vec<f32>,
    pub seq_idx: usize,
}

impl Process for Seq {
    // right now only retriggers samples
    fn process(&mut self, voice: &mut VoiceState) {
        let state = &mut self.state;
        if !state.active.load(Ordering::Relaxed) { return; }

        let tempo = state.tempo.borrow();

        if !tempo.active.load(Ordering::Relaxed) { return; }

        let current = tempo.current() % state.period as f32;

        if current == state.steps[state.seq_idx] {
            voice.position = match voice.velocity >= 0.0 {
                true => 0.0,
                false => voice.end as f32,
            };
            state.seq_idx += 1;
            state.seq_idx %= state.steps.len();
        }
    }

    fn reset(&mut self) {
        self.state.seq_idx = 0;
    }
}
