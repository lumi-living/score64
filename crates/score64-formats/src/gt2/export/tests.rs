//! Scores as GoatTracker songs: pattern cuts, notes, ties, key-offs, song-wide commands, instruments and their
//! tables, arpeggios, errors, and a round trip through the song file.

use std::num::{NonZeroU8, NonZeroU32};

use score64_core::profile::ProfileName;
use score64_core::{
    ArpEvent, Confidence, FramesPerRow, Note, Pitch, Role, Roles, Schema, Switch, TempoChange,
};

use super::*;
use crate::gt2::Instrument;

fn preset(s: &str) -> PresetId {
    PresetId(s.into())
}

/// An instrument with a one-row wavetable program of `wave` (then a stop) and a two-row pulse program that loops.
fn ins(wave: u8) -> Ins {
    let mut instrument = Instrument::from_bytes(&[0; 25]);
    instrument.ad = 0x09;
    instrument.sr = 0xA0;
    instrument.wave = 1;
    instrument.pulse = 1;
    instrument.gate_timer = 2;
    instrument.first_wave = 0x09;
    Ins {
        instrument,
        wave: Table {
            rows: vec![(wave, 0x00), (WAVE_JUMP, 0x00)],
        },
        pulse: Table {
            rows: vec![(0x88, 0x00), (WAVE_JUMP, 0x01)],
        },
        filter: Table::default(),
        speed: Table::default(),
    }
}

fn library() -> Library {
    [(preset("lead"), ins(0x41)), (preset("bass"), ins(0x21))]
        .into_iter()
        .collect()
}

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

fn line(voice: u8, instrument: &str, notes: Vec<Note>) -> VoiceLine {
    VoiceLine {
        voice: Voice::new(voice).unwrap(),
        roles: Roles::constant(Role::Lead),
        instrument: preset(instrument),
        notes,
        arps: vec![],
    }
}

/// Two 16-row bars at 6 frames a row, looping to the start.
fn subtune(name: &str, voices: Vec<VoiceLine>) -> Subtune {
    Subtune {
        name: SubtuneName::new(name).unwrap(),
        profile: None,
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(32).unwrap(),
        loop_row: 0,
        marks: vec![],
        voices,
    }
}

fn song(subtunes: Vec<Subtune>) -> Song {
    Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes,
    }
}

fn header() -> Header {
    Header {
        name: "Test".into(),
        author: "Someone".into(),
        copyright: "2026".into(),
    }
}

/// Voice `v`'s rows in subtune `s`, joined from its patterns.
fn rows_of(sng: &Sng, s: usize, v: usize) -> Vec<Row> {
    sng.subtunes[s][v]
        .entries
        .iter()
        .flat_map(|&p| sng.patterns[usize::from(p)].rows.clone())
        .collect()
}

fn n(pitch: &str) -> u8 {
    NOTE_FIRST + pitch.parse::<Pitch>().unwrap().semitone()
}

#[test]
fn notes_ties_and_key_offs() {
    let lead = line(
        1,
        "lead",
        vec![
            note(0, 4, "C-4", Attack::Fresh),
            note(4, 4, "E-4", Attack::Legato), // tied: no instrument, command 3 00
            note(12, 2, "G-4", Attack::Fresh), // after a gap, so E-4 gets a key-off at row 8
        ],
    );
    let out = export(
        &song(vec![subtune("main", vec![lead])]),
        &library(),
        &[],
        &header(),
    )
    .unwrap();
    let rows = rows_of(&out.sng, 0, 0);
    assert_eq!(rows.len(), 32);
    let cell = |r: usize| {
        (
            rows[r].note,
            rows[r].instrument,
            rows[r].command,
            rows[r].data,
        )
    };
    assert_eq!(
        cell(0),
        (n("C-4"), 1, TEMPO, 6),
        "the tempo rides on the first free slot"
    );
    assert_eq!(cell(4), (n("E-4"), 0, TIE, 0));
    assert_eq!(cell(8), (KEY_OFF, 0, 0, 0));
    assert_eq!(cell(12), (n("G-4"), 1, 0, 0));
    assert_eq!(cell(14), (KEY_OFF, 0, 0, 0));
    assert!(rows[1..4].iter().all(|r| r.note == REST));
    assert_eq!(out.instruments, [(preset("lead"), 1)]);
    // Silent voices share one pattern of rests.
    assert_eq!(
        out.sng.subtunes[0][1].entries,
        out.sng.subtunes[0][2].entries
    );
    assert_eq!(header_text(&out.sng.name), "Test");
}

fn header_text(f: &[u8; 32]) -> String {
    sng::header_text(f)
}

