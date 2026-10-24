//! A whole-line reference for `monophonise_with`'s drops and keeps (#217), written apart from the windowed runs:
//! every trial places the whole line again with the rule's own steps, and every drop is checked against every trim
//! and fresh start. Properties check that the windowed runs give the same, and the rule's other promises.

use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use super::*;
use crate::decide::{Claims, lost_to, strength};
use crate::profile::{ProfileName, builtin};
use crate::provenance::{ContentHash, SourceRef};
use crate::score::{FramesPerRow, PresetId, Role, Roles, Schema, Song, Subtune, VoiceLine};
use crate::validate::{Context, InstrumentFacts, validate};

/// The whole line placed with `taking` (plus `also`) out and `keeps` in force; keeps with no note left are left out.
fn whole(
    all: &[Note],
    taking: &BTreeMap<usize, &Decision>,
    also: Option<usize>,
    keeps: &BTreeMap<(u32, Pitch), &Decision>,
    rules: &MonoRules,
    at: &Placement,
) -> (Vec<Note>, Vec<(DecisionId, Unapplied)>) {
    let notes: Vec<Note> = all
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != also && !taking.contains_key(i))
        .map(|(_, n)| n.clone())
        .collect();
    let mut keeps: BTreeMap<(u32, Pitch), &Decision> = keeps
        .iter()
        .filter(|&(&(row, pitch), _)| notes.iter().any(|n| (n.row, n.pitch) == (row, pitch)))
        .map(|(&k, &d)| (k, d))
        .collect();
    let mut scratch = Log::new(at, MONOPHONISE, "mono");
    let mut why = Vec::new();
    let line = place(notes, &mut keeps, rules, &mut scratch, &mut why, None);
    (line, why)
}

fn sound_of(line: &[Note], row: u32, pitch: Pitch) -> Option<NonZeroU32> {
    let i = line.iter().position(|n| (n.row, n.pitch) == (row, pitch))?;
    let l = line[i].length.get();
    let end = line.get(i + 1).map_or(l, |nx| l.min(nx.row - row));
    NonZeroU32::new(end)
}

fn undoes_whole<'d>(
    held: &[&'d Decision],
    d: &Decision,
    before: &[Note],
    after: &[Note],
) -> Option<&'d Decision> {
    for &h in held.iter().filter(|h| strength(h, d).is_lt()) {
        let Some((row, pitch)) = about(h) else {
            continue;
        };
        let b = sound_of(before, row, pitch);
        let a = sound_of(after, row, pitch);
        let fits = |l: Option<NonZeroU32>| match h.proposal {
            Proposal::TrimNote { length, .. } => l.is_some_and(|l| length <= l),
            _ => l.is_some(),
        };
        if (b.is_some() && a.is_none()) || (fits(b) && !fits(a)) {
            return Some(h);
        }
    }
    None
}

