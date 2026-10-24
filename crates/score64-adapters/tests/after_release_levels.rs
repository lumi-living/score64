//! The measurement behind `docs/reference/sid.md` §After the release (#186): the level a voice leaves after its
//! release, left alone or with the waveform register set to `$00` or `$08`, on both chips, over fresh helper processes
//! (the helper's output varies between processes, #207). Ignored by default; run it with
//! `cargo test -p score64-adapters --test after_release_levels -- --ignored --nocapture`.
//!
//! The setup: master volume 15; voice 1 at frequency `$1100` with pulse width `$800`, AD `$00`, SR `$F0` (full
//! sustain, release 0); the gate on with the waveform for 25 frames (0.5 s), then off for 10 frames (0.2 s), then the
//! waveform register left alone or written. Levels are the RMS of 0.5 s windows from the write, with each window's DC
//! taken out, in dBFS. "First second" is the louder of the first two windows; "settled" the loudest from 1 s to 4 s.

use std::io::Write as _;

use score64_adapters::emu::{Chip, Helper};

const RUNS: usize = 10;

fn dbfs(s: &[i16]) -> f64 {
    let m = s.iter().map(|&v| f64::from(v)).sum::<f64>() / s.len() as f64;
    let ms = s
        .iter()
        .map(|&v| ((f64::from(v) - m) / 32768.0).powi(2))
        .sum::<f64>()
        / s.len() as f64;
    if ms == 0.0 {
        f64::NEG_INFINITY
    } else {
        10.0 * ms.log10()
    }
}

/// The range over the runs, and how many were exactly silent.
fn range(v: &[f64]) -> String {
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let silent = v.iter().filter(|x| x.is_infinite()).count();
    format!("{lo:.1} to {hi:.1} (silent {silent}/{RUNS})")
}

#[test]
#[ignore = "a measurement for docs/reference/sid.md, not a check"]
fn levels_after_the_release() {
    let Some(helper) = Helper::locate() else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED levels_after_the_release: needs the score64-emu helper"
        );
        return;
    };
    for chip in [Chip::Mos6581, Chip::Mos8580] {
        for (name, wave) in [
            ("pulse", 0x40u8),
            ("saw", 0x20),
            ("triangle", 0x10),
            ("noise", 0x80),
        ] {
            for (case, after) in [
                ("left alone", None),
                ("$00", Some(0x00u8)),
                ("$08", Some(0x08)),
            ] {
                let (mut first, mut settled) = (Vec::new(), Vec::new());
                for _ in 0..RUNS {
                    let mut h = Helper::spawn(&helper, 48_000).unwrap();
                    h.model(chip).unwrap();
                    h.load(0x1000, &[0x60]).unwrap();
                    h.init(0x1000, 0x1000, 0).unwrap();
                    for (a, v) in [
                        (0xD418u16, 0x0Fu8),
                        (0xD400, 0x00),
                        (0xD401, 0x11),
                        (0xD402, 0x00),
                        (0xD403, 0x08),
                        (0xD405, 0x00),
                        (0xD406, 0xF0),
                        (0xD404, wave | 1),
                    ] {
                        h.poke(a, v).unwrap();
                    }
                    h.render(25, false).unwrap();
                    h.poke(0xD404, wave).unwrap();
                    h.render(10, false).unwrap();
                    if let Some(w) = after {
                        h.poke(0xD404, w).unwrap();
                    }
                    let samples = h.render(200, false).unwrap().samples;
                    let windows: Vec<f64> = samples.chunks(24_000).map(dbfs).collect();
                    first.push(
                        windows[..2]
                            .iter()
                            .copied()
                            .fold(f64::NEG_INFINITY, f64::max),
                    );
                    settled.push(
                        windows[2..]
                            .iter()
                            .copied()
                            .fold(f64::NEG_INFINITY, f64::max),
                    );
                }
                println!(
                    "{chip:?} {name} {case}: first second {}, settled {}",
                    range(&first),
                    range(&settled)
                );
            }
        }
    }
}
