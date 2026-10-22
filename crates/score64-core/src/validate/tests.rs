//! One clean song, then a counterexample for each rule, checking where the problem is reported.

use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use super::*;
use crate::profile::{VoiceRule, builtin};
use crate::provenance::Confidence;
use crate::score::{
    ArpEvent, FramesPerRow, Note, RoleSpan, Roles, Schema, Subtune, SubtuneName, TempoChange,
};

fn note(row: u32, length: u32, pitch: &str) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::CERTAIN,
        decision: None,
    }
}

fn line(voice: u8, role: Role, instrument: &str, notes: Vec<Note>) -> VoiceLine {
    VoiceLine {
        voice: Voice::new(voice).unwrap(),
        roles: Roles::constant(role),
        instrument: PresetId(instrument.into()),
        notes,
        arps: vec![],
    }
}

fn pname(s: &str) -> ProfileName {
    ProfileName::new(s).unwrap()
}

fn sname(s: &str) -> SubtuneName {
    SubtuneName::new(s).unwrap()
}

fn subtune(name: &str, profile: Option<&str>, voices: Vec<VoiceLine>) -> Subtune {
    Subtune {
        name: sname(name),
        profile: profile.map(pname),
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(32).unwrap(),
        loop_row: 0,
        marks: vec![0, 16],
        voices,
    }
}

fn theme_voices() -> Vec<VoiceLine> {
    vec![
        line(
            1,
            Role::Lead,
            "lead",
            vec![note(0, 8, "A-4"), note(8, 8, "C-5"), note(16, 16, "E-5")],
        ),
        line(
            2,
            Role::Bass,
            "bass",
            vec![note(0, 16, "A-1"), note(16, 16, "E-2")],
        ),
        line(
            3,
            Role::Drums,
            "drum",
            vec![note(0, 2, "C-3"), note(4, 1, "C-6"), note(8, 2, "C-3")],
        ),
    ]
}

/// A two-bar subtune under `sfx-shares-v3` that passes: lead on V1, bass on V2, drums on the expendable V3.
fn theme() -> Song {
    Song {
        schema: Schema::default(),
        profile: pname(builtin::SFX_SHARES_V3),
        subtunes: vec![subtune("theme", None, theme_voices())],
    }
}

/// Two project profiles: a quiet layer with a filtered bed on V1 and effects elsewhere, and a building layer that
/// adds a drone or ostinato on V2 (no filter, so the bed's sweeps don't move it).
fn project_profiles() -> Profiles {
    let bed = VoiceRule::new(VoiceUse::Music, vec![Role::Bed], true).unwrap();
    let under = VoiceRule::new(VoiceUse::Music, vec![Role::Drone, Role::Ostinato], false).unwrap();
    Profiles::with([
        Profile::new(
            pname("calm"),
            [bed.clone(), VoiceRule::effects(), VoiceRule::effects()],
        )
        .unwrap(),
        Profile::new(pname("tension"), [bed, under, VoiceRule::effects()]).unwrap(),
    ])
    .unwrap()
}

fn ctx() -> Context {
    let facts = |filter, pulse_at_start| InstrumentFacts {
        filter,
        pulse_at_start,
    };
    Context {
        profiles: project_profiles(),
        adaptive: vec![],
        instruments: [
            ("lead", facts(true, true)),
            ("bass", facts(false, true)),
            ("drum", facts(false, true)),
            ("bed", facts(true, false)),
            ("pad", facts(false, false)),
            ("filtered-drum", facts(true, true)),
        ]
        .into_iter()
        .map(|(k, v)| (PresetId(k.into()), v))
        .collect(),
        ranges: RoleRanges::default(),
    }
}

fn problems_in(song: &Song, ctx: &Context) -> Vec<(Rule, Option<u32>, Option<u8>)> {
    validate(song, ctx)
        .into_iter()
        .map(|p| (p.rule, p.bar, p.voice.map(Voice::get)))
        .collect()
}

fn problems(song: &Song) -> Vec<(Rule, Option<u32>, Option<u8>)> {
    problems_in(song, &ctx())
}

fn lines(song: &mut Song) -> &mut Vec<VoiceLine> {
    &mut song.subtunes[0].voices
}

#[test]
fn a_clean_song_passes() {
    assert_eq!(validate(&theme(), &ctx()), []);
}