/// `apply_in_order` with every decision placing the whole line, and every drop checked.
fn apply_whole<'d>(
    all: &[Note],
    order: &[&'d Decision],
    held: &[&'d Decision],
    won: &mut Claims<'d>,
    rules: &MonoRules,
    at: &Placement,
) -> Applied<'d> {
    let mut out = Applied {
        dropping: BTreeMap::new(),
        kept: BTreeMap::new(),
        saved: BTreeSet::new(),
        refused: Vec::new(),
    };
    for &d in order {
        if let Some(why) = won.claim(d) {
            out.refused.push((d.id.clone(), why));
            continue;
        }
        match d.proposal {
            Proposal::DropNote {
                row,
                pitch,
                length,
                source,
                ..
            } => {
                if let Some(h) = held_over(held, d, row, pitch) {
                    out.refused.push((d.id.clone(), lost_to(d, h)));
                    continue;
                }
                let found = on_row(all, row)
                    .filter(|&i| {
                        let n = &all[i];
                        n.pitch == pitch
                            && n.length == length
                            && (source.is_none() || n.source == source)
                    })
                    .last();
                let Some(i) = found else {
                    out.refused.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                let before = whole(all, &out.dropping, None, &out.kept, rules, at).0;
                let after = whole(all, &out.dropping, Some(i), &out.kept, rules, at).0;
                let keep = out.kept.get(&(row, pitch)).copied();
                let lost = keep.filter(|_| !after.iter().any(|n| (n.row, n.pitch) == (row, pitch)));
                let undone = undoes_whole(held, d, &before, &after);
                if let Some(by) = lost.into_iter().chain(undone).min_by(|a, b| strength(a, b)) {
                    out.refused.push((d.id.clone(), lost_to(d, by)));
                    if lost.is_some() {
                        out.saved.insert((row, pitch));
                    }
                    continue;
                }
                out.dropping.insert(i, d);
            }
            Proposal::KeepNote { row, pitch, .. } => {
                let here = (row, pitch);
                let there: Vec<usize> = on_row(all, row)
                    .filter(|&i| all[i].pitch == pitch)
                    .collect();
                if there.is_empty() {
                    out.refused.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                }
                let left: Vec<usize> = there
                    .iter()
                    .copied()
                    .filter(|i| !out.dropping.contains_key(i))
                    .collect();
                if left.is_empty() {
                    let by = there
                        .iter()
                        .map(|i| out.dropping[i])
                        .min_by(|a, b| strength(a, b))
                        .unwrap();
                    out.refused.push((d.id.clone(), lost_to(d, by)));
                    continue;
                }
                let before = whole(all, &out.dropping, None, &out.kept, rules, at).0;
                if !before.iter().any(|n| (n.row, n.pitch) == here) {
                    let mut with = out.kept.clone();
                    with.insert(here, d);
                    let (next, why) = whole(all, &out.dropping, None, &with, rules, at);
                    if !next.iter().any(|n| (n.row, n.pitch) == here) {
                        assert_eq!(why.len(), 1, "only the new keep can fail: {why:?}");
                        assert_eq!(why[0].0, d.id);
                        out.refused.push(why[0].clone());
                        continue;
                    }
                    assert!(why.is_empty(), "{why:?}");
                    if let Some(h) = undoes_whole(held, d, &before, &next) {
                        out.refused.push((d.id.clone(), lost_to(d, h)));
                        continue;
                    }
                }
                out.kept.insert(here, d);
            }
            _ => unreachable!(),
        }
    }
    for (&here, &d) in &out.kept {
        let stronger: BTreeMap<(u32, Pitch), &Decision> = out
            .kept
            .iter()
            .filter(|(_, o)| strength(o, d).is_lt())
            .map(|(&k, &o)| (k, o))
            .collect();
        let line = whole(all, &out.dropping, None, &stronger, rules, at).0;
        if !line.iter().any(|n| (n.row, n.pitch) == here) {
            out.saved.insert(here);
        }
    }
    out
}

fn ref_monophonise_with(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
    overrides: &[Decision],
) -> Reduced {
    let mut log = Log::new(at, MONOPHONISE, "mono");
    let mut unapplied = Vec::new();
    let mut mine: Vec<&Decision> = overrides
        .iter()
        .filter(|d| &d.subtune == at.subtune && d.rank() != Rank::Rules)
        .filter(|d| match d.proposal {
            Proposal::DropNote { voice, .. }
            | Proposal::TrimNote { voice, .. }
            | Proposal::Retrigger { voice, .. }
            | Proposal::KeepNote { voice, .. } => voice == at.voice,
            _ => false,
        })
        .collect();
    // In slot order, strongest first, so the order they come in never matters. Of several for one slot, the strongest
    // claims it and the rest lose to it, as `resolve` would choose.
    mine.sort_by(|a, b| a.slot().cmp(&b.slot()).then_with(|| strength(a, b)));
    let mut won = Claims::default();
    let mut notes: Vec<Note> = notes.into_iter().collect();
    notes.sort_by(|a, b| canonical(a, b, rules.skyline));

    // The trims and fresh starts that will hold their slots (the first of each slot), strongest first. A weaker drop
    // gives way to one of its note, and a weaker keep may not make one unapplied.
    let mut held: Vec<&Decision> = Vec::new();
    for (i, d) in mine.iter().enumerate() {
        let first = i == 0 || mine[i - 1].slot() != d.slot();
        if first
            && matches!(
                d.proposal,
                Proposal::TrimNote { .. } | Proposal::Retrigger { .. }
            )
        {
            held.push(d);
        }
    }
    held.sort_by(|a, b| strength(a, b));

    // 0. Drops and keeps in force, one at a time, strongest first, each on the line the stronger ones made: a drop
    // takes its note out before the rule runs, a keep keeps its note in place. Neither may make a stronger trim or
    // fresh start unapplied.
    let mut order: Vec<&Decision> = mine
        .iter()
        .copied()
        .filter(|d| {
            matches!(
                d.proposal,
                Proposal::DropNote { .. } | Proposal::KeepNote { .. }
            )
        })
        .collect();
    order.sort_by(|a, b| strength(a, b));
    let applied = apply_whole(&notes, &order, &held, &mut won, rules, at);
    unapplied.extend(applied.refused);
    let dropping = applied.dropping;
    let saved = applied.saved;
    let mut kept = applied.kept;

    // The line with every keep that applied in force, their notes placed first. The line without any (with the drops
    // that apply) says how sure a drop they force is.
    let notes = without(&notes, &dropping);
    let base = (!kept.is_empty()).then(|| {
        let mut scratch = Log::new(at, MONOPHONISE, "mono");
        place(
            notes.clone(),
            &mut BTreeMap::new(),
            rules,
            &mut scratch,
            &mut Vec::new(),
            None,
        );
        Baseline::of(&scratch.decisions)
    });
    let mut line = place(
        notes,
        &mut kept,
        rules,
        &mut log,
        &mut unapplied,
        base.as_ref(),
    );
    // A note a keep saved records it, in place of any id this rule would give it; an earlier rule's id stays.
    for n in &mut line {
        if saved.contains(&(n.row, n.pitch))
            && let Some(d) = kept.get(&(n.row, n.pitch))
        {
            n.decision.get_or_insert(d.id.clone());
        }
    }
    for i in 1..line.len() {
        let next = line[i].row;
        let prev = &mut line[i - 1];
        if prev.row.saturating_add(prev.length.get()) > next {
            let length = NonZeroU32::new(next - prev.row).expect("rows are distinct and in order");
            let id = log.record(
                log.id("trim", prev.row),
                prev.row,
                Proposal::TrimNote {
                    voice: at.voice,
                    row: prev.row,
                    pitch: prev.pitch,
                    length,
                    source: prev.source,
                },
                Confidence::CERTAIN,
            );
            prev.length = length;
            prev.decision.get_or_insert(id);
        }
    }

    // Trims and fresh starts in force, on the line as built. A note an override changes records its id in place of
    // one this rule gave it; an earlier rule's id stays.
    let own =
        |id: &Option<DecisionId>, log: &Log| id.as_ref().is_none_or(|id| log.ids.contains(id));
    let mut trimmed = Vec::new();
    for d in &mine {
        match d.proposal {
            Proposal::TrimNote {
                row, pitch, length, ..
            } => {
                if let Some(why) = won.claim(d) {
                    unapplied.push((d.id.clone(), why));
                    continue;
                }
                let Some(n) = line.iter_mut().find(|n| n.row == row && n.pitch == pitch) else {
                    unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                // A trim only shortens: the rule has already ended the note where the next starts, if not before.
                if length > n.length {
                    unapplied.push((d.id.clone(), Unapplied::Lengthens));
                    continue;
                }
                // Shorter than the rule allows is a person's call, not a model's.
                if length < rules.min_rows && d.rank() < Rank::Person {
                    unapplied.push((d.id.clone(), Unapplied::OutOfRange));
                    continue;
                }
                trimmed.push((row, pitch));
                // The override takes the place of the rule's trim, so the note records it when it changes the
                // length or when the rule's trim had set it.
                let rules_trim = n.decision.is_some() && own(&n.decision, &log);
                if (n.length != length || rules_trim) && own(&n.decision, &log) {
                    n.decision = Some(d.id.clone());
                }
                n.length = length;
            }
            Proposal::Retrigger { row, pitch, .. } => {
                if let Some(why) = won.claim(d) {
                    unapplied.push((d.id.clone(), why));
                    continue;
                }
                let Some(n) = line.iter_mut().find(|n| n.row == row && n.pitch == pitch) else {
                    unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                if n.attack != Attack::Fresh {
                    n.attack = Attack::Fresh;
                    if own(&n.decision, &log) {
                        n.decision = Some(d.id.clone());
                    }
                }
            }
            _ => {}
        }
    }
    // A trim in force replaces the rule's own trim of that note. A note whose override didn't apply keeps the rule's
    // trim, and its id.
    log.decisions.retain(|r| match r.proposal {
        Proposal::TrimNote { row, pitch, .. } => !trimmed.contains(&(row, pitch)),
        _ => true,
    });
    // 4. Ties that lost the note they were tied to.
    let mut end = None;
    for note in &mut line {
        if note.attack == Attack::Legato && end != Some(note.row) {
            let id = log.record(
                log.id("tie", note.row),
                note.row,
                Proposal::Retrigger {
                    voice: at.voice,
                    row: note.row,
                    pitch: note.pitch,
                    source: note.source,
                },
                Confidence::CERTAIN,
            );
            note.attack = Attack::Fresh;
            note.decision.get_or_insert(id);
        }
        end = Some(note.row.saturating_add(note.length.get()));
    }

    unapplied.sort_by(|a, b| a.0.cmp(&b.0));
    Reduced {
        notes: line,
        decisions: log.decisions,
        unapplied,
    }
}

fn sub() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}
fn at(subtune: &SubtuneName) -> Placement<'_> {
    Placement {
        subtune,
        voice: Voice::new(1).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
    }
}
fn v1() -> Voice {
    Voice::new(1).unwrap()
}
fn nt(row: u32, length: u32, pitch: &str, c: f32) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(c).unwrap(),
        decision: None,
    }
}
fn person(id: &str, proposal: Proposal) -> Decision {
    let row = match proposal {
        Proposal::DropNote { row, .. }
        | Proposal::TrimNote { row, .. }
        | Proposal::Retrigger { row, .. }
        | Proposal::KeepNote { row, .. } => row,
        _ => 0,
    };
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: sub(),
        bar: row / 16,
        proposal,
        provenance: Provenance::Manual,
        confidence: Confidence::CERTAIN,
        reviewed: false,
    }
}
fn model(mut d: Decision) -> Decision {
    d.provenance = Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([1; 32]),
    };
    d
}
fn drop_of(row: u32, length: u32, pitch: &str) -> Proposal {
    Proposal::DropNote {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        source: None,
        reason: DropReason::Overlap,
    }
}
fn trim_of(row: u32, pitch: &str, length: u32) -> Proposal {
    Proposal::TrimNote {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        source: None,
    }
}
fn retrigger_of(row: u32, pitch: &str) -> Proposal {
    Proposal::Retrigger {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
        source: None,
    }
}
fn keep_of(row: u32, pitch: &str) -> Proposal {
    Proposal::KeepNote {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
    }
}
fn checked(notes: Vec<Note>) -> (Song, Context) {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: sub(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(400).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                roles: Roles::constant(Role::Lead),
                instrument: PresetId("lead".into()),
                notes,
                arps: vec![],
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
        instruments: BTreeMap::from([(PresetId("lead".into()), facts)]),
        ..Context::default()
    };
    (song, ctx)
}

