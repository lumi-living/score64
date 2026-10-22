//! Putting transcribed parts on the grid, and the report on how well they fit.

use std::num::NonZeroU8;

use proptest::prelude::*;

use super::*;
use crate::strategies;

/// 120 BPM, four rows per beat: a row every 125 ms, beats from 0 to 8 s.
fn grid() -> BeatGrid {
    BeatGrid::new(
        (0..=16).map(|b| b * 500_000).collect(),
        NonZeroU8::new(4).unwrap(),
    )
    .unwrap()
}

const SOURCE: ContentHash = ContentHash([7; 32]);

fn note(start_us: u64, end_us: u64, midi: u8) -> SourceNote {
    SourceNote {
        start_us,
        end_us,
        midi,
        confidence: Confidence::new(0.8).unwrap(),
    }
}

fn part(notes: Vec<SourceNote>) -> Part {
    Part {
        source: SOURCE,
        notes,
    }
}

#[test]
fn notes_on_the_grid_fit_exactly() {
    // A quarter note on beat 1 and an eighth note on beat 2, given out of order.
    let q = grid().quantise_part(&part(vec![
        note(1_000_000, 1_250_000, 64),
        note(500_000, 1_000_000, 60),
    ]));
    assert_eq!(q.source, SOURCE);
    let placed: Vec<_> = q
        .notes
        .iter()
        .map(|n| (n.row, n.length.get(), n.pitch.to_string()))
        .collect();
    assert_eq!(
        placed,
        [(4, 4, "C-4".to_string()), (8, 2, "E-4".to_string())]
    );
    let n = &q.notes[0];
    assert_eq!(n.attack, Attack::Fresh);
    assert_eq!(n.confidence, Confidence::new(0.8).unwrap());
    assert_eq!(n.source, SourceRef::new(SOURCE, 500_000, 1_000_000));
    assert_eq!((n.instrument.as_ref(), n.decision.as_ref()), (None, None));
    assert_eq!(
        q.report,
        QuantiseReport {
            placed: 2,
            mean_start_error: 0.0,
            max_start_error: 0.0,
            loose: 0,
            dropped: vec![],
        }
    );
}

#[test]
fn the_report_measures_how_far_starts_moved() {
    // 25 ms late is 0.2 of a row; 50 ms early is 0.4 of a row, and loose.
    let q = grid().quantise_part(&part(vec![
        note(525_000, 1_000_000, 60),
        note(950_000, 1_500_000, 62),
    ]));
    assert_eq!(q.notes.iter().map(|n| n.row).collect::<Vec<_>>(), [4, 8]);
    let r = &q.report;
    assert!((r.mean_start_error - 0.3).abs() < 1e-6, "{r:?}");
    assert!((r.max_start_error - 0.4).abs() < 1e-6, "{r:?}");
    assert_eq!(r.loose, 1);
}

#[test]
fn notes_that_cannot_be_placed_are_reported() {
    let low = note(0, 500_000, 11); // below C-0
    let high = note(0, 500_000, 105); // above G#7
    let early = note(0, 10_000, 60);
    let early_grid = BeatGrid::new(vec![1_000_000, 1_500_000], NonZeroU8::new(4).unwrap()).unwrap();
    let empty = note(2_000_000, 2_000_000, 60);
    let kept = note(1_000_000, 1_500_000, 60);
    let q = early_grid.quantise_part(&part(vec![low, high, early, empty, kept]));
    assert_eq!(q.notes.len(), 1);
    assert_eq!(
        q.report.dropped,
        [
            Dropped {
                note: low,
                reason: Unplaced::PitchOutOfRange
            },
            Dropped {
                note: high,
                reason: Unplaced::PitchOutOfRange
            },
            Dropped {
                note: early,
                reason: Unplaced::BeforeGrid
            },
            Dropped {
                note: empty,
                reason: Unplaced::Empty
            },
        ]
    );
    assert_eq!(q.report.placed, 1);
}

#[test]
fn overlaps_are_kept_for_the_reduction() {
    // A chord, and a note that rings over the next: all kept, in row then pitch order.
    let q = grid().quantise_part(&part(vec![
        note(0, 1_000_000, 67),
        note(0, 1_000_000, 60),
        note(0, 1_000_000, 64),
        note(250_000, 750_000, 72),
    ]));
    let placed: Vec<_> = q.notes.iter().map(|n| (n.row, n.pitch.midi())).collect();
    assert_eq!(placed, [(0, 60), (0, 64), (0, 67), (2, 72)]);
}

