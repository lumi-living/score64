//! Arpeggio programs: exact rows, reuse, errors, and what the wavetable plays.

use proptest::prelude::*;

use super::*;

fn step(n: u8) -> NonZeroU8 {
    NonZeroU8::new(n).unwrap()
}

#[test]
fn a_major_triad_is_three_rows_and_a_jump() {
    let mut t = Table::default();
    let start = arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)).unwrap();
    assert_eq!(start, 1);
    assert_eq!(t.rows, [(0x41, 0), (0x41, 4), (0x41, 7), (WAVE_JUMP, 1)]);
    // Two frames a step, after what is already there.
    let start = arpeggio(&mut t, 0x21, &[0, 3], step(2)).unwrap();
    assert_eq!(start, 5);
    assert_eq!(
        t.rows[4..],
        [(0x21, 0), (0x21, 0), (0x21, 3), (0x21, 3), (WAVE_JUMP, 5)]
    );
}

#[test]
fn an_identical_program_is_reused() {
    let mut t = Table::default();
    t.rows.push((0x81, 0x80));
    let a = arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)).unwrap();
    let b = arpeggio(&mut t, 0x41, &[0, 3, 7], step(1)).unwrap();
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)).unwrap(), a);
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 3, 7], step(1)).unwrap(), b);
    assert_eq!(t.rows.len(), 1 + 4 + 4);
    // Same notes, another waveform or speed: a new program.
    assert_ne!(arpeggio(&mut t, 0x21, &[0, 4, 7], step(1)).unwrap(), a);
    assert_ne!(arpeggio(&mut t, 0x41, &[0, 4, 7], step(2)).unwrap(), a);
    // The same rows copied elsewhere, jumping to the first copy, are not a program starting there.
    let mut t = Table::default();
    t.rows.extend([(0x41, 0), (0x41, 4), (WAVE_JUMP, 9)]);
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 4], step(1)).unwrap(), 4);
}

#[test]
fn what_the_wavetable_cannot_hold_is_refused() {
    let mut t = Table::default();
    assert_eq!(
        arpeggio(&mut t, 0x0F, &[0], step(1)),
        Err(WaveError::Waveform(0x0F))
    );
    assert_eq!(
        arpeggio(&mut t, 0xE0, &[0], step(1)),
        Err(WaveError::Waveform(0xE0))
    );
    assert_eq!(arpeggio(&mut t, 0x41, &[], step(1)), Err(WaveError::Empty));
    assert_eq!(
        arpeggio(&mut t, 0x41, &[0, 0x60], step(1)),
        Err(WaveError::Interval(0x60))
    );
    assert!(arpeggio(&mut t, 0x10, &[0, MAX_INTERVAL], step(1)).is_ok());
    assert!(arpeggio(&mut t, 0xDF, &[0], step(1)).is_ok());
    // 255 rows at most.
    let mut t = Table {
        rows: vec![(0x81, 0x80); 251],
    };
    assert_eq!(
        arpeggio(&mut t, 0x41, &[0, 4, 7, 11], step(1)),
        Err(WaveError::Full)
    );
    assert_eq!(t.rows.len(), 251, "nothing written");
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)), Ok(252));
    assert_eq!(t.rows.len(), 255);
}

/// What a voice plays, frame by frame, from wavetable row `start`: the waveform and the relative note each frame.
/// Panics if a step lands on a jump (the readme forbids jumping onto one) or the program stops.
fn play(table: &Table, start: u8, frames: usize) -> Vec<(u8, u8)> {
    let mut row = start;
    let mut out = Vec::new();
    while out.len() < frames {
        let (left, right) = table.rows[usize::from(row) - 1];
        if left == WAVE_JUMP {
            assert_ne!(right, 0, "the program stops");
            row = right;
            let (target, _) = table.rows[usize::from(row) - 1];
            assert_ne!(target, WAVE_JUMP, "a jump onto a jump");
            continue;
        }
        assert!(WAVEFORMS.contains(&left), "row {row} sets no waveform");
        out.push((left, right));
        row += 1;
    }
    out
}

proptest! {
    /// Whatever is already in the table, the program plays the intervals in order, each for `frames_per_step`
    /// frames, looping, with the waveform held, and its first row is never a jump.
    #[test]
    fn the_program_plays_the_chord_round_and_round(
        before in proptest::collection::vec((0x10u8..=0xDF, 0u8..=0x5F), 0..40),
        waveform in 0x10u8..=0xDF,
        intervals in proptest::collection::vec(0u8..=MAX_INTERVAL, 1..=6),
        frames in 1u8..=4,
    ) {
        let mut t = Table { rows: before };
        let start = arpeggio(&mut t, waveform, &intervals, step(frames)).unwrap();
        prop_assert_ne!(t.rows[usize::from(start) - 1].0, WAVE_JUMP);
        let cycle = intervals.len() * usize::from(frames);
        let heard = play(&t, start, 3 * cycle);
        let want: Vec<(u8, u8)> = (0..3 * cycle)
            .map(|f| (waveform, intervals[(f % cycle) / usize::from(frames)]))
            .collect();
        prop_assert_eq!(heard, want);
        // Asking again changes nothing.
        let rows = t.rows.clone();
        prop_assert_eq!(arpeggio(&mut t, waveform, &intervals, step(frames)).unwrap(), start);
        prop_assert_eq!(t.rows, rows);
    }
}
