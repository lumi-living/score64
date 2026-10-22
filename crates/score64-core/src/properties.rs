//! Property tests for the validator (#14): songs built to the rules raise no errors, and breaking a rule in its own
//! way always gets that rule reported. Together they keep the generators, the profiles and the validator in step.
//!
//! The reduction invariant (any [`strategies::note_soup`] reduces to lines with no errors) lands with the reduction
//! itself (E6).

use std::num::NonZeroU32;

use proptest::prelude::*;

use crate::profile::VoiceUse;
use crate::score::{Attack, Note, PresetId, Role, Roles, Song, Voice, VoiceLine};
use crate::strategies;
use crate::validate::{Context, Problem, Rule, Severity, validate};

fn errors(song: &Song, ctx: &Context) -> Vec<Problem> {
    validate(song, ctx)
        .into_iter()
        .filter(Problem::is_error)
        .collect()
}

fn reports(song: &Song, ctx: &Context, rule: Rule, severity: Severity) -> bool {
    validate(song, ctx)
        .iter()
        .any(|p| p.rule == rule && p.severity == severity)
}

/// The first (subtune, line) whose line has a note and satisfies `pick`, as indices.
fn find_line(
    song: &Song,
    ctx: &Context,
    pick: impl Fn(&crate::profile::Profile, &VoiceLine) -> bool,
) -> Option<(usize, usize)> {
    song.subtunes.iter().enumerate().find_map(|(i, s)| {
        let profile = ctx.profiles.get(song.profile_of(s))?;
        s.voices
            .iter()
            .position(|l| !l.notes.is_empty() && pick(profile, l))
            .map(|j| (i, j))
    })
}

fn note(row: u32, length: u32) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: "C-4".parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: crate::provenance::Confidence::CERTAIN,
        decision: None,
    }
}

