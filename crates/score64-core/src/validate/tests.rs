//! One clean song, then a counterexample for each rule, checking where the problem is reported.

use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use super::*;
use crate::adaptive::Switch;
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
        gate_timer: 2,
        hard_restart: true,
        gate_off: true,
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
        ..Context::default()
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
    let p = validate(&s, &ctx());
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(
        (
            p[0].rule,
            p[0].severity,
            p[0].bar,
            p[0].voice.map(Voice::get)
        ),
        (
            Rule::FilterAndExpendable,
            Severity::Warning,
            Some(1),
            Some(3)
        ),
        "bass on an expendable voice is allowed, with a warning"
    );
    assert!(!has_errors(&p));

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
            .all(|p| p.rule == Rule::FilterAndExpendable && p.voice.map(Voice::get) == Some(3))
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
        (Rule::FilterAndExpendable, Some(2))
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
fn rule_5_marks_off_switch_points_are_warnings() {
    // `tension` switching every two bars: its bar-1 mark (row 16) does nothing.
    let (song, mut ctx) = layered();
    ctx.adaptive = vec![
        AdaptiveSet::with_switch(
            "score",
            vec![sname("calm"), sname("tension"), sname("peak")],
            Switch::EveryBars(NonZeroU32::new(2).unwrap()),
        )
        .unwrap(),
    ];
    let p = validate(&song, &ctx);
    assert_eq!(p.len(), 3, "one per layer: {p:?}");
    assert!(p.iter().all(|p| p.rule == Rule::TimingMarks
        && p.severity == Severity::Warning
        && p.row == Some(16)
        && p.bar == Some(1)));
    assert!(p[0].message.contains("doesn't switch"), "{}", p[0].message);
    // Not a problem outside the set, and gone once the marks are only at switch points.
    assert!(
        !p.iter()
            .any(|p| p.subtune.as_ref().map(|s| s.as_str()) == Some("title"))
    );
    let mut s = song;
    for t in &mut s.subtunes[1..] {
        t.marks = vec![0];
    }
    assert_eq!(validate(&s, &ctx), []);
}

#[test]
fn rule_5_a_layer_shared_by_sets_must_switch_alike() {
    let (song, mut ctx) = layered();
    let every = |n| Switch::EveryBars(NonZeroU32::new(n).unwrap());
    let set = |name: &str, layers: &[&str], switch| {
        AdaptiveSet::with_switch(name, layers.iter().map(|l| sname(l)).collect(), switch).unwrap()
    };
    // Two sets sharing `calm`, both switching every bar: fine.
    ctx.adaptive = vec![
        set("explore", &["calm", "tension"], Switch::EveryBar),
        set("fight", &["calm"], Switch::EveryBar),
    ];
    assert_eq!(validate(&song, &ctx), []);
    // One switches every two bars: `calm` would need two sets of timing-mark values.
    ctx.adaptive[1] = set("fight", &["calm"], every(2));
    let conflicts: Vec<_> = validate(&song, &ctx)
        .into_iter()
        .filter(|p| p.severity == Severity::Error)
        .collect();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(
        conflicts[0].subtune.as_ref().map(|s| s.as_str()),
        Some("calm")
    );
    assert!(
        conflicts[0]
            .message
            .contains("adaptive sets `explore`, `fight`"),
        "{}",
        conflicts[0].message
    );
    // The off-switch mark at row 16 is not also warned about: `explore` switches there, and the error covers it.
    let p = validate(&song, &ctx);
    assert!(!p.iter().any(|p| p.severity == Severity::Warning), "{p:?}");
    // Rows that differ only in how they're declared are the same switch points.
    ctx.adaptive[1] = set("fight", &["calm"], Switch::Rows(vec![0, 16]));
    assert_eq!(validate(&song, &ctx), []);
    // The same number of switch rows, at different rows, still conflicts.
    ctx.adaptive[1] = set("fight", &["calm"], Switch::Rows(vec![0, 8]));
    let errors = |c: &Context| -> Vec<String> {
        validate(&song, c)
            .into_iter()
            .filter(|p| p.message.contains("switch at different rows"))
            .map(|p| p.subtune.unwrap().as_str().to_string())
            .collect()
    };
    assert_eq!(errors(&ctx), ["calm"]);
    // Two shared layers: an error for each.
    ctx.adaptive[1] = set("fight", &["calm", "tension"], every(2));
    assert_eq!(errors(&ctx), ["calm", "tension"]);
    // Three sets that differ: one error for the layer, naming all three.
    ctx.adaptive = vec![
        set("a", &["calm"], Switch::EveryBar),
        set("b", &["calm"], every(2)),
        set("c", &["calm"], Switch::Rows(vec![0, 8])),
    ];
    let p = validate(&song, &ctx);
    let e: Vec<_> = p
        .iter()
        .filter(|p| p.message.contains("switch at different rows"))
        .collect();
    assert_eq!(e.len(), 1, "{p:?}");
    assert!(e[0].message.contains("`a`, `b`, `c`"), "{}", e[0].message);
    // Only the third differs: still found.
    ctx.adaptive[1] = set("b", &["calm"], Switch::EveryBar);
    assert_eq!(errors(&ctx), ["calm"]);
    // A set without the layer switching at a stray mark's row doesn't excuse it.
    ctx.adaptive = vec![
        set("explore", &["calm"], every(2)),
        set("other", &["tension"], Switch::EveryBar),
    ];
    let p = validate(&song, &ctx);
    assert!(
        p.iter().any(|p| p.severity == Severity::Warning
            && p.subtune.as_ref().map(|s| s.as_str()) == Some("calm")
            && p.row == Some(16)),
        "{p:?}"
    );
}

