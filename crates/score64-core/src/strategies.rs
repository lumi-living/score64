//! proptest strategies for the model (feature `proptest`).
//!
//! - [`song`]: songs that are structurally sound. Notes on each line are in row order without overlaps, legato notes
//!   come only straight after another note, role spans and marks are inside the subtune, and subtune names are
//!   unique. Profile rules are not applied.
//! - [`checked_song`]: a song together with a validation [`Context`] it satisfies, so it has no errors.
//! - [`note_soup`]: unreduced material (any order, overlaps, chords) for the reduction to turn into valid lines.
//! - [`voice_rule`] and [`project_profile`]: random valid profile data.

use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use crate::adaptive::AdaptiveSet;
use crate::profile::{Profile, ProfileName, Profiles, VoiceRule, VoiceUse, builtin};
use crate::provenance::{
    Confidence, ContentHash, Decision, DecisionId, DropReason, Proposal, Provenance, SourceRef,
};
use crate::score::{
    ArpEvent, Attack, FramesPerRow, Note, Pitch, PresetId, Role, RoleSpan, Roles, Schema, Song,
    Subtune, SubtuneName, TempoChange, Voice, VoiceLine,
};
use crate::validate::{Context, InstrumentFacts};

/// Any playable pitch.
pub fn pitch() -> impl Strategy<Value = Pitch> {
    (Pitch::MIN.semitone()..=Pitch::MAX.semitone()).prop_map(|s| Pitch::new(s).expect("in range"))
}

/// Any voice.
pub fn voice() -> impl Strategy<Value = Voice> {
    (1u8..=3).prop_map(|n| Voice::new(n).expect("in range"))
}

/// Any role.
pub fn role() -> impl Strategy<Value = Role> {
    prop_oneof![
        Just(Role::Lead),
        Just(Role::Bass),
        Just(Role::Arp),
        Just(Role::Harmony),
        Just(Role::Drums),
        Just(Role::Drone),
        Just(Role::Ostinato),
        Just(Role::Bed),
        Just(Role::Texture),
    ]
}

/// Any built-in profile's name.
pub fn profile() -> impl Strategy<Value = ProfileName> {
    proptest::sample::select(vec![
        builtin::FULL,
        builtin::SFX_SHARES_V3,
        builtin::SFX_OWNS_V3,
        builtin::SFX_OWNS_V2_V3,
    ])
    .prop_map(|n| ProfileName::new(n).expect("valid"))
}

/// Any confidence.
pub fn confidence() -> impl Strategy<Value = Confidence> {
    (0.0f32..=1.0).prop_map(|v| Confidence::new(v).expect("in range"))
}

/// Any content hash.
pub fn content_hash() -> impl Strategy<Value = ContentHash> {
    any::<[u8; 32]>().prop_map(ContentHash)
}

/// Any source reference.
pub fn source_ref() -> impl Strategy<Value = SourceRef> {
    (content_hash(), 0u64..600_000_000, 1u64..10_000_000)
        .prop_map(|(h, start, len)| SourceRef::new(h, start, start + len).expect("non-empty"))
}

/// Any decision id.
pub fn decision_id() -> impl Strategy<Value = DecisionId> {
    "[a-z0-9][a-z0-9_-]{0,15}".prop_map(|s| DecisionId::new(s).expect("valid"))
}

/// Any plain tempo.
pub fn frames_per_row() -> impl Strategy<Value = FramesPerRow> {
    (FramesPerRow::MIN.get()..=FramesPerRow::MAX.get())
        .prop_map(|n| FramesPerRow::new(n).expect("in range"))
}

/// Any subtune name.
pub fn subtune_name() -> impl Strategy<Value = SubtuneName> {
    "[a-z0-9][a-z0-9_-]{0,11}".prop_map(|s| SubtuneName::new(s).expect("valid"))
}

/// Any preset name.
pub fn preset() -> impl Strategy<Value = PresetId> {
    "[a-z][a-z0-9-]{0,11}".prop_map(PresetId)
}

/// A voice line's notes: in row order, never overlapping, legato only when the note before ends on its row, some
/// with their own instrument.
pub fn notes() -> impl Strategy<Value = Vec<Note>> {
    let step = (
        (0u32..8, 1u32..16, pitch(), any::<bool>()),
        proptest::option::weighted(0.2, preset()),
        proptest::option::of(source_ref()),
        confidence(),
        proptest::option::of(decision_id()),
    );
    proptest::collection::vec(step, 0..24).prop_map(|steps| {
        let mut row = 0;
        let mut notes: Vec<Note> = Vec::with_capacity(steps.len());
        for ((gap, length, pitch, tie), instrument, source, confidence, decision) in steps {
            row += gap;
            let attack = if tie && gap == 0 && !notes.is_empty() {
                Attack::Legato
            } else {
                Attack::Fresh
            };
            notes.push(Note {
                row,
                length: NonZeroU32::new(length).expect("non-zero"),
                pitch,
                attack,
                instrument,
                source,
                confidence,
                decision,
            });
            row += length;
        }
        notes
    })
}

