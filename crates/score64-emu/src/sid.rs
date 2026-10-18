//! The SID, behind a [`SidEngine`] trait (ADR-011), clocked lazily: it only runs when something needs its state or when
//! audio is due.
//!
//! The default engine is reSIDfp (libresidfp 1.2.3 through `residfp-sys`) with sidplayfp 3.2.2's settings, so renders
//! sound like the oracle (#59). `resid-rs` (a reSID 0.16 port) stays available for comparison until #59 lands.

use residfp_sys::{ChipModel, CombinedWaveforms, ResidFp, SamplingMethod};

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

/// Which SID emulation to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// reSIDfp (libresidfp 1.2.3), the default.
    Residfp,
    /// resid-rs 1.1.1, a Rust port of reSID 0.16.
    ResidRs,
}

impl std::str::FromStr for Engine {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "residfp" => Ok(Engine::Residfp),
            "resid-rs" => Ok(Engine::ResidRs),
            other => Err(format!(
                "unknown SID engine {other:?} (expected residfp or resid-rs)"
            )),
        }
    }
}

/// A SID emulation: registers in, audio out.
pub trait SidEngine {
    /// Write a register (`$00`–`$1C`).
    fn write(&mut self, register: u8, value: u8);
    /// Read a register at the current cycle.
    fn read(&mut self, register: u8) -> u8;
    /// Run `cycles` CPU cycles and append the audio produced.
    fn clock(&mut self, cycles: u64, out: &mut Vec<i16>);
}

/// sidplayfp 3.2.2's reSIDfp settings (its `IniConfig` defaults).
const FILTER_CURVE_6581: f64 = 0.5;
const FILTER_RANGE_6581: f64 = 0.5;
const FILTER_CURVE_8580: f64 = 0.5;

struct Residfp(ResidFp);

impl Residfp {
    fn new(chip: Chip, clock_hz: u32, sample_rate: u32) -> Result<Self, String> {
        let model = match chip {
            Chip::Mos6581 => ChipModel::Mos6581,
            Chip::Mos8580 => ChipModel::Csg8580,
        };
        let mut sid = ResidFp::new(
            model,
            f64::from(clock_hz),
            f64::from(sample_rate),
            SamplingMethod::Resample,
        )
        .map_err(|e| e.to_string())?;
        sid.set_combined_waveforms(CombinedWaveforms::Average)
            .map_err(|e| e.to_string())?;
        sid.set_filter_6581_curve(FILTER_CURVE_6581);
        sid.set_filter_6581_range(FILTER_RANGE_6581);
        sid.set_filter_8580_curve(FILTER_CURVE_8580);
        Ok(Self(sid))
    }
}

impl SidEngine for Residfp {
    fn write(&mut self, register: u8, value: u8) {
        self.0.write(register, value);
    }

    fn read(&mut self, register: u8) -> u8 {
        self.0.read(register)
    }

    fn clock(&mut self, cycles: u64, out: &mut Vec<i16>) {
        self.0.clock(cycles, out);
    }
}

struct ResidRs(resid::Sid);

impl ResidRs {
    fn new(chip: Chip, clock_hz: u32, sample_rate: u32) -> Self {
        let model = match chip {
            Chip::Mos6581 => resid::ChipModel::Mos6581,
            Chip::Mos8580 => resid::ChipModel::Mos8580,
        };
        let mut sid = resid::Sid::new(model);
        sid.set_sampling_parameters(resid::SamplingMethod::Resample, clock_hz, sample_rate);
        Self(sid)
    }
}

impl SidEngine for ResidRs {
    fn write(&mut self, register: u8, value: u8) {
        self.0.write(register, value);
    }

    fn read(&mut self, register: u8) -> u8 {
        self.0.read(register)
    }

    fn clock(&mut self, mut cycles: u64, out: &mut Vec<i16>) {
        let mut buffer = [0i16; 4096];
        while cycles > 0 {
            let delta = u32::try_from(cycles).unwrap_or(u32::MAX);
            let mut left = delta;
            while left > 0 {
                let (n, rest) = self.0.sample(left, &mut buffer, 1);
                out.extend_from_slice(&buffer[..n]);
                left = rest;
            }
            cycles -= u64::from(delta);
        }
    }
}

/// A SID engine plus the cycle it has been clocked to and the audio it has produced.
pub struct SidClock {
    engine: Box<dyn SidEngine>,
    now: u64,
    /// Mono 16-bit samples produced so far.
    pub samples: Vec<i16>,
    /// Voices (bit 0 = voice 1) kept silent, for listening to or measuring one voice at a time.
    pub muted: u8,
}

impl SidClock {
    /// A SID of the given engine and model, producing audio at `sample_rate` from a CPU clock of `clock_hz`.
    pub fn new(
        engine: Engine,
        chip: Chip,
        clock_hz: u32,
        sample_rate: u32,
    ) -> Result<Self, String> {
        let engine: Box<dyn SidEngine> = match engine {
            Engine::Residfp => Box::new(Residfp::new(chip, clock_hz, sample_rate)?),
            Engine::ResidRs => Box::new(ResidRs::new(chip, clock_hz, sample_rate)),
        };
        Ok(Self {
            engine,
            now: 0,
            samples: Vec::new(),
            muted: 0,
        })
    }

    /// Run the SID up to `cycle`, keeping the audio. Does nothing if it is already there.
    pub fn advance_to(&mut self, cycle: u64) {
        if self.now < cycle {
            self.engine.clock(cycle - self.now, &mut self.samples);
            self.now = cycle;
        }
    }

    /// Write a register at `cycle` (which must not be in the past).
    pub fn write_at(&mut self, cycle: u64, register: u8, value: u8) {
        self.advance_to(cycle);
        // A muted voice keeps its oscillator (it may still sync or ring-modulate another voice) but never gates and
        // has no waveform selected.
        let value = match register {
            0x04 | 0x0B | 0x12 if self.muted & (1 << (register / 7)) != 0 => value & 0x06,
            _ => value,
        };
        self.engine.write(register, value);
    }

    /// Read a register at the current cycle.
    pub fn read(&mut self, register: u8) -> u8 {
        self.engine.read(register)
    }
}
