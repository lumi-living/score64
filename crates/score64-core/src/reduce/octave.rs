//! Octave folding (#41): moving a voice's notes by whole octaves, a bar at a time, into its role's pitch range.

use crate::provenance::{Confidence, Decision, DecisionId, Proposal, Provenance};
use crate::score::{Note, Pitch, Role, Roles};
use crate::validate::RoleRanges;

use super::{Placement, Reduced};

/// The rule name and version [`fold_octaves`] records in its decisions.
pub const FOLD_OCTAVES: &str = "fold-octaves@1";

/// The largest shift tried, in octaves either way: enough to cross GoatTracker's whole range (C-0 to G#7).
const MAX_SHIFT: i8 = 7;

/// Whether notes in `role` are folded. Drums and texture use noise and pitch sweeps over the whole range, so their
/// pitches stay where they are.
fn folds(role: Role) -> bool {
    !matches!(role, Role::Drums | Role::Texture)
}

/// Fold one voice line's `notes` (in row order) into the ranges of their roles, a bar at a time.
///
/// Each bar gets one whole-octave shift, so the melody keeps its shape inside the bar. Of the shifts that keep every
/// note playable, the rule picks the one that leaves the fewest notes outside the range of the role at their row; on
/// a tie, the previous bar's shift (so the line does not jump an octave between bars); then the smallest. Notes that
/// still don't fit stay out of range, and the validator warns about them (rule 6). Drum and texture notes are never
/// moved, and do not count.
///
/// Each bar with a non-zero shift gets one [`Proposal::Octave`] decision (`octave-v<voice>-b<bar>`), whose confidence
/// is the share of the bar's folded notes that end up in range. Its notes record the decision's id unless they
/// already record one.
pub fn fold_octaves(
    mut notes: Vec<Note>,
    roles: &Roles,
    ranges: &RoleRanges,
    at: &Placement,
) -> Reduced {
    let rows_per_bar = u32::from(at.rows_per_bar.get());
    let mut decisions = Vec::new();
    let mut previous = 0i8;
    let mut start = 0;
    while start < notes.len() {
        let bar = notes[start].row / rows_per_bar;
        let end = start
            + notes[start..]
                .iter()
                .take_while(|n| n.row / rows_per_bar == bar)
                .count();
        let folded: Vec<usize> = (start..end)
            .filter(|&i| folds(roles.at(notes[i].row)))
            .collect();
        start = end;
        if folded.is_empty() {
            continue;
        }
        let outside = |shift: i8| -> Option<usize> {
            folded.iter().try_fold(0, |out, &i| {
                let n = &notes[i];
                let p = shifted(n.pitch, shift)?;
                Some(out + usize::from(!ranges.of(roles.at(n.row)).contains(p)))
            })
        };
        let (shift, out) = (-MAX_SHIFT..=MAX_SHIFT)
            .filter_map(|k| outside(k).map(|out| (k, out)))
            .min_by_key(|&(k, out)| (out, k != previous, k.unsigned_abs()))
            .expect("a shift of 0 keeps every note playable");
        previous = shift;
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
    Reduced { notes, decisions }
}

/// `p` moved by `octaves`, if GoatTracker can still play it.
fn shifted(p: Pitch, octaves: i8) -> Option<Pitch> {
    let s = i16::from(p.semitone()) + 12 * i16::from(octaves);
    Pitch::new(u8::try_from(s).ok()?)
}

#[cfg(test)]
mod tests;
