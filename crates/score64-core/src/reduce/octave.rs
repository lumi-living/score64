//! Octave folding (#41): moving a voice's notes by whole octaves, a bar at a time, into its role's pitch range.

use crate::decide::{Rank, Unapplied};
use crate::provenance::{Confidence, Decision, DecisionId, Proposal, Provenance};
use crate::score::{Note, Pitch, Role, Roles};
use crate::validate::RoleRanges;

use super::{Placement, Reduced};

/// The rule name and version [`fold_octaves`] records in its decisions.
pub const FOLD_OCTAVES: &str = "fold-octaves@1";

/// The largest shift tried, in octaves either way: enough to cross GoatTracker's whole range (C-0 to G#7).
const MAX_SHIFT: i8 = 7;

/// Whether notes in `role` are folded: those whose range is narrower than everything GoatTracker can play. With the
/// default ranges, drums and texture (noise and pitch sweeps) may use it all, so their pitches stay where they are.
fn folds(ranges: &RoleRanges, role: Role) -> bool {
    let range = ranges.of(role);
    (range.low, range.high) != (Pitch::MIN, Pitch::MAX)
}

/// Fold one voice line's `notes` into the ranges of their roles, a bar at a time.
///
/// Each bar gets one whole-octave shift, so the melody keeps its shape inside the bar. Of the shifts that keep every
/// note playable, the rule picks the one that leaves the fewest notes outside the range of the role at their row. On
/// a tie it moves nothing if that is as good, so a bar moves only when moving brings notes into range; otherwise it
/// keeps the previous bar's shift (so the line does not jump an octave between bars), as long as the bars are next to
/// each other and in the same role (a bar's role is that of its first folded note); then it takes the smallest shift,
/// and of two equal ones the downward. Notes that still don't fit stay out of range,
/// and the validator warns about them (rule 6). Notes whose role may use the whole range never move, and do not
/// count.
///
/// Each bar with a non-zero shift gets one [`Proposal::Octave`] decision (`octave-v<voice>-b<bar>`). Its confidence
/// is the share of the bar's folded notes that end up in range; a tie between shifts is settled by the order above,
/// not reported as doubt, because a range wider than an octave often fits a bar at more than one shift. Its notes
/// record the decision's id unless they already record one. The result is in row order whatever the order
/// of `notes`.
pub fn fold_octaves(
    notes: Vec<Note>,
    at: &Placement,
    roles: &Roles,
    ranges: &RoleRanges,
) -> Reduced {
    fold_octaves_with(notes, at, roles, ranges, &[])
}

/// [`fold_octaves`] with a model's or a person's resolved `octave` decisions in force (#167): for this subtune and
/// voice, each sets its bar's shift in place of the rule's choice, and the next bar carries it on as it would the
/// rule's. The rule records no decision for that bar, and the bar's folded notes record the override's id (unless
/// they record an earlier rule's).
///
/// An override is unapplied when its bar has no note that folds (`no_such_note`), or when its shift would take one
/// of them past what GoatTracker can play (`out_of_range`). The rules' own decisions, and any of another kind, are
/// left alone.
pub fn fold_octaves_with(
    mut notes: Vec<Note>,
    at: &Placement,
    roles: &Roles,
    ranges: &RoleRanges,
    overrides: &[Decision],
) -> Reduced {
    let mut unapplied = Vec::new();
    // Each bar's shift in force, by bar.
    let mut forced: std::collections::BTreeMap<u32, (i8, &DecisionId)> =
        std::collections::BTreeMap::new();
    for d in overrides
        .iter()
        .filter(|d| &d.subtune == at.subtune && d.rank() != Rank::Rules)
    {
        if let Proposal::Octave { voice, shift } = d.proposal
            && voice == at.voice
        {
            forced.insert(d.bar, (shift, &d.id));
        }
    }
    notes.sort_by_key(|n| n.row);
    let rows_per_bar = u32::from(at.rows_per_bar.get());
    let mut decisions = Vec::new();
    // The last bar with folded notes: its number, its role and its shift.
    let mut previous: Option<(u32, Role, i8)> = None;
    let mut start = 0;
    while start < notes.len() {
        let bar = notes[start].row / rows_per_bar;
        let end = start
            + notes[start..]
                .iter()
                .take_while(|n| n.row / rows_per_bar == bar)
                .count();
        let folded: Vec<usize> = (start..end)
            .filter(|&i| folds(ranges, roles.at(notes[i].row)))
            .collect();
        start = end;
        let Some(&first) = folded.first() else {
            continue;
        };
        if let Some((shift, id)) = forced.remove(&bar) {
            if folded
                .iter()
                .any(|&i| shifted(notes[i].pitch, shift).is_none())
            {
                unapplied.push((id.clone(), Unapplied::OutOfRange));
            } else {
                for &i in &folded {
                    let n = &mut notes[i];
                    n.pitch = shifted(n.pitch, shift).expect("checked playable");
                    n.decision.get_or_insert_with(|| id.clone());
                }
                previous = Some((bar, roles.at(notes[first].row), shift));
                continue;
            }
        }
        let role = roles.at(notes[first].row);
        let carried = match previous {
            Some((b, r, shift)) if b + 1 == bar && r == role => shift,
            _ => 0,
        };
        let outside = |shift: i8| -> Option<usize> {
            folded.iter().try_fold(0, |out, &i| {
                let n = &notes[i];
                let p = shifted(n.pitch, shift)?;
                Some(out + usize::from(!ranges.of(roles.at(n.row)).contains(p)))
            })
        };
        let options: Vec<(i8, usize)> = (-MAX_SHIFT..=MAX_SHIFT)
            .filter_map(|k| outside(k).map(|out| (k, out)))
            .collect();
        let &(shift, out) = options
            .iter()
            .min_by_key(|&&(k, out)| (out, k != 0, k != carried, k.unsigned_abs()))
            .expect("a shift of 0 keeps every note playable");
        previous = Some((bar, role, shift));
        if shift == 0 {
            continue;
        }
        let share = (folded.len() - out) as f32 / folded.len() as f32;
        let id = DecisionId::new(format!("octave-v{}-b{bar}", at.voice.get())).expect("a valid id");
        decisions.push(Decision {
            id: id.clone(),
            subtune: at.subtune.clone(),
            bar,
            proposal: Proposal::Octave {
                voice: at.voice,
                shift,
            },
            provenance: Provenance::Rules {
                rule: FOLD_OCTAVES.into(),
            },
            confidence: Confidence::new(share).expect("a share is 0 to 1"),
            reviewed: false,
        });
        for &i in &folded {
            let n = &mut notes[i];
            n.pitch = shifted(n.pitch, shift).expect("checked playable");
            n.decision.get_or_insert_with(|| id.clone());
        }
    }
    // Overrides for bars with no folded notes.
    unapplied.extend(
        forced
            .into_values()
            .map(|(_, id)| (id.clone(), Unapplied::NoSuchNote)),
    );
    unapplied.sort_by(|a, b| a.0.cmp(&b.0));
    Reduced {
        notes,
        decisions,
        unapplied,
    }
}

/// `p` moved by `octaves`, if GoatTracker can still play it.
fn shifted(p: Pitch, octaves: i8) -> Option<Pitch> {
    let s = i16::from(p.semitone()) + 12 * i16::from(octaves);
    Pitch::new(u8::try_from(s).ok()?)
}

#[cfg(test)]
mod tests;
