//! The bass with arpeggios on its weak beats.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
use crate::provenance::DecisionId;
use crate::reduce::{Chord, ChordQuality, MonoRules, Skyline, detect_chords, monophonise};
use crate::score::{
    FramesPerRow, PresetId, Role, Roles, Schema, Song, Subtune, SubtuneName, Voice, VoiceLine,
};
use crate::strategies;
use crate::validate::{Context, InstrumentFacts, RoleRanges, validate};

fn name() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn at(subtune: &SubtuneName) -> Placement<'_> {
    placed(subtune, 16)
}

fn placed(subtune: &SubtuneName, rows_per_bar: u8) -> Placement<'_> {
    Placement {
        subtune,
        voice: Voice::new(2).unwrap(),
        rows_per_bar: NonZeroU8::new(rows_per_bar).unwrap(),
    }
}

fn note(row: u32, length: u32, pitch: &str) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::CERTAIN,
        decision: None,
    }
}

fn span(row: u32, length: u32, root: u8, quality: ChordQuality) -> ChordSpan {
    ChordSpan {
        row,
        length: NonZeroU32::new(length).unwrap(),
        chord: Chord { root, quality },
        confidence: Confidence::new(0.8).unwrap(),
    }
}

/// The default rules with these weak beats and rows per beat.
fn beats(rows_per_beat: u8, weak_beats: &[u8]) -> BassArpRules {
    BassArpRules {
        rows_per_beat: NonZeroU8::new(rows_per_beat).unwrap(),
        weak_beats: weak_beats.to_vec(),
        ..BassArpRules::default()
    }
}

/// A bass note on every beat of four rows, from `from` to `to`.
fn walking(from: u32, to: u32) -> Vec<Note> {
    (from..to)
        .step_by(4)
        .map(|r| note(r, 4, if r % 8 == 0 { "C-2" } else { "G-2" }))
        .collect()
}

fn bass_range() -> PitchRange {
    RoleRanges::default().bass
}

/// (row, length, pitch) of each note.
type Notes = Vec<(u32, u32, String)>;

/// The notes' (row, length, pitch), and each arpeggio's (row, length).
fn shape(a: &Arpeggio) -> (Notes, Vec<(u32, u32)>) {
    (
        a.notes
            .iter()
            .map(|n| (n.row, n.length.get(), n.pitch.to_string()))
            .collect(),
        a.arps.iter().map(|x| (x.row, x.length.get())).collect(),
    )
}

fn ids(a: &Arpeggio) -> Vec<&str> {
    a.decisions.iter().map(|d| d.id.as_str()).collect()
}

