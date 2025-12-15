use std::rc::Rc;
use std::cell::RefCell;

use crate::audio_processing::{
    blast_rand::X128P,
    engine::VoiceState,
    blast_time::blast_time::{TempoState, TempoMode},
};

// Processes
//
macro_rules! processes {
    ( $( $variant:ident ),* $(,)? ) => {
        pub enum Process {
            $(
                $variant($variant),
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

            pub fn update_tempo(&mut self, ts: Rc<RefCell<TempoState>>) {
                match self {
                    $(
                        Process::$variant(inner) => inner.update_tempo(ts),
                    )*
                }
            }
        }
    };
}

processes! {
    Seq,
}

pub struct Seq {
    pub state: SeqState,
}

pub struct SeqState {
    pub active: bool, // TODO: impl activation methods
    pub period: usize,
    pub tempo: Rc<RefCell<TempoState>>,
    pub steps: Vec<f32>,
    pub chance: Vec<f32>,
    pub jit: Vec<f32>,
    pub rng: X128P, // TODO: impl user-defined seed
    pub idx: usize,
}

impl Seq {
    // right now only retriggers samples
    fn process(&mut self, voice: &mut VoiceState) {
        if !self.state.active { return; }

        let state = &mut self.state;
        let tempo = state.tempo.borrow();

        if !tempo.active { return; }

        let current = tempo.current() % state.period as f32;

        if current == state.steps[state.idx] {
            let rand = state.rng.next_i64_range(0, 100);
            if rand < state.chance[state.idx] as i64 {
                voice.position = match voice.velocity >= 0.0 {
                    true => 0.0,
                    false => voice.end as f32,
                };
            }
            state.idx += 1;
            state.idx %= state.steps.len();
        }
    }

    fn reset(&mut self) {
        self.state.idx = 0;
    }

    fn update_tempo(&mut self, ts: Rc<RefCell<TempoState>>) {
        self.state.tempo = ts;
    }
}
