//! The index: rebuilt from the files, read back column by column, opened while current and rebuilt when the files,
//! the layout or the database itself say otherwise, copied into place under a reader, and never ahead of the files.

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
        stage_version: "3".into(),
        tool_version: "0.9.1".into(),
        config: json!({ "n": config }),
        inputs: vec![],
        millis: 1234,
        cost: None,
    }
}

fn cache(p: &Project) -> Cache {
    Cache::new(p.root().join(".score64/cache"))
}

/// Put an entry in the project's cache, and say where it is.
fn put(p: &Project, stage: &str, config: u32, bytes: &[u8]) -> CacheRef {
    let e = cache(p)
        .put(stage, &[("out.bin", bytes)], stage_run(config))
        .unwrap();
    CacheRef {
        stage: stage.into(),
        key: e.manifest.key,
    }
}

fn llm() -> RunProvenance {
    RunProvenance::Llm {
        model: "some-model".into(),
    }
}

/// Record a run on `track`.
fn record_run(
    p: &Project,
    track: &str,
    v: u8,
    time: i64,
    provenance: RunProvenance,
    restore: bool,
    uses: Vec<CacheRef>,
) {
    let mut h = p.history(&id(track)).unwrap();
    h.record(Run {
        version: version(v),
        time: crate::UtcTime::from_unix(time).unwrap(),
        provenance,
        restore,
        uses,
    })
    .unwrap()
    .unwrap();
}

fn record(p: &Project, track: &str, v: u8, time: i64, uses: Vec<CacheRef>) {
    record_run(p, track, v, time, llm(), false, uses);
}

/// The names of the index's own files in `.score64/`.
fn index_files(p: &Project) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(p.root().join(".score64"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("index.db"))
        .collect();
    names.sort();
    names
}

