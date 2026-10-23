//! A score exported as a GoatTracker song, packed by the real gt2reloc and played by the emulator helper: the notes,
//! ties, key-offs and arpeggios sound as the score says (#47). Runs when both gt2reloc and the helper are available;
//! otherwise it skips visibly (CI has no gt2reloc).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::num::{NonZeroU8, NonZeroU32};

use score64_adapters::emu::{Chip, Helper};
use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};
use score64_core::profile::ProfileName;
use score64_core::{
    ArpEvent, Attack, Confidence, FramesPerRow, Note, PresetId, Role, Roles, Schema, Song, Subtune,
    SubtuneName, Voice, VoiceLine,
};
use score64_formats::gt2::export::{Header, Library, export};
use score64_formats::gt2::{Ins, Instrument, Table};
use score64_formats::psid::Psid;

/// Not GoatTracker's default of 6, so a missing tempo command shows.
const FRAMES_PER_ROW: u32 = 5;

fn note(row: u32, length: u32, pitch: &str, attack: Attack) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack,
        instrument: None,
        source: None,
        confidence: Confidence::new(1.0).unwrap(),
        decision: None,
    }
}

/// A pulse instrument: AD $09, SR $A0, hard restart 2 frames ahead, first frame $09 (test bit + gate), then pulse.
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

/// Voice 1: C-4, then E-4 tied to it, a gap, then G-4. Voice 2: a long C-3 with a major arpeggio over rows 4-11.
fn score() -> Song {
    let line = |voice, notes| VoiceLine {
        voice: Voice::new(voice).unwrap(),
        roles: Roles::constant(Role::Lead),
        instrument: PresetId("pulse".into()),
        notes,
        arps: vec![],
    };
    let mut arp = line(2, vec![note(0, 16, "C-3", Attack::Fresh)]);
    arp.arps = vec![ArpEvent {
        row: 4,
        length: NonZeroU32::new(8).unwrap(),
        intervals: vec![0, 4, 7],
        decision: None,
    }];
    Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: vec![Subtune {
            name: SubtuneName::new("main").unwrap(),
            profile: None,
            frames_per_row: FramesPerRow::new(FRAMES_PER_ROW as u8).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(32).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![
                line(
                    1,
                    vec![
                        note(0, 4, "C-4", Attack::Fresh),
                        note(4, 4, "E-4", Attack::Legato),
                        note(12, 2, "G-4", Attack::Fresh),
                    ],
                ),
                arp,
            ],
        }],
    }
}

/// Each voice's frequency and control register after each frame's play call.
struct Voices {
    freq: [Vec<u16>; 3],
    control: [Vec<u8>; 3],
}

/// Export `song`, pack it with the real gt2reloc, and play `frames` frames of subtune `subtune` in the helper. `None`
/// when either tool is missing.
fn play(song: &Song, subtune: u8, frames: u32) -> Option<Voices> {
    let (Ok(packer), Some(helper)) = (Gt2Reloc::locate("2.73"), Helper::locate()) else {
        return None;
    };
    let library: Library = BTreeMap::from([(PresetId("pulse".into()), pulse())]);
    let exported = export(song, &library, &[], &Header::default()).unwrap();
    // A folder per call: the tests run in parallel.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("score64-export-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (sng, sid) = (dir.join("score.sng"), dir.join("score.sid"));
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

    let mut h = Helper::spawn(&helper, 48_000).unwrap();
    h.model(Chip::Mos8580).unwrap();
    let (init, play) = h.load_psid(&psid).unwrap();
    h.init(init, play, subtune).unwrap();
    let rendered = h.render(frames, true).unwrap();

    // Replay the writes into register state, frame by frame.
    let mut regs = [0u8; 0x19];
    let mut v = Voices {
        freq: Default::default(),
        control: Default::default(),
    };
    let mut writes = rendered.writes.iter().peekable();
    for frame in 0..frames {
        while let Some(w) = writes.next_if(|w| w.frame == frame) {
            regs[usize::from(w.register)] = w.value;
        }
        for k in 0..3 {
            let base = 7 * k;
            v.freq[k].push(u16::from_le_bytes([regs[base], regs[base + 1]]));
            v.control[k].push(regs[base + 4]);
        }
    }
    Some(v)
}

fn skipped(test: &str) {
    let _ = writeln!(
        std::io::stderr(),
        "SKIPPED {test}: needs gt2reloc on PATH and the score64-emu helper"
    );
}