fn rules_of(low: bool, ghost: f32, min: u32) -> MonoRules {
    MonoRules {
        skyline: if low {
            Skyline::Lowest
        } else {
            Skyline::Highest
        },
        ghost_below: Confidence::new(ghost).unwrap(),
        min_rows: NonZeroU32::new(min).unwrap(),
    }
}
fn any_rules(max_min: u32) -> impl Strategy<Value = MonoRules> {
    (
        any::<bool>(),
        prop_oneof![Just(0.0f32), Just(0.2f32), 0.0f32..0.6],
        1u32..=max_min,
    )
        .prop_map(|(l, g, m)| rules_of(l, g, m))
}

fn a_note(rows: u32, maxlen: u32) -> impl Strategy<Value = Note> {
    let pitch = prop_oneof![Just("C-4"), Just("E-4"), Just("G-4"), Just("B-4")]
        .prop_map(|p| p.parse::<Pitch>().unwrap());
    let confidence = prop_oneof![
        Just(0.1f32),
        Just(0.5f32),
        Just(0.8f32),
        Just(0.9f32),
        Just(1.0f32),
        0.0f32..=1.0
    ];
    let source = prop_oneof![
        3 => Just(None),
        1 => (0u64..3).prop_map(|s| SourceRef::new(ContentHash([7; 32]), s * 10, s * 10 + 5)),
    ];
    (
        0..rows,
        1..=maxlen,
        pitch,
        any::<bool>(),
        confidence,
        source,
    )
        .prop_map(|(row, length, pitch, tie, confidence, source)| Note {
            row,
            length: NonZeroU32::new(length).unwrap(),
            pitch,
            attack: if tie { Attack::Legato } else { Attack::Fresh },
            instrument: None,
            source,
            confidence: Confidence::new(confidence).unwrap(),
            decision: None,
        })
}