#[test]
fn the_bass_keeps_the_strong_beats_and_the_chord_takes_the_weak_ones() {
    let n = name();
    // A bass note per beat over one bar of C major: beats 2 and 4 become arpeggios.
    let bass = vec![
        note(0, 4, "C-2"),
        note(4, 4, "E-2"),
        note(8, 4, "G-2"),
        note(12, 4, "C-2"),
    ];
    let a = arpeggiate_on_bass(
        bass,
        &[span(0, 16, 0, ChordQuality::Major)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let (notes, arps) = shape(&a);
    // Roots in the bass range, nearest the bass note before them: C-2 after C-2, C-3 after G-2 (5 semitones, against
    // 7 down to C-2).
    assert_eq!(
        notes,
        [
            (0, 4, "C-2".into()),
            (4, 4, "C-2".into()),
            (8, 4, "G-2".into()),
            (12, 4, "C-3".into()),
        ]
    );
    assert_eq!(arps, [(4, 4), (12, 4)]);
    assert!(a.arps.iter().all(|x| x.intervals == [0, 4, 7]));
    assert_eq!(
        ids(&a),
        [
            "bassarp-interleave-v2-r4-p28",
            "bassarp-chord-v2-r4",
            "bassarp-interleave-v2-r12-p24",
            "bassarp-chord-v2-r12",
        ]
    );
    // The root and the arpeggio record the chord decision; the decision is as sure as the span.
    assert_eq!(
        a.notes[1].decision.as_ref().map(|d| d.as_str()),
        Some("bassarp-chord-v2-r4")
    );
    assert_eq!(a.arps[0].decision, a.notes[1].decision);
    assert_eq!(a.decisions[1].confidence, Confidence::new(0.8).unwrap());
}

#[test]
fn a_note_sounding_into_a_weak_beat_ends_there_and_a_broken_tie_restarts() {
    let n = name();
    // A whole-bar C-2, then a tie into bar 2 that loses its note when the arpeggio cuts bar 1's last beat.
    let mut tie = note(16, 4, "C-2");
    tie.attack = Attack::Legato;
    let a = arpeggiate_on_bass(
        vec![note(0, 16, "C-2"), tie],
        &[span(0, 16, 9, ChordQuality::Minor)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let (notes, arps) = shape(&a);
    assert_eq!(
        notes,
        [
            (0, 4, "C-2".into()),
            (4, 4, "A-1".into()),
            (12, 4, "A-1".into()),
            (16, 4, "C-2".into()),
        ]
    );
    assert_eq!(arps, [(4, 4), (12, 4)]);
    assert_eq!(a.notes[3].attack, Attack::Fresh);
    assert_eq!(
        ids(&a),
        [
            "bassarp-trim-v2-r0",
            "bassarp-chord-v2-r4",
            "bassarp-chord-v2-r12",
            "bassarp-tie-v2-r16",
        ]
    );
    // The trim and the restart are as sure as the chord that made them.
    let sure = Confidence::new(0.8).unwrap();
    assert!(a.decisions.iter().all(|d| d.confidence == sure));
}

#[test]
fn the_cuts_are_as_sure_as_the_chord_and_a_tie_broken_already_is_certain() {
    let n = name();
    let shaky = ChordSpan {
        confidence: Confidence::new(0.3).unwrap(),
        ..span(0, 16, 0, ChordQuality::Major)
    };
    // A note into beat 2, one starting in beat 4, and a tie straight after beat 4's arpeggio.
    let mut tie = note(16, 4, "C-2");
    tie.attack = Attack::Legato;
    // And in bar 3, with no chord, a tie whose note ended long before.
    let mut broken = note(40, 4, "C-2");
    broken.attack = Attack::Legato;
    let a = arpeggiate_on_bass(
        vec![
            note(0, 6, "C-2"),
            note(12, 4, "G-2"),
            tie,
            note(32, 2, "C-2"),
            broken,
        ],
        &[shaky],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let sure: Vec<_> = a
        .decisions
        .iter()
        .map(|d| (d.id.as_str(), d.confidence.get()))
        .collect();
    assert_eq!(
        sure,
        [
            ("bassarp-trim-v2-r0", 0.3),
            ("bassarp-chord-v2-r4", 0.3),
            ("bassarp-interleave-v2-r12-p31", 0.3),
            ("bassarp-chord-v2-r12", 0.3),
            ("bassarp-tie-v2-r16", 0.3),
            ("bassarp-tie-v2-r40", 1.0),
        ]
    );
}

#[test]
fn each_span_gets_its_own_weak_beats() {
    let n = name();
    // C major for a bar, then A minor for a bar.
    let a = arpeggiate_on_bass(
        walking(0, 32),
        &[
            span(0, 16, 0, ChordQuality::Major),
            span(16, 16, 9, ChordQuality::Minor),
        ],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let (notes, arps) = shape(&a);
    assert_eq!(arps, [(4, 4), (12, 4), (20, 4), (28, 4)]);
    let intervals: Vec<_> = a.arps.iter().map(|x| x.intervals.clone()).collect();
    assert_eq!(
        intervals,
        [vec![0, 4, 7], vec![0, 4, 7], vec![0, 3, 7], vec![0, 3, 7]]
    );
    // Roots nearest the bass note before: C-2 after C-2, then A-1 after C-2.
    let roots: Vec<_> = notes
        .iter()
        .filter(|x| x.0 % 8 == 4)
        .map(|x| x.2.as_str())
        .collect();
    assert_eq!(roots, ["C-2", "C-2", "A-1", "A-1"]);
}

#[test]
fn a_weak_beat_named_twice_is_one_arpeggio() {
    let n = name();
    let once = arpeggiate_on_bass(
        walking(0, 16),
        &[span(0, 16, 0, ChordQuality::Major)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let twice = arpeggiate_on_bass(
        walking(0, 16),
        &[span(0, 16, 0, ChordQuality::Major)],
        &at(&n),
        &beats(4, &[4, 2, 2, 4, 4]),
        bass_range(),
    );
    assert_eq!(shape(&twice).1, [(4, 4), (12, 4)]);
    assert_eq!(twice, once);
}

#[test]
fn a_beat_running_past_the_bar_stops_at_the_bar() {
    let n = name();
    // Six-row beats in a 16-row bar: beat 3 is rows 12–17, but the bar ends at 16.
    let a = arpeggiate_on_bass(
        walking(0, 32),
        &[span(0, 32, 0, ChordQuality::Major)],
        &at(&n),
        &beats(6, &[3]),
        bass_range(),
    );
    assert_eq!(shape(&a).1, [(12, 4), (28, 4)]);
    // The bass on the next downbeat stays.
    assert!(a.notes.iter().any(|x| x.row == 16 && x.decision.is_none()));
}

#[test]
fn a_beat_running_past_its_span_stops_with_the_span() {
    let n = name();
    // The chord ends two rows into beat 2; the bass picks up again there.
    let a = arpeggiate_on_bass(
        vec![note(0, 4, "C-2"), note(4, 2, "E-2"), note(6, 2, "G-2")],
        &[span(0, 6, 0, ChordQuality::Major)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let (notes, arps) = shape(&a);
    assert_eq!(arps, [(4, 2)]);
    assert_eq!(
        notes,
        [
            (0, 4, "C-2".into()),
            (4, 2, "C-2".into()),
            (6, 2, "G-2".into())
        ]
    );
}

#[test]
fn a_span_at_the_last_rows_does_not_overflow() {
    let n = name();
    // The last bar there is starts 15 rows before the end, and the span covers its last 12 rows.
    let end = u32::MAX;
    let a = arpeggiate_on_bass(
        vec![],
        &[span(end - 12, 12, 0, ChordQuality::Major)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    assert_eq!(shape(&a).1, [(end - 11, 4), (end - 3, 3)]);
}

#[test]
fn a_bass_note_to_the_last_row_does_not_overflow() {
    let n = name();
    let a = arpeggiate_on_bass(
        vec![note(1, u32::MAX, "C-2")],
        &[span(0, 16, 0, ChordQuality::Major)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    let (notes, arps) = shape(&a);
    assert_eq!(arps, [(4, 4), (12, 4)]);
    assert_eq!(
        notes,
        [
            (1, 3, "C-2".into()),
            (4, 4, "C-2".into()),
            (12, 4, "C-2".into())
        ]
    );
    assert_eq!(ids(&a)[0], "bassarp-trim-v2-r1");
}

#[test]
fn shuffled_input_gives_the_same_line() {
    let n = name();
    // Overlapping spans, two of them sharing a start, and a bass in a muddle.
    let spans = [
        span(0, 16, 0, ChordQuality::Major),
        span(0, 8, 9, ChordQuality::Minor),
        span(8, 16, 7, ChordQuality::Dominant7),
        span(8, 16, 7, ChordQuality::Major),
        ChordSpan {
            confidence: Confidence::new(0.4).unwrap(),
            ..span(8, 16, 7, ChordQuality::Major)
        },
    ];
    let mut bass = walking(0, 32);
    bass.push(note(4, 2, "E-2"));
    let a = arpeggiate_on_bass(
        bass.clone(),
        &spans,
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    // The shorter A minor takes beat 2 and C major, starting first, beat 4; of the spans from row 8, the simpler and
    // surer G major takes bar 2's beat 2.
    let chords: Vec<_> = a
        .decisions
        .iter()
        .filter_map(|d| match &d.proposal {
            Proposal::Chord {
                row,
                root,
                intervals,
            } => Some((*row, *root, intervals.clone(), d.confidence.get())),
            _ => None,
        })
        .collect();
    assert_eq!(
        chords,
        [
            (4, 9, vec![0, 3, 7], 0.8),
            (12, 0, vec![0, 4, 7], 0.8),
            (20, 7, vec![0, 4, 7], 0.8),
        ]
    );
    let mut spans_rev = spans.to_vec();
    spans_rev.reverse();
    bass.reverse();
    let b = arpeggiate_on_bass(
        bass.clone(),
        &spans_rev,
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    assert_eq!(b, a);
    spans_rev.rotate_left(2);
    bass.rotate_left(3);
    let c = arpeggiate_on_bass(
        bass,
        &spans_rev,
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    assert_eq!(c, a);
}

#[test]
fn beats_without_a_chord_and_the_settings() {
    let n = name();
    let bass = vec![
        note(0, 4, "C-2"),
        note(4, 4, "E-2"),
        note(8, 4, "G-2"),
        note(12, 4, "C-2"),
    ];
    // No chord: the bass as it was.
    let a = arpeggiate_on_bass(
        bass.clone(),
        &[],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    assert_eq!(a.notes, bass);
    assert!(a.arps.is_empty() && a.decisions.is_empty());
    // A chord over the second half only: beat 4 alone.
    let a = arpeggiate_on_bass(
        bass.clone(),
        &[span(8, 8, 7, ChordQuality::Major)],
        &at(&n),
        &BassArpRules::default(),
        bass_range(),
    );
    assert_eq!(shape(&a).1, [(12, 4)]);
    // 3/4 in eight-row beats isn't this bar, but weak beats past the bar are ignored rather than spilling over.
    let rules = BassArpRules {
        rows_per_beat: NonZeroU8::new(8).unwrap(),
        weak_beats: vec![0, 2, 3],
        root: BassArpRoot::OctaveAbove,
    };
    let a = arpeggiate_on_bass(
        bass,
        &[span(0, 16, 0, ChordQuality::Major)],
        &at(&n),
        &rules,
        bass_range(),
    );
    // Beat 2 of eight-row beats is rows 8–15; the root an octave above the E-2 before it is C-3 (E-3 is the
    // nearest octave-up target; C-3 and C-4 are 4 and 8 away).
    let (notes, arps) = shape(&a);
    assert_eq!(arps, [(8, 8)]);
    assert_eq!(
        notes,
        [
            (0, 4, "C-2".into()),
            (4, 4, "E-2".into()),
            (8, 8, "C-3".into())
        ]
    );
}

fn checked(a: &Arpeggio, rows_per_bar: u8) -> (Song, Context) {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: name(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(rows_per_bar).unwrap(),
            length: NonZeroU32::new(300).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(2).unwrap(),
                roles: Roles::constant(Role::Bass),
                instrument: PresetId("bass".into()),
                notes: a.notes.clone(),
                arps: a.arps.clone(),
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
        instruments: BTreeMap::from([(PresetId("bass".into()), facts)]),
        ..Context::default()
    };
    (song, ctx)
}

/// Chord spans anywhere in the first 300 rows, overlapping and ending mid-beat, or as detected in a note soup.
fn any_spans() -> impl Strategy<Value = Vec<ChordSpan>> {
    let span = (
        0u32..260,
        1u32..40,
        0u8..12,
        0..ChordQuality::ALL.len(),
        strategies::confidence(),
    )
        .prop_map(|(row, length, root, q, confidence)| ChordSpan {
            row,
            length: NonZeroU32::new(length).unwrap(),
            chord: Chord {
                root,
                quality: ChordQuality::ALL[q],
            },
            confidence,
        });
    prop_oneof![
        proptest::collection::vec(span, 0..8),
        strategies::note_soup().prop_map(|soup| detect_chords(
            &soup,
            NonZeroU8::new(4).unwrap(),
            NonZeroU32::new(300).unwrap()
        )),
    ]
}

/// Any rules: beats of 1–8 rows, weak beats including 0, repeats and beats past the bar, either root.
fn any_rules() -> impl Strategy<Value = BassArpRules> {
    (
        1u8..=8,
        proptest::collection::vec(0u8..12, 0..5),
        any::<bool>(),
    )
        .prop_map(|(beat, weak_beats, above)| BassArpRules {
            rows_per_beat: NonZeroU8::new(beat).unwrap(),
            weak_beats,
            root: if above {
                BassArpRoot::OctaveAbove
            } else {
                BassArpRoot::InRange
            },
        })
}

/// A bass line as `monophonise` leaves it, from a note soup.
fn clean(soup: Vec<Note>, at: &Placement) -> Vec<Note> {
    let rules = MonoRules {
        skyline: Skyline::Lowest,
        ..MonoRules::default()
    };
    monophonise(soup.into_iter().filter(|x| x.row < 280), at, &rules).notes
}

proptest! {
    /// Any clean bass line (a note soup monophonised as a bass) with any chord spans, bar, beats and weak beats gives
    /// a line that validates with no errors: an arpeggio of the span's chord over a root of its pitch class on
    /// exactly the first row of each weak beat inside a span, no bass note sounding under one, and every note it
    /// changed accounted for by a decision as sure as the chord that made it.
    #[test]
    fn any_bass_and_chords_give_a_valid_line_with_every_change_decided(
        bass in strategies::note_soup(),
        spans in any_spans(),
        rows_per_bar in 8u8..=32,
        rules in any_rules(),
    ) {
        let n = name();
        let at = placed(&n, rows_per_bar);
        let clean = clean(bass, &at);
        let a = arpeggiate_on_bass(clean.clone(), &spans, &at, &rules, bass_range());
        let (song, ctx) = checked(&a, rows_per_bar);
        let errors: Vec<_> = validate(&song, &ctx).into_iter().filter(|p| p.is_error()).collect();
        prop_assert!(errors.is_empty(), "{errors:#?}");

        // Where the arpeggios go, worked out row by row: the first row of a weak beat, inside a span.
        let (bar, beat) = (u32::from(rows_per_bar), u32::from(rules.rows_per_beat.get()));
        let weak = |r: u32| {
            let p = r % bar;
            p.is_multiple_of(beat) && rules.weak_beats.contains(&u8::try_from(p / beat + 1).unwrap())
        };
        // The span that takes a row: the first to start, then the shorter, the lower root, the simpler, the surer.
        let taker = |r: u32| {
            spans.iter().filter(|s| s.row <= r && r < s.row + s.length.get()).min_by(|x, y| {
                (x.row, x.length, x.chord.root, x.chord.quality)
                    .cmp(&(y.row, y.length, y.chord.root, y.chord.quality))
                    .then(y.confidence.get().total_cmp(&x.confidence.get()))
            })
        };
        let expected: Vec<u32> = (0..300).filter(|&r| weak(r) && taker(r).is_some()).collect();
        let starts: Vec<u32> = a.arps.iter().map(|x| x.row).collect();
        prop_assert_eq!(&starts, &expected);
        let chord_of = |id: &Option<DecisionId>| a.decisions.iter().find(|d| Some(&d.id) == id.as_ref());
        for arp in &a.arps {
            let s = taker(arp.row).unwrap();
            // To the beat's end, or the bar's, or the span's.
            let end = (arp.row + beat).min((arp.row / bar + 1) * bar).min(s.row + s.length.get());
            prop_assert_eq!(arp.row + arp.length.get(), end, "{:?} over {:?}", arp, s);
            prop_assert_eq!(&arp.intervals[..], s.chord.quality.intervals());
            let roots: Vec<_> = a.notes.iter().filter(|o| o.row == arp.row).collect();
            prop_assert_eq!(roots.len(), 1, "one root under {:?}", arp);
            let root = roots[0];
            prop_assert_eq!((root.length, &root.decision), (arp.length, &arp.decision));
            prop_assert_eq!(root.pitch.semitone() % 12, s.chord.root);
            if rules.root == BassArpRoot::InRange {
                prop_assert!(bass_range().contains(root.pitch), "{root:?} out of the bass range");
            }
            let d = chord_of(&arp.decision).expect("the arpeggio's decision");
            prop_assert_eq!(d.confidence, s.confidence);
            let its = matches!(d.proposal, Proposal::Chord { row, root, .. } if row == arp.row && root == s.chord.root);
            prop_assert!(its, "{:?} for {:?}", d.proposal, arp);
        }
        // No bass note sounds under an arpeggio.
        for o in a.notes.iter().filter(|o| !starts.contains(&o.row)) {
            for arp in &a.arps {
                let over = o.row < arp.row + arp.length.get() && arp.row < o.row + o.length.get();
                prop_assert!(!over, "{o:?} sounds under {arp:?}");
            }
        }
        // Each cut is as sure as the arpeggio that made it; a restart after a bass note is certain.
        let arp_at = |r: u32| a.arps.iter().find(|x| x.row <= r && r < x.row + x.length.get());
        let arp_from = |r: u32| a.arps.iter().find(|x| x.row == r);
        for d in &a.decisions {
            let made_by = match d.proposal {
                Proposal::DropNote { row, .. } => arp_at(row),
                Proposal::TrimNote { row, length, .. } => arp_from(row + length.get()),
                Proposal::Retrigger { row, .. } => {
                    match a.notes.iter().rfind(|o| o.row < row) {
                        Some(o) if starts.contains(&o.row) => arp_from(o.row),
                        _ => {
                            prop_assert_eq!(d.confidence, Confidence::CERTAIN);
                            continue;
                        }
                    }
                }
                _ => continue,
            };
            let made_by = made_by.unwrap_or_else(|| panic!("{:?} by no arpeggio", d.id));
            prop_assert_eq!(d.confidence, chord_of(&made_by.decision).unwrap().confidence, "{:?}", d.id);
        }
        // Each input note: kept as it was, trimmed or restarted by a decision, or dropped by one.
        for x in &clean {
            let out = a.notes.iter().find(|o| o.row == x.row && o.pitch == x.pitch && o.decision.as_ref().is_none_or(|d| !d.as_str().starts_with("bassarp-chord")));
            let dropped = a.decisions.iter().any(|d| matches!(d.proposal, Proposal::DropNote { row, pitch, .. } if row == x.row && pitch == x.pitch));
            match out {
                Some(o) => {
                    let trimmed = o.length != x.length;
                    let restarted = o.attack != x.attack;
                    let trim_decided = a.decisions.iter().any(|d| matches!(d.proposal, Proposal::TrimNote { row, length, .. } if row == x.row && length == o.length));
                    let tie_decided = a.decisions.iter().any(|d| matches!(d.proposal, Proposal::Retrigger { row, .. } if row == x.row));
                    prop_assert!(!trimmed || trim_decided, "{x:?} trimmed without a decision");
                    prop_assert!(!restarted || tie_decided, "{x:?} restarted without a decision");
                }
                None => prop_assert!(dropped, "{x:?} gone without a decision"),
            }
        }
        // One chord decision per arpeggio.
        prop_assert_eq!(a.arps.len(), a.decisions.iter().filter(|d| matches!(d.proposal, Proposal::Chord { .. })).count());
    }

    /// The input order does not matter: not the bass's, nor the spans'.
    #[test]
    fn the_result_does_not_depend_on_the_input_order(
        bass in strategies::note_soup(),
        spans in any_spans(),
        rules in any_rules(),
        seed in any::<u64>(),
    ) {
        let n = name();
        let at = at(&n);
        let (mut bass2, mut spans2) = (bass.clone(), spans.clone());
        // A cheap deterministic shuffle.
        let mut x = seed | 1;
        let mut next = |len: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % len as u64) as usize
        };
        for i in (1..bass2.len()).rev() {
            bass2.swap(i, next(i + 1));
        }
        for i in (1..spans2.len()).rev() {
            spans2.swap(i, next(i + 1));
        }
        prop_assert_eq!(
            arpeggiate_on_bass(bass, &spans, &at, &rules, bass_range()),
            arpeggiate_on_bass(bass2, &spans2, &at, &rules, bass_range())
        );
    }
}