/// How many recorded runs the database at `path` holds, read past [`Index`].
fn versions_in(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))
        .unwrap()
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
    assert_eq!(
        index.tracks().unwrap(),
        [(id("a"), Some("Main theme".to_string())), (id("b"), None)]
    );
    assert_eq!(index.versions(&id("a")).unwrap(), []);
    assert_eq!(index.entries().unwrap(), []);
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
    assert_eq!(a[0].provenance, llm());
    assert_eq!(a[1].time.unix(), 2_000);
    let mut want = vec![mix.clone(), song.clone()];
    want.sort();
    assert_eq!(index.uses(&version(1)).unwrap(), want);
    assert_eq!(
        index.used_by(&mix).unwrap(),
        [
            (id("a"), version(1)),
            (id("a"), version(2)),
            (id("b"), version(1))
        ]
    );
    assert_eq!(index.used_by(&other).unwrap(), [(id("a"), version(2))]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn versions_come_in_history_order_not_by_id() {
    let p = project("order");
    // Ids whose hex order (10 < 50 < 90) is not the order they ran in.
    record(&p, "a", 0x90, 1_000, vec![]);
    record(&p, "a", 0x10, 2_000, vec![]);
    record(&p, "a", 0x50, 3_000, vec![]);
    let index = Index::rebuild(&p).unwrap();
    let got: Vec<_> = index
        .versions(&id("a"))
        .unwrap()
        .iter()
        .map(|v| (v.version, v.time.unix()))
        .collect();
    assert_eq!(
        got,
        [
            (version(0x90), 1_000),
            (version(0x10), 2_000),
            (version(0x50), 3_000)
        ]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn every_provenance_and_restores_round_trip() {
    let p = project("provenance");
    let mix = put(&p, "transcribe", 1, b"notes");
    let manual = RunProvenance::Manual {
        name: "A. Person".into(),
    };
    record_run(
        &p,
        "a",
        1,
        1_000,
        RunProvenance::Rules {},
        false,
        vec![mix.clone()],
    );
    record_run(&p, "a", 2, 2_000, manual.clone(), false, vec![mix.clone()]);
    record_run(&p, "a", 1, 3_000, llm(), true, vec![mix.clone()]);
    let index = Index::rebuild(&p).unwrap();
    let got = index.versions(&id("a")).unwrap();
    assert_eq!(
        got.iter()
            .map(|v| (v.version, v.provenance.clone(), v.restore))
            .collect::<Vec<_>>(),
        [
            (version(1), RunProvenance::Rules {}, false),
            (version(2), manual, false),
            (version(1), llm(), true),
        ]
    );
    // Each one names its history file.
    let files: Vec<String> = p
        .history(&id("a"))
        .unwrap()
        .entries()
        .map(|(f, _)| f.to_string())
        .collect();
    assert_eq!(
        got.iter().map(|v| v.file.clone()).collect::<Vec<_>>(),
        files
    );
    // Version 1 was recorded twice on `a`, and both runs use `mix`: it is listed once.
    assert_eq!(
        index.used_by(&mix).unwrap(),
        [(id("a"), version(1)), (id("a"), version(2))]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn entries_carry_their_manifests() {
    let p = project("entries");
    let c = cache(&p);
    let mut paid = stage_run(1);
    paid.cost = Some(Cost {
        amount: 0.25,
        currency: "USD".into(),
    });
    let first = c
        .put(
            "transcribe",
            &[("a.bin", b"12345"), ("b.bin", b"1234567")],
            paid,
        )
        .unwrap();
    let second = c.put("song", &[("out.bin", b"xyz")], stage_run(2)).unwrap();
    let index = Index::rebuild(&p).unwrap();
    let want = |e: &crate::cache::Entry, bytes, artifacts| CacheEntry {
        entry: CacheRef {
            stage: e.manifest.stage.clone(),
            key: e.manifest.key,
        },
        created: e.manifest.created,
        stage_version: "3".into(),
        tool_version: "0.9.1".into(),
        millis: 1234,
        cost: e.manifest.run.cost.clone(),
        bytes,
        artifacts,
    };
    // By stage, then key: `song` before `transcribe`.
    assert_eq!(
        index.entries().unwrap(),
        [want(&second, 3, 1), want(&first, 12, 2)]
    );
    assert!(index.entries().unwrap()[1].created.unix() > 1_700_000_000);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn opening_keeps_a_current_index_and_rebuilds_a_missing_or_foreign_one() {
    let p = project("open");
    Index::open(&p).unwrap();
    // Nothing changed in the files: the index is opened as it is. (Rows no rebuild wrote show it wasn't rebuilt.)
    {
        let conn = Connection::open(Index::path(&p)).unwrap();
        conn.execute("INSERT INTO tracks (id, title) VALUES ('marker', NULL)", [])
            .unwrap();
    }
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().tracks, 3);
    // A run recorded since: rebuilt.
    record(&p, "a", 1, 1_000, vec![]);
    let index = Index::open(&p).unwrap();
    assert_eq!(index.summary().unwrap().tracks, 2);
    assert_eq!(index.summary().unwrap().versions, 1);
    drop(index);
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
    // And a missing one.
    std::fs::remove_file(Index::path(&p)).unwrap();
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 2);
    assert_eq!(index_files(&p), ["index.db"]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn opening_catches_up_with_every_kind_of_change() {
    let p = project("catch-up");
    let song = put(&p, "song", 1, b"song bytes");
    let mix = put(&p, "transcribe", 1, b"notes");
    record(&p, "a", 1, 1_000, vec![song.clone()]);
    record(&p, "b", 1, 1_000, vec![song.clone()]);
    let summary = |p: &Project| Index::open(p).unwrap().summary().unwrap();
    assert_eq!((summary(&p).versions, summary(&p).entries), (2, 2));

    // A new run.
    record(&p, "a", 2, 2_000, vec![song.clone()]);
    assert_eq!(summary(&p).versions, 3);
    // A deleted cache entry, as a GC would leave it.
    std::fs::remove_dir_all(
        p.root()
            .join(".score64/cache/transcribe")
            .join(mix.key.to_string()),
    )
    .unwrap();
    assert_eq!(summary(&p).entries, 1);
    // A new one.
    put(&p, "song", 2, b"another");
    assert_eq!(summary(&p).entries, 2);
    // A manifest, edited in place (the tool version isn't part of the key, so the entry stays whole).
    let manifest = p
        .root()
        .join(".score64/cache/song")
        .join(song.key.to_string())
        .join("manifest.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace("\"0.9.1\"", "\"0.10.0-dev\"")).unwrap();
    let entries = Index::open(&p).unwrap().entries().unwrap();
    let edited = entries.iter().find(|e| e.entry == song).unwrap();
    assert_eq!(edited.tool_version, "0.10.0-dev");
    // An artifact deleted from an entry: the entry isn't whole any more. Only its folder's time says so, so give the
    // clock a moment to move past the time the entry was written (some systems tick in milliseconds).
    std::thread::sleep(Duration::from_millis(50));
    std::fs::remove_file(
        p.root()
            .join(".score64/cache/song")
            .join(song.key.to_string())
            .join("out.bin"),
    )
    .unwrap();
    assert_eq!(summary(&p).entries, 1);
    // A track's history folder, deleted.
    std::fs::remove_dir_all(p.root().join("tracks/b/history")).unwrap();
    assert_eq!(summary(&p).versions, 2);
    assert_eq!(Index::open(&p).unwrap().used_by(&song).unwrap().len(), 2);
    // A whole track, deleted.
    std::fs::remove_dir_all(p.root().join("tracks/a")).unwrap();
    assert_eq!((summary(&p).tracks, summary(&p).versions), (1, 0));
    // A title, edited.
    std::fs::write(
        p.root().join("tracks/b/track.toml"),
        "schema = 1\ntitle = \"Boss fight\"\n",
    )
    .unwrap();
    assert_eq!(
        Index::open(&p).unwrap().tracks().unwrap(),
        [(id("b"), Some("Boss fight".to_string()))]
    );
    // Edited again to the same size: only the modification time tells. (Set outright, so the test doesn't depend on
    // how finely the file system keeps time.)
    let file = p.root().join("tracks/b/track.toml");
    let size = std::fs::metadata(&file).unwrap().len();
    std::fs::write(&file, "schema = 1\ntitle = \"Boss night\"\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().len(), size);
    assert_eq!(
        Index::open(&p).unwrap().tracks().unwrap(),
        [(id("b"), Some("Boss night".to_string()))]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn another_layout_is_rebuilt_even_when_no_file_changed() {
    let p = project("layout");
    record(&p, "a", 1, 1_000, vec![]);
    Index::open(&p).unwrap();
    // A row no rebuild writes, and another layout number; the files stay as they are.
    {
        let conn = Connection::open(Index::path(&p)).unwrap();
        conn.execute("INSERT INTO tracks (id, title) VALUES ('marker', NULL)", [])
            .unwrap();
        conn.pragma_update(None, "user_version", INDEX_SCHEMA + 1)
            .unwrap();
    }
    let index = Index::open(&p).unwrap();
    assert_eq!(
        index.summary().unwrap().tracks,
        2,
        "rebuilt: the marker is gone"
    );
    let layout: i64 = index
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(layout, INDEX_SCHEMA);
    let _ = std::fs::remove_dir_all(p.root());
}

/// A reader on another thread that opens the index, starts a read transaction, counts its versions, says so, holds
/// the transaction for `hold`, counts again and ends it. Gives both counts.
fn hold_a_read(
    path: PathBuf,
    hold: Duration,
) -> (
    std::thread::JoinHandle<(i64, i64)>,
    std::sync::mpsc::Receiver<()>,
) {
    let (ready, started) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let count = || -> i64 {
            conn.query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))
                .unwrap()
        };
        conn.execute_batch("BEGIN").unwrap();
        let before = count();
        ready.send(()).unwrap();
        std::thread::sleep(hold);
        let after = count();
        conn.execute_batch("COMMIT").unwrap();
        (before, after)
    });
    (reader, started)
}

#[test]
fn a_reader_in_a_transaction_delays_the_copy_and_keeps_the_old_index_meanwhile() {
    let p = project("busy");
    Index::open(&p).unwrap();
    record(&p, "a", 1, 1_000, vec![]);
    let (reader, started) = hold_a_read(Index::path(&p), Duration::from_millis(1_500));
    started.recv().unwrap();
    let start = Instant::now();
    let index = Index::rebuild(&p).unwrap();
    let took = start.elapsed();
    // The reader saw the old index throughout its transaction; the copy waited for it, then went in.
    assert_eq!(reader.join().unwrap(), (0, 0));
    assert!(took >= Duration::from_millis(1_000), "{took:?}");
    assert_eq!(index.summary().unwrap().versions, 1);
    assert_eq!(index_files(&p), ["index.db"]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_reader_holding_on_past_the_wait_fails_the_rebuild_cleanly() {
    let p = project("timeout");
    Index::open(&p).unwrap();
    record(&p, "a", 1, 1_000, vec![]);
    let (reader, started) = hold_a_read(Index::path(&p), Duration::from_millis(1_500));
    started.recv().unwrap();
    TEST_WAIT.set(Some(Duration::from_millis(300)));
    let start = Instant::now();
    let e = Index::rebuild(&p)
        .err()
        .expect("the reader outlasts the wait");
    let took = start.elapsed();
    TEST_WAIT.set(None);
    assert!(matches!(e, Error::Sql { .. }), "{e:?}");
    assert!(e.to_string().contains("index.db"), "{e}");
    assert!(
        took < Duration::from_millis(1_400),
        "gave up at the wait, not when the reader let go: {took:?}"
    );
    assert_eq!(reader.join().unwrap(), (0, 0));
    // The old index is whole, and nothing of the attempt is left.
    assert_eq!(versions_in(&Index::path(&p)), 0);
    assert_eq!(index_files(&p), ["index.db"]);
    assert_eq!(Index::rebuild(&p).unwrap().summary().unwrap().versions, 1);
    let _ = std::fs::remove_dir_all(p.root());
}

/// Leave the index as a rebuild that crashed midway through its copy would: the database half-written and its
/// rollback journal hot. The state is made on a copy, by a write transaction that spills pages into the file, and
/// copied over the index before that transaction ends.
fn crash_midway(p: &Project) {
    let path = Index::path(p);
    let scratch = path.with_file_name("scratch.db");
    std::fs::copy(&path, &scratch).unwrap();
    let (half, hot) = {
        let conn = Connection::open(&scratch).unwrap();
        conn.execute_batch("PRAGMA cache_size = 1; BEGIN IMMEDIATE; DELETE FROM versions;")
            .unwrap();
        for i in 0..2_000 {
            conn.execute(
                "INSERT INTO tracks (id, title) VALUES (?1, ?2)",
                params![format!("t{i}"), "x".repeat(200)],
            )
            .unwrap();
        }
        (
            std::fs::read(&scratch).unwrap(),
            std::fs::read(journal(&scratch)).unwrap(),
        )
    };
    std::fs::remove_file(&scratch).unwrap();
    assert_ne!(half, std::fs::read(&path).unwrap(), "pages were spilled");
    std::fs::write(&path, &half).unwrap();
    std::fs::write(journal(&path), &hot).unwrap();
}

#[test]
fn after_a_crash_midway_readers_and_open_get_the_old_index_back() {
    let p = project("crash");
    record(&p, "a", 1, 1_000, vec![]);
    let held = Index::open(&p).unwrap();
    assert_eq!(held.summary().unwrap().versions, 1);
    // A reader opened before the crash rolls the journal back at its next query.
    crash_midway(&p);
    assert_eq!(held.summary().unwrap().tracks, 2);
    assert_eq!(held.versions(&id("a")).unwrap().len(), 1);
    assert!(!journal(&Index::path(&p)).exists());
    drop(held);
    // So does the first `open` after one.
    crash_midway(&p);
    let index = Index::open(&p).unwrap();
    assert_eq!(index.summary().unwrap().tracks, 2);
    assert_eq!(index.summary().unwrap().versions, 1);
    assert!(!journal(&Index::path(&p)).exists());
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_damaged_index_is_rebuilt_on_open_and_says_so_on_a_query() {
    let p = project("damaged-index");
    record(&p, "a", 1, 1_000, vec![]);
    Index::open(&p).unwrap();
    // The right layout number, but a table gone: rebuilt.
    for table in ["inputs", "versions"] {
        {
            let conn = Connection::open(Index::path(&p)).unwrap();
            conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        }
        assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 1);
    }
    // Damaged under an open index: the query fails, saying how to fix it.
    let held = Index::open(&p).unwrap();
    {
        let conn = Connection::open(Index::path(&p)).unwrap();
        conn.execute_batch("DROP TABLE artifacts").unwrap();
    }
    let e = held.summary().unwrap_err();
    assert!(matches!(e, Error::Damaged { .. }), "{e:?}");
    assert!(
        e.to_string().contains("run `score64 index --rebuild`"),
        "{e}"
    );
    // A value no rebuild writes.
    {
        let conn = Connection::open(Index::path(&p)).unwrap();
        conn.execute("UPDATE versions SET by = 'someone'", [])
            .unwrap();
    }
    let e = held.versions(&id("a")).unwrap_err();
    assert!(
        e.to_string().contains("run `score64 index --rebuild`"),
        "{e}"
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn an_index_held_open_across_a_rebuild_sees_the_new_one() {
    let p = project("held");
    let held = Index::open(&p).unwrap();
    assert_eq!(held.summary().unwrap().versions, 0);
    record(&p, "a", 1, 1_000, vec![]);
    // The copy goes into the file the reader holds, on every system (a rename over it fails on Windows).
    let rebuilt = Index::rebuild(&p).unwrap();
    assert_eq!(rebuilt.summary().unwrap().versions, 1);
    assert_eq!(held.summary().unwrap().versions, 1);
    // And again through `open`, with both still held.
    record(&p, "a", 2, 2_000, vec![]);
    assert_eq!(Index::open(&p).unwrap().summary().unwrap().versions, 2);
    assert_eq!(held.summary().unwrap().versions, 2);
    assert_eq!(rebuilt.summary().unwrap().versions, 2);
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
    // `open` sees the change, so it fails the same way rather than give the old index as current.
    let e = Index::open(&p).err().expect("a bad history fails");
    assert!(e.to_string().contains("notes.toml"), "{e}");
    // The old index is still there and whole, and no temporary database is left.
    assert_eq!(versions_in(&Index::path(&p)), 1);
    assert_eq!(index_files(&p), ["index.db"]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_bad_decisions_file_does_not_block_the_index() {
    let p = project("decisions");
    std::fs::write(p.root().join("tracks/a/decisions.toml"), "not toml [").unwrap();
    assert_eq!(Index::rebuild(&p).unwrap().summary().unwrap().tracks, 2);
    // A bad `track.toml` does, naming it.
    std::fs::write(
        p.root().join("tracks/b/track.toml"),
        "schema = 1\ncolour = 1\n",
    )
    .unwrap();
    let e = Index::rebuild(&p).err().expect("a bad track file fails");
    assert!(e.to_string().contains("track.toml"), "{e}");
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_number_too_large_for_sqlite_fails_the_rebuild_naming_the_manifest() {
    let p = project("too-large");
    let e = put(&p, "song", 1, b"x");
    let manifest = p
        .root()
        .join(".score64/cache/song")
        .join(e.key.to_string())
        .join("manifest.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("\"millis\": 1234", &format!("\"millis\": {}", u64::MAX)),
    )
    .unwrap();
    // Still a whole entry: the cache lists it.
    assert_eq!(
        cache(&p).entries().unwrap()[0].manifest.run.millis,
        u64::MAX
    );
    let err = Index::rebuild(&p).err().expect("too large");
    assert!(matches!(err, Error::TooLarge { .. }), "{err:?}");
    assert!(err.to_string().contains("manifest.json"), "{err}");
    assert!(err.to_string().contains("run.millis"), "{err}");
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn crashed_rebuilds_leftovers_are_cleared_and_live_ones_kept() {
    let p = project("leftovers");
    let dir = p.root().join(".score64");
    std::fs::create_dir_all(&dir).unwrap();
    let old = SystemTime::now() - Duration::from_secs(11 * 60);
    let make = |name: &str, mtime: SystemTime| {
        let f = std::fs::File::create(dir.join(name)).unwrap();
        f.set_modified(mtime).unwrap();
    };
    make("index.db.1-2-3.part", old);
    make("index.db.1-2-3.part-journal", old);
    make("index.db.4-5-6.part", SystemTime::now());
    make("index.db.notes", old);
    Index::rebuild(&p).unwrap();
    assert_eq!(
        index_files(&p),
        ["index.db", "index.db.4-5-6.part", "index.db.notes"]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn damaged_cache_entries_and_leftovers_are_not_listed() {
    let p = project("damaged");
    let good = put(&p, "song", 1, b"good");
    let bad = put(&p, "song", 2, b"bad");
    let cache_dir = p.root().join(".score64/cache/song");
    std::fs::write(
        cache_dir.join(bad.key.to_string()).join("out.bin"),
        b"changed",
    )
    .unwrap();
    std::fs::create_dir_all(cache_dir.join(".tmp-1-2-3")).unwrap();
    std::fs::create_dir_all(cache_dir.join("not-a-key")).unwrap();
    let index = Index::rebuild(&p).unwrap();
    assert_eq!(index.summary().unwrap().entries, 1);
    let listed = cache(&p).entries().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].manifest.key, good.key);
    let _ = std::fs::remove_dir_all(p.root());
}
