use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering};

// sample_rate
// (mainly used by TempoState and TempoGroup)
//
pub mod sample_rate {
    use super::*;

    pub static SAMPLE_RATE: AtomicU32 = AtomicU32::new(0);

    pub fn set(sample_rate: u32) {
        SAMPLE_RATE.store(sample_rate, Ordering::Relaxed);
    }

    pub fn get() -> u32 {
        SAMPLE_RATE.load(Ordering::Relaxed)
    }
}

pub mod blast_time {
    use super::*;

    // global clock
    pub mod clock {
        use super::*;

        pub static SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

        pub fn advance(n: u64) {
            SAMPLE_COUNTER.fetch_add(n, Ordering::Relaxed);
        }

        pub fn current() -> u64 {
            SAMPLE_COUNTER.load(Ordering::Relaxed)
        }
    }
    // tempo control
    // 
    // a Process that relies on temporal parameters can be assigned
    // a TempoSolo to function at its own pace,
    // a TempoContext to synchronize with other separate Processes,
    // or a TempoGroup to default to the TempoState of the Group to which it
    //// would be assigned
    //
    // a TempoContext is created by a special command (tempocon/tc);
    // a TempoSolo is created along with the Process that instantiates it;
    // a TempoGroup is created along with the Group that instantiates it
    //
    // a TempoContext has a name that can be assigned to a Process or Group
    //
    // all TempoContexts are updated by the Conductor;
    // TempoSolos are updated by Voices, TempoGroups updated by Groups
    //
    // interval is stored as samples, but converted from
    // samples, milliseconds, or BPM, depending on initialization
    //
    #[derive(Debug)]
    pub struct TempoState {
        pub mode: TempoMode,
        pub unit: TempoUnit,
        pub interval: f32,
        pub active: bool,
        pub current: u32,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum TempoMode {
        Solo,
        Context,
        Group,
        TBD, // used by Voices and Groups;
             // Voices use this as a way to refer to a Group TempoState
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum TempoUnit {
        Samples,
        Millis,
        Bpm,
    }

    impl TempoState {
        pub fn new() -> Self {
            Self {
                // if mode is not provided (through init()), then it may change
                mode: TempoMode::TBD,
                unit: TempoUnit::Samples,
                interval: sample_rate::get() as f32,
                active: false,
                current: 0,
            }
        }

        pub fn init(&mut self, mode: TempoMode, unit: TempoUnit, interval: f32) {
            let interval_in_samps = convert_interval(&unit, interval);
            self.mode = mode;
            self.unit = unit; 
            self.interval = interval_in_samps;
        }

        pub fn clone(&self) -> TempoState {
            let mut clone = TempoState::new();
            clone.init(self.mode.clone(), self.unit.clone(), self.interval);
            clone
        }

        // store current as AtomicU32
        pub fn update(&mut self, delta_in_samples: f64) {
            self.current += delta_in_samples as u32;
        }

        // return current as f32
        pub fn current(&self) -> f32 {
            let step_f = self.current as f32 / self.interval;
            step_f
        }

        pub fn reset(&mut self) {
            self.current = 0;
        }

        pub fn set_interval(&mut self, new_interval: f32) {
            let new_interval_in_samps = convert_interval(&self.unit, new_interval);
            self.interval = new_interval_in_samps;
        }
    }

    fn convert_interval(unit: &TempoUnit, interval: f32) -> f32 {
        let frac = match unit {
            TempoUnit::Samples => return interval,
            TempoUnit::Millis => interval / 1000.0,
            TempoUnit::Bpm => 60.0 / interval,
        };
        
        let interval_in_samples = sample_rate::get() as f32 * frac;
       
        interval_in_samples
    }
}
