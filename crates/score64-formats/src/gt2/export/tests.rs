//! Scores as GoatTracker songs: pattern cuts, notes, ties, key-offs, song-wide commands, instruments and their
//! tables, arpeggios, errors, and a round trip through the song file.

use std::num::{NonZeroU8, NonZeroU32};

use score64_core::profile::ProfileName;
use score64_core::{
    ArpEvent, Confidence, FramesPerRow, Note, Pitch, Role, Roles, Schema, Switch, TempoChange,
};

use proptest::prelude::*;

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
                    note(0, 16, "C-4", Attack::Fresh),
                    note(16, 4, "D-4", Attack::Legato),
                ],
            ),
            line(2, "bass", vec![note(0, 4, "C-2", Attack::Fresh)]),
        ],
    );
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

fn arp(row: u32, length: u32, intervals: &[u8]) -> ArpEvent {
    ArpEvent {
        row,
        length: NonZeroU32::new(length).unwrap(),
        intervals: intervals.to_vec(),
        decision: None,
    }
}

fn one(lead: VoiceLine) -> Result<Exported, ExportError> {
    export(
        &song(vec![subtune("m", vec![lead])]),
        &library(),
        &[],
        &header(),
    )
}

#[test]
fn back_to_back_notes_have_no_key_off_between() {
    let out = one(line(
        1,
        "lead",
        vec![
            note(0, 4, "C-4", Attack::Fresh),
            note(4, 4, "D-4", Attack::Fresh),
        ],
    ))
    .unwrap();
    let rows = rows_of(&out.sng, 0, 0);
    assert_eq!((rows[4].note, rows[4].instrument), (n("D-4"), 1));
    assert_eq!(rows[8].note, KEY_OFF);
}

#[test]
fn the_loop_stops_a_note_that_runs_to_the_end() {
    // A note to the end, looping to a silent row 0: a key-off there, so it doesn't sound on after the jump.
    let out = one(line(1, "lead", vec![note(16, 16, "C-4", Attack::Fresh)])).unwrap();
    assert_eq!(rows_of(&out.sng, 0, 0)[0].note, KEY_OFF);
    // A note on the loop row restarts anyway.
    let out = one(line(
        1,
        "lead",
        vec![
            note(0, 4, "D-4", Attack::Fresh),
            note(16, 16, "C-4", Attack::Fresh),
        ],
    ))
    .unwrap();
    assert_eq!(rows_of(&out.sng, 0, 0)[0].note, n("D-4"));
    // A note across the loop row can't be played after the jump.
    let mut s = subtune(
        "m",
        vec![line(1, "lead", vec![note(12, 8, "C-4", Attack::Fresh)])],
    );
    s.loop_row = 16;
    assert!(matches!(
        export(&song(vec![s]), &library(), &[], &header()),
        Err(ExportError::Invalid(why)) if why.contains("across the loop row")
    ));
}

#[test]
fn the_loop_row_sets_the_tempo_it_needs() {
    let mut s = subtune(
        "m",
        vec![line(1, "lead", vec![note(16, 4, "C-4", Attack::Fresh)])],
    );
    s.loop_row = 16;
    s.tempo = vec![TempoChange {
        row: 24,
        frames_per_row: FramesPerRow::new(4).unwrap(),
    }];
    let out = export(&song(vec![s.clone()]), &library(), &[], &header()).unwrap();
    let tempo = |sng: &Sng, row: usize| {
        (0..3)
            .map(|v| rows_of(sng, 0, v)[row])
            .find(|r| r.command == TEMPO)
            .map(|r| r.data)
    };
    assert_eq!(
        tempo(&out.sng, 16),
        Some(6),
        "back to 6 after the jump from the 4 at the end"
    );
    assert_eq!(tempo(&out.sng, 24), Some(4));
    // No change in the loop: nothing extra.
    s.tempo.clear();
    let out = export(&song(vec![s]), &library(), &[], &header()).unwrap();
    assert_eq!(tempo(&out.sng, 16), None);
}

