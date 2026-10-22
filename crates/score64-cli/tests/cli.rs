//! The `score64` binary end to end: typed subcommands and `run` give the same results, and every registry command
//! runs from the command line (ADR-005 parity).

use std::io::Write as _;
use std::process::{Command, Output};

use serde_json::Value;

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
    assert_eq!(names, ["version", "helper_info"]);
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

/// Every registry command runs by name with `{}` (or fails only because the helper binary is missing).
#[test]
fn every_command_runs_from_the_cli() {
    let list = stdout_json(&score64(&["commands", "--json"]));
    for c in list.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let o = score64(&["run", name]);
        if !o.status.success() {
            assert_eq!(error_kind(&o), "helper_missing", "{name}");
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
