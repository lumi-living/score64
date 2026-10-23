//! Placing timing marks at a set's switch points, and the values that make each one register.

use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use super::*;
use crate::profile::ProfileName;
use crate::score::{FramesPerRow, Schema};
use crate::strategies;

fn n(s: &str) -> SubtuneName {
    SubtuneName::new(s).unwrap()
}

/// Layers of `bars` 16-row bars, with no marks.
fn song(names: &[&str], bars: u32) -> Song {
    Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: names
            .iter()
            .map(|s| Subtune {
                name: n(s),
                profile: None,
                frames_per_row: FramesPerRow::new(6).unwrap(),
                tempo: vec![],
                rows_per_bar: NonZeroU8::new(16).unwrap(),
                length: NonZeroU32::new(bars * 16).unwrap(),
                loop_row: 0,
                marks: vec![],
                voices: vec![],
            })
            .collect(),
    }
}

fn marks_of(song: &Song, name: &str) -> Vec<u32> {
    song.subtune(&n(name)).unwrap().1.marks.clone()
}

#[test]
fn marks_go_at_every_declared_switch_point_once() {
    let mut s = song(&["calm", "tense"], 4);
    let set = AdaptiveSet::new("score", vec![n("calm"), n("tense")]).unwrap();
    let added = set.place_marks(&mut s);
    assert_eq!(
        added,
        [
            (n("calm"), vec![0, 16, 32, 48]),
            (n("tense"), vec![0, 16, 32, 48])
        ]
    );
    assert!(set.check(&s).is_empty());
    // Again: nothing to add.
    assert!(set.place_marks(&mut s).is_empty());
    // Every two bars, or on explicit rows; marks already there are kept, and the result stays sorted.
    let mut s = song(&["a"], 8);
    s.subtunes[0].marks = vec![40];
    let every_two = AdaptiveSet::with_switch(
        "x",
        vec![n("a")],
        Switch::EveryBars(NonZeroU32::new(2).unwrap()),
    )
    .unwrap();
    every_two.place_marks(&mut s);
    assert_eq!(marks_of(&s, "a"), [0, 32, 40, 64, 96]);
    let rows = AdaptiveSet::with_switch("y", vec![n("a")], Switch::Rows(vec![8, 200])).unwrap();
    rows.place_marks(&mut s);
    assert_eq!(
        marks_of(&s, "a"),
        [0, 8, 32, 40, 64, 96],
        "a row past the end is left for rule 5 to report"
    );
    // Layers the song lacks are skipped.
    let missing = AdaptiveSet::new("z", vec![n("gone")]).unwrap();
    assert!(missing.place_marks(&mut s).is_empty());
}

fn values(t: &TimingMarks) -> Vec<(u32, u8)> {
    t.marks.iter().map(|m| (m.row, m.value)).collect()
}

#[test]
fn values_alternate_from_0x10() {
    let mut s = song(&["a"], 4);
    s.subtunes[0].marks = vec![0, 16, 32, 48];
    s.subtunes[0].loop_row = 16;
    let set = AdaptiveSet::new("x", vec![n("a")]).unwrap();
    // Looping to bar 1 (value $11) from the last mark ($11): the last takes $12, so the jump back changes the byte.
    let t = set.timing_marks(&s.subtunes[0]);
    assert_eq!(values(&t), [(0, 0x10), (16, 0x11), (32, 0x10), (48, 0x12)]);
    assert!(t.loop_changes && t.off_switch.is_empty());
    // Looping to bar 0 ($10) from $11: already different.
    s.subtunes[0].loop_row = 0;
    let t = set.timing_marks(&s.subtunes[0]);
    assert_eq!(values(&t), [(0, 0x10), (16, 0x11), (32, 0x10), (48, 0x11)]);
    assert!(t.loop_changes);
}

