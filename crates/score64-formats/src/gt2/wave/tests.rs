//! Arpeggio programs: exact rows, reuse, errors, and what the wavetable plays.

use proptest::prelude::*;

use super::*;

fn step(n: u8) -> NonZeroU8 {
    NonZeroU8::new(n).unwrap()
}

#[test]
fn a_triad_follows_the_readme_idiom() {
    // One frame a step: the readme's 4-note example, with $00 keeping the waveform.
    let mut t = Table::default();
    let start = arpeggio(&mut t, 0x41, &[0, 4, 7, 12], step(1)).unwrap();
    assert_eq!(start, 1);
    assert_eq!(
        t.rows,
        [
            (0x41, 0),
            (0x00, 4),
            (0x00, 7),
            (0x00, 12),
            (0x00, 0),
            (WAVE_JUMP, 2)
        ]
    );
    // Three frames a step, after what is already there: the readme's delayed minor chord.
    let start = arpeggio(&mut t, 0x21, &[0, 3, 7], step(3)).unwrap();
    assert_eq!(start, 7);
    assert_eq!(
        t.rows[6..],
        [(0x21, 0), (0x02, 3), (0x02, 7), (0x02, 0), (WAVE_JUMP, 8)]
    );
    // One interval is held.
    let start = arpeggio(&mut t, 0x11, &[5], step(4)).unwrap();
    assert_eq!(
        t.rows[usize::from(start) - 1..],
        [(0x11, 5), (WAVE_JUMP, 0)]
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
    assert_eq!(t.rows.len(), 1 + 5 + 5);
    // Same notes, another waveform or speed: a new program.
    assert_ne!(arpeggio(&mut t, 0x21, &[0, 4, 7], step(1)).unwrap(), a);
    assert_ne!(arpeggio(&mut t, 0x41, &[0, 4, 7], step(2)).unwrap(), a);
    // The same rows copied elsewhere, jumping into the first copy, are not a program starting there.
    let mut t = Table::default();
    t.rows
        .extend([(0x41, 0), (0x00, 4), (0x00, 0), (WAVE_JUMP, 9)]);
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 4], step(1)).unwrap(), 5);
    // A full table still finds a program it already holds.
    let mut t = Table::default();
    let a = arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)).unwrap();
    t.rows.resize(255, (0x81, 0x80));
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)), Ok(a));
}

#[test]
fn what_the_wavetable_cannot_hold_is_refused() {
    let mut t = Table::default();
    assert_eq!(
        arpeggio(&mut t, 0x0F, &[0], step(1)),
        Err(WaveError::Waveform(0x0F))
    );
    assert_eq!(
        arpeggio(&mut t, 0xE1, &[0], step(1)),
        Err(WaveError::Waveform(0xE1))
    );
    // Without the gate bit the note would release; with the test bit it would be silent.
    assert_eq!(
        arpeggio(&mut t, 0x40, &[0], step(1)),
        Err(WaveError::Waveform(0x40))
    );
    assert_eq!(
        arpeggio(&mut t, 0x49, &[0], step(1)),
        Err(WaveError::Waveform(0x49))
    );
    assert_eq!(arpeggio(&mut t, 0x41, &[], step(1)), Err(WaveError::Empty));
    assert_eq!(
        arpeggio(&mut t, 0x41, &[0, 0x60], step(1)),
        Err(WaveError::Interval(0x60))
    );
    assert_eq!(
        arpeggio(&mut t, 0x41, &[0, 4], step(17)),
        Err(WaveError::FramesPerStep(17))
    );
    assert!(arpeggio(&mut t, 0x11, &[0, MAX_INTERVAL], step(16)).is_ok());
    assert!(arpeggio(&mut t, 0xD7, &[0], step(1)).is_ok());
    assert!(
        t.rows.iter().all(|&(l, _)| l != 0x40),
        "nothing written on an error"
    );
    // 255 rows at most, and a table already past that is refused rather than wrapped.
    let mut over = Table {
        rows: vec![(0x81, 0x80); 256],
    };
    assert_eq!(
        arpeggio(&mut over, 0x41, &[0], step(1)),
        Err(WaveError::Full)
    );
    let mut t = Table {
        rows: vec![(0x81, 0x80); 250],
    };
    assert_eq!(
        arpeggio(&mut t, 0x41, &[0, 4, 7, 11], step(1)),
        Err(WaveError::Full)
    );
    assert_eq!(t.rows.len(), 250, "nothing written");
    assert_eq!(arpeggio(&mut t, 0x41, &[0, 4, 7], step(1)), Ok(251));
    assert_eq!(t.rows.len(), 255);
}