/// Soups of several kinds: dense (12 rows), wide (60 rows, long notes crossing runs), and clusters whose gaps sit
/// around `min_rows`.
fn soup() -> impl Strategy<Value = Vec<Note>> {
    prop_oneof![
        proptest::collection::vec(a_note(12, 6), 1..24),
        proptest::collection::vec(a_note(60, 20), 1..40),
        clusters(),
    ]
}

fn clusters() -> impl Strategy<Value = Vec<Note>> {
    // Up to 6 clusters, each of a few notes in 0..3 rows, the clusters 1..8 rows apart (so near any min_rows).
    proptest::collection::vec(
        (1u32..8, proptest::collection::vec(a_note(3, 14), 1..6)),
        1..7,
    )
    .prop_map(|cs| {
        let mut out = Vec::new();
        let mut base = 0;
        for (gap, notes) in cs {
            base += gap;
            for mut n in notes {
                n.row += base;
                out.push(n);
            }
            base += 2;
        }
        out
    })
}

type Pick = (proptest::sample::Index, u8, u32, u8, u8, u8, u32);

fn picks(max: usize) -> impl Strategy<Value = Vec<Pick>> {
    proptest::collection::vec(
        (
            any::<proptest::sample::Index>(),
            0u8..10,
            1u32..12,
            0u8..4,
            0u8..4,
            0u8..3,
            0u32..80,
        ),
        0..max,
    )
}

