//! Finding the loop row from the subtune's harmony.

use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use super::*;
use crate::provenance::Confidence;
use crate::score::{
    ArpEvent, Attack, FramesPerRow, Pitch, PresetId, Role, Roles, SubtuneName, Voice, VoiceLine,
};

fn note(row: u32, length: u32, semitone: u8) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: Pitch::new(semitone).unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(0.9).unwrap(),
        decision: None,
    }
}

/// A bar of a chord: three whole notes from `root`, on three voices' worth of pitches.
fn bar(b: u32, chord: [u8; 3]) -> Vec<Note> {
    chord.iter().map(|&s| note(b * 16, 16, s)).collect()
}

/// A subtune whose bar `i` plays `chords[i]` (all on voice 1, overlapping, which the chroma doesn't mind).
fn subtune(chords: &[[u8; 3]]) -> Subtune {
    let notes = chords
        .iter()
        .enumerate()
        .flat_map(|(i, c)| bar(i as u32, *c))
        .collect();
    Subtune {
        name: SubtuneName::new("main").unwrap(),
        profile: None,
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(chords.len() as u32 * 16).unwrap(),
        loop_row: 0,
        marks: vec![],
        voices: vec![VoiceLine {
            voice: Voice::new(1).unwrap(),
            roles: Roles::constant(Role::Harmony),
            instrument: PresetId("pad".into()),
            notes,
            arps: vec![],
        }],
    }
}

const C: [u8; 3] = [48, 52, 55];
const F: [u8; 3] = [53, 57, 60];
const G: [u8; 3] = [55, 59, 62];
const A_MINOR: [u8; 3] = [57, 60, 64];
const E_FLAT: [u8; 3] = [51, 55, 58];

#[test]
fn an_intro_then_a_repeated_section_loops_onto_the_repeat() {
    // Intro (E♭), then C F G A-minor twice. The ending is G, A-minor; the two bars before bar 5 are G, A-minor too,
    // so after the ending the music goes on as it did at bar 5: the second C.
    let s = subtune(&[E_FLAT, C, F, G, A_MINOR, C, F, G, A_MINOR]);
    let l = find_loop(&s, &LoopRules::default());
    assert_eq!(l.row, 5 * 16);
    assert!((l.similarity - 1.0).abs() < 1e-5, "{}", l.similarity);
    assert_eq!(l.decision.proposal, Proposal::LoopRow { row: 80 });
    assert_eq!(l.decision.id.as_str(), "loop");
    assert_eq!(l.decision.bar, 5);
    assert_eq!(
        l.decision.provenance,
        Provenance::Rules {
            rule: "find-loop@1".into()
        }
    );
    assert_eq!(
        l.decision.confidence,
        Confidence::new(l.similarity).unwrap()
    );
}

#[test]
fn a_song_that_never_repeats_loops_to_its_start() {
    let s = subtune(&[C, E_FLAT, F, A_MINOR, G]);
    let l = find_loop(&s, &LoopRules::default());
    assert_eq!((l.row, l.similarity), (0, 0.0));
    assert_eq!(l.decision.confidence.get(), 0.0);
    assert!(
        !l.candidates.is_empty(),
        "the candidates are still reported"
    );
}

#[test]
fn ties_go_to_the_longest_loop() {
    // C G C G C G: bars 2 and 4 both follow a C G like the ending; the earlier one makes the longer loop.
    let s = subtune(&[C, G, C, G, C, G]);
    let l = find_loop(&s, &LoopRules::default());
    assert_eq!(l.row, 2 * 16);
}

#[test]
fn the_context_decides_how_much_must_match() {
    // With one bar of context, any bar after an A-minor matches; with three, only the true repeat does.
    let s = subtune(&[A_MINOR, F, C, G, A_MINOR, F, C, G, A_MINOR]);
    let one = LoopRules {
        context: NonZeroU32::new(1).unwrap(),
        ..LoopRules::default()
    };
    assert_eq!(
        find_loop(&s, &one).row,
        16,
        "the earliest bar after an A-minor"
    );
    let three = LoopRules {
        context: NonZeroU32::new(3).unwrap(),
        ..LoopRules::default()
    };
    assert_eq!(find_loop(&s, &three).row, 5 * 16);
}

