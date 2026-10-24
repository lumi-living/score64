//! The `gc` command: the cap it is given, the index it keeps in step, and what it reports when something fails.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;

use score64_project::Cache;
use score64_project::cache::{Entry, Run};

use super::*;
use crate::EngineConfig;

fn project(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "score64-engine-gc-{what}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tracks/a")).unwrap();
    std::fs::write(root.join("score64.toml"), "schema = 1\n").unwrap();
    std::fs::write(root.join("tracks/a/track.toml"), "schema = 1\n").unwrap();
    root
}

fn open(root: &Path) -> Project {
    Project::open(root).unwrap()
}

/// An entry of 100 bytes in `stage`, made with config `n`.
fn put(root: &Path, stage: &str, n: u32) -> Entry {
    let run = Run {
        stage_version: "1".into(),
        tool_version: "0.1.0".into(),
        config: json!({ "n": n }),
        inputs: vec![],
        millis: 1,
        cost: None,
    };
    Cache::new(open(root).cache_dir())
        .put(stage, &[("out.bin", &[n as u8; 100])], run)
        .unwrap()
}

fn gc(root: &Path, cap_bytes: Option<u64>, dry_run: bool) -> Result<GcOutput, CommandError> {
    Engine::new(EngineConfig::default()).run::<GcCommand>(GcArgs {
        project: root.to_path_buf(),
        cap_bytes,
        dry_run,
    })
}

fn indexed(root: &Path) -> u64 {
    Index::open(&open(root)).unwrap().summary().unwrap().entries
}

#[test]
fn the_cap_is_honoured_and_the_index_follows() {
    let root = project("cap");
    for n in 0..3 {
        put(&root, "song", n);
    }
    Index::rebuild(&open(&root)).unwrap();
    assert_eq!(indexed(&root), 3);
    // Under the default cap nothing goes, and the index is left as it is.
    let out = gc(&root, None, false).unwrap();
    assert_eq!((out.cap_bytes, out.removed.len()), (4 << 30, 0));
    assert!(!out.index_rebuilt && !out.index_removed && out.complete);
    // A dry run reports and changes nothing.
    let out = gc(&root, Some(150), true).unwrap();
    assert_eq!((out.removed.len(), out.index_rebuilt), (2, false));
    assert_eq!(indexed(&root), 3);
    let out = gc(&root, Some(150), false).unwrap();
    assert_eq!(out.cap_bytes, 150);
    assert_eq!((out.removed.len(), out.unrooted_kept), (2, 1));
    assert!(out.removed.iter().all(|r| r.why == "unrooted"));
    assert_eq!(
        out.roots,
        [GcRoots {
            source: "recorded versions".into(),
            refs: 0
        }]
    );
    assert!(out.index_rebuilt && !out.index_removed && out.complete);
    assert_eq!(out.warning, None);
    assert!(!GcCommand::fails(&out));
    // The rebuilt index lists only what is left.
    assert_eq!(indexed(&root), 1);
    let out = gc(&root, Some(0), false).unwrap();
    assert_eq!(out.removed.len(), 1);
    assert_eq!(indexed(&root), 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_an_index_none_is_made() {
    let root = project("noindex");
    put(&root, "song", 1);
    let out = gc(&root, Some(0), false).unwrap();
    assert_eq!(out.removed.len(), 1);
    assert!(!out.index_rebuilt && !out.index_removed);
    assert!(!Index::path(&open(&root)).exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failed_rebuild_removes_the_stale_index_and_still_reports() {
    let root = project("badtrack");
    let gone = put(&root, "song", 1);
    Index::rebuild(&open(&root)).unwrap();
    // The roots don't need track files, but the index does: this one makes the rebuild fail.
    std::fs::write(root.join("tracks/a/track.toml"), "schema = [").unwrap();
    let out = gc(&root, Some(0), false).unwrap();
    assert_eq!(out.removed.len(), 1);
    assert_eq!(out.removed[0].name, gone.manifest.key.to_string());
    assert!(!out.index_rebuilt && out.index_removed);
    let warning = out.warning.clone().unwrap();
    assert!(warning.contains("index could not be rebuilt"), "{warning}");
    assert!(
        !Index::path(&open(&root)).exists(),
        "no index is better than a wrong one"
    );
    assert!(
        out.complete && !GcCommand::fails(&out),
        "the collection itself finished"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unreadable_roots_are_an_invalid_input_and_remove_nothing() {
    let root = project("badroots");
    put(&root, "song", 1);
    std::fs::create_dir_all(root.join("tracks/a/history")).unwrap();
    std::fs::write(root.join("tracks/a/history/notes.toml"), "x").unwrap();
    let e = gc(&root, Some(0), false).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(e.message.contains("nothing removed"), "{e}");
    assert_eq!(
        Cache::new(open(&root).cache_dir()).entries().unwrap().len(),
        1
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_collection_stopped_part_way_reports_removes_the_index_and_fails() {
    use std::os::unix::fs::PermissionsExt;
    let root = project("stopped");
    put(&root, "a", 1);
    put(&root, "b", 2);
    Index::rebuild(&open(&root)).unwrap();
    let locked = open(&root).cache_dir().join("b");
    let mode = |m| std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o555);
    if std::fs::create_dir(locked.join("probe")).is_ok() {
        // Permissions don't bind this user (root): nothing to test here.
        let _ = std::fs::remove_dir(locked.join("probe"));
        mode(0o755);
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    // `a` was put first (and sorts first in the same second), so it goes; then `b` can't be moved aside.
    let out = gc(&root, Some(0), false);
    mode(0o755);
    let out = out.unwrap();
    assert!(!out.complete && GcCommand::fails(&out), "{out:?}");
    assert_eq!(out.removed.len(), 1);
    assert_eq!(out.removed[0].stage, "a");
    assert!(out.index_removed && !out.index_rebuilt);
    assert!(out.warning.unwrap().contains("stopped part-way"));
    assert!(!Index::path(&open(&root)).exists());
    // With nothing removed, a stop is a plain error, of the I/O kind.
    mode(0o555);
    let e = gc(&root, Some(0), false).unwrap_err();
    mode(0o755);
    assert_eq!(e.kind, ErrorKind::Io, "{e}");
    let _ = std::fs::remove_dir_all(&root);
}