/// Role spans: the first at row 0, then changes at increasing rows.
pub fn roles() -> impl Strategy<Value = Roles> {
    (role(), proptest::collection::vec((1u32..64, role()), 0..3)).prop_map(|(first, later)| {
        let mut row = 0;
        let mut spans = vec![RoleSpan {
            row: 0,
            role: first,
        }];
        for (gap, role) in later {
            row += gap;
            spans.push(RoleSpan { row, role });
        }
        Roles::new(spans).expect("ordered from row 0")
    })
}

fn line_end(line: &VoiceLine) -> u32 {
    let notes = line.notes.last().map_or(0, |n| n.row + n.length.get());
    let roles = line.roles.spans().last().map_or(0, |s| s.row + 1);
    notes.max(roles)
}

/// A voice line on `voice`.
pub fn voice_line(voice: Voice) -> impl Strategy<Value = VoiceLine> {
    (roles(), preset(), notes()).prop_map(move |(roles, instrument, notes)| {
        let end = notes.last().map_or(0, |n| n.row + n.length.get());
        // One arpeggio over the first four rows, when there is room.
        let arps = (end >= 4)
            .then(|| ArpEvent {
                row: 0,
                length: NonZeroU32::new(4).expect("non-zero"),
                intervals: vec![0, 4, 7],
                decision: None,
            })
            .into_iter()
            .collect();
        VoiceLine {
            voice,
            roles,
            instrument,
            notes,
            arps,
        }
    })
}

/// A subtune called `name`, with one to three voice lines on distinct voices in voice order.
pub fn subtune(name: SubtuneName) -> impl Strategy<Value = Subtune> {
    let voices = proptest::sample::subsequence(vec![1u8, 2, 3], 1..=3);
    (
        voices,
        frames_per_row(),
        proptest::collection::vec((1u32..64, frames_per_row()), 0..3),
        prop_oneof![Just(12u8), Just(16), Just(24)],
        0u32..64,
        proptest::option::weighted(0.3, profile()),
    )
        .prop_flat_map(|(voices, fpr, tempo, rows_per_bar, tail, profile)| {
            let lines: Vec<_> = voices
                .into_iter()
                .map(|v| voice_line(Voice::new(v).expect("1-3")))
                .collect();
            (lines, Just((fpr, tempo, rows_per_bar, tail, profile)))
        })
        .prop_map(
            move |(voices, (frames_per_row, tempo, rows_per_bar, tail, profile))| {
                let length = voices.iter().map(line_end).max().unwrap_or(0) + tail + 1;
                let mut row = 0;
                let tempo = tempo
                    .into_iter()
                    .filter_map(|(gap, frames_per_row)| {
                        row += gap;
                        (row < length).then_some(TempoChange {
                            row,
                            frames_per_row,
                        })
                    })
                    .collect();
                Subtune {
                    name: name.clone(),
                    profile,
                    frames_per_row,
                    tempo,
                    rows_per_bar: NonZeroU8::new(rows_per_bar).expect("non-zero"),
                    length: NonZeroU32::new(length).expect("non-zero"),
                    loop_row: 0,
                    marks: (0..length).step_by(usize::from(rows_per_bar)).collect(),
                    voices,
                }
            },
        )
}

/// A song with one to four subtunes with distinct names.
pub fn song() -> impl Strategy<Value = Song> {
    let names = proptest::collection::btree_set(subtune_name(), 1..=4);
    (profile(), names)
        .prop_flat_map(|(profile, names)| {
            let subtunes: Vec<_> = names.into_iter().map(subtune).collect();
            (Just(profile), subtunes)
        })
        .prop_map(|(profile, subtunes)| Song {
            schema: Schema::default(),
            profile,
            subtunes,
        })
}

