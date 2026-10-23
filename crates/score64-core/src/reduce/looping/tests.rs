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
        l.candidates.iter().all(|c| c.bar >= 3),
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
    assert!(serde_json::from_str::<LoopRules>(r#"{"min_similarity":7}"#).is_err());
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
        prop_assert!(l.candidates.windows(2).all(|w| w[0].bar < w[1].bar));
        for c in &l.candidates {
            prop_assert!(c.bar >= 1 && c.bar + context <= chords.len() as u32);
            prop_assert!((0.0..=1.0).contains(&c.similarity));
        }
        // Splitting the line across two voices, listed either way round, changes nothing.
        let mut split = s.clone();
        let (low, high): (Vec<Note>, Vec<Note>) = split.voices[0].notes.iter().cloned().partition(|n| n.pitch.semitone() < 55);
        split.voices[0].notes = low;
        let mut other = split.voices[0].clone();
        other.voice = Voice::new(2).unwrap();
        other.notes = high;
        split.voices.insert(0, other);
        prop_assert_eq!(find_loop(&split, &rules), l);
    }

    /// A subtune of one chord pattern played over and over loops to an early bar, with full similarity.
    #[test]
    fn a_repeated_pattern_loops_early(times in 3usize..6) {
        let s = subtune(&[C, F, G].repeat(times));
        let l = find_loop(&s, &LoopRules::default());
        prop_assert_eq!(l.row, 3 * 16);
        prop_assert!((l.similarity - 1.0).abs() < 1e-5);
    }
}

/// A subtune of `bars` bars from its notes, on one voice.
fn from_notes(notes: Vec<Note>, bars: u32) -> Subtune {
    let mut s = subtune(&vec![C; bars as usize]);
    s.voices[0].notes = notes;
    s
}

#[test]
fn chroma_weighs_rows_sounding_and_confidence() {
    // A half-bar C at 0.5 and a whole-bar G at 1.0 in bar 0: C weighs 8 × 0.5, G 16 × 1.
    let mut c = note(0, 8, 48);
    c.confidence = Confidence::new(0.5).unwrap();
    let mut g = note(0, 16, 55);
    g.confidence = Confidence::CERTAIN;
    let s = from_notes(vec![c, g], 1);
    let chroma = chroma_by_bar(&s, 1);
    assert_eq!(chroma[0][0], 4.0);
    assert_eq!(chroma[0][7], 16.0);
    // A note held over a bar line counts in both, row by row.
    let s = from_notes(vec![note(8, 16, 48)], 2);
    let chroma = chroma_by_bar(&s, 2);
    assert!((chroma[0][0] - 8.0 * 0.9).abs() < 1e-5 && (chroma[1][0] - 8.0 * 0.9).abs() < 1e-5);
}

#[test]
fn arpeggios_share_the_note_and_count_once() {
    // C for 16 rows at 0.9: plain for 4, then an arpeggio (C E G) for 8, a second arpeggio over the same rows, then
    // plain again.
    let mut s = from_notes(vec![note(0, 16, 48)], 1);
    for _ in 0..2 {
        s.voices[0].arps.push(ArpEvent {
            row: 4,
            length: NonZeroU32::new(8).unwrap(),
            intervals: vec![0, 4, 7],
            decision: None,
        });
    }
    let chroma = chroma_by_bar(&s, 1);
    // C: 8 plain rows × 0.9 plus 8 arpeggio rows × 0.3; E and G: 8 × 0.3 each.
    assert!(
        (chroma[0][0] - (8.0 * 0.9 + 8.0 * 0.3)).abs() < 1e-4,
        "{:?}",
        chroma[0]
    );
    assert!((chroma[0][4] - 2.4).abs() < 1e-4 && (chroma[0][7] - 2.4).abs() < 1e-4);
    // An interval past the top wraps to its pitch class.
    let mut s = from_notes(vec![note(0, 16, 90)], 1);
    s.voices[0].arps.push(ArpEvent {
        row: 0,
        length: NonZeroU32::new(16).unwrap(),
        intervals: vec![0, 200],
        decision: None,
    });
    let chroma = chroma_by_bar(&s, 1);
    assert!(chroma[0][(90 + 200) % 12] > 0.0);
}

#[test]
fn notes_past_the_end_are_ignored() {
    let mut s = subtune(&[C, G, C, G]);
    let before = find_loop(&s, &LoopRules::default());
    s.voices[0].notes.push(note(64 + 10, 4, 50));
    s.voices[0].notes.push(note(60, 40, 51));
    let after = find_loop(&s, &LoopRules::default());
    assert_eq!(before.row, after.row);
}

#[test]
fn the_last_candidate_bar_counts() {
    // C G C G: the only candidate is bar 2, the last one before the ending's context.
    let l = find_loop(&subtune(&[C, G, C, G]), &LoopRules::default());
    assert_eq!(l.row, 32);
}