#[test]
fn only_switch_points_change_the_byte() {
    // Switching every two bars, with an extra mark mid-way: it gets no DXY, and the switch marks still alternate.
    let mut s = song(&["a"], 8);
    s.subtunes[0].marks = vec![40];
    let set = AdaptiveSet::with_switch(
        "x",
        vec![n("a")],
        Switch::EveryBars(NonZeroU32::new(2).unwrap()),
    )
    .unwrap();
    set.place_marks(&mut s);
    let t = set.timing_marks(&s.subtunes[0]);
    assert_eq!(values(&t), [(0, 0x10), (32, 0x11), (64, 0x10), (96, 0x11)]);
    assert_eq!(t.off_switch, [40]);
}

#[test]
fn a_loop_with_fewer_than_two_marks_changes_only_once() {
    let mut s = song(&["a"], 2);
    s.subtunes[0].marks = vec![0, 16];
    s.subtunes[0].loop_row = 16;
    let set = AdaptiveSet::new("x", vec![n("a")]).unwrap();
    assert!(!set.timing_marks(&s.subtunes[0]).loop_changes);
    // Explicit rows all before the loop: after the first pass there is nothing to change.
    let mut s = song(&["a"], 4);
    s.subtunes[0].loop_row = 32;
    let early = AdaptiveSet::with_switch("y", vec![n("a")], Switch::Rows(vec![0, 16])).unwrap();
    early.place_marks(&mut s);
    assert!(!early.timing_marks(&s.subtunes[0]).loop_changes);
    // No marks at all: nothing to write, and nothing to warn about beyond rule 5.
    s.subtunes[0].marks.clear();
    let t = early.timing_marks(&s.subtunes[0]);
    assert!(t.marks.is_empty() && t.loop_changes);
}

proptest! {
    /// For any checked song whose subtunes form an adaptive set switching every 1-3 bars, with any loop row and an
    /// extra mark anywhere, placing marks leaves no switch point unmarked; values are all at least $10; only switch
    /// points get one; every mark differs from the one heard before it; and so does the first mark heard after the
    /// loop from the last, whenever the loop holds two or more.
    #[test]
    fn placed_marks_satisfy_rule_5_and_always_change(
        (song, _ctx) in strategies::checked_song(),
        every in 1u32..4,
        loop_at in any::<prop::sample::Index>(),
        extra in any::<prop::sample::Index>(),
    ) {
        let mut song = song;
        let names: Vec<SubtuneName> = song.subtunes.iter().map(|s| s.name.clone()).collect();
        for s in &mut song.subtunes {
            let length = s.length.get() as usize;
            s.marks = vec![extra.index(length) as u32];
            s.loop_row = loop_at.index(length) as u32;
        }
        let set = AdaptiveSet::with_switch("all", names, Switch::EveryBars(NonZeroU32::new(every).unwrap())).unwrap();
        set.place_marks(&mut song);
        let missing: Vec<_> = set
            .check(&song)
            .into_iter()
            .filter(|p| matches!(p, LayerProblem::Mark { .. }))
            .collect();
        prop_assert!(missing.is_empty(), "{missing:#?}");
        let step = every as usize;
        for s in &song.subtunes {
            let t = set.timing_marks(s);
            let bar = u32::from(s.rows_per_bar.get());
            prop_assert!(t.marks.iter().all(|m| m.value >= FIRST_MARK_VALUE));
            prop_assert!(t.marks.iter().all(|m| m.row % (bar * step as u32) == 0));
            prop_assert!(t.off_switch.iter().all(|r| r % (bar * step as u32) != 0));
            prop_assert_eq!(t.marks.len() + t.off_switch.len(), s.marks.len());
            prop_assert!(t.marks.windows(2).all(|w| w[0].value != w[1].value && w[0].row < w[1].row));
            let in_loop: Vec<_> = t.marks.iter().filter(|m| m.row >= s.loop_row).collect();
            prop_assert_eq!(t.loop_changes, in_loop.len() >= 2);
            if in_loop.len() >= 2 {
                prop_assert_ne!(in_loop[0].value, t.marks.last().unwrap().value);
            }
        }
    }
}
