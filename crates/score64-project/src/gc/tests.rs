//! Cache GC: roots stay over the cap, the rest goes oldest-first, broken entries and old leftovers go, newer ones stay;
//! entries put meanwhile, links and entries in use are left alone.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use serde_json::json;

use super::*;
use crate::cache::{Entry, Run as StageRun, TEMP_DIRS_MADE};
use crate::history::{Run, RunProvenance};
use crate::project::Id;

fn project(what: &str) -> Project {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("score64-gc-{what}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tracks/a")).unwrap();
    std::fs::write(root.join("score64.toml"), "schema = 1\n").unwrap();
    std::fs::write(root.join("tracks/a/track.toml"), "schema = 1\n").unwrap();
    Project::open(&root).unwrap()
}

fn cache(p: &Project) -> Cache {
    Cache::new(p.cache_dir())
}

fn stage_run(n: u32) -> StageRun {
    StageRun {
        stage_version: "1".into(),
        tool_version: "0.1.0".into(),
        config: json!({ "n": n }),
        inputs: vec![],
        millis: 1,
        cost: None,
    }
}

/// An entry of `bytes` bytes in `stage` of `c`, made with config `n`.
fn put_in(c: &Cache, stage: &str, n: u32, bytes: usize) -> Entry {
    c.put(stage, &[("out.bin", &vec![n as u8; bytes])], stage_run(n))
        .unwrap()
}

/// An entry of `bytes` bytes, made with config `n`.
fn put(p: &Project, n: u32, bytes: usize) -> Entry {
    put_in(&cache(p), "song", n, bytes)
}

fn as_ref(e: &Entry) -> CacheRef {
    CacheRef {
        stage: e.manifest.stage.clone(),
        key: e.manifest.key,
    }
}

/// Set an entry's `created` (and so its age order) by rewriting its manifest.
fn created_at(e: &Entry, unix: i64) {
    let path = e.dir.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m["created"] = json!(crate::UtcTime::from_unix(unix).unwrap().to_string());
    std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
}

/// Record a run on track `a` using `uses`.
fn record(p: &Project, v: u8, uses: Vec<CacheRef>) {
    record_on(p, "a", v, uses);
}

fn record_on(p: &Project, track: &str, v: u8, uses: Vec<CacheRef>) {
    let mut h = p.history(&Id::new(track).unwrap()).unwrap();
    h.record(Run {
        version: ContentHash::from_hex(&format!("{v:02x}").repeat(32)).unwrap(),
        time: crate::UtcTime::from_unix(1_000 + i64::from(v)).unwrap(),
        provenance: RunProvenance::Rules {},
        restore: false,
        uses,
    })
    .unwrap();
}

fn keys(p: &Project) -> Vec<CacheRef> {
    cache(p).entries().unwrap().iter().map(as_ref).collect()
}

fn gc(cap: u64) -> Gc {
    Gc {
        cap_bytes: cap,
        ..Gc::default()
    }
}

/// An hour from now: every leftover and broken directory made by a test is old by then.
fn later() -> SystemTime {
    SystemTime::now() + Duration::from_secs(3600)
}

fn whys(r: &Report) -> Vec<(String, &'static str)> {
    let mut out: Vec<_> = r.removed.iter().map(|x| (x.name.clone(), x.why)).collect();
    out.sort();
    out
}

fn stage_dir(p: &Project) -> PathBuf {
    p.cache_dir().join("song")
}

/// The names under `dir` that start with `.tmp`.
fn temps(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".tmp"))
        .collect()
}

fn temp_dirs_made() -> u64 {
    TEMP_DIRS_MADE.with(std::cell::Cell::get)
}

#[test]
fn the_defaults_are_a_4_gib_cap_and_ten_minutes() {
    assert_eq!(
        Gc::default(),
        Gc {
            cap_bytes: 4 * 1024 * 1024 * 1024,
            tmp_age: Duration::from_secs(600),
            dry_run: false,
        }
    );
}