#[test]
fn silent_bars_are_not_candidates_and_short_subtunes_loop_to_0() {
    let mut s = subtune(&[C, G, C, G]);
    s.voices[0].notes.retain(|n| n.row >= 32); // bars 0 and 1 silent
    let l = find_loop(&s, &LoopRules::default());
    assert!(
        l.candidates.iter().all(|&(a, _)| a >= 3),
        "{:?}",
        l.candidates
    );
    let tiny = subtune(&[C]);
    let l = find_loop(&tiny, &LoopRules::default());
    assert_eq!(l.row, 0);
    assert!(l.candidates.is_empty());
}

#[test]
fn arpeggios_count_as_their_chords() {
    // The same harmony twice, once as held chords and once as a root note with an arpeggio over it: they match.
    let mut s = subtune(&[E_FLAT, C, G, E_FLAT, C, G]);
    let line = &mut s.voices[0];
    line.notes.retain(|n| n.row < 48);
    for (b, root) in [(3u32, 51u8), (4, 48), (5, 55)] {
        line.notes.push(note(b * 16, 16, root));
        line.arps.push(ArpEvent {
            row: b * 16,
            length: NonZeroU32::new(16).unwrap(),
            intervals: vec![0, 4, 7], // each of these chords is a major triad
            decision: None,
        });
    }
    let l = find_loop(&s, &LoopRules::default());
    assert_eq!(l.row, 3 * 16, "{:?}", l.candidates);
    assert!(l.similarity > 0.95, "{}", l.similarity);
}

#[test]
fn rules_read_with_defaults() {
    let r: LoopRules = serde_json::from_str(r#"{"context":4}"#).unwrap();
    assert_eq!(r.context.get(), 4);
    assert_eq!(r.min_similarity, LoopRules::default().min_similarity);
    assert!(serde_json::from_str::<LoopRules>(r#"{"context":0}"#).is_err());
}

proptest! {
    /// The loop row is a bar start inside the subtune, the similarity and confidence are 0 to 1, the candidates are
    /// in bar order, and the voices' order does not matter.
    #[test]
    fn any_subtune_loops_to_a_bar_inside_it(
        chords in proptest::collection::vec(prop_oneof![Just(C), Just(F), Just(G), Just(A_MINOR), Just(E_FLAT)], 1..24),
        context in 1u32..4,
    ) {
        let s = subtune(&chords);
        let rules = LoopRules { context: NonZeroU32::new(context).unwrap(), ..LoopRules::default() };
        let l = find_loop(&s, &rules);
        prop_assert!(l.row < s.length.get());
        prop_assert_eq!(l.row % 16, 0);
        prop_assert!((0.0..=1.0).contains(&l.similarity));
        prop_assert!(l.candidates.windows(2).all(|w| w[0].0 < w[1].0));
        for &(a, sim) in &l.candidates {
            prop_assert!(a >= 1 && a + context <= chords.len() as u32);
            prop_assert!((0.0..=1.0).contains(&sim));
        }
        // Splitting the line across two voices, listed either way round, changes nothing.
        let mut split = s.clone();
        let (low, high): (Vec<Note>, Vec<Note>) = split.voices[0].notes.iter().cloned().partition(|n| n.pitch.semitone() < 55);
        split.voices[0].notes = low;
        let mut other = split.voices[0].clone();
        other.voice = Voice::new(2).unwrap();
        other.notes = high;
        split.voices.insert(0, other);
        prop_assert_eq!(find_loop(&split, &rules).row, l.row);
    }

    /// A subtune of one chord pattern played over and over loops to an early bar, with full similarity.
    #[test]
    fn a_repeated_pattern_loops_early(times in 3usize..6) {
        let s = subtune(&[C, F, G].repeat(times));
        let l = find_loop(&s, &LoopRules::default());
        prop_assert!(l.row <= 3 * 16, "{}", l.row);
        prop_assert!((l.similarity - 1.0).abs() < 1e-5);
    }
}
