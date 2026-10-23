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

const FRAMES_PER_ROW: u32 = 6;

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

#[test]
fn an_exported_score_plays_as_written() {
    let (Ok(packer), Some(helper)) = (Gt2Reloc::locate("2.73"), Helper::locate()) else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED an_exported_score_plays_as_written: needs gt2reloc on PATH and the score64-emu helper"
        );
        return;
    };
    let library: Library = BTreeMap::from([(PresetId("pulse".into()), pulse())]);
    let exported = export(&score(), &library, &[], &Header::default()).unwrap();
    let dir = std::env::temp_dir().join(format!("score64-export-{}", std::process::id()));
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
    h.init(init, play, 0).unwrap();
    let frames = 32 * FRAMES_PER_ROW;
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
    // Row 0 starts where the first note does (its first frame sets the test bit, $09): a few frames in, after init
    // and the player's first row fetch. Rows then follow every 6 frames.
    let start = v.control[0].iter().position(|&c| c != 0).unwrap() as u32;
    assert!(start < 2 * FRAMES_PER_ROW, "row 0 starts at frame {start}");
    let frame = |row: u32, offset: u32| (start + row * FRAMES_PER_ROW + offset) as usize;
    let freq = |k: usize, row: u32, offset: u32| f64::from(v.freq[k][frame(row, offset)]);
    let gate = |k: usize, row: u32| v.control[k][frame(row, 3)] & 1 == 1;
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
    assert!((frame(13, 0)..frame(15, 0)).all(|f| f64::from(v.freq[1][f]) == c3));
}