#[test]
fn an_exported_score_plays_as_written() {
    let frames = 32 * FRAMES_PER_ROW;
    let Some(v) = play(&score(), 0, frames) else {
        return skipped("an_exported_score_plays_as_written");
    };
    // Row 0 starts where the first note does (its first frame sets the test bit, $09): a few frames in, after init
    // and the player's first row fetch. Rows then follow every FRAMES_PER_ROW frames.
    let start = v.control[0].iter().position(|&c| c != 0).unwrap() as u32;
    assert!(start < 2 * FRAMES_PER_ROW, "row 0 starts at frame {start}");
    let frame = |row: u32, offset: u32| (start + row * FRAMES_PER_ROW + offset) as usize;
    let freq = |k: usize, row: u32, offset: u32| f64::from(v.freq[k][frame(row, offset)]);
    // On a row's second frame: the gate timer closes the gate 2 frames before a key-off's row, which at 5 frames a
    // row reaches into the row before.
    let gate = |k: usize, row: u32| v.control[k][frame(row, 1)] & 1 == 1;
    let ratio = |a: f64, b: f64, semitones: f64| {
        let want = 2f64.powf(semitones / 12.0);
        assert!((a / b - want).abs() < 0.01, "{a} / {b} is not {want}");
    };

    // Voice 1: E-4 is 4 semitones over C-4, G-4 is 7.
    let c4 = freq(0, 1, 3);
    ratio(freq(0, 5, 3), c4, 4.0);
    ratio(freq(0, 13, 3), c4, 7.0);
    // The tie: the gate never drops between C-4 and E-4. A fresh E-4 would close it for its hard restart, 2 frames
    // (the gate timer) before row 4; the key-off before row 8 closes it the same 2 frames early.
    assert!((frame(0, 1)..frame(8, 0) - 2).all(|f| v.control[0][f] & 1 == 1));
    assert_eq!(v.control[0][frame(8, 0) - 2] & 1, 0);
    assert!(!gate(0, 9) && !gate(0, 11));
    assert!(gate(0, 13));
    // G-4's own key-off before row 14.
    assert!(!gate(0, 15));

    // Voice 2: C-3 alone before the arpeggio, then C-3, E-3 and G-3 a frame each, then C-3 alone again.
    let c3 = freq(1, 2, 0);
    let arp: Vec<f64> = (frame(5, 0)..frame(11, 0))
        .map(|f| f64::from(v.freq[1][f]))
        .collect();
    let mut distinct = arp.clone();
    distinct.sort_by(f64::total_cmp);
    distinct.dedup();
    assert_eq!(distinct.len(), 3, "{arp:?}");
    ratio(distinct[0], c3, 0.0);
    ratio(distinct[1], c3, 4.0);
    ratio(distinct[2], c3, 7.0);
    assert!(
        arp.windows(2).all(|w| w[0] != w[1]),
        "a new step every frame: {arp:?}"
    );
    // Exactly rows 4 to 11: C-3 up to row 4, then the cycle root, third, fifth from row 4's first frame, one step a
    // frame, then C-3 alone again from row 12 (no stepping on its following frames).
    assert_eq!(freq(1, 3, FRAMES_PER_ROW - 1), c3);
    for f in frame(4, 0)..frame(12, 0) {
        let step = [0.0, 4.0, 7.0][(f - frame(4, 0)) % 3];
        ratio(f64::from(v.freq[1][f]), c3, step);
    }
    assert!((0..3).all(|o| freq(1, 12, o) == c3));
    assert!((frame(13, 0)..frame(15, 0)).all(|f| f64::from(v.freq[1][f]) == c3));
}

/// After the jump back to a silent loop row, the last note stops and the loop row's tempo returns.
#[test]
fn the_loop_stops_the_last_note_and_restores_the_tempo() {
    // Loop to row 16, which is silent; C-4 at row 20, E-4 from row 28 to the end; 4 frames a row from row 24.
    let mut song = score();
    let s = &mut song.subtunes[0];
    s.loop_row = 16;
    s.tempo = vec![score64_core::TempoChange {
        row: 24,
        frames_per_row: FramesPerRow::new(4).unwrap(),
    }];
    s.voices.truncate(1);
    s.voices[0].notes = vec![
        note(20, 4, "C-4", Attack::Fresh),
        note(28, 4, "E-4", Attack::Fresh),
    ];
    let fpr = FRAMES_PER_ROW as usize;
    let Some(v) = play(&song, 0, 400) else {
        return skipped("the_loop_stops_the_last_note_and_restores_the_tempo");
    };
    // C-4's first frame (the $09 test bit) marks row 20 the first time round.
    let starts: Vec<usize> = (1..v.control[0].len())
        .filter(|&f| v.control[0][f] == 0x09 && v.control[0][f - 1] != 0x09)
        .collect();
    let row20 = starts[0];
    // Rows 20-23 at 5 frames, 24-31 at 4, then the jump to row 16: rows 16-19 at 5 again.
    let jump = row20 + 4 * fpr + 8 * 4;
    assert!(starts.len() >= 3, "{starts:?}");
    assert_eq!(
        starts[1],
        row20 + 4 * fpr + 4 * 4,
        "E-4 on row 28, after 4 rows at 5 and 4 at 4"
    );
    assert_eq!(
        starts[2],
        jump + 4 * fpr,
        "C-4 again, 4 rows at the loop row's tempo after the jump"
    );
    // E-4 stops at the loop row: its key-off closes the gate (2 frames early), and it stays closed until C-4.
    assert!(
        (jump..jump + 4 * fpr).all(|f| v.control[0][f] & 1 == 0),
        "silent after the jump"
    );
}