#[test]
fn under_the_cap_nothing_goes() {
    let p = project("under");
    put(&p, 1, 100);
    put(&p, 2, 100);
    let r = gc(1000).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(r.removed, []);
    assert_eq!(r.unrooted_kept, (2, 200));
    assert_eq!(keys(&p).len(), 2);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn at_the_cap_nothing_goes_and_one_byte_over_the_oldest_does() {
    let p = project("boundary");
    let old = put(&p, 1, 100);
    let new = put(&p, 2, 100);
    created_at(&old, 10);
    created_at(&new, 20);
    let r = gc(200).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(r.removed, [], "a cache exactly at the cap fits");
    let r = gc(199).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(whys(&r), [(old.manifest.key.to_string(), "unrooted")]);
    assert_eq!(r.unrooted_kept, (1, 100));
    assert_eq!(keys(&p), [as_ref(&new)]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_cap_of_zero_removes_every_unrooted_entry_even_an_empty_one() {
    let p = project("zero");
    let empty = put(&p, 1, 0);
    let root = put(&p, 2, 0);
    record(&p, 1, vec![as_ref(&root)]);
    let r = gc(0).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(whys(&r), [(empty.manifest.key.to_string(), "unrooted")]);
    assert_eq!((r.rooted, r.unrooted_kept), ((1, 0), (0, 0)));
    assert_eq!(keys(&p), [as_ref(&root)]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn over_the_cap_unrooted_entries_go_oldest_first_and_roots_stay() {
    let p = project("over");
    let old = put(&p, 1, 100);
    let mid = put(&p, 2, 100);
    let new = put(&p, 3, 100);
    let root = put(&p, 4, 100);
    created_at(&old, 10);
    created_at(&mid, 20);
    created_at(&new, 30);
    created_at(&root, 5); // the oldest of all, but a recorded version uses it
    // An older version uses `root`; the latest uses nothing. All of the history counts, not just the latest.
    record(&p, 1, vec![as_ref(&root)]);
    record(&p, 2, vec![]);
    let r = gc(250).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(
        r.removed
            .iter()
            .map(|x| (x.name.clone(), x.why))
            .collect::<Vec<_>>(),
        [
            (old.manifest.key.to_string(), "unrooted"),
            (mid.manifest.key.to_string(), "unrooted")
        ]
    );
    assert_eq!((r.rooted, r.unrooted_kept), ((1, 100), (1, 100)));
    let mut left = keys(&p);
    left.sort();
    let mut want = vec![as_ref(&new), as_ref(&root)];
    want.sort();
    assert_eq!(left, want);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn roots_over_the_cap_are_kept_and_reported() {
    let p = project("roots");
    let a = put(&p, 1, 300);
    let b = put(&p, 2, 300);
    record(&p, 1, vec![as_ref(&a), as_ref(&b)]);
    let r = gc(100).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(r.removed, []);
    assert_eq!((r.rooted, r.roots_over_cap), ((2, 600), 500));
    // A root naming an entry the cache doesn't hold is counted, not an error.
    record(
        &p,
        2,
        vec![CacheRef {
            stage: "song".into(),
            key: ContentHash::from_hex(&"ee".repeat(32)).unwrap(),
        }],
    );
    let r = gc(100).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(r.missing_roots, 1);
    // Each source says how many distinct entries it names: the same one in two runs is one.
    assert_eq!(
        r.roots,
        [RootCount {
            source: "recorded versions".into(),
            refs: 3
        }]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_history_without_its_track_file_still_counts() {
    let p = project("orphan");
    let kept = put(&p, 1, 100);
    std::fs::create_dir_all(p.root().join("tracks/gone")).unwrap();
    std::fs::write(p.root().join("tracks/gone/track.toml"), "schema = 1\n").unwrap();
    record_on(&p, "gone", 1, vec![as_ref(&kept)]);
    std::fs::remove_file(p.root().join("tracks/gone/track.toml")).unwrap();
    // A stray file under tracks/ is no track and no error.
    std::fs::write(p.root().join("tracks/README"), "notes").unwrap();
    let r = gc(0).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!((r.removed.len(), r.rooted), (0, (1, 100)));
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_dry_run_removes_nothing() {
    let p = project("dry");
    put(&p, 1, 100);
    put(&p, 2, 100);
    let stage = stage_dir(&p);
    std::fs::create_dir_all(stage.join("not-an-entry")).unwrap();
    let before = temp_dirs_made();
    let r = Gc {
        dry_run: true,
        ..gc(50)
    }
    .collect_at(&p, &[&RecordedVersions], later())
    .unwrap();
    assert!(r.dry_run);
    assert_eq!(r.removed.len(), 3, "{:?}", r.removed);
    assert_eq!(keys(&p).len(), 2);
    assert!(stage.join("not-an-entry").is_dir());
    assert_eq!(temp_dirs_made(), before, "nothing is moved aside");
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn broken_entries_and_old_leftovers_go_young_ones_and_newer_entries_stay() {
    let p = project("broken");
    let good = put(&p, 1, 10);
    let damaged = put(&p, 2, 10);
    let newer = put(&p, 3, 10);
    std::fs::write(damaged.dir.join("out.bin"), b"changed").unwrap();
    // Its 7 changed bytes and its manifest.
    let damaged_bytes = 7 + std::fs::metadata(damaged.dir.join("manifest.json"))
        .unwrap()
        .len();
    let text = std::fs::read_to_string(newer.dir.join("manifest.json")).unwrap();
    std::fs::write(
        newer.dir.join("manifest.json"),
        text.replace("\"schema\": 2", "\"schema\": 3"),
    )
    .unwrap();
    let stage = stage_dir(&p);
    std::fs::create_dir_all(stage.join(".tmp-young")).unwrap();
    std::fs::create_dir_all(stage.join(".tmp-old")).unwrap();
    std::fs::write(stage.join(".tmp-old/half.bin"), b"half").unwrap();
    // Now, everything is young: the damaged entry may be a write in progress.
    let r = gc(1 << 20).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(r.removed, []);
    assert_eq!(r.newer, 1);
    // An hour on, with a ten-minute age, all of it is old.
    let report = gc(1 << 20)
        .collect_at(&p, &[&RecordedVersions], later())
        .unwrap();
    let mut why: Vec<_> = report
        .removed
        .iter()
        .map(|x| (x.name.clone(), x.why, x.bytes))
        .collect();
    why.sort();
    assert_eq!(
        why,
        [
            (".tmp-old".into(), "leftover", 4),
            (".tmp-young".into(), "leftover", 0),
            (damaged.manifest.key.to_string(), "broken", damaged_bytes),
        ]
    );
    assert_eq!(report.newer, 1);
    assert!(newer.dir.is_dir(), "a newer layout is left alone");
    assert_eq!(keys(&p), [as_ref(&good)]);
    assert_eq!(
        temps(&stage),
        Vec::<String>::new(),
        "nothing of GC's own is left"
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn ages_come_from_the_name_else_the_modification_time() {
    let p = project("ages");
    let stage = stage_dir(&p);
    std::fs::create_dir_all(&stage).unwrap();
    let t = 2_000_000_000u64;
    let at = epoch_plus(t);
    // Named with when they were made: a writer's, and a collector's aside.
    for name in [format!(".tmp-t{t}-1-0-0"), format!(".tmp-gc-t{t}-1-0-0")] {
        std::fs::create_dir_all(stage.join(&name)).unwrap();
    }
    let run = |age: u64, now: SystemTime| {
        Gc {
            tmp_age: Duration::from_secs(age),
            ..gc(u64::MAX)
        }
        .collect_at(&p, &[&RecordedVersions], now)
        .unwrap()
    };
    // Exactly 100 s old is not more than 100 s; the names' time counts, not the (recent) modification time.
    assert_eq!(run(100, at + Duration::from_secs(100)).removed, []);
    let r = run(99, at + Duration::from_secs(100));
    assert_eq!(
        whys(&r),
        [
            (format!(".tmp-gc-t{t}-1-0-0"), "leftover"),
            (format!(".tmp-t{t}-1-0-0"), "leftover")
        ]
    );
    // A name without a time (an older build's) and a broken directory go by their modification time.
    std::fs::create_dir_all(stage.join(".tmp-4242-0-1")).unwrap();
    std::fs::create_dir_all(stage.join("not-an-entry")).unwrap();
    let modified = |name: &str| {
        std::fs::metadata(stage.join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    let (a, b) = (modified("not-an-entry"), modified(".tmp-4242-0-1"));
    assert_eq!(run(100, a.min(b) + Duration::from_secs(100)).removed, []);
    let r = run(100, a.max(b) + Duration::from_secs(101));
    assert_eq!(
        whys(&r),
        [
            (".tmp-4242-0-1".into(), "leftover"),
            ("not-an-entry".into(), "broken")
        ]
    );
    // A name's time still to come (a clock ahead of this one) doesn't keep it young for ever: its modification time
    // counts instead.
    let ahead = format!(".tmp-t{}-2-0-0", t + 365 * 24 * 3600);
    std::fs::create_dir_all(stage.join(&ahead)).unwrap();
    let made = modified(&ahead);
    assert_eq!(run(100, made + Duration::from_secs(100)).removed, []);
    assert_eq!(
        whys(&run(100, made + Duration::from_secs(101))),
        [(ahead, "leftover")]
    );
    let _ = std::fs::remove_dir_all(p.root());
}

fn epoch_plus(secs: u64) -> SystemTime {
    std::time::UNIX_EPOCH + Duration::from_secs(secs)
}

#[test]
fn temporary_names_carry_when_they_were_made() {
    let name = temp_name(".tmp-");
    let made = made_at(&name).unwrap();
    let now = SystemTime::now();
    assert!(
        now.duration_since(made).unwrap() < Duration::from_secs(5),
        "{name}"
    );
    assert_ne!(
        temp_name(".tmp-"),
        temp_name(".tmp-"),
        "never the same name"
    );
    assert!(temp_name(".tmp-gc-").starts_with(".tmp-gc-t"));
    for (name, secs) in [
        (".tmp-t123-1-2-3", Some(123)),
        (".tmp-gc-t123-x", Some(123)),
        (".tmp-123-4-5", None),
        (".tmp-t-1", None),
        (".tmp-t12a-1", None),
        (".tmp-t12", None),
        (".tmp-t12-", None),
        ("t123-1", None),
    ] {
        assert_eq!(
            made_at(name),
            secs.map(epoch_plus),
            "{name}: {:?}",
            made_at(name)
        );
    }
}

#[test]
fn odd_stage_folders_and_stray_files_are_skipped() {
    let p = project("odd");
    let root = p.cache_dir();
    let key = put(&p, 1, 10).manifest.key.to_string();
    for dir in [".hidden/junk", "bad name/junk", "con/junk"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("notes.txt"), "x").unwrap();
    std::fs::write(stage_dir(&p).join("stray.txt"), "x").unwrap();
    std::fs::write(stage_dir(&p).join("ab".repeat(32)), "x").unwrap();
    let r = Gc {
        tmp_age: Duration::ZERO,
        ..gc(u64::MAX)
    }
    .collect_at(&p, &[&RecordedVersions], later())
    .unwrap();
    assert_eq!(r.removed, []);
    for path in [
        ".hidden/junk",
        "bad name/junk",
        "con/junk",
        "notes.txt",
        "song/stray.txt",
        &format!("song/{key}"),
    ] {
        assert!(root.join(path).exists(), "{path}");
    }
    assert!(root.join("song").join("ab".repeat(32)).is_file());
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_leftovers_size_counts_its_nested_folders() {
    let p = project("nested");
    let stage = stage_dir(&p);
    let left = stage.join(".tmp-t1-1-0-0");
    std::fs::create_dir_all(left.join("a/b")).unwrap();
    std::fs::write(left.join("top.bin"), b"abc").unwrap();
    std::fs::write(left.join("a/b/deep.bin"), b"defghij").unwrap();
    let r = gc(u64::MAX).collect(&p, &[&RecordedVersions]).unwrap();
    assert_eq!(r.removed.len(), 1);
    assert_eq!(r.removed[0].bytes, 10);
    assert!(!left.exists());
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_vanished_entry_is_not_removed_again() {
    let p = project("vanished");
    let stage = stage_dir(&p);
    std::fs::create_dir_all(&stage).unwrap();
    let gone = stage.join("ab".repeat(32));
    assert!(matches!(gc(0).remove(&stage, &gone, None), Gone::Vanished));
    assert!(matches!(
        gc(0).remove(&stage, &gone, Some(("song", ContentHash([0xab; 32])))),
        Gone::Vanished
    ));
    assert_eq!(
        temps(&stage),
        Vec::<String>::new(),
        "its aside name is freed"
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn an_entry_that_cant_be_moved_aside_stays_whole_and_is_in_use() {
    let p = project("inuse");
    let e = put(&p, 1, 10);
    let stage = stage_dir(&p);
    let refuse = |_: &Path, _: &Path| Err(std::io::Error::other("a file is open in it"));
    let delete = |d: &Path| std::fs::remove_dir_all(d);
    assert!(matches!(
        gc(0).remove_with(&stage, &e.dir, None, refuse, delete),
        Gone::InUse
    ));
    assert_eq!(keys(&p), [as_ref(&e)]);
    assert_eq!(temps(&stage), Vec::<String>::new());
    let not_found = |_: &Path, _: &Path| Err(std::io::ErrorKind::NotFound.into());
    assert!(matches!(
        gc(0).remove_with(&stage, &e.dir, None, not_found, delete),
        Gone::Vanished
    ));
    // A leftover that can't be moved stays too: nothing is deleted in place.
    let left = stage.join(".tmp-t1-1-0-0");
    std::fs::create_dir_all(&left).unwrap();
    assert!(matches!(
        gc(0).remove_with(&stage, &left, None, refuse, delete),
        Gone::InUse
    ));
    assert!(left.is_dir());
    // Moved aside but not deleted: off its key, so removed, with the error to stop on.
    let stuck = |_: &Path| Err(std::io::Error::other("read-only"));
    let std_rename = |a: &Path, b: &Path| std::fs::rename(a, b);
    assert!(matches!(
        gc(0).remove_with(&stage, &e.dir, None, std_rename, stuck),
        Gone::Aside(Error::Io { .. })
    ));
    assert_eq!(keys(&p), []);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_broken_entry_put_whole_meanwhile_stays() {
    let p = project("refixed");
    let c = cache(&p);
    let e = put(&p, 1, 10);
    std::fs::write(e.dir.join("out.bin"), b"damaged").unwrap();
    let (_, other) = list(&c, &p.cache_dir()).unwrap();
    assert_eq!(other.len(), 1, "listed as broken");
    // A writer puts it again, replacing the damaged copy, before the collector gets to it.
    put(&p, 1, 10);
    let before = temp_dirs_made();
    let mut report = Report::default();
    gc(0).sweep_others(&c, other, later(), &mut report).unwrap();
    assert_eq!(report.removed, []);
    assert_eq!(
        temp_dirs_made(),
        before,
        "checked again before moving anything"
    );
    assert_eq!(keys(&p), [as_ref(&e)]);
    // Moved aside and found whole, it goes back.
    let stage = stage_dir(&p);
    assert!(matches!(
        gc(0).remove(&stage, &e.dir, Some(("song", e.manifest.key))),
        Gone::Whole
    ));
    assert_eq!(keys(&p), [as_ref(&e)]);
    assert_eq!(temps(&stage), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn an_unrooted_entry_damaged_meanwhile_is_not_counted_as_removed() {
    let p = project("redamaged");
    let c = cache(&p);
    let e = put(&p, 1, 10);
    let (whole, _) = list(&c, &p.cache_dir()).unwrap();
    std::fs::write(e.dir.join("out.bin"), b"damaged").unwrap();
    let before = temp_dirs_made();
    let mut report = Report::default();
    gc(0)
        .sweep_unrooted(&BTreeSet::new(), whole, &mut report)
        .unwrap();
    assert_eq!(report.removed, []);
    assert_eq!(temp_dirs_made(), before);
    assert!(e.dir.is_dir(), "left for the broken-entry pass");
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn entries_put_while_a_collection_runs_are_never_lost() {
    let p = project("race");
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        let writer = s.spawn(|| {
            let made: Vec<Entry> = (0..150).map(|n| put(&p, n, 64)).collect();
            done.store(true, Ordering::Release);
            made
        });
        let mut runs = 0;
        while !done.load(Ordering::Acquire) || runs == 0 {
            let r = gc(u64::MAX).collect(&p, &[&RecordedVersions]).unwrap();
            assert_eq!(r.removed, [], "under an endless cap nothing goes");
            runs += 1;
        }
        let made = writer.join().unwrap();
        for e in &made {
            assert!(
                cache(&p).get("song", e.manifest.key).is_some(),
                "lost {e:?}"
            );
        }
    });
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn entries_put_while_a_collection_sweeps_everything_old_are_never_lost() {
    // Every temporary directory counts as old, so the collector may take a writer's half-written copy, failing its
    // put; but no put that succeeded loses its entry.
    let p = project("race-old");
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        let writer = s.spawn(|| {
            let c = cache(&p);
            let made: Vec<Entry> = (0..150)
                .filter_map(|n| {
                    c.put("song", &[("out.bin", &[n as u8; 64])], stage_run(n))
                        .ok()
                })
                .collect();
            done.store(true, Ordering::Release);
            made
        });
        let sweep = Gc {
            tmp_age: Duration::ZERO,
            ..gc(u64::MAX)
        };
        while !done.load(Ordering::Acquire) {
            // A writer adding files to a directory being deleted can fail the deletion; that's a stop, not a loss.
            if let Ok(r) = sweep.collect_at(&p, &[&RecordedVersions], later()) {
                assert!(r.removed.iter().all(|x| x.why == "leftover"), "{r:?}");
            }
        }
        for e in &writer.join().unwrap() {
            assert!(
                cache(&p).get("song", e.manifest.key).is_some(),
                "lost {e:?}"
            );
        }
    });
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_failing_root_source_removes_nothing() {
    struct Broken;
    impl Roots for Broken {
        fn name(&self) -> &str {
            "approvals"
        }
        fn refs(&self, _: &Project) -> Result<Vec<CacheRef>, SourceError> {
            Err("tracks/a/approvals.toml: unreadable".into())
        }
    }
    let p = project("fail");
    put(&p, 1, 100);
    let e = gc(0)
        .collect(&p, &[&RecordedVersions, &Broken])
        .unwrap_err();
    let text = e.to_string();
    assert!(
        text.contains("approvals")
            && text.contains("tracks/a/approvals.toml: unreadable")
            && text.contains("nothing removed"),
        "{text}"
    );
    assert!(!e.changed() && e.report.removed.is_empty());
    assert_eq!(keys(&p).len(), 1);
    // An unreadable history is the same, and says which file.
    std::fs::create_dir_all(p.root().join("tracks/a/history")).unwrap();
    std::fs::write(p.root().join("tracks/a/history/notes.toml"), "x").unwrap();
    let e = gc(0).collect(&p, &[&RecordedVersions]).unwrap_err();
    assert!(
        e.to_string().contains("recorded versions") && e.to_string().contains("notes.toml"),
        "{e}"
    );
    assert_eq!(keys(&p).len(), 1);
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn another_source_of_roots_adds_to_them() {
    struct Pinned(CacheRef);
    impl Roots for Pinned {
        fn name(&self) -> &str {
            "pinned"
        }
        fn refs(&self, _: &Project) -> Result<Vec<CacheRef>, SourceError> {
            Ok(vec![self.0.clone(), self.0.clone()])
        }
    }
    let p = project("extra");
    let kept = put(&p, 1, 100);
    put(&p, 2, 100);
    let pinned = Pinned(as_ref(&kept));
    let r = gc(0).collect(&p, &[&RecordedVersions, &pinned]).unwrap();
    assert_eq!(r.rooted, (1, 100));
    assert_eq!(
        r.roots,
        [
            RootCount {
                source: "recorded versions".into(),
                refs: 0
            },
            RootCount {
                source: "pinned".into(),
                refs: 1
            }
        ]
    );
    assert_eq!(keys(&p), [as_ref(&kept)]);
    let _ = std::fs::remove_dir_all(p.root());
}

#[cfg(unix)]
#[test]
fn links_are_never_followed() {
    use std::os::unix::fs::symlink;
    let p = project("links");
    let outside = p.root().join("outside");
    // A stage that is a link to a folder outside the cache, holding an entry, a broken folder and a leftover.
    let far = Cache::new(outside.join("far"));
    let far_entry = put_in(&far, "linked", 1, 10);
    std::fs::create_dir_all(outside.join("far/linked/not-an-entry")).unwrap();
    std::fs::create_dir_all(outside.join("far/linked/.tmp-t1-1-0-0")).unwrap();
    std::fs::create_dir_all(p.cache_dir()).unwrap();
    symlink(outside.join("far/linked"), p.cache_dir().join("linked")).unwrap();
    // An entry that is a link to an entry outside.
    let other = Cache::new(outside.join("other"));
    let linked_entry = put_in(&other, "song", 2, 10);
    let own = put(&p, 3, 10);
    symlink(
        &linked_entry.dir,
        stage_dir(&p).join(linked_entry.manifest.key.to_string()),
    )
    .unwrap();
    // A leftover holding a link to a big file outside.
    std::fs::write(outside.join("big.bin"), vec![0u8; 1000]).unwrap();
    let left = stage_dir(&p).join(".tmp-t1-1-0-1");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::write(left.join("own.bin"), b"abc").unwrap();
    symlink(outside.join("big.bin"), left.join("big.bin")).unwrap();
    symlink(&outside, left.join("all")).unwrap();
    let r = Gc {
        tmp_age: Duration::ZERO,
        ..gc(0)
    }
    .collect_at(&p, &[&RecordedVersions], later())
    .unwrap();
    let mut got: Vec<_> = r
        .removed
        .iter()
        .map(|x| (x.stage.clone(), x.name.clone(), x.bytes))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("song".into(), ".tmp-t1-1-0-1".into(), 3),
            ("song".into(), own.manifest.key.to_string(), 10)
        ]
    );
    assert_eq!(r.unrooted_kept, (0, 0), "linked entries aren't counted");
    assert!(far.get("linked", far_entry.manifest.key).is_some());
    assert!(outside.join("far/linked/not-an-entry").is_dir());
    assert!(outside.join("far/linked/.tmp-t1-1-0-0").is_dir());
    assert!(other.get("song", linked_entry.manifest.key).is_some());
    assert_eq!(
        std::fs::metadata(outside.join("big.bin")).unwrap().len(),
        1000
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[cfg(unix)]
#[test]
fn an_error_after_a_removal_comes_with_the_report_so_far() {
    use std::os::unix::fs::PermissionsExt;
    let p = project("stopped");
    let c = cache(&p);
    let first = put_in(&c, "a", 1, 10);
    let second = put_in(&c, "b", 2, 10);
    created_at(&first, 10);
    created_at(&second, 20);
    let locked = p.cache_dir().join("b");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::create_dir(locked.join("probe")).is_ok() {
        // Permissions don't bind this user (root): nothing to test here.
        let _ = std::fs::remove_dir(locked.join("probe"));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(p.root());
        return;
    }
    let e = gc(0).collect(&p, &[&RecordedVersions]).unwrap_err();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(e.error, Error::Io { .. }), "{e}");
    assert!(e.changed());
    assert_eq!(
        whys(&e.report),
        [(first.manifest.key.to_string(), "unrooted")]
    );
    assert_eq!(keys(&p), [as_ref(&second)], "the other stays whole");
    let _ = std::fs::remove_dir_all(p.root());
}

#[cfg(unix)]
#[test]
fn an_entry_moved_aside_but_not_deleted_is_reported_removed_and_stops() {
    use std::os::unix::fs::PermissionsExt;
    let p = project("stuck");
    let e = put(&p, 1, 10);
    // A read-only folder inside the entry: renaming the entry aside works, deleting what's in that folder doesn't.
    let locked = e.dir.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("x"), b"x").unwrap();
    let mode =
        |dir: &Path, m| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(m)).unwrap();
    mode(&locked, 0o555);
    if std::fs::remove_file(locked.join("x")).is_ok() {
        // Permissions don't bind this user (root): nothing to test here.
        mode(&locked, 0o755);
        let _ = std::fs::remove_dir_all(p.root());
        return;
    }
    let stopped = gc(0).collect(&p, &[&RecordedVersions]).unwrap_err();
    for t in temps(&stage_dir(&p)) {
        let _ = std::fs::set_permissions(
            stage_dir(&p).join(t).join("locked"),
            std::fs::Permissions::from_mode(0o755),
        );
    }
    assert!(matches!(stopped.error, Error::Io { .. }), "{stopped}");
    assert!(stopped.changed());
    assert_eq!(
        whys(&stopped.report),
        [(e.manifest.key.to_string(), "unrooted")]
    );
    assert_eq!(keys(&p), [], "off its key all the same");
    let _ = std::fs::remove_dir_all(p.root());
}
