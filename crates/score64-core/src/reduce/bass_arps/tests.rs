//! The bass with arpeggios on its weak beats.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
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
    Placement {
        subtune,
        voice: Voice::new(2).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
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

fn checked(a: &Arpeggio) -> (Song, Context) {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: name(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
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

proptest! {
    /// Any clean bass line (a note soup monophonised as a bass) with the chords detected in any soup gives a line
    /// that validates with no errors, and every note it changed is accounted for by a decision.
    #[test]
    fn any_bass_and_chords_give_a_valid_line_with_every_change_decided(
        bass in strategies::note_soup(),
        harmony in strategies::note_soup(),
        above in any::<bool>(),
    ) {
        let n = name();
        let clean = monophonise(
            bass.into_iter().filter(|x| x.row < 280),
            &at(&n),
            &MonoRules { skyline: Skyline::Lowest, ..MonoRules::default() },
        )
        .notes;
        let spans = detect_chords(&harmony, NonZeroU8::new(4).unwrap(), NonZeroU32::new(300).unwrap());
        let rules = BassArpRules {
            root: if above { BassArpRoot::OctaveAbove } else { BassArpRoot::InRange },
            ..BassArpRules::default()
        };
        let a = arpeggiate_on_bass(clean.clone(), &spans, &at(&n), &rules, bass_range());
        let (song, ctx) = checked(&a);
        let errors: Vec<_> = validate(&song, &ctx).into_iter().filter(|p| p.is_error()).collect();
        prop_assert!(errors.is_empty(), "{errors:#?}");
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
        // One arpeggio per chord decision, each over its own root note.
        prop_assert_eq!(a.arps.len(), a.decisions.iter().filter(|d| matches!(d.proposal, Proposal::Chord { .. })).count());
        for arp in &a.arps {
            prop_assert!(a.notes.iter().any(|o| o.row == arp.row && o.length == arp.length && o.decision == arp.decision));
        }
    }
}
