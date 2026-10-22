//! Detecting chords beat by beat, and playing them as arpeggios.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
use crate::score::{
    FramesPerRow, PresetId, Role, Roles, Schema, Song, Subtune, SubtuneName, Voice, VoiceLine,
};
use crate::strategies;
use crate::validate::{Context, InstrumentFacts, RoleRanges, validate};

fn name() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn at(subtune: &SubtuneName) -> Placement<'_> {
    Placement {
        subtune,
        voice: Voice::new(2).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
    }
}

fn beat() -> NonZeroU8 {
    NonZeroU8::new(4).unwrap()
}

fn rows(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn note(row: u32, length: u32, pitch: &str) -> Note {
    Note {
        row,
        length: rows(length),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(0.9).unwrap(),
        decision: None,
    }
}

/// A chord's notes over rows `row..row + length`.
fn chord(row: u32, length: u32, pitches: &[&str]) -> Vec<Note> {
    pitches.iter().map(|p| note(row, length, p)).collect()
}

fn found(spans: &[ChordSpan]) -> Vec<(u32, u32, u8, ChordQuality)> {
    spans
        .iter()
        .map(|s| (s.row, s.length.get(), s.chord.root, s.chord.quality))
        .collect()
}

#[test]
fn triads_and_sevenths_are_named() {
    use ChordQuality::*;
    let cases: [(&[&str], u8, ChordQuality); 7] = [
        (&["C-4", "E-4", "G-4"], 0, Major),
        (&["A-3", "C-4", "E-4"], 9, Minor),
        (&["D-4", "G-4", "A-4"], 2, Sus4),
        (&["D-4", "E-4", "A-4"], 2, Sus2),
        (&["G-3", "B-3", "D-4", "F-4"], 7, Dominant7),
        (&["D-4", "F-4", "A-4", "C-5"], 2, Minor7),
        (&["F-3", "A-3", "C-4", "E-4"], 5, Major7),
    ];
    for (pitches, root, quality) in cases {
        let spans = detect_chords(&chord(0, 4, pitches), beat(), rows(4));
        assert_eq!(found(&spans), [(0, 4, root, quality)], "{pitches:?}");
        assert!(
            spans[0].confidence.get() > 0.8,
            "{pitches:?}: {:?}",
            spans[0].confidence
        );
    }
}

#[test]
fn an_inversion_has_its_real_root() {
    // E G C: a C major triad in first inversion.
    let spans = detect_chords(&chord(0, 4, &["E-3", "G-3", "C-4"]), beat(), rows(4));
    assert_eq!(found(&spans), [(0, 4, 0, ChordQuality::Major)]);
}

#[test]
fn a_bare_fifth_reads_as_the_simplest_chord() {
    // C and G only: major and minor (and sus) fit equally; the tie goes to major.
    let spans = detect_chords(&chord(0, 4, &["C-4", "G-4"]), beat(), rows(4));
    assert_eq!(found(&spans), [(0, 4, 0, ChordQuality::Major)]);
}

#[test]
fn a_passing_tone_lowers_the_confidence_but_not_the_chord() {
    let clean = detect_chords(&chord(0, 4, &["C-4", "E-4", "G-4"]), beat(), rows(4));
    let mut notes = chord(0, 4, &["C-4", "E-4", "G-4"]);
    notes.push(note(0, 1, "D-4"));
    let passing = detect_chords(&notes, beat(), rows(4));
    assert_eq!(found(&passing), found(&clean));
    assert!(passing[0].confidence < clean[0].confidence);
}

#[test]
fn beats_with_the_same_chord_merge_and_silence_has_none() {
    // Bar of four beats: C major, C major, silence, A minor (as 3 + 1 rows, the last beat cut by the subtune's end).
    let mut notes = chord(0, 8, &["C-4", "E-4", "G-4"]);
    notes.extend(chord(12, 4, &["A-3", "C-4", "E-4"]));
    let spans = detect_chords(&notes, beat(), rows(15));
    assert_eq!(
        found(&spans),
        [
            (0, 8, 0, ChordQuality::Major),
            (12, 3, 9, ChordQuality::Minor)
        ]
    );
}

#[test]
fn a_note_counts_where_it_sounds() {
    // A held C under an E-G that starts a beat later: the first beat is a lone C (no chord, under the floor); in the
    // second the held C completes C major.
    let mut notes = vec![note(0, 8, "C-4")];
    notes.extend(chord(4, 4, &["E-4", "G-4"]));
    let spans = detect_chords(&notes, beat(), rows(8));
    assert_eq!(found(&spans), [(4, 4, 0, ChordQuality::Major)]);
}

#[test]
fn arpeggios_sit_in_range_and_record_their_decisions() {
    let n = name();
    let range = RoleRanges::default().arp; // C-3 to C-6
    let spans = detect_chords(&chord(0, 4, &["G-1", "B-1", "D-2"]), beat(), rows(4));
    let a = arpeggiate(&spans, &at(&n), range);
    assert_eq!(a.notes.len(), 1);
    let root = a.notes[0].pitch;
    assert!(range.contains(root), "{root}");
    assert_eq!(root.semitone() % 12, 7, "a G");
    assert_eq!(a.arps[0].intervals, [0, 4, 7]);
    assert_eq!((a.arps[0].row, a.arps[0].length.get()), (0, 4));
    let d = &a.decisions[0];
    assert_eq!(d.id.as_str(), "chord-v2-r0");
    assert_eq!(
        d.proposal,
        Proposal::Chord {
            row: 0,
            root: 7,
            intervals: vec![0, 4, 7]
        }
    );
    assert_eq!(
        d.provenance,
        Provenance::Rules {
            rule: "chords@1".into()
        }
    );
    assert_eq!(a.notes[0].decision.as_ref(), Some(&d.id));
    assert_eq!(a.arps[0].decision.as_ref(), Some(&d.id));
    assert_eq!(d.confidence, spans[0].confidence);
}

#[test]
fn every_chord_tone_stays_playable() {
    // A range at the very top: a B major 7th's root must sit low enough for its seventh (11 semitones up).
    let n = name();
    let top = PitchRange {
        low: "G-7".parse().unwrap(),
        high: Pitch::MAX,
    };
    let spans = [ChordSpan {
        row: 0,
        length: rows(4),
        chord: Chord {
            root: 11,
            quality: ChordQuality::Major7,
        },
        confidence: Confidence::CERTAIN,
    }];
    let a = arpeggiate(&spans, &at(&n), top);
    let root = a.notes[0].pitch.semitone();
    assert_eq!(root % 12, 11);
    assert!(root + 11 <= Pitch::MAX.semitone());
}

/// The arp line checked in a 512-row subtune under `full`, on voice 2 as an arp.
fn checked(a: Arpeggio) -> (Song, Context) {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: name(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: rows(512),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(2).unwrap(),
                roles: Roles::constant(Role::Arp),
                instrument: PresetId("arp".into()),
                notes: a.notes,
                arps: a.arps,
            }],
        }],
    };
    let facts = InstrumentFacts {
        filter: false,
        pulse_at_start: true,
        gate_timer: 2,
        hard_restart: true,
        gate_off: true,
    };
    let ctx = Context {
        instruments: BTreeMap::from([(PresetId("arp".into()), facts)]),
        ..Context::default()
    };
    (song, ctx)
}

