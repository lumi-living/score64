//! The helper's output is byte-identical across processes for identical input (#207): two helpers fed the same
//! requests give the same samples, on both chips, with the filter in use. Runs when the helper has been built into the
//! same target directory (`cargo test --workspace` does that) or `SCORE64_EMU` points at it; otherwise it skips
//! visibly. `cargo test -p score64-adapters` alone doesn't rebuild the helper, so after changing it, build it first
//! (`cargo build -p score64-emu`) or run the workspace's tests.

use std::io::Write as _;

use score64_adapters::emu::{Chip, Helper};

/// $1000: RTS (init and play).
const PLAYER: [u8; 1] = [0x60];

/// FNV-1a over the samples.
fn hash(samples: &[i16]) -> u64 {
    samples.iter().fold(0xcbf2_9ce4_8422_2325, |h, &s| {
        (h ^ u64::from(s as u16)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// One helper process: a filtered pulse note, its release, then a second Init and a sawtooth note.
fn render(binary: &std::path::Path, chip: Chip) -> (u64, usize) {
    let mut h = Helper::spawn(binary, 48_000).expect("spawn helper");
    h.model(chip).unwrap();
    h.load(0x1000, &PLAYER).unwrap();
    h.init(0x1000, 0x1000, 0).unwrap();
    let mut samples = Vec::new();
    for (a, v) in [
        (0xD418u16, 0x1Fu8),
        (0xD417, 0xF1),
        (0xD416, 0x40),
        (0xD401, 0x11),
        (0xD403, 0x08),
        (0xD405, 0x09),
        (0xD406, 0xA0),
        (0xD404, 0x41),
    ] {
        h.poke(a, v).unwrap();
    }
    samples.extend(h.render(25, false).unwrap().samples);
    h.poke(0xD404, 0x40).unwrap();
    samples.extend(h.render(25, false).unwrap().samples);
    h.init(0x1000, 0x1000, 0).unwrap();
    for (a, v) in [
        (0xD418u16, 0x0Fu8),
        (0xD401, 0x22),
        (0xD406, 0xF0),
        (0xD404, 0x21),
    ] {
        h.poke(a, v).unwrap();
    }
    samples.extend(h.render(25, false).unwrap().samples);
    assert!(
        samples.iter().any(|&s| s != samples[0]),
        "{chip:?} is silent"
    );
    (hash(&samples), samples.len())
}

#[test]
fn two_helpers_give_identical_samples() {
    let Some(binary) = Helper::locate() else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED two_helpers_give_identical_samples: build score64-emu (cargo test --workspace) or set SCORE64_EMU"
        );
        return;
    };
    for chip in [Chip::Mos6581, Chip::Mos8580] {
        let first = render(&binary, chip);
        let second = render(&binary, chip);
        assert_eq!(first, second, "{chip:?}: the two helpers' samples differ");
    }
}