#[test]
fn scores_the_validator_would_refuse_are_refused_not_written_wrong() {
    let invalid = |lead: VoiceLine, loop_row: u32, tempo_row: Option<u32>| {
        let mut s = subtune("m", vec![lead]);
        s.loop_row = loop_row;
        s.tempo = tempo_row
            .map(|row| TempoChange {
                row,
                frames_per_row: FramesPerRow::new(4).unwrap(),
            })
            .into_iter()
            .collect();
        match export(&song(vec![s]), &library(), &[], &header()) {
            Err(ExportError::Invalid(why)) => why,
            other => panic!("{other:?}"),
        }
    };
    let c = || line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)]);
    assert!(
        invalid(
            line(1, "lead", vec![note(30, 4, "C-4", Attack::Fresh)]),
            0,
            None
        )
        .contains("past the end")
    );
    assert!(invalid(c(), 32, None).contains("loop row 32"));
    assert!(invalid(c(), 0, Some(40)).contains("tempo change at row 40"));
    let overlap = line(
        1,
        "lead",
        vec![
            note(0, 8, "C-4", Attack::Fresh),
            note(4, 4, "D-4", Attack::Fresh),
        ],
    );
    assert!(invalid(overlap, 0, None).contains("overlap"));
    let loose_tie = line(
        1,
        "lead",
        vec![
            note(0, 4, "C-4", Attack::Fresh),
            note(8, 4, "D-4", Attack::Legato),
        ],
    );
    assert!(invalid(loose_tie, 0, None).contains("no note to tie to"));
}

#[test]
fn an_arpeggio_follows_the_notes_it_spans() {
    // Over two fresh notes: a pointer on each, since a new note resets the wave pointer.
    let mut l = line(
        1,
        "lead",
        vec![
            note(0, 8, "C-3", Attack::Fresh),
            note(8, 8, "F-3", Attack::Fresh),
        ],
    );
    l.arps = vec![arp(0, 16, &[0, 4, 7])];
    let rows = rows_of(&one(l).unwrap().sng, 0, 0);
    assert_eq!(
        (rows[0].command, rows[8].command),
        (SET_WAVE_POINTER, SET_WAVE_POINTER)
    );
    assert_eq!(rows[0].data, rows[8].data);
    assert_eq!(rows[16].command, 0, "the event ends with the note");
    // Starting over silence: from the first note inside it.
    let mut l = line(1, "lead", vec![note(4, 8, "C-3", Attack::Fresh)]);
    l.arps = vec![arp(0, 12, &[0, 3])];
    let rows = rows_of(&one(l).unwrap().sng, 0, 0);
    assert_eq!(
        (rows[0].command, rows[4].command),
        (TEMPO, SET_WAVE_POINTER)
    );
    // A tie inside keeps the program; the restore comes where the event ends, though the tied note sounds on.
    let mut l = line(
        1,
        "lead",
        vec![
            note(0, 4, "C-3", Attack::Fresh),
            note(4, 12, "D-3", Attack::Legato),
        ],
    );
    l.arps = vec![arp(0, 8, &[0, 4])];
    let rows = rows_of(&one(l).unwrap().sng, 0, 0);
    assert_eq!(
        (rows[4].command, rows[8].command, rows[8].data),
        (TIE, SET_WAVE_POINTER, 1)
    );
    // Two arpeggios back to back on one note: the second sets its own program, with no restore between.
    let mut l = line(1, "lead", vec![note(0, 16, "C-3", Attack::Fresh)]);
    l.arps = vec![arp(4, 4, &[0, 4, 7]), arp(8, 4, &[0, 3, 7])];
    let out = one(l).unwrap();
    let rows = rows_of(&out.sng, 0, 0);
    assert_eq!(
        (rows[4].command, rows[8].command, rows[12].command),
        (SET_WAVE_POINTER, SET_WAVE_POINTER, SET_WAVE_POINTER)
    );
    assert_ne!(rows[4].data, rows[8].data);
    assert_eq!(rows[12].data, 1, "back to the instrument's own program");
    // An event ending where a new note starts leaves the pointer to that note.
    let mut l = line(
        1,
        "lead",
        vec![
            note(0, 8, "C-3", Attack::Fresh),
            note(8, 8, "D-3", Attack::Fresh),
        ],
    );
    l.arps = vec![arp(0, 8, &[0, 4])];
    let rows = rows_of(&one(l).unwrap().sng, 0, 0);
    assert_eq!((rows[8].note, rows[8].command), (n("D-3"), 0));
}