fn overrides(soup: &[Note], picks: &[Pick], dups: bool) -> Vec<Decision> {
    picks
        .iter()
        .enumerate()
        .map(|(k, &(i, kind, len, rank, prefix, sure, stray))| {
            let o = &soup[i.index(soup.len())];
            let pitch = o.pitch.to_string();
            let p = match kind {
                0 | 9 => drop_of(o.row, o.length.get(), &pitch),
                1 => drop_of(o.row, len, &pitch),
                2 => Proposal::DropNote {
                    voice: v1(),
                    row: o.row,
                    pitch: o.pitch,
                    length: o.length,
                    source: o.source,
                    reason: DropReason::Repeat,
                },
                3 => trim_of(o.row, &pitch, len.min(o.length.get() + 1)),
                4 => retrigger_of(o.row, &pitch),
                5 | 6 => keep_of(o.row, &pitch),
                7 => keep_of(stray, &pitch),
                _ => keep_of(o.row, ["C-4", "E-4", "G-4", "B-4"][len as usize % 4]),
            };
            let id = if dups && prefix == 0 {
                "dup".to_owned()
            } else {
                format!("{}{k}", ["a", "b", "c", "d"][usize::from(prefix)])
            };
            let mut d = person(&id, p);
            d.confidence = Confidence::new([1.0, 0.5, 0.2][usize::from(sure)]).unwrap();
            match rank {
                0 => {}
                1 => d = model(d),
                2 => {
                    d = model(d);
                    d.reviewed = true;
                }
                _ => {
                    d.provenance = Provenance::Rules {
                        rule: MONOPHONISE.into(),
                    }
                }
            }
            d
        })
        .collect()
}

fn shuffled<T: Clone>(v: &[T], seed: u64) -> Vec<T> {
    let mut v = v.to_vec();
    let mut state = seed;
    for i in (1..v.len()).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        v.swap(i, (state >> 33) as usize % (i + 1));
    }
    v
}

