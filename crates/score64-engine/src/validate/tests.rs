//! The command on real files: clean, warnings only, errors, project context, and bad input.

use std::num::{NonZeroU8, NonZeroU32};

use score64_core::{
    Attack, Confidence, FramesPerRow, Note, ProfileName, Role, Roles, Schema, Subtune, SubtuneName,
    Voice, VoiceLine, builtin,
};

use super::*;
use crate::{EngineConfig, ErrorKind};

/// A fresh scratch directory for one test.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("score64-validate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

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

/// Lead on V1 and bass on V2 under `sfx-shares-v3`, two bars.
pub(crate) fn song(bass_pitch: &str) -> Song {
    let line = |v, role, instrument: &str, notes| VoiceLine {
        voice: Voice::new(v).unwrap(),
        roles: Roles::constant(role),
        instrument: PresetId(instrument.into()),
        notes,
        arps: vec![],
    };
    Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::SFX_SHARES_V3).unwrap(),
        subtunes: vec![Subtune {
            name: SubtuneName::new("main").unwrap(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(32).unwrap(),
            loop_row: 0,
            marks: vec![0, 16],
            voices: vec![
                line(
                    1,
                    Role::Lead,
                    "lead",
                    vec![note(0, 16, "A-4"), note(16, 16, "C-5")],
                ),
                line(2, Role::Bass, "bass", vec![note(0, 32, bass_pitch)]),
            ],
        }],
    }
}

pub(crate) const CONTEXT: &str = r#"
[instruments.lead]
filter = true
pulse_at_start = true
gate_timer = 2
hard_restart = true
gate_off = true

[instruments.bass]
filter = false
pulse_at_start = true
gate_timer = 2
hard_restart = true
gate_off = true
"#;

fn run(args: ValidateArgs) -> Result<ValidateOutput, CommandError> {
    Engine::new(EngineConfig::default()).run::<Validate>(args)
}

fn write_song(dir: &Path, name: &str, song: &Song) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, serde_json::to_string_pretty(song).unwrap()).unwrap();
    p
}

#[test]
fn clean_warnings_and_errors() {
    let dir = scratch("kinds");
    let ctx = dir.join("context.toml");
    std::fs::write(&ctx, CONTEXT).unwrap();

    let clean = write_song(&dir, "clean.json", &song("A-1"));
    let out = run(ValidateArgs {
        song: clean,
        context: Some(ctx.clone()),
    })
    .unwrap();
    assert_eq!((out.errors, out.warnings, out.problems.len()), (0, 0, 0));

    // A bass two octaves too high is a matter of taste: a warning, no error.
    let high = write_song(&dir, "high.json", &song("A-4"));
    let out = run(ValidateArgs {
        song: high,
        context: Some(ctx.clone()),
    })
    .unwrap();
    assert_eq!((out.errors, out.warnings), (0, 1));
    assert_eq!(out.problems[0].severity, Severity::Warning);

    // Without the context no instrument has facts: an error per note.
    let clean = dir.join("clean.json");
    let out = run(ValidateArgs {
        song: clean,
        context: None,
    })
    .unwrap();
    assert_eq!(out.errors, 3);
    assert!(
        out.problems
            .iter()
            .all(|p| p.message.contains("no facts for instrument"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn songs_and_contexts_read_from_json_or_toml() {
    let dir = scratch("formats");
    let toml_song = dir.join("song.toml");
    std::fs::write(&toml_song, toml::to_string(&song("A-1")).unwrap()).unwrap();
    let json_ctx = dir.join("context.json");
    let file: ContextFile = toml::from_str(CONTEXT).unwrap();
    std::fs::write(&json_ctx, serde_json::to_string(&file).unwrap()).unwrap();
    let out = run(ValidateArgs {
        song: toml_song,
        context: Some(json_ctx),
    })
    .unwrap();
    assert_eq!(out.errors, 0);

    // A project profile named in the song comes from the context file.
    let mut s = song("A-1");
    s.subtunes[0].profile = Some(ProfileName::new("duo").unwrap());
    let named = write_song(&dir, "named.json", &s);
    let ctx = dir.join("duo.toml");
    std::fs::write(
        &ctx,
        format!(
            "{CONTEXT}\n[[profile]]\nname = \"duo\"\nv1 = {{ usage = \"music\", roles = [\"lead\"], filter = true }}\nv2 = {{ usage = \"music\", roles = [\"bass\"] }}\nv3 = {{ usage = \"effects\" }}\n"
        ),
    )
    .unwrap();
    assert_eq!(
        run(ValidateArgs {
            song: named.clone(),
            context: Some(ctx)
        })
        .unwrap()
        .errors,
        0
    );
    let unknown = run(ValidateArgs {
        song: named,
        context: None,
    })
    .unwrap();
    assert!(
        unknown
            .problems
            .iter()
            .any(|p| p.message.contains("no profile is called `duo`"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_files_are_invalid_input() {
    let dir = scratch("bad");
    let missing = run(ValidateArgs {
        song: dir.join("nope.json"),
        context: None,
    })
    .unwrap_err();
    assert_eq!(missing.kind, ErrorKind::InvalidInput);

    let wrong_ext = dir.join("song.txt");
    std::fs::write(&wrong_ext, "{}").unwrap();
    assert!(
        run(ValidateArgs {
            song: wrong_ext,
            context: None
        })
        .unwrap_err()
        .message
        .contains(".json or .toml")
    );

    let typo = dir.join("typo.json");
    let mut v = serde_json::to_value(song("A-1")).unwrap();
    v["subtunes"][0]["colour"] = "blue".into();
    std::fs::write(&typo, v.to_string()).unwrap();
    let e = run(ValidateArgs {
        song: typo,
        context: None,
    })
    .unwrap_err();
    assert!(e.message.contains("colour"), "{}", e.message);

    let song = write_song(&dir, "ok.json", &song("A-1"));
    let clash = dir.join("clash.toml");
    std::fs::write(&clash, "[[profile]]\nname = \"full\"\nv1 = { usage = \"music\" }\nv2 = { usage = \"music\" }\nv3 = { usage = \"music\" }\n").unwrap();
    let e = run(ValidateArgs {
        song,
        context: Some(clash),
    })
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(e.message.contains("built-in"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn whole_song_problems_have_no_location_and_schemas_are_checked() {
    let dir = scratch("edges");
    let song = write_song(&dir, "SONG.JSON", &song("A-1"));
    // An upper-case extension reads.
    let ctx = dir.join("context.toml");
    // An adaptive set naming a layer the song lacks: a whole-song problem.
    std::fs::write(
        &ctx,
        format!("{CONTEXT}\n[[adaptive]]\nname = \"score\"\nlayers = [\"calm\"]\n"),
    )
    .unwrap();
    let out = run(ValidateArgs {
        song: song.clone(),
        context: Some(ctx),
    })
    .unwrap();
    assert_eq!(out.errors, 1);
    assert!(Validate::fails(&out));
    let p = serde_json::to_value(&out.problems[0]).unwrap();
    for absent in ["subtune", "bar", "row", "voice"] {
        assert!(p.get(absent).is_none(), "{absent} absent: {p}");
    }
    for present in ["rule", "severity", "message", "hint"] {
        assert!(p.get(present).is_some(), "{present} present: {p}");
    }
    // A context file from a newer Score64 is refused, not read in part.
    let newer = dir.join("newer.toml");
    std::fs::write(&newer, "schema = 2\n").unwrap();
    let e = run(ValidateArgs {
        song,
        context: Some(newer),
    })
    .unwrap_err();
    assert!(
        e.message.contains("schema 2 is not supported"),
        "{}",
        e.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}