#[test]
fn rule_1_monophony() {
    let mut s = theme();
    lines(&mut s)[0].notes[1].row = 6; // overlaps the first note (rows 0–7)
    assert_eq!(problems(&s), [(Rule::Monophony, Some(0), Some(1))]);

    let mut s = theme();
    lines(&mut s)[1].notes[1].attack = Attack::Legato; // follows straight on: fine
    lines(&mut s)[2].notes[1].attack = Attack::Legato; // a gap before it: not legato
    assert_eq!(problems(&s), [(Rule::Monophony, Some(0), Some(3))]);

    let mut s = theme();
    let extra = lines(&mut s)[1].clone();
    lines(&mut s).push(extra);
    assert_eq!(problems(&s), [(Rule::Monophony, None, Some(2))]);
}

#[test]
fn rule_2_grid() {
    let mut s = theme();
    lines(&mut s)[0].notes[2] = note(16, 20, "E-5"); // to row 36 of 32
    s.subtunes[0].loop_row = 40;
    s.subtunes[0].tempo = vec![TempoChange {
        row: 32,
        frames_per_row: FramesPerRow::new(5).unwrap(),
    }];
    lines(&mut s)[2].arps.push(ArpEvent {
        row: 30,
        length: NonZeroU32::new(4).unwrap(),
        intervals: vec![0, 4, 7],
        decision: None,
    });
    let rules: Vec<_> = problems(&s).into_iter().map(|p| p.0).collect();
    assert_eq!(rules, [Rule::Grid; 4]);
}

#[test]
fn rule_3_voices_roles_and_profile_names() {
    let mut s = theme();
    lines(&mut s)[2].roles = Roles::new(vec![
        RoleSpan {
            row: 0,
            role: Role::Drums,
        },
        RoleSpan {
            row: 16,
            role: Role::Bass,
        }, // bass on the expendable voice from bar 2
    ])
    .unwrap();
    let p = problems(&s);
    assert!(p.contains(&(Rule::Voices, Some(1), Some(3))), "{p:?}");

    let mut s = theme();
    s.subtunes[0].profile = Some(pname(builtin::SFX_OWNS_V3)); // V3 belongs to effects
    assert_eq!(problems(&s), [(Rule::Voices, Some(0), Some(3))]);

    let mut s = theme();
    s.subtunes[0].profile = Some(pname("loud"));
    let p = validate(&s, &ctx());
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(
        p[0].message.contains("no profile is called `loud`"),
        "{}",
        p[0].message
    );
}

#[test]
fn rule_4_filter_and_pulse() {
    let mut s = theme();
    lines(&mut s)[2].notes[2].instrument = Some(PresetId("filtered-drum".into()));
    lines(&mut s)[2].notes[0].instrument = Some(PresetId("pad".into())); // no pulse width at start
    let p = validate(&s, &ctx());
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(
        p.iter()
            .all(|p| p.rule == Rule::FilterAndPulse && p.voice.map(Voice::get) == Some(3))
    );
    assert!(p.iter().any(|p| p.message.contains("filter")));
    assert!(p.iter().any(|p| p.message.contains("pulse width")));

    let mut s = theme();
    lines(&mut s)[0].instrument = PresetId("unknown".into());
    assert_eq!(problems(&s).len(), 3, "one per note without facts");
}

/// An adaptive set the project declares: `calm`, `tension`, `peak`, after a title tune.
fn layered() -> (Song, Context) {
    let bed = || line(1, Role::Bed, "bed", vec![note(0, 32, "C-3")]);
    let song = Song {
        schema: Schema::default(),
        profile: pname(builtin::FULL),
        subtunes: vec![
            subtune("title", None, theme_voices()),
            subtune("calm", Some("calm"), vec![bed()]),
            subtune(
                "tension",
                Some("tension"),
                vec![bed(), line(2, Role::Drone, "pad", vec![note(0, 32, "C-2")])],
            ),
            subtune("peak", Some(builtin::SFX_SHARES_V3), theme_voices()),
        ],
    };
    let mut ctx = ctx();
    ctx.adaptive = vec![
        AdaptiveSet::new(
            "score",
            vec![sname("calm"), sname("tension"), sname("peak")],
        )
        .unwrap(),
    ];
    (song, ctx)
}

#[test]
fn project_profiles_keep_the_filter_and_roles_where_declared() {
    let (song, ctx) = layered();
    assert_eq!(validate(&song, &ctx), []);

    let mut s = song.clone();
    s.subtunes[2].voices[1].instrument = PresetId("bed".into()); // a filtered drone under a filtered bed
    let p = validate(&s, &ctx);
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(
        (p[0].rule, p[0].voice.map(Voice::get)),
        (Rule::FilterAndPulse, Some(2))
    );
    assert!(
        p[0].message.contains("`tension` does not allow"),
        "{}",
        p[0].message
    );

    let mut s = song;
    s.subtunes[2].voices[1].roles = Roles::constant(Role::Lead); // a lead sneaking into the building layer
    // Both the role and, for a lead, the low C-2 are wrong here.
    assert_eq!(
        problems_in(&s, &ctx),
        [
            (Rule::Voices, Some(0), Some(2)),
            (Rule::PitchRange, Some(0), Some(2))
        ]
    );
}