/// Any proposal.
pub fn proposal() -> impl Strategy<Value = Proposal> {
    prop_oneof![
        (voice(), role()).prop_map(|(voice, role)| Proposal::VoiceRole { voice, role }),
        content_hash().prop_map(|source| Proposal::LeadSource { source }),
        (voice(), -3i8..=3).prop_map(|(voice, shift)| Proposal::Octave { voice, shift }),
        (0u8..12, proptest::collection::vec(0u8..24, 1..5))
            .prop_map(|(root, intervals)| Proposal::Chord { root, intervals }),
        (0u32..1024).prop_map(|row| Proposal::LoopRow { row }),
        frames_per_row().prop_map(|frames_per_row| Proposal::FramesPerRow { frames_per_row }),
        (
            voice(),
            0u32..1024,
            pitch(),
            proptest::option::of(source_ref()),
            1u32..64
        )
            .prop_map(|(voice, row, pitch, source, length)| Proposal::TrimNote {
                voice,
                row,
                pitch,
                source,
                length: NonZeroU32::new(length).expect("non-zero"),
            }),
        (
            voice(),
            0u32..1024,
            pitch(),
            proptest::option::of(source_ref()),
            drop_reason()
        )
            .prop_map(|(voice, row, pitch, source, reason)| Proposal::DropNote {
                voice,
                row,
                pitch,
                source,
                reason,
            }),
        (voice(), 0u32..1024, pitch()).prop_map(|(voice, row, pitch)| Proposal::Retrigger {
            voice,
            row,
            pitch
        }),
    ]
}

/// Any reason to drop a note.
pub fn drop_reason() -> impl Strategy<Value = DropReason> {
    prop_oneof![
        Just(DropReason::Ghost),
        Just(DropReason::TooShort),
        Just(DropReason::Repeat),
        Just(DropReason::Skyline),
        Just(DropReason::Overlap),
    ]
}

/// Any provenance.
pub fn provenance() -> impl Strategy<Value = Provenance> {
    prop_oneof![
        "[a-z]{1,10}@[0-9]".prop_map(|rule| Provenance::Rules { rule }),
        ("[a-z0-9.-]{1,20}", content_hash())
            .prop_map(|(model, prompt)| Provenance::Llm { model, prompt }),
        Just(Provenance::Manual),
    ]
}

/// Any decision.
pub fn decision() -> impl Strategy<Value = Decision> {
    (
        decision_id(),
        subtune_name(),
        0u32..256,
        proposal(),
        provenance(),
        confidence(),
        any::<bool>(),
    )
        .prop_map(
            |(id, subtune, bar, proposal, provenance, confidence, reviewed)| Decision {
                id,
                subtune,
                bar,
                proposal,
                provenance,
                confidence,
                reviewed,
            },
        )
}

/// Any valid voice rule.
pub fn voice_rule() -> impl Strategy<Value = VoiceRule> {
    prop_oneof![
        (role_list(), any::<bool>()).prop_map(|(roles, filter)| VoiceRule::new(
            VoiceUse::Music,
            roles,
            filter
        )
        .expect("valid")),
        role_list()
            .prop_map(|roles| VoiceRule::new(VoiceUse::Expendable, roles, false).expect("valid")),
        Just(VoiceRule::effects()),
    ]
}

/// Zero to two distinct roles (none means any).
fn role_list() -> impl Strategy<Value = Vec<Role>> {
    proptest::collection::btree_set(role_index(), 0..3)
        .prop_map(|set| set.into_iter().map(role_of).collect())
}

fn role_index() -> impl Strategy<Value = u8> {
    0u8..9
}

fn role_of(i: u8) -> Role {
    [
        Role::Lead,
        Role::Bass,
        Role::Arp,
        Role::Harmony,
        Role::Drums,
        Role::Drone,
        Role::Ostinato,
        Role::Bed,
        Role::Texture,
    ][usize::from(i % 9)]
}

/// A valid project profile called `name`: three random voice rules, at least one of them music.
pub fn project_profile(name: ProfileName) -> impl Strategy<Value = Profile> {
    (voice_rule(), voice_rule(), voice_rule()).prop_map(move |(a, b, c)| {
        let voices = if [&a, &b, &c].iter().any(|r| r.is_music()) {
            [a, b, c]
        } else {
            [VoiceRule::music(), b, c]
        };
        Profile::new(name.clone(), voices).expect("has a music voice")
    })
}

/// Unreduced notes on the grid, as transcription hands them to the reduction: any order, overlapping, chords (equal
/// rows), repeats, any attack, any playable pitch, within 256 rows.
pub fn note_soup() -> impl Strategy<Value = Vec<Note>> {
    let note = (0u32..256, 1u32..32, pitch(), any::<bool>(), confidence()).prop_map(
        |(row, length, pitch, tie, confidence)| Note {
            row,
            length: NonZeroU32::new(length).expect("non-zero"),
            pitch,
            attack: if tie { Attack::Legato } else { Attack::Fresh },
            instrument: None,
            source: None,
            confidence,
            decision: None,
        },
    );
    proptest::collection::vec(note, 0..48)
}

