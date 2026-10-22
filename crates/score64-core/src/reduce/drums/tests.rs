//! Drum hits on a voice of their own, and interleaved with another line.

use std::collections::BTreeMap;
use std::num::NonZeroU8;

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
use crate::score::{
    FramesPerRow, Role, Roles, Schema, Song, Subtune, SubtuneName, Voice, VoiceLine,
};
use crate::strategies;
use crate::validate::{Context, InstrumentFacts, validate};

fn name() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn at(subtune: &SubtuneName) -> Placement<'_> {
    Placement {
        subtune,
        voice: Voice::new(3).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
    }
}

fn sound(instrument: &str, pitch: &str, rows: u32) -> DrumSound {
    DrumSound {
        instrument: PresetId(instrument.into()),
        pitch: pitch.parse().unwrap(),
        rows: NonZeroU32::new(rows).unwrap(),
    }
}

/// A kit with every kind but `other`.
fn kit() -> DrumKit {
    DrumKit {
        kick: Some(sound("kick", "C-3", 2)),
        snare: Some(sound("snare", "C-5", 3)),
        hat: Some(sound("hat", "C-7", 1)),
        other: None,
    }
}

/// A transcribed hit: General MIDI drum note `gm` on `row`.
fn hit(row: u32, gm: u8) -> Note {
    Note {
        row,
        length: NonZeroU32::new(1).unwrap(),
        pitch: Pitch::from_midi(gm).unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(0.9).unwrap(),
        decision: None,
    }
}

fn line_note(row: u32, length: u32, pitch: &str) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(0.9).unwrap(),
        decision: None,
    }
}

/// (row, length, instrument or "-") of each note.
fn shape(notes: &[Note]) -> Vec<(u32, u32, String)> {
    notes
        .iter()
        .map(|n| {
            let i = n
                .instrument
                .as_ref()
                .map_or("-".to_string(), |p| p.0.clone());
            (n.row, n.length.get(), i)
        })
        .collect()
}

fn reasons(r: &Reduced) -> Vec<DropReason> {
    r.decisions
        .iter()
        .filter_map(|d| match d.proposal {
            Proposal::DropNote { reason, .. } => Some(reason),
            _ => None,
        })
        .collect()
}

#[test]
fn general_midi_drum_notes_have_kinds() {
    let kinds: Vec<_> = [35, 36, 37, 38, 40, 42, 46, 49, 51, 41, 45, 60]
        .map(DrumKind::from_midi)
        .into();
    use DrumKind::*;
    assert_eq!(
        kinds,
        [
            Kick, Kick, Snare, Snare, Snare, Hat, Hat, Hat, Hat, Other, Other, Other
        ]
    );
}

#[test]
fn hits_become_the_kits_sounds() {
    // Kick on 0, hats on 2 and 6, snare on 4: each plays its preset at its pitch, for its rows or to the next hit.
    let n = name();
    let r = fold_drums(
        [hit(4, 38), hit(0, 36), hit(2, 42), hit(6, 42)],
        &at(&n),
        &kit(),
    );
    assert_eq!(
        shape(&r.notes),
        [
            (0, 2, "kick".into()),
            (2, 1, "hat".into()),
            (4, 2, "snare".into()),
            (6, 1, "hat".into())
        ]
    );
    assert_eq!(r.notes[0].pitch.to_string(), "C-3");
    assert!(r.decisions.is_empty());
}

#[test]
fn one_hit_a_row_by_priority_and_unmapped_kinds_go() {
    // Kick and hat on 0: the kick stays. A tom (GM 45) has no sound in this kit.
    let n = name();
    let r = fold_drums([hit(0, 42), hit(0, 36), hit(4, 45)], &at(&n), &kit());
    assert_eq!(shape(&r.notes), [(0, 2, "kick".into())]);
    assert_eq!(reasons(&r), [DropReason::Clash, DropReason::Unmapped]);
    let ids: Vec<_> = r.decisions.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["drums-clash-v3-r0-p30", "drums-unmapped-v3-r4-p33"]);
    assert!(r.decisions.iter().all(|d| d.provenance
        == Provenance::Rules {
            rule: "fold-drums@1".into()
        }));
}