proptest! {
    /// On any note soup: spans are in order, apart, inside the subtune and on beat boundaries; the arpeggios they give
    /// validate with no errors (pitch-range warnings allowed); and the order of the notes does not matter.
    #[test]
    fn any_soup_gives_ordered_spans_and_a_valid_arp_line(
        soup in strategies::note_soup(),
        seed in any::<u64>(),
        length in 1u32..300,
    ) {
        let length = rows(length);
        let spans = detect_chords(&soup, beat(), length);
        let mut end = 0;
        for s in &spans {
            prop_assert!(s.row >= end, "{s:?} overlaps");
            prop_assert_eq!(s.row % 4, 0);
            end = s.row + s.length.get();
            prop_assert!(end <= length.get());
            prop_assert!(s.chord.root < 12);
        }
        let n = name();
        let a = arpeggiate(&spans, &at(&n), RoleRanges::default().arp);
        prop_assert_eq!(a.notes.len(), spans.len());
        for (note, arp) in a.notes.iter().zip(&a.arps) {
            let top = *arp.intervals.last().unwrap();
            prop_assert!(note.pitch.semitone() + top <= Pitch::MAX.semitone());
            prop_assert_eq!((note.row, note.length), (arp.row, arp.length));
            prop_assert_eq!(&note.decision, &arp.decision);
        }
        // Spans come in row order, so equal ids would sit side by side.
        let mut ids: Vec<_> = a.decisions.iter().map(|d| d.id.clone()).collect();
        ids.dedup();
        prop_assert_eq!(ids.len(), a.decisions.len());
        let (song, ctx) = checked(a);
        // Every beat met the floor, or half of it carrying a chord on, so no span is under half.
        prop_assert!(spans.iter().all(|s| s.confidence.get() >= MIN_SCORE / 2.0));
        let errors: Vec<_> = validate(&song, &ctx).into_iter().filter(|p| p.is_error()).collect();
        prop_assert!(errors.is_empty(), "{errors:#?}");
        let mut shuffled = soup;
        let mut x = seed | 1;
        for i in (1..shuffled.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            shuffled.swap(i, (x % (i as u64 + 1)) as usize);
        }
        prop_assert_eq!(detect_chords(&shuffled, beat(), length), spans);
    }
}

