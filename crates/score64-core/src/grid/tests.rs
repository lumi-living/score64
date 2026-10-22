//! Tempo snapping against the real PAL frame rate, and quantising source times onto rows.

use proptest::prelude::*;

use super::*;

fn rpb(n: u8) -> NonZeroU8 {
    NonZeroU8::new(n).unwrap()
}

fn fpr(n: u8) -> FramesPerRow {
    FramesPerRow::new(n).unwrap()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn nominal_tempos_match_the_reference_table() {
    for (frames, bpm) in [
        (12, 62.5),
        (10, 75.0),
        (8, 93.75),
        (6, 125.0),
        (5, 150.0),
        (4, 187.5),
    ] {
        assert_eq!(
            bpm_at(fpr(frames), rpb(4), NOMINAL_FRAME_RATE),
            bpm,
            "{frames}"
        );
    }
    // The real PAL rate is 50.1246 Hz, so a PAL C64 plays 6 frames per row at about 125.31 BPM.
    assert!(close(PAL_FRAME_RATE, 50.1246, 1e-4), "{PAL_FRAME_RATE}");
    assert!(close(bpm_at(fpr(6), rpb(4), PAL_FRAME_RATE), 125.31, 0.01));
}

#[test]
fn snapping_reports_error_and_drift_against_the_real_pal_rate() {
    // Made at a tracker tempo: snaps to it, but the real PAL rate runs 0.25 % fast, about 149 ms per minute.
    let s = snap_tempo(125.0, rpb(4)).unwrap();
    assert_eq!((s.frames_per_row, s.nominal_bpm), (fpr(6), 125.0));
    assert!(close(s.error, 0.002_49, 1e-5), "{}", s.error);
    assert!(
        close(s.drift_ms_per_minute, 149.1, 0.1),
        "{}",
        s.drift_ms_per_minute
    );

    // 120 BPM sits between 6 (125.3, +4.4 %) and 7 (107.4, -10.5 %) frames per row: 6 is nearer.
    let s = snap_tempo(120.0, rpb(4)).unwrap();
    assert_eq!(s.frames_per_row, fpr(6));
    assert!(close(s.error, 0.0443, 1e-3), "{}", s.error);
    assert!(s.drift_ms_per_minute > 0.0, "the C64 runs ahead");

    // Slower than the source gives a negative error and drift.
    let s = snap_tempo(110.0, rpb(4)).unwrap();
    assert_eq!(s.frames_per_row, fpr(7));
    assert!(s.error < 0.0 && s.drift_ms_per_minute < 0.0);

    // Three rows per beat (triplet feel) at 125 BPM wants 8 frames per row.
    assert_eq!(snap_tempo(125.0, rpb(3)).unwrap().frames_per_row, fpr(8));
}

#[test]
fn tempos_out_of_reach_snap_to_the_ends_with_their_error() {
    let fast = snap_tempo(1000.0, rpb(4)).unwrap();
    assert_eq!(fast.frames_per_row, FramesPerRow::MIN);
    assert!(fast.error < -0.7, "{}", fast.error);
    let slow = snap_tempo(5.0, rpb(4)).unwrap();
    assert_eq!(slow.frames_per_row, FramesPerRow::MAX);
    for bad in [0.0, -60.0, f64::NAN, f64::INFINITY] {
        assert_eq!(snap_tempo(bad, rpb(4)), None, "{bad}");
    }
}

#[test]
fn steady_and_rubato_grids() {
    // 120 BPM: a beat every 500 ms, four rows per beat.
    let g = BeatGrid::new(vec![0, 500_000, 1_000_000, 1_500_000], rpb(4)).unwrap();
    assert!(close(g.median_bpm(), 120.0, 1e-9));
    assert!(close(g.row_at(250_000), 2.0, 1e-9));
    assert_eq!(
        g.quantise(0, 500_000),
        Some(Quantised {
            row: 0,
            length: NonZeroU32::new(4).unwrap(),
            start_error: 0.0
        })
    );
    // A slightly late note snaps back, and says how far.
    let q = g.quantise(130_000, 380_000).unwrap();
    assert_eq!((q.row, q.length.get()), (1, 2));
    assert!(
        close(f64::from(q.start_error), -0.04, 1e-6),
        "{}",
        q.start_error
    );
    // Past the last beat the last interval carries on.
    assert!(close(g.row_at(1_750_000), 14.0, 1e-9));

    // Rubato: the second beat is 600 ms long. Halfway through it is beat 1.5, row 6, with no drift from beat 1.
    let r = BeatGrid::new(vec![0, 500_000, 1_100_000], rpb(4)).unwrap();
    assert!(close(r.row_at(800_000), 6.0, 1e-9));
    assert!(close(r.median_bpm(), 60e6 / 550_000.0, 1e-9));

    // Before the first beat the first interval extends backwards; more than half a row early is off the grid.
    let late = BeatGrid::new(vec![100_000, 600_000], rpb(4)).unwrap();
    assert!(close(late.row_at(0), -0.8, 1e-9));
    assert_eq!(late.quantise(0, 200_000), None);
    assert_eq!(late.quantise(60_000, 200_000).unwrap().row, 0);
    // A very short note still gets one row; an empty span gets none.
    assert_eq!(g.quantise(500_000, 500_010).unwrap().length.get(), 1);
    assert_eq!(g.quantise(500_000, 500_000), None);
}

#[test]
fn grids_are_checked_on_read() {
    assert!(BeatGrid::new(vec![0], rpb(4)).is_err());
    assert!(BeatGrid::new(vec![0, 500_000, 500_000], rpb(4)).is_err());
    let g = BeatGrid::new(vec![0, 500_000], rpb(4)).unwrap();
    let json = serde_json::to_string(&g).unwrap();
    assert_eq!(serde_json::from_str::<BeatGrid>(&json).unwrap(), g);
    assert!(
        serde_json::from_str::<BeatGrid>(r#"{"beats_us":[0,1],"rows_per_beat":4,"swing":1}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<BeatGrid>(r#"{"beats_us":[5,1],"rows_per_beat":4}"#).is_err());
}

proptest! {
    /// Snapping always picks a legal row length, and no other row length is nearer.
    #[test]
    fn snap_is_nearest(bpm in 1.0f64..2000.0, rows in 1u8..=8) {
        let s = snap_tempo(bpm, rpb(rows)).unwrap();
        for f in FramesPerRow::MIN.get()..=FramesPerRow::MAX.get() {
            let e = bpm_at(fpr(f), rpb(rows), PAL_FRAME_RATE) / bpm - 1.0;
            prop_assert!(s.error.abs() <= e.abs() + 1e-12, "{f} is nearer than {}", s.frames_per_row.get());
        }
    }

    /// Quantised notes start within half a row of their source time and last at least a row.
    #[test]
    fn quantise_is_within_half_a_row(
        gaps in proptest::collection::vec(100_000u64..1_000_000, 1..16),
        start in 0u64..5_000_000,
        len in 1u64..2_000_000,
    ) {
        let beats: Vec<u64> = std::iter::once(0).chain(gaps.iter().scan(0, |t, g| { *t += g; Some(*t) })).collect();
        let g = BeatGrid::new(beats, rpb(4)).unwrap();
        if let Some(q) = g.quantise(start, start + len) {
            prop_assert!(q.start_error.abs() <= 0.5 + 1e-6);
            prop_assert!(q.length.get() >= 1);
        }
    }
}