#[test]
fn rule_5_reports_each_stray_mark_once() {
    let (mut song, mut ctx) = layered();
    let every_two = Switch::EveryBars(NonZeroU32::new(2).unwrap());
    // Two sets that switch alike share `calm`, whose bar-1 mark is written twice: one warning, not four.
    ctx.adaptive = vec![
        AdaptiveSet::with_switch("explore", vec![sname("calm")], every_two.clone()).unwrap(),
        AdaptiveSet::with_switch("rest", vec![sname("calm")], every_two).unwrap(),
    ];
    song.subtunes[1].marks = vec![0, 16, 16];
    let marks: Vec<_> = validate(&song, &ctx)
        .into_iter()
        .filter(|p| p.rule == Rule::TimingMarks)
        .collect();
    assert_eq!(marks.len(), 1, "{marks:?}");
    assert_eq!(
        (marks[0].severity, marks[0].row),
        (Severity::Warning, Some(16))
    );
}

#[test]
fn rule_5_leaves_marks_past_the_end_to_rule_2() {
    let (mut song, ctx) = layered();
    song.subtunes[1].marks = vec![0, 16, 40]; // `calm` is 32 rows
    let p = validate(&song, &ctx);
    assert!(p.iter().any(|p| p.rule == Rule::Grid), "{p:?}");
    assert!(!p.iter().any(|p| p.rule == Rule::TimingMarks), "{p:?}");
}

