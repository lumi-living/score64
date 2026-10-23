//! What the game sees of an adaptive set (#52): a score with two layers, exported with timing marks, packed by the
//! real gt2reloc with `-H1`, and traced in the emulator helper with start+$3F watched. The byte must change at
//! exactly each switch-point frame, worked out from the set's switch rows and the tempo map, including the first
//! switch point after each loop back, and nowhere else. Runs when gt2reloc and the helper are available; otherwise it
//! skips visibly (CI has no gt2reloc).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::num::{NonZeroU8, NonZeroU32};

use score64_adapters::emu::Helper;
use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};
use score64_core::profile::ProfileName;
use score64_core::{
    AdaptiveSet, Attack, Confidence, FramesPerRow, Note, PresetId, Role, Roles, Schema, Song,
    Subtune, SubtuneName, Switch, TempoChange, Voice, VoiceLine,
};
use score64_formats::gt2::export::{Header, Library, export};
use score64_formats::gt2::{Ins, Instrument, Table};
use score64_formats::psid::Psid;

const ROWS: u32 = 64;
const BAR: u8 = 16;
const LOOP_ROW: u32 = 16;

fn note(row: u32, pitch: &str) -> Note {
    Note {
        row,
        length: NonZeroU32::new(8).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(1.0).unwrap(),
        decision: None,
    }
}

/// A pulse instrument: hard restart 2 frames ahead, first frame $09 (test bit + gate).
fn pulse() -> Ins {
    let mut instrument = Instrument::from_bytes(&[0; 25]);
    instrument.ad = 0x09;
    instrument.sr = 0xA0;
    instrument.wave = 1;
    instrument.pulse = 1;
    instrument.gate_timer = 2;
    instrument.first_wave = 0x09;
    instrument.set_name("test pulse");
    Ins {
        instrument,
        wave: Table {
            rows: vec![(0x41, 0x00), (0xFF, 0x00)],
        },
        pulse: Table {
            rows: vec![(0x88, 0x00), (0xFF, 0x00)],
        },
        filter: Table::default(),
        speed: Table::default(),
    }
}

/// Where each layer's notes start: three bar starts and one mid-bar row, so switch points fall on bar starts and
/// mid-bar, with a note and without.
const NOTE_ROWS: [u32; 4] = [0, 16, 24, 48];

/// A layer: four bars with notes on [`NOTE_ROWS`], 5 frames a row and then 3 from row 40, looping to bar 2.
fn layer(name: &str, pitch: &str) -> Subtune {
    Subtune {
        name: SubtuneName::new(name).unwrap(),
        profile: None,
        frames_per_row: FramesPerRow::new(5).unwrap(),
        tempo: vec![TempoChange {
            row: 40,
            frames_per_row: FramesPerRow::new(3).unwrap(),
        }],
        rows_per_bar: NonZeroU8::new(BAR).unwrap(),
        length: NonZeroU32::new(ROWS).unwrap(),
        loop_row: LOOP_ROW,
        marks: vec![],
        voices: vec![VoiceLine {
            voice: Voice::new(1).unwrap(),
            roles: Roles::constant(Role::Lead),
            instrument: PresetId("pulse".into()),
            notes: NOTE_ROWS.iter().map(|&row| note(row, pitch)).collect(),
            arps: vec![],
        }],
    }
}

/// The frame row 0 starts on: the player's first row fetch after init. Checked here at 5 frames a row, with and
/// without `-D1`; it was also measured at 3, 12, 31 and 127.
const ROW0: u64 = 7;

/// What start+$3F holds in a freshly loaded `-H1` player: the last byte of the author field, a space for a short one.
const FRESH: u8 = 0x20;

#[test]
fn a_packed_adaptive_song_marks_exactly_its_switch_points() {
    let (Ok(packer), Some(helper)) = (Gt2Reloc::locate("2.73"), Helper::locate()) else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED a_packed_adaptive_song_marks_exactly_its_switch_points: needs gt2reloc on PATH and the \
             score64-emu helper"
        );
        return;
    };
    // Every bar (bar starts, with a note on all but row 32), and explicit rows (mid-bar, with a note on 24 and
    // none on 40, the tempo change, or 52), each packed without and with sound-effect support.
    for (switch, rows) in [
        (Switch::EveryBar, vec![0, 16, 32, 48]),
        (Switch::Rows(vec![0, 24, 40, 52]), vec![0, 24, 40, 52]),
    ] {
        for effects in [false, true] {
            check(&packer, &helper, switch.clone(), &rows, effects);
        }
    }
}

