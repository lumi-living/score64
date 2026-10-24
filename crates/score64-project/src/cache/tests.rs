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
            std::thread::spawn(move || c.put("transcribe", &[("notes.json", b"same")], run()))
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
        .put("transcribe", &[("notes.json", b"[1,2,3]")], run())
        .unwrap();
    // An artifact changed on disk: a miss, not a wrong hit.
    std::fs::write(entry.dir.join("notes.json"), b"[9,9,9]").unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    // An artifact gone: a miss.
    std::fs::remove_file(entry.dir.join("notes.json")).unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    // A manifest gone: a miss, and putting again replaces what is in the way.
    std::fs::remove_file(entry.dir.join("manifest.json")).unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    let again = c
        .put("transcribe", &[("notes.json", b"[1,2,3]")], run())
        .unwrap();
    assert_eq!(c.get("transcribe", key()), Some(again));
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
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
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
        text.replace("\"schema\": 2", "\"schema\": 3"),
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
            matches!(c.put(stage, &[], run()), Err(Error::BadName(_))),
            "{stage:?}"
        );
        assert_eq!(c.get(stage, key()), None);
    }
    for artifact in [
        "../escape",
        "dir/file",
        ".tmp-1-1",
        "manifest.json",
        "MANIFEST.JSON",
        "",
        "con",
        "nul.json",
        "COM1.bin",
        "lpt9",
        "trailing.",
    ] {
        assert!(
            matches!(
                c.put("transcribe", &[(artifact, b"x")], run()),
                Err(Error::BadName(_))
            ),
            "{artifact:?}"
        );
    }
    // The same artifact twice is refused too, also when only the case differs (one file on macOS and Windows).
    for (a, b) in [("a.bin", "a.bin"), ("A.bin", "a.bin"), ("x.JSON", "X.json")] {
        assert!(
            matches!(
                c.put("transcribe", &[(a, b"x"), (b, b"y")], run()),
                Err(Error::BadName(_))
            ),
            "{a} {b}"
        );
    }
    // Names that only look like devices are fine.
    assert!(
        c.put(
            "transcribe",
            &[("console.log", b"x"), ("com10", b"y")],
            run()
        )
        .is_ok()
    );
    assert!(!dir.join("escape").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn manifests_refuse_unknown_fields() {
    let (c, dir) = cache("unknown");
    let entry = c.put("transcribe", &[], run()).unwrap();
    let text = std::fs::read_to_string(entry.dir.join("manifest.json")).unwrap();
    let extra = text.replacen('{', "{\n  \"extra\": true,", 1);
    assert!(serde_json::from_str::<Manifest>(&extra).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_entry_is_stored_under_its_own_runs_key() {
    let (c, dir) = cache("ownkey");
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    assert_eq!(entry.manifest.key, key());
    let mut other = run();
    other.config = json!({ "model": "other" });
    let e2 = c
        .put("transcribe", &[("a.bin", b"x")], other.clone())
        .unwrap();
    assert_eq!(
        e2.manifest.key,
        Cache::key("transcribe", "1", &other.config, &other.inputs)
    );
    assert_ne!(e2.dir, entry.dir);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_damaged_entry_with_files_in_it_is_replaced() {
    // Renaming onto a directory that still has files fails: the damaged entry must be moved aside first.
    let (c, dir) = cache("replace");
    let entry = c
        .put("transcribe", &[("notes.json", b"[1,2,3]")], run())
        .unwrap();
    std::fs::write(entry.dir.join("notes.json"), b"damaged").unwrap();
    std::fs::write(entry.dir.join("junk"), b"left over").unwrap();
    assert_eq!(c.get("transcribe", key()), None);
    let fresh = c
        .put("transcribe", &[("notes.json", b"[1,2,3]")], run())
        .unwrap();
    assert_eq!(c.get("transcribe", key()), Some(fresh.clone()));
    assert!(!fresh.dir.join("junk").exists());
    // Nothing is left beside it.
    let names: Vec<_> = std::fs::read_dir(dir.join("cache").join("transcribe"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [key().to_string()]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_in_an_entrys_place_is_replaced() {
    let (c, dir) = cache("file");
    let stage = dir.join("cache").join("transcribe");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join(key().to_string()), b"not a directory").unwrap();
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    assert_eq!(c.get("transcribe", key()), Some(entry));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_entry_from_a_newer_score64_is_left_alone() {
    let (c, dir) = cache("newer");
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    let path = entry.dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let newer = text.replace("\"schema\": 2", "\"schema\": 3");
    std::fs::write(&path, &newer).unwrap();
    assert!(matches!(
        c.put("transcribe", &[("a.bin", b"x")], run()),
        Err(Error::Newer(_))
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer, "untouched");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_taken_temporary_name_is_skipped() {
    let (_, dir) = cache("taken");
    std::fs::create_dir_all(dir.join("taken")).unwrap();
    let mut names = ["taken", "fresh"].into_iter().map(String::from);
    let got = temp_dir_named(&dir, || names.next().unwrap()).unwrap();
    assert_eq!(got, dir.join("fresh"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn leftover_temporary_directories_get_in_nobodys_way() {
    let (c, dir) = cache("leftovers");
    let stage = dir.join("cache").join("transcribe");
    std::fs::create_dir_all(&stage).unwrap();
    for n in 0..8 {
        std::fs::create_dir(stage.join(format!(".tmp-{}-{n}", std::process::id()))).unwrap();
    }
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    assert_eq!(c.get("transcribe", key()), Some(entry));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_entry_under_another_stage_is_a_miss() {
    let (c, dir) = cache("otherstage");
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    let copy = dir.join("cache").join("separate").join(key().to_string());
    std::fs::create_dir_all(&copy).unwrap();
    for name in ["a.bin", "manifest.json"] {
        std::fs::copy(entry.dir.join(name), copy.join(name)).unwrap();
    }
    assert_eq!(c.get("separate", key()), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cost_has_its_currency() {
    let (c, dir) = cache("cost");
    let mut r = run();
    r.cost = Some(Cost {
        amount: 0.012,
        currency: "USD".into(),
    });
    let entry = c.put("transcribe", &[], r.clone()).unwrap();
    let back = c.get("transcribe", entry.manifest.key).unwrap();
    assert_eq!(back.manifest.run.cost, r.cost);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn config_keys_sort_at_every_depth() {
    let a = json!({ "b": { "y": 1, "x": [ { "q": 1, "p": 2 } ] }, "a": 0 });
    let b = json!({ "a": 0, "b": { "x": [ { "p": 2, "q": 1 } ], "y": 1 } });
    assert_eq!(Cache::key("s", "1", &a, &[]), Cache::key("s", "1", &b, &[]));
}

#[test]
fn put_records_when_it_was_written_and_the_manifest_keeps_it() {
    let (c, dir) = cache("created");
    let before = crate::UtcTime::now();
    let put = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    let after = crate::UtcTime::now();
    let created = put.manifest.created;
    assert!(before <= created && created <= after, "{created}");
    let text = std::fs::read_to_string(put.dir.join("manifest.json")).unwrap();
    assert!(
        text.contains(&format!("\"created\": \"{created}\"")),
        "{text}"
    );
    assert_eq!(
        c.get("transcribe", key()).unwrap().manifest.created,
        created
    );
    assert_eq!(c.entries().unwrap()[0].manifest.created, created);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn entries_lists_each_entry_once_under_its_own_name() {
    let (c, dir) = cache("listing");
    let entry = c.put("transcribe", &[("a.bin", b"x")], run()).unwrap();
    // A copy under the key's uppercase spelling: a second folder on a case-sensitive file system, the same one on
    // others. Either way the entry is listed once, at its lowercase name.
    let upper = entry
        .dir
        .with_file_name(entry.manifest.key.to_string().to_ascii_uppercase());
    let distinct = std::fs::create_dir(&upper).is_ok();
    if distinct {
        for name in ["a.bin", "manifest.json"] {
            std::fs::copy(entry.dir.join(name), upper.join(name)).unwrap();
        }
    }
    let listed = c.entries().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].dir, entry.dir);
    // Under its uppercase spelling alone, it isn't listed: `get` looks for the lowercase name.
    if distinct {
        std::fs::remove_dir_all(&upper).unwrap();
    }
    let aside = dir.join("aside");
    std::fs::rename(&entry.dir, &aside).unwrap();
    std::fs::rename(&aside, &upper).unwrap();
    assert_eq!(c.entries().unwrap(), []);
    std::fs::rename(&upper, &aside).unwrap();
    std::fs::rename(&aside, &entry.dir).unwrap();
    assert_eq!(c.entries().unwrap().len(), 1);

    // A whole entry in a folder whose name isn't a valid stage name is not listed, even if its manifest agrees.
    let other = c.put("ab", &[("a.bin", b"x")], run()).unwrap();
    let root = dir.join("cache");
    std::fs::rename(root.join("ab"), root.join("a b")).unwrap();
    let manifest = root
        .join("a b")
        .join(other.manifest.key.to_string())
        .join("manifest.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("\"stage\": \"ab\"", "\"stage\": \"a b\""),
    )
    .unwrap();
    assert!(
        std::fs::read_to_string(&manifest)
            .unwrap()
            .contains("\"a b\"")
    );
    let listed = c.entries().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].manifest.stage, "transcribe");
    let _ = std::fs::remove_dir_all(dir);
}