#[test]
fn the_highest_arpeggio_note_is_b7() {
    let mut ok = line(1, "lead", vec![note(0, 4, "G#7", Attack::Fresh)]);
    ok.arps = vec![arp(0, 4, &[0, 3])];
    assert!(one(ok).is_ok());
    let mut high = line(1, "lead", vec![note(0, 4, "G#7", Attack::Fresh)]);
    high.arps = vec![arp(0, 4, &[0, 4])];
    assert!(matches!(one(high), Err(ExportError::TooHigh { .. })));
    let mut huge = line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)]);
    huge.arps = vec![arp(0, 4, &[0, 255])];
    assert!(
        matches!(one(huge), Err(ExportError::TooHigh { .. })),
        "no overflow"
    );
}

#[test]
fn wavetable_commands_and_vibrato_point_into_the_songs_tables() {
    // An instrument whose wavetable runs 9XY (pulse), AXY (filter) and 4XY (vibrato, speedtable), with all four
    // tables of its own, placed after another instrument that has one row of each.
    let mut rich = ins(0x41);
    rich.wave.rows = vec![
        (0x41, 0),
        (0xF9, 0x01),
        (0xFA, 0x01),
        (0xF4, 0x01),
        (0xF3, 0x00),
        (WAVE_JUMP, 0),
    ];
    rich.filter.rows = vec![(0x90, 0xF1), (WAVE_JUMP, 0x01)];
    rich.speed.rows = vec![(0x02, 0x30)];
    rich.instrument.filter = 1;
    rich.instrument.vibrato = 1;
    let mut first = ins(0x21);
    first.filter.rows = vec![(0x00, 0x80)];
    first.speed.rows = vec![(0x01, 0x10)];
    let lib: Library = [(preset("first"), first), (preset("rich"), rich)]
        .into_iter()
        .collect();
    let s = subtune(
        "m",
        vec![
            line(1, "first", vec![note(0, 4, "C-4", Attack::Fresh)]),
            line(2, "rich", vec![note(0, 4, "C-4", Attack::Fresh)]),
        ],
    );
    let out = export(&song(vec![s]), &lib, &[], &header()).unwrap();
    let r = &out.sng.instruments[1];
    assert_eq!((r.wave, r.pulse, r.filter, r.vibrato), (3, 3, 2, 2));
    assert_eq!(
        out.sng.wave.rows[2..],
        [
            (0x41, 0),
            (0xF9, 0x03),
            (0xFA, 0x02),
            (0xF4, 0x02),
            (0xF3, 0x00),
            (WAVE_JUMP, 0)
        ],
        "pulse row 1 of 2 moves to 3, filter 1 to 2, speed 1 to 2; a zero parameter stays"
    );
    assert_eq!(out.sng.filter.rows[1..], [(0x90, 0xF1), (WAVE_JUMP, 0x02)]);
    assert_eq!(out.sng.speed.rows, [(0x01, 0x10), (0x02, 0x30)]);
}

#[test]
fn songs_past_goattrackers_limits_are_refused() {
    // 62 instruments fit; the 63rd is left free.
    let lib: Library = (0..63)
        .map(|i| (preset(&format!("i{i}")), ins(0x41)))
        .collect();
    let voices = |count: usize| {
        let mut s = subtune("m", vec![]);
        s.length = NonZeroU32::new(count as u32).unwrap();
        s.voices = vec![VoiceLine {
            notes: (0..count as u32)
                .map(|r| {
                    let mut n = note(r, 1, "C-4", Attack::Fresh);
                    n.instrument = Some(preset(&format!("i{r}")));
                    n
                })
                .collect(),
            ..line(1, "i0", vec![])
        }];
        s
    };
    assert!(export(&song(vec![voices(62)]), &lib, &[], &header()).is_ok());
    assert!(matches!(
        export(&song(vec![voices(63)]), &lib, &[], &header()),
        Err(ExportError::TooMuch(why)) if why.contains("62 instruments")
    ));
    // More than 208 distinct patterns: 210 one-row bars, each a different note or instrument.
    let mut s = subtune("m", vec![]);
    s.rows_per_bar = NonZeroU8::new(1).unwrap();
    s.length = NonZeroU32::new(210).unwrap();
    let pitches = ["C-3", "D-3", "E-3", "F-3", "G-3"];
    s.voices = vec![VoiceLine {
        notes: (0..210u32)
            .map(|r| {
                let mut n = note(r, 1, pitches[(r % 5) as usize], Attack::Fresh);
                n.instrument = Some(preset(&format!("i{}", r / 5)));
                n
            })
            .collect(),
        ..line(1, "i0", vec![])
    }];
    assert!(matches!(
        export(&song(vec![s]), &lib, &[], &header()),
        Err(ExportError::TooMuch(why)) if why.contains("208")
    ));
    // Tables past 255 rows.
    let mut big = ins(0x41);
    big.pulse.rows = vec![(0x88, 0); 200];
    let lib: Library = [(preset("a"), big.clone()), (preset("b"), big)]
        .into_iter()
        .collect();
    let s = subtune(
        "m",
        vec![
            line(1, "a", vec![note(0, 4, "C-4", Attack::Fresh)]),
            line(2, "b", vec![note(0, 4, "C-4", Attack::Fresh)]),
        ],
    );
    assert!(matches!(
        export(&song(vec![s]), &lib, &[], &header()),
        Err(ExportError::TooMuch(why)) if why.contains("255 rows")
    ));
}

