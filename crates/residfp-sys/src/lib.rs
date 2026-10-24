//! reSIDfp (libresidfp 1.2.3, GPL-2.0-or-later) for Score64's emulator helper (ADR-011).
//!
//! The library is vendored in `vendor/libresidfp` with one documented patch (`vendor/SCORE64-PATCHES.md`) and built
//! by `build.rs`. `unsafe` stays in [`ffi`]; [`ResidFp`] is the safe interface. Only `score64-emu` may depend on this
//! crate (ADR-002).

/// SID revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipModel {
    /// MOS 6581.
    Mos6581,
    /// CSG 8580.
    Csg8580,
}

/// How strongly combined waveforms (for example pulse + sawtooth) come out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombinedWaveforms {
    /// The default, and sidplayfp's.
    Average,
    /// Weaker.
    Weak,
    /// Stronger.
    Strong,
}

/// Output sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplingMethod {
    /// Linear interpolation: fast, lower quality.
    Decimate,
    /// Sinc resampling: high quality (sidplayfp's default).
    Resample,
}

/// A setting the library rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejected(pub &'static str);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "reSIDfp rejected {}", self.0)
    }
}

impl std::error::Error for Rejected {}

/// One emulated SID.
pub struct ResidFp {
    raw: ffi::Handle,
}

impl ResidFp {
    /// A SID of the given model, sampling at `sample_rate` from a CPU clock of `clock_hz`.
    pub fn new(
        model: ChipModel,
        clock_hz: f64,
        sample_rate: f64,
        method: SamplingMethod,
    ) -> Result<Self, Rejected> {
        let raw = ffi::Handle::new().ok_or(Rejected("allocation"))?;
        let mut sid = Self { raw };
        sid.set_chip_model(model)?;
        if !sid.raw.set_sampling(clock_hz, method, sample_rate) {
            return Err(Rejected("the sampling parameters"));
        }
        Ok(sid)
    }

    /// Change the chip model.
    pub fn set_chip_model(&mut self, model: ChipModel) -> Result<(), Rejected> {
        if self.raw.set_chip_model(model) {
            Ok(())
        } else {
            Err(Rejected("the chip model"))
        }
    }

    /// Combined-waveform strength.
    pub fn set_combined_waveforms(&mut self, cws: CombinedWaveforms) -> Result<(), Rejected> {
        if self.raw.set_combined_waveforms(cws) {
            Ok(())
        } else {
            Err(Rejected("the combined-waveform strength"))
        }
    }

    /// 6581 filter curve, 0 (dark) to 1 (bright); sidplayfp's default is 0.5.
    pub fn set_filter_6581_curve(&mut self, v: f64) {
        self.raw.set_filter_6581_curve(v);
    }

    /// 6581 filter range adjustment, 0 to 1; sidplayfp's default is 0.5.
    pub fn set_filter_6581_range(&mut self, v: f64) {
        self.raw.set_filter_6581_range(v);
    }

    /// 8580 filter curve, 0 (dark) to 1 (bright); sidplayfp's default is 0.5.
    pub fn set_filter_8580_curve(&mut self, v: f64) {
        self.raw.set_filter_8580_curve(v);
    }

    /// Turn filter emulation on or off.
    pub fn enable_filter(&mut self, enable: bool) {
        self.raw.enable_filter(enable);
    }

    /// Reset the chip.
    pub fn reset(&mut self) {
        self.raw.reset();
    }

    /// Write a register (`$00`–`$1F`).
    pub fn write(&mut self, register: u8, value: u8) {
        self.raw.write(register & 0x1F, value);
    }

    /// Read a register (`$00`–`$1F`).
    pub fn read(&mut self, register: u8) -> u8 {
        self.raw.read(register & 0x1F)
    }

    /// Run the chip for `cycles` CPU cycles and append the audio produced to `out`.
    pub fn clock(&mut self, mut cycles: u64, out: &mut Vec<i16>) {
        // Chunks keep the scratch buffer small. The library emits at most one sample per cycle, so a buffer of CHUNK
        // samples is always enough. (Not cycles * rate / clock: the resampler rounds its step down and emits slightly
        // more than that, about 0.03% at 192 kHz.)
        const CHUNK: u32 = 16_384;
        let mut buf = vec![0i16; CHUNK as usize];
        while cycles > 0 {
            let n = cycles.min(u64::from(CHUNK)) as u32;
            let produced = self.raw.clock(n, &mut buf);
            out.extend_from_slice(&buf[..produced]);
            cycles -= u64::from(n);
        }
    }
}