#[test]
fn rule_6_arpeggios_stay_at_or_below_b7() {
    // An arpeggio over the theme's first lead note: fine low, an error when its top note passes B-7.
    let arp = |intervals: Vec<u8>| crate::score::ArpEvent {
        row: 0,
        length: NonZeroU32::new(2).unwrap(),
        intervals,
        decision: None,
    };
    let mut s = theme();
    lines(&mut s)[0].arps = vec![arp(vec![0, 4, 7])];
    assert!(
        validate(&s, &ctx())
            .iter()
            .all(|p| p.severity != Severity::Error)
    );
    let pitch = lines(&mut s)[0].notes[0].pitch.semitone();
    lines(&mut s)[0].arps = vec![arp(vec![0, 95 - pitch + 1])];
    let p = validate(&s, &ctx());
    assert!(
        p.iter().any(|p| p.rule == Rule::PitchRange
            && p.severity == Severity::Error
            && p.message.contains("passes B-7")),
        "{p:?}"
    );
    lines(&mut s)[0].arps = vec![arp(vec![0, 95 - pitch])];
    assert!(
        validate(&s, &ctx())
            .iter()
            .all(|p| p.severity != Severity::Error),
        "B-7 itself is fine"
    );
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
fn severities_split_taste_from_errors() {
    // Taste: pitch ranges, lead on an expendable voice, a long expendable note. All warnings.
    let mut s = theme();
    lines(&mut s)[1].notes[0].pitch = "A-4".parse().unwrap();
    lines(&mut s)[2].roles = Roles::constant(Role::Lead);
    lines(&mut s)[2].notes[2] = note(8, 8, "C-4");
    let p = validate(&s, &ctx());
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(p.iter().all(|p| p.severity == Severity::Warning), "{p:?}");
    assert!(
        p.iter()
            .any(|p| p.message.contains("8-row note on expendable voice 3")),
        "{p:?}"
    );
    assert!(
        p.iter()
            .any(|p| p.message.contains("Lead on expendable voice 3")),
        "{p:?}"
    );
    assert!(!has_errors(&p));

    // The threshold is the project's.
    let lenient = Context {
        expendable_long_rows: 8,
        ..ctx()
    };
    assert_eq!(validate(&s, &lenient).len(), 2);

    // Mechanics: an overlap is an error.
    lines(&mut s)[0].notes[1].row = 6;
    assert!(has_errors(&validate(&s, &ctx())));
}

#[test]
fn rule_9_hard_restart() {
    // A line of quick notes at 3 frames per row, on instruments whose gate timers the cases set.
    let quick = |gate_timer: u8, hard_restart, gate_off| InstrumentFacts {
        filter: false,
        pulse_at_start: true,
        gate_timer,
        hard_restart,
        gate_off,
    };
    let song = |notes: Vec<Note>| {
        let mut s = theme();
        s.subtunes[0].frames_per_row = FramesPerRow::new(3).unwrap();
        s.subtunes[0].voices = vec![line(1, Role::Lead, "quick", notes)];
        s
    };
    let ctx_with = |facts: Vec<(&str, InstrumentFacts)>| {
        let mut c = ctx();
        c.instruments
            .extend(facts.into_iter().map(|(k, v)| (PresetId(k.into()), v)));
        c
    };
    let hr = |s: &Song, c: &Context| -> Vec<(Severity, Option<u32>)> {
        validate(s, c)
            .into_iter()
            .filter(|p| p.rule == Rule::HardRestart)
            .map(|p| (p.severity, p.row))
            .collect()
    };
    let one_row = || vec![note(0, 1, "C-4"), note(1, 1, "D-4"), note(2, 2, "E-4")];

    // Gate timer 1 at 3 frames per row: two gated frames are left before each gate-off. Legal, probably clipped.
    let c = ctx_with(vec![("quick", quick(1, true, true))]);
    assert_eq!(
        hr(&song(one_row()), &c),
        [(Severity::Warning, Some(1)), (Severity::Warning, Some(2))]
    );
    // Gate timer 0 leaves every frame gated.
    let c = ctx_with(vec![("quick", quick(0, true, true))]);
    assert_eq!(hr(&song(one_row()), &c), []);
    // Without hard restart ($80) the gate still goes off early: the same warnings.
    let c = ctx_with(vec![("quick", quick(1, false, true))]);
    assert_eq!(hr(&song(one_row()).clone(), &c).len(), 2);
    // A legato instrument ($40) does no gate-off before its notes, so fresh notes straight after another won't
    // retrigger: the score and the instrument disagree.
    let c = ctx_with(vec![("quick", quick(1, true, false))]);
    let p: Vec<Problem> = validate(&song(one_row()), &c)
        .into_iter()
        .filter(|p| p.rule == Rule::HardRestart)
        .collect();
    assert_eq!(
        p.iter().map(|p| (p.severity, p.row)).collect::<Vec<_>>(),
        [(Severity::Warning, Some(1)), (Severity::Warning, Some(2))]
    );
    assert!(p[0].message.contains("won't retrigger"), "{}", p[0].message);
    // Marked legato, they agree; after silence the gate is already off, so the note does attack.
    let mut tied = one_row();
    tied[1].attack = Attack::Legato;
    tied[2].attack = Attack::Legato;
    assert_eq!(hr(&song(tied), &c), []);
    assert_eq!(
        hr(&song(vec![note(0, 1, "C-4"), note(2, 1, "D-4")]), &c),
        []
    );

    // Gate timer not below the tempo: GoatTracker stops playback. Here the cut error follows from it.
    let c = ctx_with(vec![("quick", quick(3, true, true))]);
    let p = hr(&song(one_row()), &c);
    assert!(p.iter().all(|(s, _)| *s == Severity::Error), "{p:?}");
    assert!(
        p.contains(&(Severity::Error, Some(0))),
        "tempo check at the first row: {p:?}"
    );
    let msgs: Vec<String> = validate(&song(one_row()), &c)
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert!(
        msgs.iter().any(|m| m.contains("stops playback")),
        "{msgs:?}"
    );
    assert!(msgs.iter().any(|m| m.contains("never sounds")), "{msgs:?}");

    // A tempo change mid-note is checked on the row it takes effect.
    let c = ctx_with(vec![("quick", quick(4, true, true))]);
    let mut s = song(vec![note(0, 8, "C-4")]);
    s.subtunes[0].frames_per_row = FramesPerRow::new(6).unwrap();
    s.subtunes[0].tempo = vec![TempoChange {
        row: 4,
        frames_per_row: FramesPerRow::new(4).unwrap(),
    }];
    assert_eq!(hr(&s, &c), [(Severity::Error, Some(4))]);

    // Skipped: legato notes, the first note, and a note after silence.
    let c = ctx_with(vec![("quick", quick(1, true, true))]);
    let mut tied = one_row();
    tied[1].attack = Attack::Legato;
    tied[2].attack = Attack::Legato;
    assert_eq!(hr(&song(tied), &c), []);
    assert_eq!(
        hr(&song(vec![note(0, 1, "C-4"), note(2, 1, "D-4")]), &c),
        []
    );

    // After a tie, the gate has been on since the tied run's attack: 2 rows, 6 frames, minus 1 is plenty.
    let mut run = vec![note(0, 1, "C-4"), note(1, 1, "D-4"), note(2, 1, "E-4")];
    run[1].attack = Attack::Legato;
    assert_eq!(hr(&song(run), &c), []);

    // The lead time is the gate timer of the instrument playing before the new note (GoatTracker loads it at each
    // row from the channel's instrument); whether the gate goes off is the incoming instrument's.
    let c = ctx_with(vec![
        ("long", quick(1, true, true)),
        ("short", quick(0, true, true)),
    ]);
    let mut pair = vec![note(0, 1, "C-4"), note(1, 1, "D-4")];
    pair[0].instrument = Some(PresetId("long".into()));
    pair[1].instrument = Some(PresetId("short".into()));
    assert_eq!(hr(&song(pair.clone()), &c), [(Severity::Warning, Some(1))]);
    pair.swap(0, 1);
    pair[0].row = 0;
    pair[1].row = 1;
    assert_eq!(
        hr(&song(pair), &c),
        [],
        "gate timer 0 before, so nothing is cut"
    );

    // Out of GoatTracker's range.
    let c = ctx_with(vec![("quick", quick(64, true, true))]);
    assert!(
        validate(&song(one_row()), &c)
            .iter()
            .any(|p| p.message.contains("allows 0 to 63"))
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
        "warning, rule 6, theme, bar 1, V2: A-4 is outside the Bass range C-1 to G-3 (fold down 2 octaves to A-2)"
    );
    let json = serde_json::to_value(p).unwrap();
    assert_eq!(json["rule"], "pitch_range");
    assert_eq!(json["severity"], "warning");
    assert_eq!(json["bar"], 0);
}

proptest! {
    /// The generators keep their promise of structurally sound songs: the validator finds no monophony or grid
    /// problems in them. It never panics, and reports problems in subtune, bar, voice order.
    #[test]
    fn generated_songs_are_structurally_sound(song in crate::strategies::song()) {
        let p = validate(&song, &ctx());
        let structural: Vec<_> = p.iter().filter(|p| matches!(p.rule, Rule::Monophony | Rule::Grid)).collect();
        prop_assert!(structural.is_empty(), "{structural:?}");
        let index = |p: &Problem| p.subtune.as_ref().and_then(|n| song.subtune(n)).map(|(i, _)| i);
        for w in p.windows(2) {
            prop_assert!((index(&w[0]), w[0].bar, w[0].voice) <= (index(&w[1]), w[1].bar, w[1].voice));
        }
    }
}