#[test]
fn a_song_with_timing_marks_says_it_needs_them_packed() {
    let mut s = subtune(
        "calm",
        vec![line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)])],
    );
    s.marks = vec![0, 16];
    let set = AdaptiveSet::new("score", vec![SubtuneName::new("calm").unwrap()]).unwrap();
    assert!(
        export(&song(vec![s.clone()]), &library(), &[set], &header())
            .unwrap()
            .timing_marks
    );
    assert!(
        !export(&song(vec![s]), &library(), &[], &header())
            .unwrap()
            .timing_marks
    );
}

proptest! {
    /// Any song the validator passes exports, unless it needs more command slots or patterns than GoatTracker has,
    /// and the result passes GoatTracker's checks and round-trips through its file.
    #[test]
    fn every_valid_song_exports((song, ctx) in score64_core::strategies::checked_song()) {
        let lib: Library = ctx.instruments.keys().map(|p| (p.clone(), ins(0x41))).collect();
        match export(&song, &lib, &ctx.adaptive, &header()) {
            Ok(out) => {
                prop_assert!(out.sng.check().is_ok());
                let bytes = out.sng.write().unwrap();
                prop_assert_eq!(Sng::read(&bytes).unwrap(), out.sng);
            }
            Err(ExportError::CommandClash { .. } | ExportError::TooMuch(_)) => {}
            Err(e) => prop_assert!(false, "{e}"),
        }
    }
}

#[test]
fn an_arpeggio_ending_on_a_tie_is_refused() {
    let mut l = line(
        1,
        "lead",
        vec![
            note(0, 8, "C-3", Attack::Fresh),
            note(8, 8, "D-3", Attack::Legato),
        ],
    );
    l.arps = vec![arp(0, 8, &[0, 4])];
    assert!(matches!(
        one(l),
        Err(ExportError::CommandClash { voice: Some(_), row: 8, what, .. }) if what.contains("tie")
    ));
}

#[test]
fn a_tie_on_the_loop_row_needs_a_note_sounding_at_the_end() {
    let looped = |notes: Vec<Note>| {
        let mut s = subtune("m", vec![line(1, "lead", notes)]);
        s.loop_row = 16;
        export(&song(vec![s]), &library(), &[], &header())
    };
    // Silence at the end: after the jump the tie has nothing to tie to.
    assert!(matches!(
        looped(vec![note(8, 8, "C-3", Attack::Fresh), note(16, 8, "D-3", Attack::Legato)]),
        Err(ExportError::Invalid(why)) if why.contains("after the jump back")
    ));
    // A note running to the end: it ties into the loop row's note both times round.
    assert!(
        looped(vec![
            note(8, 8, "C-3", Attack::Fresh),
            note(16, 8, "D-3", Attack::Legato),
            note(28, 4, "E-3", Attack::Fresh),
        ])
        .is_ok()
    );
}

#[test]
fn overlapping_arpeggios_are_refused() {
    let mut l = line(1, "lead", vec![note(0, 16, "C-3", Attack::Fresh)]);
    l.arps = vec![arp(0, 8, &[0, 4]), arp(4, 8, &[0, 3])];
    assert!(matches!(
        one(l),
        Err(ExportError::Invalid(why)) if why.contains("arpeggios overlap at row 4")
    ));
}