#[test]
fn a_part_bar_or_trailing_silence_is_left_out_and_halves_the_confidence() {
    // C F G three times, then four rows more of nothing: the ending is the last whole bars.
    let mut s = subtune(&[C, F, G].repeat(3));
    let clean = find_loop(&s, &LoopRules::default());
    s.length = NonZeroU32::new(9 * 16 + 4).unwrap();
    let part = find_loop(&s, &LoopRules::default());
    assert_eq!(part.row, clean.row);
    assert!((part.decision.confidence.get() - clean.decision.confidence.get() / 2.0).abs() < 1e-5);
    // Two silent bars at the end: skipped, the same loop at half the confidence.
    let mut s = subtune(&[C, F, G].repeat(3));
    s.length = NonZeroU32::new(11 * 16).unwrap();
    let silent = find_loop(&s, &LoopRules::default());
    assert_eq!(silent.row, clean.row);
    assert!(silent.decision.confidence < clean.decision.confidence);
}

#[test]
fn a_silent_ending_gives_no_candidates() {
    let mut s = subtune(&[C, G, C, G]);
    s.voices[0].notes.clear();
    assert!(find_loop(&s, &LoopRules::default()).candidates.is_empty());
}

#[test]
fn a_part_bar_with_music_is_left_out_too() {
    // C F G three times, then four rows of an E♭ note in a part bar: the ending is still the last whole bars.
    let mut s = subtune(&[C, F, G].repeat(3));
    let clean = find_loop(&s, &LoopRules::default());
    s.length = NonZeroU32::new(9 * 16 + 4).unwrap();
    s.voices[0].notes.push(note(144, 4, 51));
    let part = find_loop(&s, &LoopRules::default());
    assert_eq!(part.row, clean.row);
    assert!((part.decision.confidence.get() - clean.decision.confidence.get() / 2.0).abs() < 1e-5);
}

#[test]
fn arpeggios_listed_out_of_order_still_count() {
    // Two arpeggios over one C, listed later one first: both rows 0–7 and 8–15 sound as the chord.
    let mut s = from_notes(vec![note(0, 16, 48)], 1);
    for row in [8, 0] {
        s.voices[0].arps.push(ArpEvent {
            row,
            length: NonZeroU32::new(8).unwrap(),
            intervals: vec![0, 4, 7],
            decision: None,
        });
    }
    let chroma = chroma_by_bar(&s, 1);
    assert!((chroma[0][4] - 16.0 * 0.3).abs() < 1e-4, "{:?}", chroma[0]);
}

fn loop_decision(id: &str, row: u32, provenance: crate::provenance::Provenance) -> Decision {
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: SubtuneName::new("main").unwrap(),
        bar: row / 16,
        proposal: Proposal::LoopRow { row },
        provenance,
        confidence: Confidence::CERTAIN,
        reviewed: false,
    }
}

#[test]
fn a_loop_row_in_force_replaces_the_rules_choice() {
    use crate::provenance::Provenance;
    let s = subtune(&[E_FLAT, C, F, G, C, F, G]);
    let plain = find_loop(&s, &LoopRules::default());
    assert_eq!((plain.row, plain.by.as_str()), (64, "loop"));
    // A person loops to bar 2 instead; the rule's own decision is still there for review.
    let r = find_loop_with(
        &s,
        &LoopRules::default(),
        &[loop_decision("mine", 32, Provenance::Manual)],
    );
    assert_eq!((r.row, r.by.as_str()), (32, "mine"));
    assert_eq!(r.decision, plain.decision);
    assert_eq!(r.candidates, plain.candidates);
    assert_eq!(
        r.similarity,
        plain
            .candidates
            .iter()
            .find(|c| c.bar == 2)
            .unwrap()
            .similarity
    );
    // Mid-bar is a person's call too; no candidate scores it.
    let r = find_loop_with(
        &s,
        &LoopRules::default(),
        &[loop_decision("mid", 20, Provenance::Manual)],
    );
    assert_eq!((r.row, r.similarity), (20, 0.0));
    assert!(r.unapplied.is_empty());
}

#[test]
fn a_loop_row_that_cant_apply_says_why() {
    use crate::provenance::Provenance;
    let s = subtune(&[C, F, G, C]);
    let r = find_loop_with(
        &s,
        &LoopRules::default(),
        &[
            loop_decision("past", 64, Provenance::Manual),
            loop_decision("first", 16, Provenance::Manual),
            loop_decision("second", 32, Provenance::Manual),
            loop_decision(
                "rules",
                48,
                Provenance::Rules {
                    rule: FIND_LOOP.into(),
                },
            ),
        ],
    );
    assert_eq!((r.row, r.by.as_str()), (16, "first"));
    let reasons: Vec<_> = r
        .unapplied
        .iter()
        .map(|(id, w)| (id.as_str(), *w))
        .collect();
    assert_eq!(
        reasons,
        [
            ("past", Unapplied::PastEnd),
            ("second", Unapplied::Outranked)
        ]
    );
}