#[test]
fn interleaving_cuts_the_line_and_drops_hats() {
    // A bass note over rows 0–8 and another from 8; a snare on 4 and a hat on 10.
    let n = name();
    let drums = fold_drums([hit(4, 38), hit(10, 42)], &at(&n), &kit()).notes;
    let bass = vec![line_note(0, 8, "C-2"), line_note(8, 8, "G-1")];
    let r = interleave_drums(bass, drums, &at(&n), &kit(), &InterleaveRules::default());
    assert_eq!(
        shape(&r.notes),
        [
            (0, 4, "-".into()),
            (4, 3, "snare".into()),
            (8, 8, "-".into())
        ]
    );
    assert_eq!(reasons(&r), [DropReason::Interleave]);
    let trim = r
        .decisions
        .iter()
        .find(|d| matches!(d.proposal, Proposal::TrimNote { .. }))
        .unwrap();
    assert_eq!(trim.id.as_str(), "inter-trim-v3-r0");
    assert_eq!(r.notes[0].decision.as_ref(), Some(&trim.id));
    // With hats allowed, the hat cuts the second bass note too.
    let drums = fold_drums([hit(4, 38), hit(10, 42)], &at(&n), &kit()).notes;
    let bass = vec![line_note(0, 8, "C-2"), line_note(8, 8, "G-1")];
    let rules = InterleaveRules {
        hats: true,
        ..InterleaveRules::default()
    };
    let r = interleave_drums(bass, drums, &at(&n), &kit(), &rules);
    assert_eq!(
        shape(&r.notes),
        [
            (0, 4, "-".into()),
            (4, 3, "snare".into()),
            (8, 2, "-".into()),
            (10, 1, "hat".into())
        ]
    );
}

#[test]
fn a_clash_on_one_row_goes_to_the_drum_or_the_line() {
    let n = name();
    let drums = || fold_drums([hit(0, 36)], &at(&n), &kit()).notes;
    let bass = || vec![line_note(0, 4, "C-2")];
    let r = interleave_drums(
        bass(),
        drums(),
        &at(&n),
        &kit(),
        &InterleaveRules::default(),
    );
    assert_eq!(shape(&r.notes), [(0, 2, "kick".into())]);
    assert_eq!(reasons(&r), [DropReason::Clash]);
    let rules = InterleaveRules {
        on_clash: Clash::Line,
        ..InterleaveRules::default()
    };
    let r = interleave_drums(bass(), drums(), &at(&n), &kit(), &rules);
    assert_eq!(shape(&r.notes), [(0, 4, "-".into())]);
    assert_eq!(reasons(&r), [DropReason::Clash]);
}

#[test]
fn a_tie_after_a_hit_starts_fresh() {
    let n = name();
    let drums = fold_drums([hit(4, 38)], &at(&n), &kit()).notes;
    let mut tied = line_note(4, 4, "D-2");
    tied.attack = Attack::Legato;
    // The tie on 4 loses its row to the snare; the tie on 8 follows the snare, not its own line, so it restarts.
    let mut after = line_note(8, 4, "E-2");
    after.attack = Attack::Legato;
    let bass = vec![line_note(0, 4, "C-2"), tied, after];
    let r = interleave_drums(bass, drums, &at(&n), &kit(), &InterleaveRules::default());
    let attacks: Vec<_> = r.notes.iter().map(|n| n.attack).collect();
    assert_eq!(attacks, [Attack::Fresh, Attack::Fresh, Attack::Fresh]);
    assert!(
        r.decisions
            .iter()
            .any(|d| d.id.as_str() == "inter-tie-v3-r8")
    );
}

#[test]
fn kits_round_trip_and_name_their_kinds() {
    let k = kit();
    let json = serde_json::to_string(&k).unwrap();
    assert!(!json.contains("other"), "{json}");
    assert_eq!(serde_json::from_str::<DrumKit>(&json).unwrap(), k);
    let snare = fold_drums([hit(0, 38)], &at(&name()), &k).notes.remove(0);
    assert_eq!(k.kind_of(&snare), Some(DrumKind::Snare));
    assert_eq!(k.kind_of(&line_note(0, 1, "C-4")), None);
}