fn weighted(row: u32, length: u32, pitch: &str, confidence: f32) -> Note {
    let mut n = note(row, length, pitch);
    n.confidence = Confidence::new(confidence).unwrap();
    n
}

#[test]
fn the_bass_settles_chords_with_the_same_notes_whatever_the_weights() {
    // D E A is D sus2 and A sus4; with D lowest it must read as D sus2 for any confidences.
    for (d, e, a) in [
        (0.9, 0.9, 0.9),
        (0.31, 0.77, 0.53),
        (0.99, 0.12, 0.47),
        (0.2, 0.6, 0.8),
    ] {
        let notes = [
            weighted(0, 4, "D-4", d),
            weighted(0, 4, "E-4", e),
            weighted(0, 4, "A-4", a),
        ];
        let spans = detect_chords(&notes, beat(), rows(4));
        assert_eq!(
            found(&spans),
            [(0, 4, 2, ChordQuality::Sus2)],
            "{d} {e} {a}"
        );
    }
}

#[test]
fn missing_tones_and_a_cluster_weaken_or_drop_the_chord() {
    // A lone C is a third of C major, under the floor: no chord. A bare fifth is two thirds.
    assert!(detect_chords(&[note(0, 4, "C-4")], beat(), rows(4)).is_empty());
    let fifth = detect_chords(&chord(0, 4, &["C-4", "G-4"]), beat(), rows(4));
    assert!((fifth[0].confidence.get() - 2.0 / 3.0).abs() < 1e-5);
    // A faint G with C and E counts for little: nearly the same as C and E alone.
    let mut faint = chord(0, 4, &["C-4", "E-4"]);
    faint.push(weighted(0, 1, "G-4", 0.01));
    let s = detect_chords(&faint, beat(), rows(4));
    assert!(s[0].confidence.get() < 0.7, "{:?}", s[0].confidence);
    // Four semitones in a row is no chord.
    let cluster = detect_chords(&chord(0, 4, &["C-4", "C#4", "D-4", "D#4"]), beat(), rows(4));
    assert!(cluster.is_empty(), "{cluster:?}");
}

#[test]
fn the_score_is_tones_on_minus_tones_off() {
    // C E G at 0.9 for four rows (3.6 each) and a passing D for one (0.9): (2 × 10.8 − 11.7) / 11.7.
    let mut notes = chord(0, 4, &["C-4", "E-4", "G-4"]);
    notes.push(note(0, 1, "D-4"));
    let spans = detect_chords(&notes, beat(), rows(4));
    assert!((spans[0].confidence.get() - (2.0 * 10.8 - 11.7) / 11.7).abs() < 1e-4);
}