#[test]
fn the_restore_points_at_the_sounding_notes_own_program() {
    // `bass` is the second instrument, so its own program starts at wavetable row 3.
    let mut bass = line(2, "bass", vec![note(0, 16, "C-2", Attack::Fresh)]);
    bass.arps = vec![arp(0, 8, &[0, 7])];
    let s = subtune(
        "m",
        vec![
            line(1, "lead", vec![note(0, 4, "C-4", Attack::Fresh)]),
            bass,
        ],
    );
    let out = export(&song(vec![s]), &library(), &[], &header()).unwrap();
    assert_eq!(out.sng.instruments[1].wave, 3);
    let rows = rows_of(&out.sng, 0, 1);
    assert_eq!((rows[8].command, rows[8].data), (SET_WAVE_POINTER, 3));
}

#[test]
fn an_arpeggio_running_to_the_end_cant_end_on_a_loop_row_tie() {
    let mut l = line(
        1,
        "lead",
        vec![
            note(8, 8, "C-3", Attack::Fresh),
            note(16, 8, "D-3", Attack::Legato),
            note(24, 8, "E-3", Attack::Fresh),
        ],
    );
    l.arps = vec![arp(24, 8, &[0, 4])];
    let mut s = subtune("m", vec![l]);
    s.loop_row = 16;
    assert!(matches!(
        export(&song(vec![s.clone()]), &library(), &[], &header()),
        Err(ExportError::CommandClash { row: 16, what, .. }) if what.contains("tie")
    ));
    // Ending before the end, it restores inside the subtune as usual.
    s.voices[0].arps = vec![arp(24, 4, &[0, 4])];
    assert!(export(&song(vec![s]), &library(), &[], &header()).is_ok());
}

/// Decisions in force (#167) give songs the exporter writes: a loop row a person chose, where no note sounds across
/// it, and tempo changes from tempo decisions. A loop row inside a note is refused before it gets here.
#[test]
fn loop_rows_and_tempos_in_force_export() {
    use score64_core::provenance::{Decision, DecisionId, Proposal, Provenance};
    use score64_core::reduce::{LoopRules, apply_tempo, find_loop_with};
    use score64_core::{Unapplied, check};
    let person = |id: &str, bar: u32, proposal: Proposal| Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: SubtuneName::new("main").unwrap(),
        bar,
        proposal,
        provenance: Provenance::Manual,
        confidence: Confidence::new(1.0).unwrap(),
        reviewed: false,
    };
    // Notes every 8 rows, 4 long: rows 4–7, 12–15, ... are free.
    let notes = (0..4)
        .map(|k| note(k * 8, 4, "C-4", Attack::Fresh))
        .collect();
    let mut sub = subtune("main", vec![line(1, "lead", notes)]);
    let decisions = [
        person("loop-mid", 1, Proposal::LoopRow { row: 20 }),
        person(
            "faster",
            1,
            Proposal::FramesPerRow {
                frames_per_row: FramesPerRow::new(4).unwrap(),
            },
        ),
    ];
    let checked = check(decisions, &song(vec![sub.clone()]), &Default::default());
    assert!(checked.unapplied.is_empty());
    let done = apply_tempo(&mut sub, &checked.valid);
    assert!(done.unapplied.is_empty());
    let choice = find_loop_with(&sub, &LoopRules::default(), &checked.valid);
    assert_eq!((choice.row, choice.by.as_str()), (20, "loop-mid"));
    sub.loop_row = choice.row;
    let exported = export(&song(vec![sub.clone()]), &library(), &[], &header()).unwrap();
    // The restart is the pattern that starts at row 20.
    assert_eq!(
        exported.sng.subtunes[0][0].restart, 2,
        "patterns 0–15, 16–19, 20–31"
    );
    // The tempo decision is an F04 at row 16. The loop row's tempo is the ending's, so it needs no command of its own.
    let tempo: Vec<(usize, u8)> = rows_of(&exported.sng, 0, 0)
        .iter()
        .enumerate()
        .filter(|(_, r)| r.command == 0xF)
        .map(|(i, r)| (i, r.data))
        .collect();
    assert_eq!(tempo, [(0, 6), (16, 4)]);
    // Inside a note (row 2 of the 0–3 note): refused, and the subtune keeps the rule's row.
    let inside = find_loop_with(
        &sub,
        &LoopRules::default(),
        &[person("in-note", 0, Proposal::LoopRow { row: 2 })],
    );
    assert_eq!(
        inside.unapplied,
        [(DecisionId::new("in-note").unwrap(), Unapplied::InsideNote)]
    );
    assert_eq!(inside.by.as_str(), "loop");
}
