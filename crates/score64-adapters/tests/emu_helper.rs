//! The client against the real helper binary (client → helper direction, #8). Runs when the helper has been built
//! into the same target directory (`cargo test --workspace` does that) or `SCORE64_EMU` points at it; otherwise it
//! skips visibly.

use std::io::Write as _;

use score64_adapters::emu::{Chip, Helper, HelperError};
use score64_formats::emu_protocol::ErrorCode;

/// $1000: RTS (init). $1003: copy the mailbox at $0340 to the volume register, start voice 1, RTS.
const PLAYER: [u8; 20] = [
    0x60, 0xEA, 0xEA, // init
    0xAD, 0x40, 0x03, 0x8D, 0x18, 0xD4, // LDA $0340, STA $D418
    0xA9, 0x10, 0x8D, 0x01, 0xD4, // LDA #$10, STA $D401
    0xA9, 0x21, 0x8D, 0x04, 0xD4, // LDA #$21, STA $D404
    0x60,
];

#[test]
fn renders_through_the_real_helper() {
    let Some(binary) = Helper::locate() else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED renders_through_the_real_helper: build score64-emu (cargo test --workspace) or set SCORE64_EMU"
        );
        return;
    };
    let mut h = Helper::spawn(&binary, 48_000).expect("spawn helper");
    assert_eq!(h.info().sample_rate, 48_000);
    assert!(
        h.info().helper.starts_with("score64-emu "),
        "{:?}",
        h.info()
    );

    h.load(0x1000, &PLAYER).unwrap();
    assert!(matches!(
        h.render(1, false),
        Err(HelperError::Refused {
            code: ErrorCode::Sequence,
            ..
        })
    ));
    h.model(Chip::Mos8580).unwrap();
    h.poke(0x0340, 0x0F).unwrap();
    h.init(0x1000, 0x1003, 0).unwrap();

    let mut samples = 0;
    let mut writes = Vec::new();
    for _ in 0..5 {
        let r = h.render(10, true).unwrap();
        assert_eq!(r.frames, 10);
        assert!(r.max_play_cycles > 0 && r.max_play_cycles < 100);
        samples += r.samples.len();
        writes.extend(r.writes);
    }
    // 50 PAL frames of 19,656 cycles at 985,248 Hz is 0.9975 s.
    assert!((47_870..=47_900).contains(&samples), "{samples} samples");
    assert_eq!(writes.len(), 150);
    assert_eq!(
        (writes[0].frame, writes[0].register, writes[0].value),
        (0, 0x18, 0x0F)
    );
    assert_eq!(writes.last().unwrap().frame, 49);
    h.quit().unwrap();
}
