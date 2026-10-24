//! Run history: UTC times, recording rules, file names and order, and reading back with its checks.

use std::sync::atomic::{AtomicU32, Ordering};

use proptest::prelude::*;

use super::*;

/// A fresh, empty folder for one test.
fn folder(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("score64-history-{what}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn version(byte: u8) -> VersionId {
    VersionId::from_hex(&format!("{byte:02x}").repeat(32)).unwrap()
}

fn at(seconds: i64) -> UtcTime {
    UtcTime::from_unix(seconds).unwrap()
}

/// 2026-10-24T09:15:02Z.
const T: i64 = 1_792_833_302;

fn run(v: u8, time: i64) -> Run {
    Run {
        version: version(v),
        time: at(time),
        provenance: RunProvenance::Manual {
            name: "A. Person".into(),
        },
        restore: false,
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    n.sort();
    n
}

#[test]
fn utc_times_are_civil_dates_in_whole_seconds() {
    assert_eq!(at(0).to_string(), "1970-01-01T00:00:00Z");
    assert_eq!(at(T).to_string(), "2026-10-24T09:15:02Z");
    assert_eq!(at(1_709_251_199).to_string(), "2024-02-29T23:59:59Z");
    assert_eq!(at(253_402_300_799).to_string(), "9999-12-31T23:59:59Z");
    assert_eq!(UtcTime::from_unix(253_402_300_800), None);
    assert_eq!(UtcTime::from_unix(-1), None);
    assert_eq!(at(T).name_part(), "2026-10-24T091502Z");
}

proptest! {
    #[test]
    fn a_time_survives_toml(seconds in 0i64..=253_402_300_799) {
        let t = at(seconds);
        let back = UtcTime::try_from(Datetime::from(t)).unwrap();
        prop_assert_eq!(back, t);
    }
}

#[test]
fn only_utc_whole_seconds_are_times() {
    #[derive(Deserialize)]
    struct W {
        #[allow(dead_code)]
        t: UtcTime,
    }
    let read = |s: &str| toml::from_str::<W>(&format!("t = {s}")).map(|_| ());
    assert!(read("2026-10-24T09:15:02Z").is_ok());
    assert!(read("2026-10-24T09:15:02.000Z").is_ok());
    for bad in [
        "2026-10-24T09:15:02",
        "2026-10-24T09:15:02+02:00",
        "2026-10-24T09:15:02.5Z",
        "2026-10-24",
        "09:15:02",
        "2026-02-30T00:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert!(read(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_run_is_recorded_only_when_its_version_changes() {
    let dir = folder("record");
    let mut h = History::read(&dir).unwrap();
    assert_eq!(h.latest(), None, "no folder is an empty history");
    let first = h.record(run(1, T)).unwrap().unwrap();
    assert_eq!(
        first.file_name().unwrap(),
        "2026-10-24T091502Z-010101010101.toml"
    );
    // The same version again: nothing written.
    assert_eq!(h.record(run(1, T + 60)).unwrap(), None);
    // A new version names the latest as its parent.
    h.record(run(2, T + 120)).unwrap().unwrap();
    let latest = h.latest().unwrap();
    assert_eq!(latest.parent, Some(version(1)));
    assert_eq!(names(&dir).len(), 2);
    // Reading back gives the same history.
    assert_eq!(History::read(&dir).unwrap(), h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_entry_is_written_the_same_way_every_time() {
    let dir = folder("text");
    let mut h = History::read(&dir).unwrap();
    h.record(run(1, T)).unwrap();
    let mut llm = run(2, T + 1);
    llm.provenance = RunProvenance::Llm {
        model: "some-model".into(),
    };
    let path = h.record(llm).unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        format!(
            "schema = 1\nversion = \"{}\"\nparent = \"{}\"\ntime = 2026-10-24T09:15:03Z\n\n[provenance]\nby = \"llm\"\nmodel = \"some-model\"\n",
            version(2),
            version(1)
        )
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_restore_is_recorded_with_the_current_version_as_parent() {
    let dir = folder("restore");
    let mut h = History::read(&dir).unwrap();
    h.record(run(1, T)).unwrap();
    h.record(run(2, T + 10)).unwrap();
    let restore = |v: u8, time: i64| Run {
        restore: true,
        ..run(v, time)
    };
    // Back to version 1: recorded although it was seen before.
    h.record(restore(1, T + 20)).unwrap().unwrap();
    let latest = h.latest().unwrap();
    assert!(latest.restore);
    assert_eq!(
        (latest.version, latest.parent),
        (version(1), Some(version(2)))
    );
    // Restoring the version already current writes nothing.
    assert_eq!(h.record(restore(1, T + 30)).unwrap(), None);
    // A version never recorded can't be restored.
    assert!(matches!(
        h.record(restore(9, T + 40)),
        Err(Error::Refused(_))
    ));
    assert_eq!(History::read(&dir).unwrap(), h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runs_in_one_second_are_numbered_and_keep_their_order() {
    let dir = folder("second");
    let mut h = History::read(&dir).unwrap();
    for (v, n) in [(1u8, ""), (2, "-2"), (3, "-3"), (1, "-4")] {
        let mut r = run(v, T);
        r.restore = v == 1 && n == "-4";
        let path = h.record(r).unwrap().unwrap();
        let hex = format!("{v:02x}").repeat(6);
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!("2026-10-24T091502Z-{hex}{n}.toml")
        );
    }
    let back = History::read(&dir).unwrap();
    let order: Vec<VersionId> = back.entries().map(|(_, e)| e.version).collect();
    assert_eq!(order, [version(1), version(2), version(3), version(1)]);
    assert_eq!(back, h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn some_runs_are_refused() {
    let dir = folder("refused");
    let mut h = History::read(&dir).unwrap();
    h.record(run(1, T)).unwrap();
    // The clock went back.
    assert!(matches!(h.record(run(2, T - 1)), Err(Error::Refused(_))));
    // A provenance naming no one.
    for provenance in [
        RunProvenance::Llm { model: " ".into() },
        RunProvenance::Manual { name: "".into() },
    ] {
        let r = Run {
            provenance,
            ..run(2, T + 1)
        };
        assert!(matches!(h.record(r), Err(Error::Refused(_))));
    }
    assert_eq!(names(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A history folder holding one recorded run, and that entry's file.
fn one_entry(what: &str) -> (PathBuf, PathBuf) {
    let dir = folder(what);
    let mut h = History::read(&dir).unwrap();
    let path = h.record(run(1, T)).unwrap().unwrap();
    (dir, path)
}

fn read_error(dir: &Path) -> String {
    History::read(dir).unwrap_err().to_string()
}

#[test]
fn reading_refuses_anything_that_isnt_one_history() {
    // A file that isn't an entry.
    let (dir, _) = one_entry("stray");
    std::fs::write(dir.join("notes.txt"), "hello").unwrap();
    assert!(read_error(&dir).contains("not a history entry"));
    let _ = std::fs::remove_dir_all(&dir);

    // A writer's leftover part file is skipped.
    let (dir, _) = one_entry("part");
    std::fs::write(dir.join("x.toml.123-0.part"), "half").unwrap();
    assert_eq!(History::read(&dir).unwrap().entries().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);

    // A name that doesn't match the content.
    let (dir, path) = one_entry("rename");
    std::fs::rename(&path, dir.join("2026-10-24T091503Z-010101010101.toml")).unwrap();
    assert!(read_error(&dir).contains("doesn't match"));
    let _ = std::fs::remove_dir_all(&dir);

    // An unknown field, with its line.
    let (dir, path) = one_entry("field");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace("schema = 1\n", "schema = 1\ncolour = \"red\"\n"),
    )
    .unwrap();
    let e = read_error(&dir);
    assert!(e.contains("colour") && e.contains(":2:"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);

    // An unknown field inside a provenance with none of its own.
    let (dir, path) = one_entry("rules");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace(
            "by = \"manual\"\nname = \"A. Person\"",
            "by = \"rules\"\nname = \"A. Person\"",
        ),
    )
    .unwrap();
    assert!(read_error(&dir).contains("name"));
    let _ = std::fs::remove_dir_all(&dir);

    // A newer schema.
    let (dir, path) = one_entry("schema");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("schema = 1", "schema = 2")).unwrap();
    assert!(read_error(&dir).contains("schema 2"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reading_checks_the_chain() {
    // Write entries by hand: each a file with the right name for its content.
    let write = |dir: &Path, name: &str, entry: &Entry| {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), toml::to_string(entry).unwrap()).unwrap();
    };
    let entry = |v: u8, parent: Option<u8>, time: i64, restore: bool| Entry {
        schema: Schema,
        version: version(v),
        parent: parent.map(version),
        time: at(time),
        restore,
        provenance: RunProvenance::Rules {},
    };
    let name = |v: u8, time: i64, n: &str| {
        format!(
            "{}-{}{n}.toml",
            at(time).name_part(),
            format!("{v:02x}").repeat(6)
        )
    };

    // A first entry with a parent.
    let dir = folder("orphan");
    write(&dir, &name(1, T, ""), &entry(1, Some(9), T, false));
    assert!(read_error(&dir).contains("first entry has no parent"));
    let _ = std::fs::remove_dir_all(&dir);

    // A parent that isn't the version before.
    let dir = folder("chain");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(2, T + 1, ""), &entry(2, Some(3), T + 1, false));
    assert!(read_error(&dir).contains("parent should be"));
    let _ = std::fs::remove_dir_all(&dir);

    // The same version twice, not as a restore.
    let dir = folder("repeat");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(1, T + 1, ""), &entry(1, Some(1), T + 1, false));
    assert!(read_error(&dir).contains("same version"));
    let _ = std::fs::remove_dir_all(&dir);

    // A restore of a version never recorded.
    let dir = folder("ghost");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(5, T + 1, ""), &entry(5, Some(1), T + 1, true));
    assert!(read_error(&dir).contains("never recorded"));
    let _ = std::fs::remove_dir_all(&dir);

    // Two entries with one time and number.
    let dir = folder("twins");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(2, T, ""), &entry(2, Some(1), T, false));
    assert!(read_error(&dir).contains("same time and number"));
    let _ = std::fs::remove_dir_all(&dir);

    // Numbered entries order by number, -10 after -9, whatever the file system lists first.
    let dir = folder("ten");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    for k in 2..=10u8 {
        write(
            &dir,
            &name(k, T, &format!("-{k}")),
            &entry(k, Some(k - 1), T, false),
        );
    }
    let h = History::read(&dir).unwrap();
    assert_eq!(h.latest().unwrap().version, version(10));
    let _ = std::fs::remove_dir_all(&dir);
}
