use std::rc::Rc;
use std::cell::RefCell;

use crate::audio_processing::{
    engine::VoiceState,
    gart_time::gart_time::TempoState,
};

// Processes 
macro_rules! declare_processes {
    ( $( $variant:ident => $ty:ty ),* $(,)? ) => {
        pub enum Process {
            $(
                $variant($ty),
            )*
        }

        impl Process {
            pub fn process(&mut self, voice: &mut VoiceState) {
                match self {
                    $(
                        Process::$variant(inner) => inner.process(voice),
                    )*
                }
            }

            pub fn reset(&mut self) {
                match self {
                    $(
                        Process::$variant(inner) => inner.reset(),
                    )*
                }
            }
        }
    };
}

declare_processes! {
    Seq => Seq,
}

pub struct Seq {
    pub state: SeqState,
}

pub struct SeqState {
    pub active: bool,
    pub period: usize,
    pub tempo: Rc<RefCell<TempoState>>,
    pub steps: Vec<f32>,
    pub chance: Vec<f32>,
    pub jit: Vec<f32>,
    pub seq_idx: usize,
}

impl Seq {
    // right now only retriggers samples
    fn process(&mut self, voice: &mut VoiceState) {
        let state = &mut self.state;
        if !state.active { return; }

        let tempo = state.tempo.borrow();

        if !tempo.active { return; }

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
