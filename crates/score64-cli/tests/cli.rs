//! The `score64` binary end to end: typed subcommands and `run` give the same results, and every registry command
//! runs from the command line (ADR-005 parity).

use std::io::Write as _;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn score64(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_score64"))
        .args(args)
        .output()
        .expect("run score64")
}

fn stdout_json(o: &Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("JSON on stdout")
}

fn error_kind(o: &Output) -> String {
    assert!(!o.status.success());
    let e: Value = serde_json::from_slice(&o.stderr).expect("JSON on stderr");
    e["error"]["kind"].as_str().unwrap().to_owned()
}

#[test]
fn version_typed_and_by_name_agree() {
    let typed = stdout_json(&score64(&["version", "--json"]));
    assert_eq!(typed["score64"], env!("CARGO_PKG_VERSION"));
    assert_eq!(typed, stdout_json(&score64(&["run", "version"])));
    let text = score64(&["version"]);
    assert!(String::from_utf8_lossy(&text.stdout).starts_with("score64 "));
}

#[test]
fn commands_lists_the_registry_with_schemas() {
    let list = stdout_json(&score64(&["commands", "--json"]));
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["version", "helper_info", "validate"]);
    for c in list.as_array().unwrap() {
        assert!(
            ["read_only", "writes_project", "destructive"].contains(&c["effect"].as_str().unwrap()),
            "{c}"
        );
        assert_eq!(c["args"]["type"], "object");
        assert_eq!(c["output"]["type"], "object");
    }
}

#[test]
fn errors_are_json_with_a_kind() {
    assert_eq!(error_kind(&score64(&["run", "nope"])), "unknown_command");
    assert_eq!(
        error_kind(&score64(&["run", "version", r#"{"x": 1}"#])),
        "invalid_args"
    );
    assert_eq!(
        error_kind(&score64(&["run", "version", "not json"])),
        "invalid_args"
    );
}

/// Every registry command runs by name. With `{}`, a command either runs, fails only because the helper binary is
/// missing, or (if its schema requires arguments) answers `invalid_args`; commands with required arguments are run
/// with real ones in their own tests.
#[test]
fn every_command_runs_from_the_cli() {
    let list = stdout_json(&score64(&["commands", "--json"]));
    for c in list.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let needs_args = c["args"]["required"]
            .as_array()
            .is_some_and(|r| !r.is_empty());
        let o = score64(&["run", name]);
        if !o.status.success() {
            let kind = error_kind(&o);
            assert!(
                kind == "helper_missing" || (needs_args && kind == "invalid_args"),
                "{name}: {kind}"
            );
            assert_eq!(o.status.code(), Some(2), "{name}: command failures exit 2");
        }
    }
}

#[test]
fn helper_info_starts_the_helper() {
    let o = score64(&["run", "helper_info"]);
    if !o.status.success() && error_kind(&o) == "helper_missing" {
        // Straight to the process's stderr: libtest captures eprintln! of passing tests, and a skip must show.
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED helper_info_starts_the_helper: score64-emu not built"
        );
        return;
    }
    let info = stdout_json(&o);
    assert!(info["helper"].as_str().unwrap().starts_with("score64-emu "));
    assert_eq!(info["starts"], 1);
}

mod validate {
    use std::num::{NonZeroU8, NonZeroU32};
    use std::path::{Path, PathBuf};

    use score64_core::{
        Attack, Confidence, FramesPerRow, Note, PresetId, ProfileName, Role, Roles, Schema, Song,
        Subtune, SubtuneName, Voice, VoiceLine, builtin,
    };

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "score64-cli-validate-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A lead on V1 under `sfx-shares-v3`, at `pitch`.
    fn song(pitch: &str) -> Song {
        let note = |row, length| Note {
            row,
            length: NonZeroU32::new(length).unwrap(),
            pitch: pitch.parse().unwrap(),
            attack: Attack::Fresh,
            instrument: None,
            source: None,
            confidence: Confidence::CERTAIN,
            decision: None,
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
                length: NonZeroU32::new(16).unwrap(),
                loop_row: 0,
                marks: vec![0],
                voices: vec![VoiceLine {
                    voice: Voice::new(1).unwrap(),
                    roles: Roles::constant(Role::Lead),
                    instrument: PresetId("lead".into()),
                    notes: vec![note(0, 8), note(8, 8)],
                    arps: vec![],
                }],
            }],
        }
    }

    const CONTEXT: &str = "[instruments.lead]\nfilter = true\npulse_at_start = true\ngate_timer = 2\nhard_restart = true\ngate_off = true\n";

    fn files(dir: &Path, pitch: &str) -> (String, String) {
        let s = dir.join(format!("song-{pitch}.json"));
        std::fs::write(&s, serde_json::to_string(&song(pitch)).unwrap()).unwrap();
        let c = dir.join("context.toml");
        std::fs::write(&c, CONTEXT).unwrap();
        (s.display().to_string(), c.display().to_string())
    }

    #[test]
    fn exit_codes_follow_severity() {
        let dir = scratch("codes");
        // Clean: exit 0 and a summary.
        let (s, c) = files(&dir, "A-4");
        let o = score64(&["validate", &s, "--context", &c]);
        assert_eq!(o.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&o.stdout).contains("0 errors, 0 warnings"));
        // Warnings only (a lead below its range): still exit 0, with the warning printed.
        let (s, c) = files(&dir, "C-1");
        let o = score64(&["validate", &s, "--context", &c]);
        assert_eq!(o.status.code(), Some(0), "warnings never fail");
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(
            text.contains("warning, rule 6") && text.contains("0 errors, 2 warnings"),
            "{text}"
        );
        // Errors (no instrument facts without the context): exit 1.
        let o = score64(&["validate", &s]);
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stdout).contains("error, rule 4"));
        // A command failure (missing file): exit 2, JSON error on stderr.
        let o = score64(&["validate", &dir.join("nope.json").display().to_string()]);
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(error_kind(&o), "invalid_input");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_output_matches_run_validate() {
        let dir = scratch("json");
        let (s, c) = files(&dir, "C-1");
        let o = score64(&["validate", &s, "--context", &c, "--json"]);
        assert_eq!(o.status.code(), Some(0));
        let typed: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(
            (typed["errors"].as_u64(), typed["warnings"].as_u64()),
            (Some(0), Some(2))
        );
        let p = &typed["problems"][0];
        for field in [
            "rule", "severity", "subtune", "bar", "row", "voice", "message", "hint",
        ] {
            assert!(p.get(field).is_some(), "problem has {field}: {p}");
        }
        assert_eq!(p["severity"], "warning");
        let by_name = stdout_json(&score64(&[
            "run",
            "validate",
            &json!({ "song": s, "context": c }).to_string(),
        ]));
        assert_eq!(by_name, typed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
