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
        uses: vec![],
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
        Err(Error::NotRecorded(_))
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
    // A provenance naming no one.
    for provenance in [
        RunProvenance::Llm { model: " ".into() },
        RunProvenance::Manual { name: "".into() },
    ] {
        let r = Run {
            provenance,
            ..run(2, T + 1)
        };
        assert!(matches!(h.record(r), Err(Error::NoOne(_))));
    }
    // A stage the cache can't hold.
    let r = Run {
        uses: vec![CacheRef {
            stage: "../out".into(),
            key: version(7),
        }],
        ..run(2, T + 1)
    };
    assert!(matches!(h.record(r), Err(Error::BadStage(_))));
    assert_eq!(names(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clock_behind_the_latest_run_records_at_the_latest_time() {
    // Another machine's clock was ahead: its run is at T + 1000.
    let dir = folder("skew");
    let mut h = History::read(&dir).unwrap();
    h.record(run(1, T + 1000)).unwrap();
    let path = h.record(run(2, T)).unwrap().unwrap();
    let latest = h.latest().unwrap();
    assert_eq!((latest.version, latest.time), (version(2), at(T + 1000)));
    assert!(
        path.to_string_lossy().ends_with("-020202020202-2.toml"),
        "{path:?}"
    );
    assert_eq!(
        History::read(&dir).unwrap().latest().unwrap().version,
        version(2)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uses_are_sorted_and_kept() {
    let dir = folder("uses");
    let mut h = History::read(&dir).unwrap();
    let r = |stage: &str, k: u8| CacheRef {
        stage: stage.into(),
        key: version(k),
    };
    let path = h
        .record(Run {
            uses: vec![r("song", 2), r("transcribe", 1), r("song", 2), r("song", 1)],
            ..run(1, T)
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        h.latest().unwrap().uses,
        [r("song", 1), r("song", 2), r("transcribe", 1)]
    );
    assert_eq!(History::read(&dir).unwrap(), h);
    // Out of order on disk is refused, pointing at the list.
    let text = std::fs::read_to_string(&path).unwrap();
    let swapped = text.replacen("stage = \"song\"", "stage = \"zzz\"", 1);
    std::fs::write(&path, swapped).unwrap();
    let e = read_error(&dir);
    assert!(e.contains("sorted") && e.contains(":"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_writers_in_one_second_both_read_back() {
    // Both read the folder while it was empty.
    let dir = folder("race");
    let mut first = History::read(&dir).unwrap();
    let mut second = History::read(&dir).unwrap();
    first.record(run(1, T)).unwrap().unwrap();
    // The second reads the folder again before writing, so it follows the first.
    let path = second.record(run(2, T)).unwrap().unwrap();
    assert!(
        path.to_string_lossy().ends_with("-020202020202-2.toml"),
        "{path:?}"
    );
    assert_eq!(second.latest().unwrap().parent, Some(version(1)));
    let back = History::read(&dir).unwrap();
    assert_eq!(back.entries().count(), 2);
    // A writer recording a version another one just recorded writes nothing.
    assert_eq!(first.record(run(2, T + 5)).unwrap(), None);
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
fn reading_skips_other_files_and_refuses_bad_entries() {
    // Files the system or an editor leaves are skipped: dot-files, and names not ending in `.toml`.
    let (dir, _) = one_entry("stray");
    for other in [
        ".DS_Store",
        "Thumbs.db",
        "desktop.ini",
        "notes.txt",
        "x.toml~",
        ".x.toml.swp",
        "a.toml.1-0.part",
    ] {
        std::fs::write(dir.join(other), "x").unwrap();
    }
    assert_eq!(History::read(&dir).unwrap().entries().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);

    // A `.toml` that isn't named as an entry is refused: hex in capitals, numbers that aren't -2 to -9999.
    for bad in [
        "notes.toml",
        "2026-10-24T091502Z-0101010101AB.toml",
        "2026-10-24T091502Z-010101010101-1.toml",
        "2026-10-24T091502Z-010101010101-01.toml",
        "2026-10-24T091502Z-010101010101-+2.toml",
        "2026-10-24T091502Z-010101010101-10000.toml",
        "2026-10-24T091502Z-010101010101-4294967295.toml",
    ] {
        let (dir, path) = one_entry("badname");
        std::fs::copy(&path, dir.join(bad)).unwrap();
        assert!(read_error(&dir).contains("not a history entry"), "{bad}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A folder named like an entry.
    let (dir, _) = one_entry("folder");
    std::fs::create_dir(dir.join("2026-10-24T091503Z-020202020202.toml")).unwrap();
    assert!(read_error(&dir).contains("a folder"));
    let _ = std::fs::remove_dir_all(&dir);

    // A name that doesn't match the content: its time, or its version.
    for other in [
        "2026-10-24T091503Z-010101010101.toml",
        "2026-10-24T091502Z-020202020202.toml",
    ] {
        let (dir, path) = one_entry("rename");
        std::fs::rename(&path, dir.join(other)).unwrap();
        assert!(read_error(&dir).contains("doesn't match"), "{other}");
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    // An unknown field inside a provenance with none of its own, and a provenance naming no one.
    for (from, to, says) in [
        (
            "by = \"manual\"\nname = \"A. Person\"",
            "by = \"rules\"\nname = \"A. Person\"",
            "name",
        ),
        ("name = \"A. Person\"", "name = \" \"", "names no one"),
    ] {
        let (dir, path) = one_entry("prov");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace(from, to)).unwrap();
        assert!(read_error(&dir).contains(says), "{to}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A newer schema.
    let (dir, path) = one_entry("schema");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("schema = 1", "schema = 2")).unwrap();
    assert!(read_error(&dir).contains("schema 2"));
    let _ = std::fs::remove_dir_all(&dir);

    // A leap second.
    let (dir, path) = one_entry("leap");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("09:15:02Z", "23:59:60Z")).unwrap();
    assert!(History::read(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reading_checks_the_graph() {
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
        uses: vec![],
    };
    let name = |v: u8, time: i64, n: &str| {
        format!(
            "{}-{}{n}.toml",
            at(time).name_part(),
            format!("{v:02x}").repeat(6)
        )
    };

    // Two branches each added a run after version 1, and git merged them: both read, the later one is the latest.
    let dir = folder("merge");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(
        &dir,
        &name(2, T + 60, ""),
        &entry(2, Some(1), T + 60, false),
    );
    write(
        &dir,
        &name(3, T + 300, ""),
        &entry(3, Some(1), T + 300, false),
    );
    let h = History::read(&dir).unwrap();
    assert_eq!(h.latest().unwrap().version, version(3));
    let _ = std::fs::remove_dir_all(&dir);

    // Two runs with one time and number, from two machines: both read, ordered by name.
    let dir = folder("twins");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(3, T + 1, ""), &entry(3, Some(1), T + 1, false));
    write(&dir, &name(2, T + 1, ""), &entry(2, Some(1), T + 1, false));
    assert_eq!(
        History::read(&dir).unwrap().latest().unwrap().version,
        version(3)
    );
    let _ = std::fs::remove_dir_all(&dir);

    // A parent no entry records, pointing at the line.
    let dir = folder("orphan");
    write(&dir, &name(1, T, ""), &entry(1, Some(9), T, false));
    let e = read_error(&dir);
    assert!(
        e.contains("isn't a version any entry here records") && e.contains(":3:"),
        "{e}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Its own parent.
    let dir = folder("self");
    write(&dir, &name(1, T, ""), &entry(1, Some(1), T, false));
    assert!(read_error(&dir).contains("its own version"));
    let _ = std::fs::remove_dir_all(&dir);

    // A restore of a version no other entry records.
    let dir = folder("ghost");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(5, T + 1, ""), &entry(5, Some(1), T + 1, true));
    assert!(read_error(&dir).contains("restores a version no other entry"));
    let _ = std::fs::remove_dir_all(&dir);

    // Going back to a version without saying it's a restore is allowed (A, B, A): only a restore is checked.
    let dir = folder("back");
    write(&dir, &name(1, T, ""), &entry(1, None, T, false));
    write(&dir, &name(2, T + 1, ""), &entry(2, Some(1), T + 1, false));
    write(&dir, &name(1, T + 2, ""), &entry(1, Some(2), T + 2, false));
    assert_eq!(
        History::read(&dir).unwrap().latest().unwrap().version,
        version(1)
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Numbered entries order by number, -10 after -9, whatever order their names or versions would give.
    let dir = folder("ten");
    write(&dir, &name(20, T, ""), &entry(20, None, T, false));
    for k in 2..=10u8 {
        let v = 20 - k;
        let parent = if k == 2 { 20 } else { v + 1 };
        write(
            &dir,
            &name(v, T, &format!("-{k}")),
            &entry(v, Some(parent), T, false),
        );
    }
    let h = History::read(&dir).unwrap();
    let order: Vec<VersionId> = h.entries().map(|(_, e)| e.version).collect();
    let want: Vec<VersionId> = std::iter::once(20)
        .chain((10..=18).rev())
        .map(version)
        .collect();
    assert_eq!(order, want);
    let _ = std::fs::remove_dir_all(&dir);
}