#[test]
fn a_faint_seventh_costs_more_than_it_brings() {
    // C E G and a faint B: the triad, because a seventh chord pays for its fourth tone.
    let mut notes = chord(0, 4, &["C-4", "E-4", "G-4"]);
    notes.push(weighted(0, 1, "B-4", 0.2));
    assert_eq!(
        found(&detect_chords(&notes, beat(), rows(4)))[0].3,
        ChordQuality::Major
    );
}

#[test]
fn a_chords_root_must_sound() {
    // E and G: C major would fit as well as E minor, but C does not sound.
    let spans = detect_chords(&chord(0, 4, &["E-4", "G-4"]), beat(), rows(4));
    assert_eq!(found(&spans), [(0, 4, 4, ChordQuality::Minor)]);
    // With G in the bass the bass rule can't settle it (G is neither chord's root): only the root check keeps C major
    // out.
    let spans = detect_chords(&chord(0, 4, &["G-3", "E-4", "G-4"]), beat(), rows(4));
    assert_eq!(found(&spans), [(0, 4, 4, ChordQuality::Minor)]);
}

#[test]
fn rows_and_confidence_both_weigh() {
    // C and E shared; G against A decides C major or A minor. A sounds longer: A minor.
    let mut notes = chord(0, 4, &["C-4", "E-4"]);
    notes.push(note(0, 4, "A-3"));
    notes.push(note(0, 1, "G-4"));
    assert_eq!(found(&detect_chords(&notes, beat(), rows(4)))[0].2, 9);
    // A is surer: A minor again.
    let mut notes = chord(0, 4, &["C-4", "E-4"]);
    notes.push(weighted(0, 4, "A-3", 0.9));
    notes.push(weighted(0, 4, "G-4", 0.2));
    assert_eq!(found(&detect_chords(&notes, beat(), rows(4)))[0].2, 9);
}

#[test]
fn only_adjacent_beats_merge_and_spans_average_their_beats() {
    // C, silence, C: two spans.
    let mut notes = chord(0, 4, &["C-4", "E-4", "G-4"]);
    notes.extend(chord(8, 4, &["C-4", "E-4", "G-4"]));
    let spans = detect_chords(&notes, beat(), rows(12));
    assert_eq!(
        found(&spans).iter().map(|s| s.0).collect::<Vec<_>>(),
        [0, 8]
    );
    // A clean beat and one with a passing tone: the span's confidence is their mean.
    let mut notes = chord(0, 8, &["C-4", "E-4", "G-4"]);
    notes.push(note(4, 1, "D-4"));
    let spans = detect_chords(&notes, beat(), rows(8));
    let second = (2.0 * 10.8 - 11.7) / 11.7;
    assert!((spans[0].confidence.get() - (1.0 + second) / 2.0).abs() < 1e-4);
}

#[test]
fn roots_sit_nearest_the_middle_of_the_range() {
    // Arp range C-3 to C-6, middle F#4 (54): a G chord's root is G-4 (55), not G-3 or G-5.
    let n = name();
    let g = ChordSpan {
        row: 32,
        length: rows(4),
        chord: Chord {
            root: 7,
            quality: ChordQuality::Major,
        },
        confidence: Confidence::CERTAIN,
    };
    let a = arpeggiate(&[g], &at(&n), RoleRanges::default().arp);
    assert_eq!(a.notes[0].pitch.to_string(), "G-4");
    assert_eq!(a.decisions[0].bar, 2);
    // A range narrower than an octave with no A in it: the playable A nearest its middle, not the highest.
    let narrow = PitchRange {
        low: "C-4".parse().unwrap(),
        high: "E-4".parse().unwrap(),
    };
    let span = ChordSpan {
        chord: Chord {
            root: 9,
            quality: ChordQuality::Major,
        },
        ..g
    };
    let a = arpeggiate(&[span], &at(&n), narrow);
    assert_eq!(a.notes[0].pitch.to_string(), "A-3");
}