#[test]
fn patterns_cut_at_bars_and_the_loop_row_and_are_shared() {
    // Bar 0 and bar 1 are the same, so they share a pattern; the loop row 20 starts one of its own.
    let lead = line(
        1,
        "lead",
        vec![
            note(0, 4, "C-4", Attack::Fresh),
            note(16, 4, "C-4", Attack::Fresh),
        ],
    );
    let mut s = subtune("main", vec![lead]);
    s.loop_row = 20;
    let out = export(&song(vec![s.clone()]), &library(), &[], &header()).unwrap();
    let voice1 = &out.sng.subtunes[0][0];
    // Patterns: rows 0-15 (with the tempo), 16-19, 20-31.
    let lengths: Vec<usize> = voice1
        .entries
        .iter()
        .map(|&p| out.sng.patterns[usize::from(p)].rows.len())
        .collect();
    assert_eq!(lengths, [16, 4, 12]);
    assert_eq!(voice1.restart, 2, "the pattern that starts at the loop row");
    assert!(out.sng.subtunes[0].iter().all(|o| o.restart == 2));
    // Without the tempo on row 0 the two bars are equal and share.
    s.loop_row = 0;
    s.voices[0].notes[1].row = 16;
    let out = export(&song(vec![s]), &library(), &[], &header()).unwrap();
    let e = &out.sng.subtunes[0][1].entries;
    assert_eq!(e[0], e[1], "two bars of rests: one pattern");
}

#[test]
fn bars_longer_than_a_pattern_are_split() {
    let mut s = subtune(
        "long",
        vec![line(1, "lead", vec![note(0, 1, "C-4", Attack::Fresh)])],
    );
    s.rows_per_bar = NonZeroU8::new(192).unwrap();
    s.length = NonZeroU32::new(384).unwrap();
    let out = export(&song(vec![s]), &library(), &[], &header()).unwrap();
    let lengths: Vec<usize> = out.sng.subtunes[0][0]
        .entries
        .iter()
        .map(|&p| out.sng.patterns[usize::from(p)].rows.len())
        .collect();
    assert_eq!(lengths, [128, 64, 128, 64]);
}

#[test]
fn instruments_bring_their_tables_along() {
    let s = subtune(
        "main",
        vec![
            line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)]),
            line(2, "bass", vec![note(0, 4, "C-2", Attack::Fresh)]),
        ],
    );
    let out = export(&song(vec![s]), &library(), &[], &header()).unwrap();
    assert_eq!(out.instruments, [(preset("lead"), 1), (preset("bass"), 2)]);
    let (lead, bass) = (&out.sng.instruments[0], &out.sng.instruments[1]);
    assert_eq!((lead.wave, lead.pulse), (1, 1));
    // The second instrument's pointers and jump targets move past the first's rows.
    assert_eq!((bass.wave, bass.pulse), (3, 3));
    assert_eq!(
        out.sng.wave.rows,
        [(0x41, 0), (WAVE_JUMP, 0), (0x21, 0), (WAVE_JUMP, 0)]
    );
    assert_eq!(
        out.sng.pulse.rows,
        [(0x88, 0), (WAVE_JUMP, 1), (0x88, 0), (WAVE_JUMP, 3)]
    );
    // A note's own preset overrides its line's.
    let mut one = line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)]);
    one.notes[0].instrument = Some(preset("bass"));
    let out = export(
        &song(vec![subtune("m", vec![one])]),
        &library(),
        &[],
        &header(),
    )
    .unwrap();
    assert_eq!(out.instruments, [(preset("bass"), 1)]);
}

#[test]
fn arpeggios_point_the_voice_at_a_wavetable_program_and_back() {
    let mut lead = line(1, "lead", vec![note(0, 16, "C-3", Attack::Fresh)]);
    lead.arps = vec![ArpEvent {
        row: 4,
        length: NonZeroU32::new(8).unwrap(),
        intervals: vec![0, 4, 7],
        decision: None,
    }];
    let out = export(
        &song(vec![subtune("main", vec![lead])]),
        &library(),
        &[],
        &header(),
    )
    .unwrap();
    let rows = rows_of(&out.sng, 0, 0);
    // The program follows the instrument's two rows: the arpeggio with the instrument's waveform.
    assert_eq!((rows[4].command, rows[4].data), (SET_WAVE_POINTER, 3));
    assert_eq!(
        out.sng.wave.rows[2..],
        [(0x41, 0), (0x00, 4), (0x00, 7), (0x00, 0), (WAVE_JUMP, 4)]
    );
    // Back to the instrument's own program at row 12, where the note sounds on.
    assert_eq!((rows[12].command, rows[12].data), (SET_WAVE_POINTER, 1));
}