#[test]
fn rule_5_declared_adaptive_sets() {
    let (mut s, ctx) = layered();
    s.subtunes[2].marks = vec![0]; // bar 2 of `tension` unmarked
    let p = validate(&s, &ctx);
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(
        (
            p[0].rule,
            p[0].bar,
            p[0].row,
            p[0].subtune.as_ref().unwrap().as_str()
        ),
        (Rule::TimingMarks, Some(1), Some(16), "tension")
    );

    let (mut s, ctx) = layered();
    s.subtunes.swap(2, 3);
    let p = validate(&s, &ctx);
    assert!(
        p.iter().filter(|p| p.rule == Rule::TimingMarks).count() == 2,
        "{p:?}"
    );
    assert!(
        p[0].message.contains("adaptive set `score` puts it at"),
        "{}",
        p[0].message
    );

    let (mut s, ctx) = layered();
    s.subtunes.pop();
    let p = validate(&s, &ctx);
    assert!(
        p.iter()
            .any(|p| p.subtune.is_none() && p.message.contains("`peak`")),
        "{p:?}"
    );

    // Outside declared sets, marks are optional.
    let mut s = theme();
    s.subtunes[0].marks.clear();
    assert_eq!(validate(&s, &self::ctx()), []);
}

#[test]
fn rule_6_pitch_ranges_with_fold_hints() {
    let mut s = theme();
    lines(&mut s)[1].notes[0].pitch = "A-4".parse().unwrap(); // bass two octaves too high
    let p = validate(&s, &ctx());
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(
        (p[0].rule, p[0].voice.map(Voice::get)),
        (Rule::PitchRange, Some(2))
    );
    assert_eq!(p[0].hint, "fold down 2 octaves to A-2");

    let mut s = theme();
    lines(&mut s)[0].notes[0].pitch = "C-1".parse().unwrap();
    assert_eq!(validate(&s, &ctx())[0].hint, "fold up 2 octaves to C-3");

    // An unknown profile skips the profile checks but still checks pitch.
    let mut s = theme();
    s.subtunes[0].profile = Some(pname("loud"));
    lines(&mut s)[1].notes[0].pitch = "A-4".parse().unwrap();
    assert!(
        validate(&s, &ctx())
            .iter()
            .any(|p| p.rule == Rule::PitchRange)
    );
}

#[test]
fn rule_8_pack_budget() {
    let budget = PackBudget {
        max_bytes: 4096,
        zero_page: 0xF8,
    };
    assert_eq!(
        check_pack(
            PackFacts {
                bytes: 4000,
                zero_page: 0xF8
            },
            budget
        ),
        []
    );
    let p = check_pack(
        PackFacts {
            bytes: 5000,
            zero_page: 0xFB,
        },
        budget,
    );
    assert_eq!(p.len(), 2);
    assert!(
        p[1].message.contains("$FB–$FC") && p[1].message.contains("$F8–$F9"),
        "{}",
        p[1].message
    );
}

#[test]
fn problems_read_well_and_serialise() {
    let mut s = theme();
    lines(&mut s)[1].notes[0].pitch = "A-4".parse().unwrap();
    let p = &validate(&s, &ctx())[0];
    assert_eq!(
        p.to_string(),
        "rule 6, theme, bar 1, V2: A-4 is outside the Bass range C-1 to G-3 (fold down 2 octaves to A-2)"
    );
    let json = serde_json::to_value(p).unwrap();
    assert_eq!(json["rule"], "pitch_range");
    assert_eq!(json["bar"], 0);
}

proptest! {
    /// The validator never panics, and reports problems in subtune, bar, voice order.
    #[test]
    fn any_song_validates_without_panicking(song in crate::strategies::song()) {
        let p = validate(&song, &ctx());
        let index = |p: &Problem| p.subtune.as_ref().and_then(|n| song.subtune(n)).map(|(i, _)| i);
        for w in p.windows(2) {
            prop_assert!((index(&w[0]), w[0].bar, w[0].voice) <= (index(&w[1]), w[1].bar, w[1].voice));
        }
    }
}