/// Every check of the dense properties in `tests`, with the windowed result the whole-line one, on any soup.
fn all_checks(
    soup: &[Note],
    rules: &MonoRules,
    overrides: &[Decision],
    seed: u64,
) -> Result<(), TestCaseError> {
    let n = sub();
    let r = monophonise_with(soup.to_vec(), &at(&n), rules, overrides);
    // The windowed result is the whole-line one.
    let reference = ref_monophonise_with(soup.to_vec(), &at(&n), rules, overrides);
    prop_assert_eq!(&r, &reference, "windowed differs from whole-line");
    let resolution = crate::decide::resolve(overrides.to_vec());
    let resolved = monophonise_with(soup.to_vec(), &at(&n), rules, &resolution.chosen);
    prop_assert_eq!(&r.notes, &resolved.notes);
    prop_assert_eq!(&r.decisions, &resolved.decisions);
    let (song, ctx) = checked(r.notes.clone());
    let errors: Vec<_> = validate(&song, &ctx)
        .into_iter()
        .filter(|p| p.is_error())
        .collect();
    prop_assert!(errors.is_empty(), "{errors:#?}");
    for w in r.notes.windows(2) {
        prop_assert!(w[0].row + w[0].length.get() <= w[1].row, "overlap {w:?}");
    }
    prop_assert_eq!(
        &monophonise_with(soup.to_vec(), &at(&n), rules, &shuffled(overrides, seed)),
        &r
    );
    prop_assert_eq!(
        &monophonise_with(shuffled(soup, seed ^ 0x55), &at(&n), rules, overrides),
        &r
    );
    let ids: BTreeSet<&DecisionId> = overrides.iter().map(|d| &d.id).collect();
    if ids.len() < overrides.len() {
        return Ok(());
    }
    let mut expect = resolved.unapplied.clone();
    for (loser, by) in &resolution.overruled {
        if loser.rank() != Rank::Rules {
            let winner = overrides.iter().find(|d| &d.id == by).unwrap();
            expect.push((loser.id.clone(), lost_to(loser, winner)));
        }
    }
    expect.sort_by(|a, b| a.0.cmp(&b.0));
    prop_assert_eq!(&r.unapplied, &expect);
    let refused: BTreeSet<&DecisionId> = r.unapplied.iter().map(|(id, _)| id).collect();
    let applied: Vec<&Decision> = overrides
        .iter()
        .filter(|d| d.rank() != Rank::Rules && !refused.contains(&d.id))
        .collect();
    for (id, why) in &r.unapplied {
        let d = overrides.iter().find(|d| &d.id == id).unwrap();
        prop_assert!(d.rank() != Rank::Rules);
        prop_assert!(
            !(d.rank() == Rank::Person && *why == Unapplied::Overruled),
            "{id:?} overruled"
        );
    }
    let mut left: Vec<(u32, Pitch, u32)> = soup
        .iter()
        .map(|x| (x.row, x.pitch, x.length.get()))
        .collect();
    let drops = r
        .decisions
        .iter()
        .chain(applied.iter().copied())
        .filter_map(|d| match d.proposal {
            Proposal::DropNote {
                row, pitch, length, ..
            } => Some((row, pitch, length.get())),
            _ => None,
        });
    for dropped in drops {
        let i = left.iter().position(|x| *x == dropped);
        prop_assert!(i.is_some(), "{dropped:?} dropped twice, or never there");
        left.swap_remove(i.unwrap());
    }
    prop_assert_eq!(left.len(), r.notes.len());
    for x in &r.notes {
        let i = left
            .iter()
            .position(|&(row, pitch, _)| (row, pitch) == (x.row, x.pitch));
        prop_assert!(i.is_some(), "{x:?} came from nowhere");
        left.swap_remove(i.unwrap());
    }
    for d in &applied {
        match d.proposal {
            Proposal::KeepNote { row, pitch, .. } => {
                prop_assert!(
                    r.notes.iter().any(|x| (x.row, x.pitch) == (row, pitch)),
                    "{} applied but gone",
                    d.id.as_str()
                );
            }
            // Applied trims and fresh starts did what they say.
            Proposal::TrimNote {
                row, pitch, length, ..
            } => {
                let x = r.notes.iter().find(|x| (x.row, x.pitch) == (row, pitch));
                prop_assert!(
                    x.is_some_and(|x| x.length == length),
                    "trim {} applied but {:?}",
                    d.id.as_str(),
                    x
                );
            }
            Proposal::Retrigger { row, pitch, .. } => {
                let x = r.notes.iter().find(|x| (x.row, x.pitch) == (row, pitch));
                prop_assert!(
                    x.is_some_and(|x| x.attack == Attack::Fresh),
                    "fresh {} applied but {:?}",
                    d.id.as_str(),
                    x
                );
            }
            _ => {}
        }
    }
    for x in &r.notes {
        if let Some(id) = &x.decision {
            let ok = r.decisions.iter().any(|d| &d.id == id) || applied.iter().any(|d| &d.id == id);
            prop_assert!(ok, "{id:?} unknown");
        }
    }
    // A weaker decision never makes a stronger one unapplied.
    for (id, why) in &r.unapplied {
        let t = overrides.iter().find(|d| &d.id == id).unwrap();
        let fewer: Vec<Decision> = overrides
            .iter()
            .filter(|o| strength(o, t).is_le())
            .cloned()
            .collect();
        let again = monophonise_with(soup.to_vec(), &at(&n), rules, &fewer);
        prop_assert!(
            again.unapplied.iter().any(|(i, _)| i == id),
            "{id:?} is {why:?} because of a weaker decision"
        );
    }
    // Stronger: each prefix of the strength order decides the same for its own members as the whole set does.
    let mut sorted: Vec<&Decision> = overrides
        .iter()
        .filter(|d| d.rank() != Rank::Rules)
        .collect();
    sorted.sort_by(|a, b| strength(a, b));
    for cut in 1..sorted.len() {
        let prefix: Vec<Decision> = sorted[..cut].iter().map(|d| (*d).clone()).collect();
        let p = monophonise_with(soup.to_vec(), &at(&n), rules, &prefix);
        let t = sorted[cut - 1];
        let in_p = p.unapplied.iter().find(|(i, _)| i == &t.id).map(|x| x.1);
        let in_r = r.unapplied.iter().find(|(i, _)| i == &t.id).map(|x| x.1);
        prop_assert!(
            !(in_r.is_some() && in_p.is_none()),
            "{:?}: {:?} with the stronger ones, {:?} with all",
            t.id,
            in_p,
            in_r
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters: 100000, ..ProptestConfig::default() })]

    #[test]
    fn windowed_equals_whole_and_all_checks_hold(
        soup in soup(),
        rules in any_rules(6),
        picks in picks(24),
        seed in any::<u64>(),
        dups in proptest::bool::weighted(0.15),
    ) {
        let overrides = overrides(&soup, &picks, dups);
        all_checks(&soup, &rules, &overrides, seed)?;
    }

    /// Two parts far enough apart that no note of the first reaches the second: together they give what each gives.
    #[test]
    fn far_parts_do_not_interact(
        a in proptest::collection::vec(a_note(20, 8), 1..20),
        b in proptest::collection::vec(a_note(20, 8), 1..20),
        rules in any_rules(6),
        pa in picks(12),
        pb in picks(12),
    ) {
        let n = sub();
        let reach = a.iter().map(|x| x.row + x.length.get()).max().unwrap() + 1;
        let gap = reach.max(a.iter().map(|x| x.row).max().unwrap() + rules.min_rows.get());
        let b: Vec<Note> = b
            .into_iter()
            .map(|mut x| {
                x.row += gap;
                x
            })
            .collect();
        // Ties after A count from A's end, which can't reach B. A keep of a stray row could name the other part.
        let nostray = |p: &[Pick]| {
            p.iter()
                .map(|&(i, k, l, r, x, s, t)| (i, if k == 7 { 5 } else { k }, l, r, x, s, t))
                .collect::<Vec<_>>()
        };
        let (pa, pb) = (nostray(&pa), nostray(&pb));
        let mut oa = overrides(&a, &pa, false);
        for d in &mut oa {
            d.id = DecisionId::new(format!("x{}", d.id.as_str())).unwrap();
        }
        let mut ob = overrides(&b, &pb, false);
        for d in &mut ob {
            d.id = DecisionId::new(format!("y{}", d.id.as_str())).unwrap();
        }
        let ra = monophonise_with(a.clone(), &at(&n), &rules, &oa);
        let rb = monophonise_with(b.clone(), &at(&n), &rules, &ob);
        let all: Vec<Note> = a.iter().chain(&b).cloned().collect();
        let both: Vec<Decision> = oa.iter().chain(&ob).cloned().collect();
        let r = monophonise_with(all, &at(&n), &rules, &both);
        let mut notes = ra.notes.clone();
        notes.extend(rb.notes.clone());
        prop_assert_eq!(&r.notes, &notes);
        let key = |d: &Decision| format!("{d:?}");
        let mut got: Vec<String> = r.decisions.iter().map(key).collect();
        let mut want: Vec<String> = ra.decisions.iter().chain(&rb.decisions).map(key).collect();
        got.sort();
        want.sort();
        prop_assert_eq!(got, want);
        let mut un = ra.unapplied.clone();
        un.extend(rb.unapplied.clone());
        un.sort_by(|x, y| x.0.cmp(&y.0));
        prop_assert_eq!(&r.unapplied, &un);
    }
}