proptest! {
    // Counterexample tests discard songs without what they need (an expendable voice, a declared set, ...), so allow
    // plenty of rejects; PROPTEST_CASES can raise the case count for longer runs.
    #![proptest_config(ProptestConfig { cases: 256, max_global_rejects: 1 << 16, ..ProptestConfig::default() })]

    /// No false positives: a song fitted to its context validates without errors.
    #[test]
    fn checked_songs_have_no_errors((song, ctx) in strategies::checked_song()) {
        let e = errors(&song, &ctx);
        prop_assert!(e.is_empty(), "{e:#?}");
    }

    /// Rule 1: a repeated note overlaps.
    #[test]
    fn overlaps_are_monophony_errors((mut song, ctx) in strategies::checked_song()) {
        let found = find_line(&song, &ctx, |_, _| true);
        prop_assume!(found.is_some());
        let (i, j) = found.unwrap();
        let notes = &mut song.subtunes[i].voices[j].notes;
        let again = notes[0].clone();
        notes.insert(1, again);
        prop_assert!(reports(&song, &ctx, Rule::Monophony, Severity::Error));
    }

    /// Rule 2: a note running past the end of its subtune.
    #[test]
    fn notes_past_the_end_are_grid_errors((mut song, ctx) in strategies::checked_song()) {
        let found = find_line(&song, &ctx, |_, _| true);
        prop_assume!(found.is_some());
        let (i, j) = found.unwrap();
        let length = song.subtunes[i].length.get();
        let line = &mut song.subtunes[i].voices[j];
        let end = line.notes.iter().map(|n| n.row + n.length.get()).max().unwrap();
        line.notes.push(note(end, length - end + 1));
        prop_assert!(reports(&song, &ctx, Rule::Grid, Severity::Error));
    }

    /// Rule 3: music on a voice the profile gives to sound effects.
    #[test]
    fn music_on_an_effects_voice_is_a_voices_error((mut song, ctx) in strategies::checked_song()) {
        let target = song.subtunes.iter().enumerate().find_map(|(i, s)| {
            let profile = ctx.profiles.get(song.profile_of(s))?;
            (1..=3)
                .filter_map(Voice::new)
                .find(|&v| profile.voice(v).usage() == VoiceUse::Effects)
                .map(|v| (i, v))
        });
        prop_assume!(target.is_some());
        let (i, voice) = target.unwrap();
        let instrument = PresetId(format!("s{i}-v1"));
        song.subtunes[i].voices.retain(|l| l.voice != voice);
        song.subtunes[i].voices.push(VoiceLine {
            voice,
            roles: Roles::constant(Role::Lead),
            instrument,
            notes: vec![note(0, 1)],
            arps: vec![],
        });
        prop_assert!(reports(&song, &ctx, Rule::Voices, Severity::Error));
    }

    /// Rule 4: the filter on a voice that may not use it.
    #[test]
    fn a_forbidden_filter_is_an_error((song, mut ctx) in strategies::checked_song()) {
        let found = find_line(&song, &ctx, |p, l| !p.may_filter(l.voice));
        prop_assume!(found.is_some());
        let (i, j) = found.unwrap();
        let line = &song.subtunes[i].voices[j];
        let instrument = line.instrument_of(&line.notes[0]).clone();
        ctx.instruments.get_mut(&instrument).unwrap().filter = true;
        prop_assert!(reports(&song, &ctx, Rule::FilterAndExpendable, Severity::Error));
    }

    /// Rule 4: an expendable voice's note without a pulse width at its start.
    #[test]
    fn expendable_notes_need_a_pulse_width((song, mut ctx) in strategies::checked_song()) {
        let found = find_line(&song, &ctx, |p, l| p.voice(l.voice).usage() == VoiceUse::Expendable);
        prop_assume!(found.is_some());
        let (i, j) = found.unwrap();
        let line = &song.subtunes[i].voices[j];
        let instrument = line.instrument_of(&line.notes[0]).clone();
        ctx.instruments.get_mut(&instrument).unwrap().pulse_at_start = false;
        prop_assert!(reports(&song, &ctx, Rule::FilterAndExpendable, Severity::Error));
    }

    /// Rule 5: a layer of a declared adaptive set without a mark where it may switch.
    #[test]
    fn a_missing_switch_mark_is_an_error((mut song, ctx) in strategies::checked_song()) {
        prop_assume!(!ctx.adaptive.is_empty());
        song.subtunes[0].marks.retain(|&m| m != 0);
        prop_assert!(reports(&song, &ctx, Rule::TimingMarks, Severity::Error));
    }

    /// Rule 6: a note outside its role's range is a warning, never an error.
    #[test]
    fn out_of_range_pitches_are_warnings((mut song, ctx) in strategies::checked_song()) {
        let found = find_line(&song, &ctx, |_, l| {
            !matches!(l.roles.at(l.notes[0].row), Role::Drums | Role::Texture)
        });
        prop_assume!(found.is_some());
        let (i, j) = found.unwrap();
        song.subtunes[i].voices[j].notes[0].pitch = crate::score::Pitch::MIN;
        prop_assert!(reports(&song, &ctx, Rule::PitchRange, Severity::Warning));
        prop_assert!(errors(&song, &ctx).is_empty(), "taste only");
    }

    /// Rule 9: a gate timer not below the tempo.
    #[test]
    fn a_gate_timer_at_the_tempo_is_an_error((song, mut ctx) in strategies::checked_song()) {
        let found = find_line(&song, &ctx, |_, _| true);
        prop_assume!(found.is_some());
        let (i, j) = found.unwrap();
        let subtune = &song.subtunes[i];
        let line = &subtune.voices[j];
        let first = &line.notes[0];
        let tempo = subtune.frames_per_row_at(first.row).get();
        let instrument = line.instrument_of(first).clone();
        ctx.instruments.get_mut(&instrument).unwrap().gate_timer = tempo;
        prop_assert!(reports(&song, &ctx, Rule::HardRestart, Severity::Error));
    }

    /// On any note soup, rule 1 is reported exactly when the soup is not already a clean line: notes in row order,
    /// none starting before the previous one ends, legato only straight after a note.
    #[test]
    fn monophony_is_reported_exactly_when_a_soup_is_not_a_line(soup in strategies::note_soup()) {
        let mut end = 0;
        let mut clean = true;
        for (i, n) in soup.iter().enumerate() {
            if (i > 0 && n.row < end) || (n.attack == Attack::Legato && (i == 0 || n.row != end)) {
                clean = false;
            }
            end = end.max(n.row + n.length.get());
        }
        let mut song = crate::score::Song {
            schema: crate::score::Schema::default(),
            profile: crate::profile::ProfileName::new(crate::profile::builtin::FULL).unwrap(),
            subtunes: vec![],
        };
        song.subtunes.push(crate::score::Subtune {
            name: crate::score::SubtuneName::new("soup").unwrap(),
            profile: None,
            frames_per_row: crate::score::FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: std::num::NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(300).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                roles: Roles::constant(Role::Lead),
                instrument: PresetId("any".into()),
                notes: soup,
                arps: vec![],
            }],
        });
        let monophony = validate(&song, &Context::default()).iter().any(|p| p.rule == Rule::Monophony);
        prop_assert_eq!(monophony, !clean);
    }
}