/// Random instrument settings, to be fitted to a voice rule.
type Knobs = (bool, bool, u8, bool, bool);

fn knobs() -> impl Strategy<Value = Knobs> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<u8>(),
        any::<bool>(),
        any::<bool>(),
    )
}

/// Facts for an instrument on a voice with `rule`, in a subtune whose slowest-to-fastest tempos are all at least
/// `min_fpr` frames per row: the filter only if allowed, a pulse width at note start on expendable voices, and a gate
/// timer below every row's tempo and at most 63.
fn fitted(
    (filter, pulse, gate, hard_restart, gate_off): Knobs,
    rule: &VoiceRule,
    min_fpr: u8,
) -> InstrumentFacts {
    InstrumentFacts {
        filter: filter && rule.filter(),
        pulse_at_start: pulse || rule.usage() == VoiceUse::Expendable,
        // Below every row's tempo, and within GoatTracker's 0–63.
        gate_timer: gate % min_fpr.min(64),
        hard_restart,
        gate_off,
    }
}

/// A song and a [`Context`] it satisfies: [`crate::validate()`] finds no errors (warnings, which are matters of
/// taste, may remain).
///
/// Built from [`song`], then fitted. Each subtune names a built-in or project profile; lines on effects-only voices
/// are dropped; roles a voice does not allow become one it does; every line and per-note instrument gets facts that fit
/// its voice; and, sometimes, all subtunes form one adaptive set, which their every-bar marks already satisfy.
pub fn checked_song() -> impl Strategy<Value = (Song, Context)> {
    let project = proptest::collection::vec(0u8..1, 0..3).prop_flat_map(|slots| {
        let profiles: Vec<_> = (0..slots.len())
            .map(|i| project_profile(ProfileName::new(format!("project-{i}")).expect("valid")))
            .collect();
        profiles
    });
    (
        song(),
        project,
        proptest::collection::vec(any::<u8>(), 4),
        proptest::collection::vec(knobs(), 24),
        any::<bool>(),
    )
        .prop_map(|(mut song, project, choices, knobs, adaptive)| {
            let profiles = Profiles::with(project).expect("unique project names");
            let names: Vec<ProfileName> = profiles.iter().map(|p| p.name().clone()).collect();
            let mut instruments = std::collections::BTreeMap::new();
            for (i, subtune) in song.subtunes.iter_mut().enumerate() {
                let name = names[usize::from(choices[i % choices.len()]) % names.len()].clone();
                let profile = profiles.get(&name).expect("listed").clone();
                subtune.profile = Some(name);
                let min_fpr = subtune
                    .tempo
                    .iter()
                    .map(|t| t.frames_per_row.get())
                    .chain([subtune.frames_per_row.get()])
                    .min()
                    .expect("at least the starting tempo");
                subtune.voices.retain(|l| profile.voice(l.voice).is_music());
                for line in &mut subtune.voices {
                    let rule = profile.voice(line.voice);
                    let spans = line
                        .roles
                        .spans()
                        .iter()
                        .map(|s| RoleSpan {
                            row: s.row,
                            role: if rule.allows(s.role) {
                                s.role
                            } else {
                                rule.roles()[0]
                            },
                        })
                        .collect();
                    line.roles = Roles::new(spans).expect("rows unchanged");
                    let slot = i * 6 + usize::from(line.voice.get() - 1) * 2;
                    let main = PresetId(format!("s{i}-v{}", line.voice.get()));
                    let alt = PresetId(format!("s{i}-v{}-alt", line.voice.get()));
                    instruments.insert(main.clone(), fitted(knobs[slot % 24], rule, min_fpr));
                    instruments.insert(alt.clone(), fitted(knobs[(slot + 1) % 24], rule, min_fpr));
                    line.instrument = main;
                    for note in &mut line.notes {
                        if note.instrument.is_some() {
                            note.instrument = Some(alt.clone());
                        }
                    }
                }
            }
            let sets = if adaptive {
                let layers = song.subtunes.iter().map(|s| s.name.clone()).collect();
                vec![AdaptiveSet::new("all", layers).expect("unique names")]
            } else {
                vec![]
            };
            let ctx = Context {
                profiles,
                adaptive: sets,
                instruments,
                ..Context::default()
            };
            (song, ctx)
        })
}