#[test]
fn timing_marks_and_tempo_changes_take_free_slots() {
    let mut s = subtune(
        "calm",
        vec![
            line(
                1,
                "lead",
                vec![
                    note(0, 4, "C-4", Attack::Fresh),
                    note(16, 4, "D-4", Attack::Legato),
                ],
            ),
            line(2, "bass", vec![note(0, 4, "C-2", Attack::Fresh)]),
        ],
    );
    s.voices[0].notes[1].attack = Attack::Legato;
    s.tempo = vec![TempoChange {
        row: 16,
        frames_per_row: FramesPerRow::new(5).unwrap(),
    }];
    s.marks = vec![0, 16];
    let set = AdaptiveSet::with_switch(
        "score",
        vec![SubtuneName::new("calm").unwrap()],
        Switch::EveryBar,
    )
    .unwrap();
    let out = export(&song(vec![s]), &library(), &[set], &header()).unwrap();
    let (v1, v2, v3) = (
        rows_of(&out.sng, 0, 0),
        rows_of(&out.sng, 0, 1),
        rows_of(&out.sng, 0, 2),
    );
    // Row 0: tempo on voice 1, the first mark ($10) on voice 2.
    assert_eq!((v1[0].command, v1[0].data), (TEMPO, 6));
    assert_eq!((v2[0].command, v2[0].data), (TIMING_MARK, 0x10));
    // Row 16: voice 1 holds the tie, so the tempo goes to voice 2 and the mark ($11) to voice 3.
    assert_eq!((v1[16].command, v1[16].data), (TIE, 0));
    assert_eq!((v2[16].command, v2[16].data), (TEMPO, 5));
    assert_eq!((v3[16].command, v3[16].data), (TIMING_MARK, 0x11));
}

#[test]
fn what_cant_be_written_is_refused() {
    let lib = library();
    // An unknown preset.
    let s = subtune(
        "m",
        vec![line(1, "flute", vec![note(0, 4, "C-4", Attack::Fresh)])],
    );
    assert_eq!(
        export(&song(vec![s]), &lib, &[], &header()),
        Err(ExportError::UnknownPreset(preset("flute")))
    );
    // A tie and an arpeggio on one row.
    let mut lead = line(
        1,
        "lead",
        vec![
            note(0, 4, "C-4", Attack::Fresh),
            note(4, 4, "E-4", Attack::Legato),
        ],
    );
    lead.arps = vec![ArpEvent {
        row: 4,
        length: NonZeroU32::new(4).unwrap(),
        intervals: vec![0, 3],
        decision: None,
    }];
    assert!(matches!(
        export(&song(vec![subtune("m", vec![lead])]), &lib, &[], &header()),
        Err(ExportError::CommandClash {
            voice: Some(_),
            row: 4,
            ..
        })
    ));
    // Three ties on a row that changes tempo: no free slot.
    let tied = |v| {
        line(
            v,
            "lead",
            vec![
                note(0, 4, "C-4", Attack::Fresh),
                note(4, 4, "D-4", Attack::Legato),
            ],
        )
    };
    let mut s = subtune("m", vec![tied(1), tied(2), tied(3)]);
    s.tempo = vec![TempoChange {
        row: 4,
        frames_per_row: FramesPerRow::new(4).unwrap(),
    }];
    assert!(matches!(
        export(&song(vec![s]), &lib, &[], &header()),
        Err(ExportError::CommandClash {
            voice: None,
            row: 4,
            ..
        })
    ));
    // An arpeggio passing B-7.
    let mut high = line(1, "lead", vec![note(0, 4, "G#7", Attack::Fresh)]);
    high.arps = vec![ArpEvent {
        row: 0,
        length: NonZeroU32::new(4).unwrap(),
        intervals: vec![0, 4],
        decision: None,
    }];
    assert!(matches!(
        export(&song(vec![subtune("m", vec![high])]), &lib, &[], &header()),
        Err(ExportError::TooHigh { row: 0, .. })
    ));
    // An instrument with nothing to arpeggiate.
    let mut lib2 = lib.clone();
    lib2.get_mut(&preset("lead")).unwrap().wave.rows[0].0 = 0x40; // no gate bit
    let mut arp = line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)]);
    arp.arps = vec![ArpEvent {
        row: 0,
        length: NonZeroU32::new(4).unwrap(),
        intervals: vec![0, 4],
        decision: None,
    }];
    assert_eq!(
        export(&song(vec![subtune("m", vec![arp])]), &lib2, &[], &header()),
        Err(ExportError::NoArpWaveform(preset("lead")))
    );
}

#[test]
fn the_song_round_trips_through_its_file() {
    let s1 = subtune(
        "a",
        vec![
            line(
                1,
                "lead",
                vec![
                    note(0, 4, "C-4", Attack::Fresh),
                    note(4, 4, "E-4", Attack::Legato),
                ],
            ),
            line(2, "bass", vec![note(0, 8, "C-2", Attack::Fresh)]),
        ],
    );
    let s2 = subtune(
        "b",
        vec![line(3, "bass", vec![note(8, 8, "G-2", Attack::Fresh)])],
    );
    let out = export(&song(vec![s1, s2]), &library(), &[], &header()).unwrap();
    let bytes = out.sng.write().unwrap();
    assert_eq!(Sng::read(&bytes).unwrap(), out.sng);
    assert!(crate::annotate::sng(&bytes).is_ok());
    assert_eq!(out.sng.subtunes.len(), 2);
}