/// The song a drum or interleaved line is checked in: a 1024-row subtune under `full`, the line on voice 3, every
/// instrument fitted.
fn checked(notes: Vec<Note>, role: Role) -> (Song, Context) {
    let mut instruments = BTreeMap::new();
    let facts = InstrumentFacts {
        filter: false,
        pulse_at_start: true,
        gate_timer: 2,
        hard_restart: true,
        gate_off: true,
    };
    for i in ["bass", "kick", "snare", "hat"] {
        instruments.insert(PresetId(i.into()), facts);
    }
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: name(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(1024).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(3).unwrap(),
                roles: Roles::constant(role),
                instrument: PresetId("bass".into()),
                notes,
                arps: vec![],
            }],
        }],
    };
    (
        song,
        Context {
            instruments,
            ..Context::default()
        },
    )
}

/// Drum soups: General MIDI drum notes 35–59 on any rows, some repeated.
fn drum_soup() -> impl Strategy<Value = Vec<Note>> {
    proptest::collection::vec((0u32..256, 35u8..60, 0.0f32..=1.0), 0..48).prop_map(|hits| {
        hits.into_iter()
            .map(|(row, gm, c)| {
                let mut h = hit(row, gm);
                h.confidence = Confidence::new(c).unwrap();
                h
            })
            .collect()
    })
}

fn errors(song: &Song, ctx: &Context) -> Vec<crate::validate::Problem> {
    validate(song, ctx)
        .into_iter()
        .filter(|p| p.is_error())
        .collect()
}

proptest! {
    /// Any drum soup folds to a valid drum line; every hit is kept or dropped by a decision; order does not matter.
    #[test]
    fn drum_soups_fold_to_valid_lines(soup in drum_soup(), reverse in any::<bool>()) {
        let n = name();
        let r = fold_drums(soup.clone(), &at(&n), &kit());
        prop_assert_eq!(r.notes.len() + reasons(&r).len(), soup.len());
        let (song, ctx) = checked(r.notes.clone(), Role::Drums);
        let e = errors(&song, &ctx);
        prop_assert!(e.is_empty(), "{e:#?}");
        let mut flipped = soup;
        if reverse {
            flipped.reverse();
        }
        prop_assert_eq!(fold_drums(flipped, &at(&n), &kit()), r);
    }

    /// Interleaving any folded drums with any clean line gives a valid line; every line note is kept (trimmed by a
    /// decision if shorter) or dropped by one.
    #[test]
    fn interleaving_keeps_the_line_valid(
        soup in drum_soup(),
        line in strategies::notes(),
        hats in any::<bool>(),
        line_wins in any::<bool>(),
    ) {
        let n = name();
        let drums = fold_drums(soup, &at(&n), &kit()).notes;
        let rules = InterleaveRules { hats, on_clash: if line_wins { Clash::Line } else { Clash::Drum } };
        // The line's own instruments are left to the voice line's; notes() may give some their own.
        let line: Vec<Note> = line.into_iter().map(|mut n| { n.instrument = None; n }).collect();
        let r = interleave_drums(line.clone(), drums.clone(), &at(&n), &kit(), &rules);
        let (song, ctx) = checked(r.notes.clone(), Role::Bass);
        let e = errors(&song, &ctx);
        prop_assert!(e.is_empty(), "{e:#?}");
        for original in &line {
            let kept = r.notes.iter().find(|k| k.row == original.row && kit().kind_of(k).is_none());
            match kept {
                Some(k) => {
                    prop_assert!(k.length <= original.length);
                    let trimmed = k.length != original.length;
                    let decided = r.decisions.iter().any(|d| matches!(d.proposal,
                        Proposal::TrimNote { row, length, .. } if row == k.row && length == k.length));
                    prop_assert_eq!(trimmed, decided);
                }
                None => {
                    let dropped = r.decisions.iter().any(|d| matches!(d.proposal,
                        Proposal::DropNote { row, reason: DropReason::Clash, .. } if row == original.row));
                    prop_assert!(dropped, "line note at {} vanished", original.row);
                }
            }
        }
        // Drums never sound longer than their kit says.
        for d in r.notes.iter().filter(|n| kit().kind_of(n).is_some()) {
            let kind = kit().kind_of(d).unwrap();
            prop_assert!(d.length <= kit().sound(kind).unwrap().rows);
            prop_assert!(hats || kind != DrumKind::Hat);
        }
    }
}
