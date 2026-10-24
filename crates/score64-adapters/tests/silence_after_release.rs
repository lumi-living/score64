//! `silence_after_release` end to end (#186): a score exported with it, packed by the real gt2reloc and played in the
//! emulator helper. Without it, a gap after a key-off keeps a faint ghost of the last note, because the oscillator
//! keeps running; with it, the gap goes quiet on both chips. Runs when gt2reloc and the helper are available;
//! otherwise it skips visibly (CI has no gt2reloc).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::num::{NonZeroU8, NonZeroU32};

use score64_adapters::emu::{Chip, Helper};
use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};
use score64_core::profile::ProfileName;
use score64_core::{
    Attack, Confidence, FramesPerRow, Note, PresetId, Role, Roles, Schema, Song, Subtune,
    SubtuneName, Voice, VoiceLine,
};
use score64_formats::gt2::export::{Header, Library, Options, export_with};
use score64_formats::gt2::{Ins, Instrument, Table};
use score64_formats::psid::Psid;

/// Frames a row, and rows in the subtune: a 4-row note, then 60 rows (6 s) of gap, then the loop.
const FPR: u32 = 5;
const ROWS: u32 = 64;

fn instrument(wave: u8) -> Ins {
    let mut instrument = Instrument::from_bytes(&[0; 25]);
    instrument.ad = 0x09;
    instrument.sr = 0xA0; // release 0
    instrument.wave = 1;
    instrument.pulse = 1;
    instrument.gate_timer = 2;
    instrument.first_wave = 0x09;
    Ins {
        instrument,
        wave: Table {
            rows: vec![(wave, 0x00), (0xFF, 0x00)],
        },
        pulse: Table {
            rows: vec![(0x88, 0x00), (0xFF, 0x00)],
        },
        filter: Table::default(),
        speed: Table::default(),
    }
}

fn song() -> Song {
    Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: vec![Subtune {
            name: SubtuneName::new("main").unwrap(),
            profile: None,
            frames_per_row: FramesPerRow::new(FPR as u8).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(ROWS).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                roles: Roles::constant(Role::Lead),
                instrument: PresetId("tone".into()),
                notes: vec![Note {
                    row: 0,
                    length: NonZeroU32::new(4).unwrap(),
                    pitch: "C-4".parse().unwrap(),
                    attack: Attack::Fresh,
                    instrument: None,
                    source: None,
                    confidence: Confidence::new(1.0).unwrap(),
                    decision: None,
                }],
                arps: vec![],
            }],
        }],
    }
}

fn ac_dbfs(s: &[i16]) -> f64 {
    let m = s.iter().map(|&x| f64::from(x)).sum::<f64>() / s.len() as f64;
    let ms = s
        .iter()
        .map(|&x| ((f64::from(x) - m) / 32768.0).powi(2))
        .sum::<f64>()
        / s.len() as f64;
    10.0 * ms.max(1e-20).log10()
}

/// The gap's level over its last 3 s (rows 34–63), in dBFS with the DC taken out.
fn gap_level(
    packer: &Gt2Reloc,
    helper: &std::path::Path,
    wave: u8,
    silence: bool,
    chip: Chip,
) -> f64 {
    let library: Library = BTreeMap::from([(PresetId("tone".into()), instrument(wave))]);
    let options = Options {
        silence_after_release: if silence {
            [PresetId("tone".into())].into()
        } else {
            Default::default()
        },
        ..Options::default()
    };
    let exported = export_with(&song(), &library, &[], &Header::default(), &options).unwrap();
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("score64-silence-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (sng, sid) = (dir.join("s.sng"), dir.join("s.sid"));
    std::fs::write(&sng, exported.sng.write().unwrap()).unwrap();
    let packed = packer.pack(
        &sng,
        &sid,
        &PackOptions::new(0x10, ZeroPage::new(0xF8).unwrap()),
    );
    let bytes = std::fs::read(&sid);
    let _ = std::fs::remove_dir_all(&dir);
    packed.unwrap();
    let psid = Psid::parse(&bytes.unwrap()).unwrap();
    let mut h = Helper::spawn(helper, 48_000).unwrap();
    h.model(chip).unwrap();
    let (init, play) = h.load_psid(&psid).unwrap();
    h.init(init, play, 0).unwrap();
    // Row 0 starts on frame 7; the gap's last 30 rows start at row 34.
    h.render(7 + 34 * FPR, false).unwrap();
    ac_dbfs(&h.render(30 * FPR, false).unwrap().samples)
}

#[test]
fn a_gap_after_a_key_off_goes_silent_with_the_setting() {
    let (Ok(packer), Some(helper)) = (Gt2Reloc::locate("2.73"), Helper::locate()) else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED a_gap_after_a_key_off_goes_silent_with_the_setting: needs gt2reloc on PATH and the score64-emu helper"
        );
        return;
    };
    for (wave, name) in [(0x41u8, "pulse"), (0x21, "saw"), (0x11, "triangle")] {
        for chip in [Chip::Mos6581, Chip::Mos8580] {
            let off = gap_level(&packer, &helper, wave, false, chip);
            let on = gap_level(&packer, &helper, wave, true, chip);
            // Without the setting the ghost is there; with it, the gap is exactly silent on both chips. The helper
            // renders byte for byte the same since #207, so this needs no margin for run-to-run differences.
            let floor = -150.0;
            assert!(
                off > -80.0,
                "{name} on {chip:?}: no ghost without the setting ({off:.1} dBFS)"
            );
            assert!(
                on < floor && on < off - 15.0,
                "{name} on {chip:?}: the gap is at {on:.1} dBFS with the setting, against {off:.1} without"
            );
            println!("{name:8} {chip:?}: off {off:.1}, on {on:.1} dBFS");
        }
    }
}
