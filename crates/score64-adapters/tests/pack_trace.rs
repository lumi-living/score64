//! The fixture song packed by the real gt2reloc with `-ZF8`, traced in the emulator helper over init, both subtunes
//! for a minute each, and a sound-effect call on every voice: it touches zero page only at `$F8`/`$F9` and writes
//! nothing outside its own memory (#51). Runs when gt2reloc and the helper are available; otherwise it skips visibly
//! (CI has no gt2reloc).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::PathBuf;

use score64_adapters::emu::Helper;
use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};
use score64_formats::psid::Psid;

/// Where the effect data and the play wrapper go: outside the tune, read but never written.
const EFFECT: u16 = 0xC000;
const WRAPPER: u16 = 0xC800;
/// The wrapper's mailbox, which only the host writes: the voice's channel offset, then 1 to fire.
const CHANNEL: u16 = 0xC7FE;
const FIRE: u16 = 0xC7FF;

/// A play routine that fires the effect at `EFFECT` on channel `[CHANNEL]` when `[FIRE]` is set, then plays a frame.
/// It writes nothing itself (its `JSR` uses the stack).
fn wrapper(start: u16) -> Vec<u8> {
    let [fx_lo, fx_hi] = EFFECT.to_le_bytes();
    let [ch_lo, ch_hi] = CHANNEL.to_le_bytes();
    let [fi_lo, fi_hi] = FIRE.to_le_bytes();
    let [e_lo, e_hi] = (start + 6).to_le_bytes();
    let [p_lo, p_hi] = (start + 3).to_le_bytes();
    vec![
        0xAD, fi_lo, fi_hi, // LDA FIRE
        0xF0, 0x0A, // BEQ play
        0xA9, fx_lo, // LDA #<EFFECT
        0xA0, fx_hi, // LDY #>EFFECT
        0xAE, ch_lo, ch_hi, // LDX CHANNEL
        0x20, e_lo, e_hi, // JSR start+6
        0x4C, p_lo, p_hi, // play: JMP start+3
    ]
}

#[test]
fn the_packed_fixture_keeps_to_its_pair_and_its_memory() {
    let (Ok(packer), Some(helper)) = (Gt2Reloc::locate("2.73"), Helper::locate()) else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED the_packed_fixture_keeps_to_its_pair_and_its_memory: needs gt2reloc on PATH and the score64-emu helper"
        );
        return;
    };
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gt2-original");
    let dir = std::env::temp_dir().join(format!("score64-pack-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("tune.sid");
    let packed = packer
        .pack(
            &fixtures.join("sixteen-steps.sng"),
            &out,
            &PackOptions::new(0x10, ZeroPage::new(0xF8).unwrap()),
        )
        .unwrap();
    let psid = Psid::parse(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let start = psid.load_address;
    let last = start + packed.size as u16 - 1;
    let effect = std::fs::read(fixtures.join("sfx-gunshot.bin")).unwrap();

    let mut h = Helper::spawn(&helper, 48_000).unwrap();
    h.load(psid.load_address, &psid.data).unwrap();
    h.load(EFFECT, &effect).unwrap();
    h.load(WRAPPER, &wrapper(start)).unwrap();
    let (mut zero_page, mut outside) = (BTreeSet::new(), BTreeSet::new());
    for subtune in 0..psid.songs as u8 {
        h.trace(true, Some((start, last)), &[]).unwrap();
        h.init(start, WRAPPER, subtune).unwrap();
        // A minute of music (the song loops within it), with an effect on each voice in turn every 2 seconds.
        for second in 0..60u32 {
            if second % 2 == 1 {
                let voice = (second / 2 % 3) as u8;
                h.poke(CHANNEL, 7 * voice).unwrap();
                h.poke(FIRE, 1).unwrap();
                h.render(1, false).unwrap();
                h.poke(FIRE, 0).unwrap();
                h.render(49, false).unwrap();
            } else {
                h.render(50, false).unwrap();
            }
        }
        let t = h.report().unwrap();
        zero_page.extend(t.zero_page);
        outside.extend(t.pages_written_outside);
    }
    assert_eq!(zero_page, [0xF8, 0xF9].into(), "zero page used");
    assert_eq!(
        outside,
        BTreeSet::new(),
        "pages written outside ${start:04X}–${last:04X}"
    );
}