fn check(packer: &Gt2Reloc, helper: &std::path::Path, switch: Switch, rows: &[u32], effects: bool) {
    let every_bar = switch == Switch::EveryBar;
    let mut song = Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: vec![layer("calm", "C-4"), layer("tense", "G-4")],
    };
    let set = AdaptiveSet::with_switch(
        "layers",
        vec![song.subtunes[0].name.clone(), song.subtunes[1].name.clone()],
        switch,
    )
    .unwrap();
    set.place_marks(&mut song);
    let library: Library = BTreeMap::from([(PresetId("pulse".into()), pulse())]);
    let exported = export(
        &song,
        &library,
        std::slice::from_ref(&set),
        &Header::default(),
    )
    .unwrap();
    assert!(exported.timing_marks, "the song needs -H1");

    let dir = std::env::temp_dir().join(format!("score64-timing-marks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (sng, sid) = (dir.join("score.sng"), dir.join("score.sid"));
    std::fs::write(&sng, exported.sng.write().unwrap()).unwrap();
    let options = PackOptions {
        timing_marks: true,
        sound_effects: effects,
        ..PackOptions::new(0x10, ZeroPage::new(0xF8).unwrap())
    };
    let packed = packer.pack(&sng, &sid, &options);
    let bytes = std::fs::read(&sid);
    let _ = std::fs::remove_dir_all(&dir);
    packed.unwrap();
    let psid = Psid::parse(&bytes.unwrap()).unwrap();
    let mark = psid.load_address + 0x3F;
    assert_eq!(psid.data[0x3F], FRESH, "start+$3F as loaded");

    let mut h = Helper::spawn(helper, 48_000).unwrap();
    h.load_psid(&psid).unwrap();
    // Init doesn't reset the byte: the first layer finds the loaded value, the second the first's last mark.
    let mut before = FRESH;
    let mut pitches = Vec::new();
    for (index, subtune) in song.subtunes.iter().enumerate() {
        let name = subtune.name.as_str();
        let marks = set.timing_marks(subtune).marks;
        assert_eq!(set.switch_rows(subtune), rows, "{name}");
        let value_at = |row: u32| {
            marks
                .iter()
                .find(|m| m.row == row)
                .unwrap_or_else(|| panic!("{name}: no mark at row {row}"))
                .value
        };
        // Three passes: the first, then two from the loop row.
        let first_pass = subtune.frames(0, ROWS);
        let loop_pass = subtune.frames(LOOP_ROW, ROWS);
        let frames = (ROW0 + first_pass + 2 * loop_pass) as u32;
        h.trace(false, None, &[mark]).unwrap();
        h.init(psid.init_address, psid.play_address, index as u8)
            .unwrap();
        let rendered = h.render(frames, true).unwrap();
        let traced = h.report().unwrap();
        let watched = &traced.watched[0];
        assert_eq!(watched.start, before, "{name}: start+$3F after init");

        // Row 0 starts on the first note's first frame, which writes $09 (gate + test bit) to voice 1's control
        // register: an independent clock, from the SID writes rather than the mark.
        let note_frames: Vec<u64> = rendered
            .writes
            .iter()
            .filter(|w| w.register == 0x04 && w.value == 0x09)
            .map(|w| u64::from(w.frame))
            .collect();
        assert_eq!(note_frames.first(), Some(&ROW0), "{name}: row 0's frame");
        let at = |pass: u64, row: u32| {
            if pass == 0 {
                ROW0 + subtune.frames(0, row)
            } else {
                ROW0 + first_pass + (pass - 1) * loop_pass + subtune.frames(LOOP_ROW, row)
            }
        };
        let mut want: Vec<(u32, u8)> = rows
            .iter()
            .map(|&r| (at(0, r) as u32, value_at(r)))
            .collect();
        for pass in 1..=2 {
            want.extend(
                rows.iter()
                    .filter(|&&r| r >= LOOP_ROW)
                    .map(|&r| (at(pass, r) as u32, value_at(r))),
            );
        }
        assert_eq!(
            want.len(),
            10,
            "{name}: 4 + 3 + 3 switch points over three passes"
        );
        assert!(
            want.last().unwrap().0 < frames,
            "{name}: every switch point is inside the render"
        );
        if every_bar {
            // The tempo map's frames, as player.md states them.
            let after_row0: Vec<u64> = rows.iter().map(|&r| subtune.frames(0, r)).collect();
            assert_eq!(after_row0, [0, 80, 160, 224], "{name}");
        }
        assert_eq!(
            watched.changes, want,
            "{name}: start+$3F changes (frame, value)"
        );
        // A switch point with a note changes the byte on the note's own first frame, on every pass.
        for &r in rows.iter().filter(|r| NOTE_ROWS.contains(r)) {
            for pass in 0..=2 {
                if pass > 0 && r < LOOP_ROW {
                    continue;
                }
                let f = at(pass, r);
                assert!(
                    note_frames.contains(&f),
                    "{name}: row {r}, pass {pass}: frame {f} has no note start"
                );
            }
        }
        // The subtune is the one asked for: its first note's pitch.
        pitches.push(
            rendered
                .writes
                .iter()
                .find(|w| w.register == 0x01 && u64::from(w.frame) >= ROW0)
                .map(|w| w.value),
        );
        // Across the loop the value changes: the last mark before the jump differs from the first after it.
        let first_in_loop = *rows.iter().find(|&&r| r >= LOOP_ROW).unwrap();
        assert_ne!(
            value_at(*rows.last().unwrap()),
            value_at(first_in_loop),
            "{name}"
        );
        before = watched.changes.last().unwrap().1;
    }
    assert_ne!(
        pitches[0], pitches[1],
        "the two layers play their own notes"
    );
}
