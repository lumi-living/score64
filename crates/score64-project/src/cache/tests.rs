//! The stage cache: keys, whole entries, races and damage.

use std::sync::Arc;

use serde_json::json;

use super::*;

/// A fresh cache directory for one test.
fn cache(name: &str) -> (Cache, PathBuf) {
    let dir = std::env::temp_dir().join(format!("score64-cache-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (Cache::new(dir.join("cache")), dir)
}

fn run() -> Run {
    Run {
        stage_version: "1".into(),
        tool_version: "0.0.0".into(),
        config: json!({ "model": "basic", "threshold": 0.5 }),
        inputs: vec![ContentHash([1; 32])],
        millis: 120,
        cost: None,
    }
}

fn key() -> ContentHash {
    let r = run();
    Cache::key("transcribe", &r.stage_version, &r.config, &r.inputs)
}

#[test]
fn an_entry_put_is_got_whole() {
    let (c, dir) = cache("roundtrip");
    let put = c
        .put(
            "transcribe",
            key(),
            &[("notes.json", b"[1,2,3]"), ("midi.mid", b"MThd")],
            run(),
        )
        .unwrap();
    let got = c.get("transcribe", key()).unwrap();
    assert_eq!(got, put);
    assert_eq!(
        got.dir,
        dir.join("cache").join("transcribe").join(key().to_string())
    );
    assert_eq!(
        std::fs::read(got.artifact("notes.json").unwrap()).unwrap(),
        b"[1,2,3]"
    );
    assert_eq!(got.artifact("missing.bin"), None);
    // The manifest lists artifacts by name, with their hashes and sizes.
    let names: Vec<_> = got
        .manifest
        .artifacts
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["midi.mid", "notes.json"]);
    assert_eq!(got.manifest.artifacts[1].bytes, 7);
    assert_eq!(
        got.manifest.artifacts[1].hash,
        ContentHash(*blake3::hash(b"[1,2,3]").as_bytes())
    );
    assert_eq!(got.manifest.run, run());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_key_is_the_work() {
    let r = run();
    let k = |stage: &str, version: &str, config: &Value, inputs: &[ContentHash]| {
        Cache::key(stage, version, config, inputs)
    };
    let base = k("transcribe", "1", &r.config, &r.inputs);
    // The same work, the same key; config keys in any order.
    assert_eq!(
        base,
        k(
            "transcribe",
            "1",
            &json!({ "threshold": 0.5, "model": "basic" }),
            &r.inputs
        )
    );
    // Any change, another key.
    assert_ne!(base, k("separate", "1", &r.config, &r.inputs));
    assert_ne!(base, k("transcribe", "2", &r.config, &r.inputs));
    assert_ne!(
        base,
        k(
            "transcribe",
            "1",
            &json!({ "model": "basic", "threshold": 0.6 }),
            &r.inputs
        )
    );
    assert_ne!(
        base,
        k("transcribe", "1", &r.config, &[ContentHash([2; 32])])
    );
    assert_ne!(
        base,
        k(
            "transcribe",
            "1",
            &r.config,
            &[ContentHash([1; 32]), ContentHash([1; 32])]
        )
    );
}

#[test]
fn racing_writers_all_succeed_with_one_entry() {
    let (c, dir) = cache("race");
    let c = Arc::new(c);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let c = Arc::clone(&c);
            std::thread::spawn(move || {
                c.put("transcribe", key(), &[("notes.json", b"same")], run())
            })
        })
        .collect();
    let entries: Vec<Entry> = threads
        .into_iter()
        .map(|t| t.join().unwrap().unwrap())
        .collect();
    assert!(entries.windows(2).all(|w| w[0] == w[1]));
    // One entry, and no temporary directories left behind.
    let stage = dir.join("cache").join("transcribe");
    let names: Vec<_> = std::fs::read_dir(&stage)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [key().to_string()]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn damaged_or_half_written_entries_are_misses() {
    let (c, dir) = cache("damage");
    let entry = c
        .put("transcribe", key(), &[("notes.json", b"[1,2,3]")], run())
        .unwrap();
    // An artifact changed on disk: a miss, not a wrong hit.
    std::fs::write(entry.dir.join("notes.json"), b"[9,9,9]").unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    // An artifact gone: a miss.
    std::fs::remove_file(entry.dir.join("notes.json")).unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    // A manifest gone: a miss. Putting again repairs nothing in place (the old directory is in the way), so the
    // caller sees the error rather than a silent bad entry.
    std::fs::remove_file(entry.dir.join("manifest.json")).unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    // A temporary directory from a crashed writer is never an entry.
    let other = Cache::key("transcribe", "1", &json!({}), &[]);
    let tmp = dir.join("cache").join("transcribe").join(".tmp-1-0");
    std::fs::create_dir_all(&tmp).unwrap();
    assert_eq!(c.get("transcribe", other), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_manifest_for_another_stage_or_key_is_a_miss() {
    let (c, dir) = cache("mismatch");
    let entry = c
        .put("transcribe", key(), &[("a.bin", b"x")], run())
        .unwrap();
    // Copy the entry under another key: its manifest names the first key, so it is no hit there.
    let other = Cache::key("transcribe", "9", &json!({}), &[]);
    let copy = dir.join("cache").join("transcribe").join(other.to_string());
    std::fs::create_dir_all(&copy).unwrap();
    for name in ["a.bin", "manifest.json"] {
        std::fs::copy(entry.dir.join(name), copy.join(name)).unwrap();
    }
    assert_eq!(c.get("transcribe", other), None);
    // A newer manifest schema is not read.
    let text = std::fs::read_to_string(entry.dir.join("manifest.json")).unwrap();
    std::fs::write(
        entry.dir.join("manifest.json"),
        text.replace("\"schema\": 1", "\"schema\": 2"),
    )
    .unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn names_stay_inside_the_cache() {
    let (c, dir) = cache("names");
    for stage in ["", "../up", "a/b", ".hidden", "tab\tbad"] {
        assert!(
            matches!(c.put(stage, key(), &[], run()), Err(Error::BadName(_))),
            "{stage:?}"
        );
        assert_eq!(c.get(stage, key()), None);
    }
    for artifact in ["../escape", "dir/file", ".tmp-1-1", "manifest.json", ""] {
        assert!(
            matches!(
                c.put("transcribe", key(), &[(artifact, b"x")], run()),
                Err(Error::BadName(_))
            ),
            "{artifact:?}"
        );
    }
    // The same artifact twice is refused too.
    assert!(matches!(
        c.put(
            "transcribe",
            key(),
            &[("a.bin", b"x"), ("a.bin", b"y")],
            run()
        ),
        Err(Error::BadName(_))
    ));
    assert!(!dir.join("escape").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn manifests_refuse_unknown_fields() {
    let (c, dir) = cache("unknown");
    let entry = c.put("transcribe", key(), &[], run()).unwrap();
    let text = std::fs::read_to_string(entry.dir.join("manifest.json")).unwrap();
    let extra = text.replacen('{', "{\n  \"extra\": true,", 1);
    assert!(serde_json::from_str::<Manifest>(&extra).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