#[allow(unsafe_code)]
mod ffi {
    //! The C shim (`shim/shim.cpp`) and the only `unsafe` code in the crate.

    use super::{ChipModel, CombinedWaveforms, SamplingMethod};
    use std::os::raw::{c_double, c_int, c_short, c_uchar, c_uint};

    #[repr(C)]
    pub struct Opaque {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        fn score64_residfp_new() -> *mut Opaque;
        fn score64_residfp_free(sid: *mut Opaque);
        fn score64_residfp_set_chip_model(sid: *mut Opaque, model: c_int) -> c_int;
        fn score64_residfp_set_combined_waveforms(sid: *mut Opaque, cws: c_int) -> c_int;
        fn score64_residfp_set_sampling(
            sid: *mut Opaque,
            clock: c_double,
            method: c_int,
            rate: c_double,
        ) -> c_int;
        fn score64_residfp_set_filter_6581_curve(sid: *mut Opaque, v: c_double);
        fn score64_residfp_set_filter_6581_range(sid: *mut Opaque, v: c_double);
        fn score64_residfp_set_filter_8580_curve(sid: *mut Opaque, v: c_double);
        fn score64_residfp_enable_filter(sid: *mut Opaque, enable: c_int);
        fn score64_residfp_reset(sid: *mut Opaque);
        fn score64_residfp_write(sid: *mut Opaque, reg: c_int, value: c_uchar);
        fn score64_residfp_read(sid: *mut Opaque, reg: c_int) -> c_uchar;
        fn score64_residfp_clock(sid: *mut Opaque, cycles: c_uint, buf: *mut c_short) -> c_int;
    }

    /// Owns a `reSIDfp::residfp`. Not `Send`/`Sync` (raw pointer), matching the library: one thread per instance.
    pub struct Handle(*mut Opaque);

    impl Handle {
        pub fn new() -> Option<Self> {
            // SAFETY: allocates a fresh object or returns null; no other preconditions.
            let p = unsafe { score64_residfp_new() };
            (!p.is_null()).then_some(Self(p))
        }

        pub fn set_chip_model(&mut self, model: ChipModel) -> bool {
            let m = match model {
                ChipModel::Mos6581 => 0,
                ChipModel::Csg8580 => 1,
            };
            // SAFETY: self.0 is a live object owned by this handle (invariant of Handle).
            unsafe { score64_residfp_set_chip_model(self.0, m) == 1 }
        }

        pub fn set_combined_waveforms(&mut self, cws: CombinedWaveforms) -> bool {
            let v = match cws {
                CombinedWaveforms::Average => 0,
                CombinedWaveforms::Weak => 1,
                CombinedWaveforms::Strong => 2,
            };
            // SAFETY: as above.
            unsafe { score64_residfp_set_combined_waveforms(self.0, v) == 1 }
        }

        pub fn set_sampling(&mut self, clock: f64, method: SamplingMethod, rate: f64) -> bool {
            let m = match method {
                SamplingMethod::Decimate => 0,
                SamplingMethod::Resample => 1,
            };
            // SAFETY: as above.
            unsafe { score64_residfp_set_sampling(self.0, clock, m, rate) == 1 }
        }

        pub fn set_filter_6581_curve(&mut self, v: f64) {
            // SAFETY: as above.
            unsafe { score64_residfp_set_filter_6581_curve(self.0, v) }
        }

        pub fn set_filter_6581_range(&mut self, v: f64) {
            // SAFETY: as above.
            unsafe { score64_residfp_set_filter_6581_range(self.0, v) }
        }

        pub fn set_filter_8580_curve(&mut self, v: f64) {
            // SAFETY: as above.
            unsafe { score64_residfp_set_filter_8580_curve(self.0, v) }
        }

        pub fn enable_filter(&mut self, enable: bool) {
            // SAFETY: as above.
            unsafe { score64_residfp_enable_filter(self.0, c_int::from(enable)) }
        }

        pub fn reset(&mut self) {
            // SAFETY: as above.
            unsafe { score64_residfp_reset(self.0) }
        }

        pub fn write(&mut self, reg: u8, value: u8) {
            // SAFETY: as above; the register is masked to 0..=31 by the caller.
            unsafe { score64_residfp_write(self.0, c_int::from(reg), value) }
        }

