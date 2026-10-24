//! Cache GC: roots stay over the cap, the rest goes oldest-first, broken entries and old leftovers go, newer ones stay.

use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;

use super::*;
use crate::cache::{Entry, Run as StageRun};
use crate::history::{Run, RunProvenance};
use crate::project::Id;
use crate::version::VersionId;

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
    Cache::new(p.root().join(".score64/cache"))
}

/// An entry of `bytes` bytes, made with config `n`.
fn put(p: &Project, n: u32, bytes: usize) -> Entry {
    let run = StageRun {
        stage_version: "1".into(),
        tool_version: "0.1.0".into(),
        config: json!({ "n": n }),
        inputs: vec![],
        millis: 1,
        cost: None,
    };
    cache(p)
        .put("song", &[("out.bin", &vec![n as u8; bytes])], run)
        .unwrap()
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
    let mut h = p.history(&Id::new("a").unwrap()).unwrap();
    h.record(Run {
        version: VersionId::from_hex(&format!("{v:02x}").repeat(32)).unwrap(),
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
            key: VersionId::from_hex(&"ee".repeat(32)).unwrap(),
        }],
    );
    assert_eq!(
        gc(100)
            .collect(&p, &[&RecordedVersions])
            .unwrap()
            .missing_roots,
        1
    );
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_dry_run_removes_nothing() {
    let p = project("dry");
    put(&p, 1, 100);
    put(&p, 2, 100);
    let stage = p.root().join(".score64/cache/song");
    std::fs::create_dir_all(stage.join("not-an-entry")).unwrap();
    let r = Gc {
        dry_run: true,
        ..gc(50)
    }
    .collect(&p, &[&RecordedVersions])
    .unwrap();
    assert!(r.dry_run);
    assert_eq!(r.removed.len(), 3, "{:?}", r.removed);
    assert_eq!(keys(&p).len(), 2);
    assert!(stage.join("not-an-entry").is_dir());
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn broken_entries_and_old_leftovers_go_young_ones_and_newer_entries_stay() {
    let p = project("broken");
    let good = put(&p, 1, 10);
    let damaged = put(&p, 2, 10);
    let newer = put(&p, 3, 10);
    std::fs::write(damaged.dir.join("out.bin"), b"changed").unwrap();
    let text = std::fs::read_to_string(newer.dir.join("manifest.json")).unwrap();
    std::fs::write(
        newer.dir.join("manifest.json"),
        text.replace("\"schema\": 2", "\"schema\": 3"),
    )
    .unwrap();
    let stage = p.root().join(".score64/cache/song");
    std::fs::create_dir_all(stage.join(".tmp-young")).unwrap();
    std::fs::create_dir_all(stage.join(".tmp-old")).unwrap();
    std::fs::write(stage.join(".tmp-old/half.bin"), b"half").unwrap();
    // Old enough with an age of zero; young with an hour.
    let r = Gc {
        tmp_age: Duration::ZERO,
        ..gc(1 << 20)
    };
    std::thread::sleep(Duration::from_millis(20));
    let report = Gc {
        tmp_age: Duration::from_secs(3600),
        ..r.clone()
    }
    .collect(&p, &[&RecordedVersions])
    .unwrap();
    let mut why: Vec<_> = report
        .removed
        .iter()
        .map(|x| (x.name.clone(), x.why))
        .collect();
    why.sort();
    assert_eq!(why, [(damaged.manifest.key.to_string(), "broken")]);
    assert_eq!(report.newer, 1);
    assert!(stage.join(".tmp-young").is_dir() && stage.join(".tmp-old").is_dir());
    let report = r.collect(&p, &[&RecordedVersions]).unwrap();
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
            (".tmp-young".into(), "leftover", 0)
        ]
    );
    assert!(newer.dir.is_dir(), "a newer layout is left alone");
    assert_eq!(keys(&p), [as_ref(&good)]);
    // Nothing of GC's own is left behind.
    let left: Vec<String> = std::fs::read_dir(&stage)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".tmp"))
        .collect();
    assert_eq!(left, Vec::<String>::new());
    let _ = std::fs::remove_dir_all(p.root());
}

#[test]
fn a_failing_root_source_removes_nothing() {
    struct Broken;
    impl Roots for Broken {
        fn name(&self) -> &str {
            "broken"
        }
        fn refs(&self, _: &Project) -> Result<Vec<CacheRef>, Error> {
            Err(Error::Roots("unreadable approvals".into()))
        }
    }
    let p = project("fail");
    put(&p, 1, 100);
    let e = gc(0)
        .collect(&p, &[&RecordedVersions, &Broken])
        .unwrap_err();
    assert!(e.to_string().contains("nothing removed"), "{e}");
    assert_eq!(keys(&p).len(), 1);
    // An unreadable history is the same.
    std::fs::create_dir_all(p.root().join("tracks/a/history")).unwrap();
    std::fs::write(p.root().join("tracks/a/history/notes.toml"), "x").unwrap();
    assert!(gc(0).collect(&p, &[&RecordedVersions]).is_err());
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
        fn refs(&self, _: &Project) -> Result<Vec<CacheRef>, Error> {
            Ok(vec![self.0.clone()])
        }
    }
    let p = project("extra");
    let kept = put(&p, 1, 100);
    put(&p, 2, 100);
    let pinned = Pinned(as_ref(&kept));
    let r = gc(0).collect(&p, &[&RecordedVersions, &pinned]).unwrap();
    assert_eq!(r.rooted, (1, 100));
    assert_eq!(keys(&p), [as_ref(&kept)]);
    let _ = std::fs::remove_dir_all(p.root());
}