#[test]
fn an_empty_part_gives_an_empty_report() {
    let q = grid().quantise_part(&part(vec![]));
    assert!(q.notes.is_empty());
    assert_eq!(q.report.placed, 0);
    assert_eq!(q.report.mean_start_error, 0.0);
    assert_eq!(q.report.max_start_error, 0.0);
}

#[test]
fn parts_round_trip_through_json() {
    let p = part(vec![note(500_000, 1_000_000, 60)]);
    let json = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<Part>(&json).unwrap(), p);
    assert!(serde_json::from_str::<Part>(&json.replace("\"midi\"", "\"pitch\"")).is_err());
    let q = grid().quantise_part(&p);
    let json = serde_json::to_string(&q).unwrap();
    assert_eq!(serde_json::from_str::<QuantisedPart>(&json).unwrap(), q);
}

/// Any part: up to 64 notes anywhere in the first 9 s (past the last beat too), any MIDI pitch, any length up to 2 s,
/// empty ones included.
fn any_part() -> impl Strategy<Value = Part> {
    (
        strategies::content_hash(),
        proptest::collection::vec(
            (0u64..9_000_000, 0u64..2_000_000, any::<u8>(), 0.0f32..=1.0),
            0..64,
        ),
    )
        .prop_map(|(source, notes)| Part {
            source,
            notes: notes
                .into_iter()
                .map(|(start_us, len, midi, c)| SourceNote {
                    start_us,
                    end_us: start_us + len,
                    midi: midi & 0x7f,
                    confidence: Confidence::new(c).unwrap(),
                })
                .collect(),
        })
}

/// A grid of 2–32 beats, each 200–1,000 ms, starting up to 1 s in, at 1–8 rows per beat.
fn any_grid() -> impl Strategy<Value = BeatGrid> {
    (
        0u64..1_000_000,
        proptest::collection::vec(200_000u64..1_000_000, 1..32),
        1u8..=8,
    )
        .prop_map(|(first, gaps, rows)| {
            let mut beats = vec![first];
            for g in gaps {
                beats.push(beats.last().unwrap() + g);
            }
            BeatGrid::new(beats, NonZeroU8::new(rows).unwrap()).unwrap()
        })
}

proptest! {
    /// Every note is either placed or dropped with a reason; placed notes moved at most half a row, point back at
    /// their own source span, keep their pitch and confidence, and come in row then pitch order.
    #[test]
    fn every_note_is_placed_or_accounted_for(grid in any_grid(), part in any_part()) {
        let q = grid.quantise_part(&part);
        let r = &q.report;
        prop_assert_eq!(r.placed + r.dropped.len(), part.notes.len());
        prop_assert_eq!(r.placed, q.notes.len());
        prop_assert!(r.mean_start_error <= r.max_start_error + 1e-6);
        prop_assert!(r.max_start_error <= 0.5 + 1e-6, "{}", r.max_start_error);
        prop_assert!(r.loose <= r.placed);
        prop_assert!(q.notes.windows(2).all(|w| (w[0].row, w[0].pitch) <= (w[1].row, w[1].pitch)));
        for n in &q.notes {
            let s = n.source.expect("placed notes point at their source");
            prop_assert_eq!(s.content(), part.source);
            let original = part
                .notes
                .iter()
                .find(|o| (o.start_us, o.end_us, o.midi) == (s.start_us(), s.end_us(), n.pitch.midi()));
            prop_assert!(original.is_some(), "{n:?} has no source note");
            prop_assert_eq!(original.unwrap().confidence, n.confidence);
            let row = grid.row_at(s.start_us());
            prop_assert!((f64::from(n.row) - row).abs() <= 0.5 + 1e-9);
        }
        for d in &r.dropped {
            let ok = match d.reason {
                Unplaced::Empty => d.note.end_us <= d.note.start_us,
                Unplaced::PitchOutOfRange => Pitch::from_midi(d.note.midi).is_none(),
                Unplaced::BeforeGrid => grid.row_at(d.note.start_us).round() < 0.0,
            };
            prop_assert!(ok, "{d:?}");
        }
    }
}