#[test]
fn a_doubled_tone_costs_the_chord_nothing() {
    // C in three octaves over E and G: as complete a C major as C E G.
    let doubled = detect_chords(
        &chord(0, 4, &["C-2", "C-3", "C-4", "E-4", "G-4"]),
        beat(),
        rows(4),
    );
    assert_eq!(found(&doubled), [(0, 4, 0, ChordQuality::Major)]);
    assert!(
        (doubled[0].confidence.get() - 1.0).abs() < 1e-5,
        "{:?}",
        doubled[0].confidence
    );
}

#[test]
fn a_held_bass_keeps_its_chord_under_a_running_melody() {
    // C held for three beats; E-G in beats 1 and 3; in beat 2 the melody runs C D E F, one row each. The C major
    // already sounding carries on through beat 2.
    let mut notes = vec![note(0, 12, "C-3")];
    notes.extend(chord(0, 4, &["E-4", "G-4"]));
    notes.extend(chord(8, 4, &["E-4", "G-4"]));
    for (i, p) in ["C-5", "D-5", "E-5", "F-5"].iter().enumerate() {
        notes.push(note(4 + i as u32, 1, p));
    }
    let spans = detect_chords(&notes, beat(), rows(12));
    assert_eq!(found(&spans), [(0, 12, 0, ChordQuality::Major)]);
    // Heard on its own, that beat reads as a D minor 7th (D, F and the held C): only the chord already sounding,
    // which a new one must clearly beat, keeps it C.
    let mut cold: Vec<Note> = notes
        .iter()
        .filter(|n| (4..8).contains(&n.row))
        .cloned()
        .collect();
    cold.push(note(4, 4, "C-3"));
    assert_eq!(
        found(&detect_chords(&cold, beat(), rows(8)))[0].3,
        ChordQuality::Minor7
    );
}

#[test]
fn the_floor_sits_between_a_lone_note_and_a_weak_dyad() {
    // C with an E faint enough that the chord scores about 0.45: a chord. Fainter, about 0.35: none.
    let with_e = |confidence| {
        let mut notes = vec![note(0, 4, "C-4")];
        notes.push(weighted(0, 1, "E-4", confidence));
        detect_chords(&notes, beat(), rows(4))
    };
    let weak = with_e(0.108);
    assert_eq!(weak.len(), 1);
    assert!(
        (weak[0].confidence.get() - 0.45).abs() < 0.02,
        "{:?}",
        weak[0].confidence
    );
    assert!(with_e(0.015).is_empty());
}

#[test]
fn a_seventh_chords_share_is_over_its_four_tones() {
    // G B D at 3.6 each and a faint F at 0.2: F is under a quarter of the tones' mean (0.25 × 11 / 4), so it counts
    // as 0.2 / 0.6875 of a tone, and the share is over all four.
    let mut weight = [0.0f32; 12];
    weight[7] = 3.6;
    weight[11] = 3.6;
    weight[2] = 3.6;
    weight[5] = 0.2;
    let g7 = Chord {
        root: 7,
        quality: ChordQuality::Dominant7,
    };
    let share = (3.0 + 0.2 / (0.25 * 11.0 / 4.0)) / 4.0;
    let expected = (2.0 * 11.0 - 11.0) / 11.0 * share - 0.05;
    assert!((score(&weight, g7).unwrap() - expected).abs() < 1e-5);
    // The triad wins: the faint F costs the seventh more than it brings.
    assert_eq!(
        best_chord(&weight, Some(7)).unwrap().0.quality,
        ChordQuality::Major
    );
}

#[test]
fn a_clear_new_chord_still_takes_over() {
    // C major then F major, both full: the F beat changes the chord at once.
    let mut notes = chord(0, 4, &["C-4", "E-4", "G-4"]);
    notes.extend(chord(4, 4, &["F-4", "A-4", "C-5"]));
    let spans = detect_chords(&notes, beat(), rows(8));
    assert_eq!(
        found(&spans),
        [
            (0, 4, 0, ChordQuality::Major),
            (4, 4, 5, ChordQuality::Major)
        ]
    );
}
