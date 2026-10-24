//! The index: rebuilt from the files, read back, opened or rebuilt by layout, and never ahead of the files.

use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;

use super::*;
use crate::cache::Run as StageRun;
use crate::history::Run;

/// A project with tracks `a` (titled) and `b`, and nothing else yet.
fn project(what: &str) -> Project {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("score64-index-{what}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tracks/a")).unwrap();
    std::fs::create_dir_all(root.join("tracks/b")).unwrap();
    std::fs::write(root.join("score64.toml"), "schema = 1\n").unwrap();
    std::fs::write(
        root.join("tracks/a/track.toml"),
        "schema = 1\ntitle = \"Main theme\"\n",
    )
    .unwrap();
    std::fs::write(root.join("tracks/b/track.toml"), "schema = 1\n").unwrap();
    Project::open(&root).unwrap()
}

fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}

fn version(byte: u8) -> VersionId {
    VersionId::from_hex(&format!("{byte:02x}").repeat(32)).unwrap()
}

fn stage_run(config: u32) -> StageRun {
    StageRun {
        stage_version: "1".into(),
        tool_version: "0.1.0".into(),
        config: json!({ "n": config }),
        inputs: vec![],
        millis: 12,
        cost: None,
    }
}

/// Put an entry in the project's cache, and say where it is.
fn put(p: &Project, stage: &str, config: u32, bytes: &[u8]) -> CacheRef {
    let cache = Cache::new(p.root().join(".score64/cache"));
    let e = cache
        .put(stage, &[("out.bin", bytes)], stage_run(config))
        .unwrap();
    CacheRef {
        stage: stage.into(),
        key: e.manifest.key,
    }
}

fn record(p: &Project, track: &str, v: u8, time: i64, uses: Vec<CacheRef>) {
    let mut h = p.history(&id(track)).unwrap();
    h.record(Run {
        version: version(v),
        time: crate::UtcTime::from_unix(time).unwrap(),
        provenance: RunProvenance::Llm {
            model: "some-model".into(),
        },
        restore: false,
        uses,
    })
    .unwrap()
    .unwrap();
}

#[test]
fn an_empty_project_indexes_its_tracks() {
    let p = project("empty");
    let index = Index::open(&p).unwrap();
    assert!(Index::path(&p).is_file());
    assert_eq!(
        index.summary().unwrap(),
        Summary {
            tracks: 2,
            ..Summary::default()
        }
    );
    assert_eq!(index.versions(&id("a")).unwrap(), []);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn versions_entries_and_their_links_come_from_the_files() {
    let p = project("full");
    let mix = put(&p, "transcribe", 1, b"notes");
    let song = put(&p, "song", 1, b"song bytes");
    let other = put(&p, "song", 2, b"another");
    record(&p, "a", 1, 1_000, vec![mix.clone(), song.clone()]);
    record(&p, "a", 2, 2_000, vec![mix.clone(), other.clone()]);
    record(&p, "b", 1, 3_000, vec![mix.clone(), song.clone()]);

    let index = Index::rebuild(&p).unwrap();
    assert_eq!(
        index.summary().unwrap(),
        Summary {
            tracks: 2,
            versions: 3,
            entries: 3,
            artifacts: 3,
            bytes: (5 + 10 + 7) as u64,
        }
    );
    let a = index.versions(&id("a")).unwrap();
    assert_eq!(
        a.iter().map(|v| (v.version, v.parent)).collect::<Vec<_>>(),
        [(version(1), None), (version(2), Some(version(1)))]
    );
    assert_eq!(
        a[0].provenance,
        RunProvenance::Llm {
            model: "some-model".into()
        }
    );
    assert_eq!(a[1].time.unix(), 2_000);
    let mut want = vec![mix.clone(), song.clone()];
    want.sort();
    assert_eq!(index.uses(&version(1)).unwrap(), want);
    assert_eq!(
        index.used_by(&mix).unwrap(),
        [
            ("a".to_string(), version(1)),
            ("a".to_string(), version(2)),
            ("b".to_string(), version(1)),
        ]
    );
    assert_eq!(
        index.used_by(&other).unwrap(),
        [("a".to_string(), version(2))]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn opening_rebuilds_a_missing_or_foreign_index_and_keeps_a_current_one() {
    let p = project("open");
    Index::open(&p).unwrap();
    // A current index is opened as it is: a run recorded since doesn't show until a rebuild.
    record(&p, "a", 1, 1_000, vec![]);
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 0);
    assert_eq!(Index::rebuild(&p).unwrap().summary().unwrap().versions, 1);
    // Another layout is rebuilt.
    {
        let conn = Connection::open(Index::path(&p)).unwrap();
        conn.pragma_update(None, "user_version", INDEX_SCHEMA + 1)
            .unwrap();
    }
    record(&p, "a", 2, 2_000, vec![]);
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 2);
    // So is a file that isn't a database at all.
    std::fs::write(Index::path(&p), "not sqlite").unwrap();
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 2);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_bad_file_fails_the_rebuild_and_leaves_the_old_index() {
    let p = project("bad");
    record(&p, "a", 1, 1_000, vec![]);
    Index::rebuild(&p).unwrap();
    std::fs::write(p.root().join("tracks/a/history/notes.toml"), "x").unwrap();
    let e = Index::rebuild(&p).err().expect("a bad history fails");
    assert!(e.to_string().contains("notes.toml"), "{e}");
    // The old index is still there and whole, and no temporary database is left.
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 1);
    let left: Vec<String> = std::fs::read_dir(p.root().join(".score64"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("index.db"))
        .collect();
    assert_eq!(left, ["index.db"]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn damaged_cache_entries_and_leftovers_are_not_listed() {
    let p = project("damaged");
    let good = put(&p, "song", 1, b"good");
    let bad = put(&p, "song", 2, b"bad");
    let cache = p.root().join(".score64/cache/song");
    std::fs::write(cache.join(bad.key.to_string()).join("out.bin"), b"changed").unwrap();
    std::fs::create_dir_all(cache.join(".tmp-1-2-3")).unwrap();
    std::fs::create_dir_all(cache.join("not-a-key")).unwrap();
    let index = Index::rebuild(&p).unwrap();
    assert_eq!(index.summary().unwrap().entries, 1);
    let listed = Cache::new(p.root().join(".score64/cache"))
        .entries()
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].manifest.key, good.key);
    let _ = std::fs::remove_dir_all(p.root());
}