/// What a voice plays, frame by frame, from wavetable row `start`, as GoatTracker's player steps the table: a
/// waveform row sets the waveform and note; `$00` sets the note only; `$01`–`$0F` waits that many frames and then sets
/// the note; a jump takes no frame, and `$00` as its target stops the table, holding what plays. Panics on a jump onto
/// a jump, which the readme forbids.
fn play(table: &Table, start: u8, frames: usize) -> Vec<(u8, u8)> {
    let mut row = Some(start);
    let (mut wave, mut note) = (0, 0);
    let mut wait = None;
    let mut out = Vec::new();
    while out.len() < frames {
        if let Some(r) = row {
            let (left, right) = table.rows[usize::from(r) - 1];
            if left == WAVE_JUMP {
                row = (right != 0).then_some(right);
                if let Some(target) = row {
                    assert_ne!(
                        table.rows[usize::from(target) - 1].0,
                        WAVE_JUMP,
                        "a jump onto a jump"
                    );
                }
                continue;
            }
            match left {
                0x00 => {
                    note = right;
                    row = Some(r + 1);
                }
                0x01..=0x0F => {
                    let left_to_wait = wait.unwrap_or(left);
                    if left_to_wait == 0 {
                        note = right;
                        wait = None;
                        row = Some(r + 1);
                    } else {
                        wait = Some(left_to_wait - 1);
                    }
                }
                w => {
                    assert!(WAVEFORMS.contains(&w), "row {r} runs a command");
                    wave = w;
                    note = right;
                    row = Some(r + 1);
                }
            }
        }
        out.push((wave, note));
    }
    out
}

#[test]
fn the_interpreter_plays_the_readmes_delayed_chord_at_three_ticks_a_step() {
    // Readme §3.4.1: "21 00 / 02 03 / 02 07 / 02 00 / FF 02", "each step takes 3 ticks".
    let t = Table {
        rows: vec![(0x21, 0), (0x02, 3), (0x02, 7), (0x02, 0), (WAVE_JUMP, 2)],
    };
    let notes: Vec<u8> = play(&t, 1, 12).into_iter().map(|(_, n)| n).collect();
    assert_eq!(notes, [0, 0, 0, 3, 3, 3, 7, 7, 7, 0, 0, 0]);
}

proptest! {
    /// Whatever is already in the table, the program plays the intervals in order, each for `frames_per_step`
    /// frames, looping (or holding a single interval), with the waveform held, and its first row sets the waveform.
    #[test]
    fn the_program_plays_the_chord_round_and_round(
        before in proptest::collection::vec((0x10u8..=0xDF, 0u8..=0x5F), 0..40),
        waveform in (0x10u8..=0xDF).prop_filter("gated, no test bit", |w| w & GATE != 0 && w & TEST == 0),
        intervals in proptest::collection::vec(0u8..=MAX_INTERVAL, 1..=6),
        frames in 1u8..=MAX_FRAMES_PER_STEP,
    ) {
        let mut t = Table { rows: before };
        let start = arpeggio(&mut t, waveform, &intervals, step(frames)).unwrap();
        prop_assert_eq!(t.rows[usize::from(start) - 1].0, waveform);
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
