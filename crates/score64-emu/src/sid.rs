//! The SID (`resid-rs`), clocked lazily: it only runs when something needs its state or when audio is due.

use resid::{ChipModel, SamplingMethod, Sid};

/// Which SID revision to emulate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    /// MOS 6581 (original).
    Mos6581,
    /// MOS 8580 (later revision).
    Mos8580,
}

impl std::str::FromStr for Chip {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "6581" => Ok(Chip::Mos6581),
            "8580" => Ok(Chip::Mos8580),
            other => Err(format!(
                "unknown SID model {other:?} (expected 6581 or 8580)"
            )),
        }
    }
}

/// A SID plus the cycle it has been clocked to and the audio it has produced.
pub struct SidClock {
    sid: Sid,
    now: u64,
    /// Mono 16-bit samples produced so far.
    pub samples: Vec<i16>,
}

impl SidClock {
    /// A SID of the given model, producing audio at `sample_rate` from a CPU clock of `clock_hz`.
    pub fn new(chip: Chip, clock_hz: u32, sample_rate: u32) -> Self {
        let model = match chip {
            Chip::Mos6581 => ChipModel::Mos6581,
            Chip::Mos8580 => ChipModel::Mos8580,
        };
        let mut sid = Sid::new(model);
        sid.set_sampling_parameters(SamplingMethod::Resample, clock_hz, sample_rate);
        Self {
            sid,
            now: 0,
            samples: Vec::new(),
        }
    }

    /// Run the SID up to `cycle`, keeping the audio. Does nothing if it is already there.
    pub fn advance_to(&mut self, cycle: u64) {
        let mut buffer = [0i16; 4096];
        while self.now < cycle {
            let delta = u32::try_from(cycle - self.now).unwrap_or(u32::MAX);
            let mut left = delta;
            while left > 0 {
                let (n, rest) = self.sid.sample(left, &mut buffer, 1);
                self.samples.extend_from_slice(&buffer[..n]);
                left = rest;
            }
            self.now += u64::from(delta);
        }
    }

    /// Write a register at `cycle` (which must not be in the past).
    pub fn write_at(&mut self, cycle: u64, register: u8, value: u8) {
        self.advance_to(cycle);
        self.sid.write(register, value);
    }

    /// Read a register at the current cycle.
    pub fn read(&self, register: u8) -> u8 {
        self.sid.read(register)
    }
}