/// Chains of runs at a fixed `min_rows`: each run's notes start within `min - 1` rows, runs start `min..min + 2` rows
/// after the last start of the one before, and notes are long enough to reach one or two runs on.
fn chain() -> impl Strategy<Value = (MonoRules, Vec<Note>)> {
    (
        2u32..6,
        any::<bool>(),
        prop_oneof![Just(0.0f32), Just(0.3f32)],
    )
        .prop_flat_map(|(min, low, ghost)| {
            let run = proptest::collection::vec(
                (
                    0..min,
                    1..=3 * min + 2,
                    0usize..3,
                    prop_oneof![
                        Just(0.1f32),
                        Just(0.5f32),
                        Just(0.6f32),
                        Just(0.8f32),
                        Just(0.9f32),
                        Just(1.0f32)
                    ],
                    any::<bool>(),
                ),
                1..5,
            );
            (
                Just(rules_of(low, ghost, min)),
                proptest::collection::vec((0..3u32, run), 2..6),
            )
                .prop_map(move |(rules, runs)| {
                    let mut out = Vec::new();
                    let mut base = 0u32;
                    for (gap, notes) in runs {
                        let mut last = 0;
                        for (off, len, p, c, tie) in notes {
                            let mut x = nt(base + off, len, ["C-4", "E-4", "G-4"][p], c);
                            if tie {
                                x.attack = Attack::Legato;
                            }
                            last = last.max(off);
                            out.push(x);
                        }
                        base += last + min + gap;
                    }
                    (rules, out)
                })
        })
}

proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters: 100000, ..ProptestConfig::default() })]

    #[test]
    fn chains_windowed_equals_whole_and_all_checks_hold(
        (rules, soup) in chain(),
        picks in picks(20),
        seed in any::<u64>(),
        dups in proptest::bool::weighted(0.1),
    ) {
        let overrides = overrides(&soup, &picks, dups);
        all_checks(&soup, &rules, &overrides, seed)?;
    }
}

/// Heavier soups: denser, wider and longer than `soup`'s.
fn heavy() -> impl Strategy<Value = (MonoRules, Vec<Note>)> {
    prop_oneof![
        (any_rules(8), proptest::collection::vec(a_note(8, 8), 1..32)),
        (
            any_rules(8),
            proptest::collection::vec(a_note(30, 12), 1..48)
        ),
        (
            any_rules(8),
            proptest::collection::vec(a_note(120, 30), 1..60)
        ),
        (any_rules(8), clusters()),
        chain(),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters: 100000, ..ProptestConfig::default() })]

    /// Only equality, cheap, on heavier soups and more decisions.
    #[test]
    fn eq_only_heavy(
        (rules, soup) in heavy(),
        picks in picks(40),
        dups in proptest::bool::weighted(0.1),
    ) {
        let n = sub();
        let overrides = overrides(&soup, &picks, dups);
        let r = monophonise_with(soup.clone(), &at(&n), &rules, &overrides);
        let reference = ref_monophonise_with(soup, &at(&n), &rules, &overrides);
        prop_assert_eq!(&r, &reference, "windowed differs from whole-line");
    }
}
