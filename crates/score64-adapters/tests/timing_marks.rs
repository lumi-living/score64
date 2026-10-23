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

/// A layer: four bars, a note on every bar line, 5 frames a row and then 3 from row 40, looping to bar 2.
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
            notes: (0..ROWS / u32::from(BAR))
                .map(|bar| note(bar * u32::from(BAR), pitch))
                .collect(),
            arps: vec![],
        }],
    }
}

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
    let mut song = Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: vec![layer("calm", "C-4"), layer("tense", "G-4")],
    };
    let set = AdaptiveSet::with_switch(
        "layers",
        vec![song.subtunes[0].name.clone(), song.subtunes[1].name.clone()],
        Switch::EveryBar,
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
        ..PackOptions::new(0x10, ZeroPage::new(0xF8).unwrap())
    };
    let packed = packer.pack(&sng, &sid, &options);
    let bytes = std::fs::read(&sid);
    let _ = std::fs::remove_dir_all(&dir);
    packed.unwrap();
    let psid = Psid::parse(&bytes.unwrap()).unwrap();
    let mark = psid.load_address + 0x3F;

    let mut h = Helper::spawn(&helper, 48_000).unwrap();
    h.load_psid(&psid).unwrap();
    for (index, subtune) in song.subtunes.iter().enumerate() {
        let marks = set.timing_marks(subtune).marks;
        let rows: Vec<u32> = set.switch_rows(subtune);
        assert_eq!(rows, [0, 16, 32, 48]);
        // Three passes: the first, then two from the loop row.
        let first_pass = subtune.frames(0, ROWS);
        let loop_pass = subtune.frames(LOOP_ROW, ROWS);
        let frames = (first_pass + 2 * loop_pass) as u32;
        h.trace(false, None, &[mark]).unwrap();
        h.init(psid.init_address, psid.play_address, index as u8)
            .unwrap();
        let rendered = h.render(frames, true).unwrap();
        let traced = h.report().unwrap();
        let changes = &traced.watched[0].changes;

        // Row 0 starts on the first note's first frame, which writes $09 (gate + test bit) to voice 1's control
        // register: an independent clock, from the SID writes rather than the mark.
        let row0 = rendered
            .writes
            .iter()
            .find(|w| w.register == 0x04 && w.value == 0x09)
            .map(|w| u64::from(w.frame))
            .expect("the first note");
        let value_at = |row: u32| marks.iter().find(|m| m.row == row).unwrap().value;
        let mut want: Vec<(u32, u8)> = rows
            .iter()
            .map(|&r| ((row0 + subtune.frames(0, r)) as u32, value_at(r)))
            .collect();
        for pass in 0..2 {
            let base = row0 + first_pass + pass * loop_pass;
            want.extend(
                rows.iter()
                    .filter(|&&r| r >= LOOP_ROW)
                    .map(|&r| ((base + subtune.frames(LOOP_ROW, r)) as u32, value_at(r))),
            );
        }
        // Only the frames the render covered.
        want.retain(|&(f, _)| f < frames);
        assert_eq!(
            changes,
            &want,
            "{}: start+$3F changes (frame, value), row 0 at frame {row0}, start value ${:02X}",
            subtune.name.as_str(),
            traced.watched[0].start
        );
        // Across the loop the value changes: the last mark before the jump differs from the first after it.
        assert_ne!(value_at(48), value_at(16), "{}", subtune.name.as_str());
    }
}