        pub fn read(&mut self, reg: u8) -> u8 {
            // SAFETY: as above.
            unsafe { score64_residfp_read(self.0, c_int::from(reg)) }
        }

        /// Runs `cycles` cycles into `buf`, which must hold at least `cycles` samples.
        pub fn clock(&mut self, cycles: u32, buf: &mut [i16]) -> usize {
            // Checked before the call: afterwards would be too late. SID::clock emits at most one sample per cycle.
            assert!(
                buf.len() >= cycles as usize,
                "sample buffer shorter than the cycle count"
            );
            // SAFETY: as above; buf holds at least `cycles` samples, the most SID::clock can write for `cycles`.
            let n = unsafe { score64_residfp_clock(self.0, cycles, buf.as_mut_ptr()) };
            usize::try_from(n).unwrap_or(0).min(buf.len())
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: self.0 came from score64_residfp_new and is freed exactly once.
            unsafe { score64_residfp_free(self.0) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_sound() {
        let mut sid = ResidFp::new(
            ChipModel::Mos6581,
            985_248.0,
            48_000.0,
            SamplingMethod::Resample,
        )
        .unwrap();
        // Voice 1: A-4 sawtooth, fast attack, full sustain, master volume 15.
        for (reg, v) in [
            (0x18, 0x0F),
            (0x05, 0x00),
            (0x06, 0xF0),
            (0x00, 0xD6),
            (0x01, 0x1C),
            (0x04, 0x21),
        ] {
            sid.write(reg, v);
        }
        let mut out = Vec::new();
        sid.clock(985_248 / 10, &mut out);
        let samples = out.len();
        assert!(
            (4790..=4810).contains(&samples),
            "{samples} samples for 0.1 s"
        );
        let peak = out.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 1000, "peak {peak}");
    }

    /// Two SIDs fed the same writes give the same samples, filter included: no state is shared between instances
    /// (#207, `vendor/SCORE64-PATCHES.md` patch 2).
    #[test]
    fn identical_writes_give_identical_samples() {
        for model in [ChipModel::Mos6581, ChipModel::Csg8580] {
            let render = || {
                let mut sid =
                    ResidFp::new(model, 985_248.0, 48_000.0, SamplingMethod::Resample).unwrap();
                // Voice 1 through the low-pass filter, resonance 15, cutoff $400: a pulse note, then its release.
                for (reg, v) in [
                    (0x18, 0x1F),
                    (0x17, 0xF1),
                    (0x15, 0x00),
                    (0x16, 0x40),
                    (0x01, 0x11),
                    (0x03, 0x08),
                    (0x05, 0x09),
                    (0x06, 0xA0),
                    (0x04, 0x41),
                ] {
                    sid.write(reg, v);
                }
                let mut out = Vec::new();
                sid.clock(985_248 / 4, &mut out);
                sid.write(0x04, 0x40);
                sid.clock(985_248 / 4, &mut out);
                out
            };
            let first = render();
            assert!(first.iter().any(|&s| s != first[0]), "{model:?} is silent");
            assert!(render() == first, "{model:?}: a second SID differs");
        }
    }

    #[test]
    fn rejects_nothing_valid() {
        let mut sid = ResidFp::new(
            ChipModel::Csg8580,
            985_248.0,
            44_100.0,
            SamplingMethod::Decimate,
        )
        .unwrap();
        sid.set_combined_waveforms(CombinedWaveforms::Strong)
            .unwrap();
        sid.set_chip_model(ChipModel::Mos6581).unwrap();
    }

    /// The resampler emits slightly more than `cycles * rate / clock` samples (it rounds its step down), so buffers
    /// sized from that ratio overflow at high rates. Regression test for the buffer sizing in `ResidFp::clock`.
    #[test]
    fn high_rates_fit_the_buffer() {
        for rate in [8_000.0, 48_000.0, 192_000.0] {
            let mut sid = ResidFp::new(
                ChipModel::Mos6581,
                985_248.0,
                rate,
                SamplingMethod::Resample,
            )
            .unwrap();
            let mut out = Vec::new();
            sid.clock(985_248, &mut out);
            let expected = rate as usize;
            assert!(
                out.len() >= expected && out.len() <= expected + expected / 1000 + 64,
                "{rate}: {}",
                out.len()
            );
        }
    }
}
