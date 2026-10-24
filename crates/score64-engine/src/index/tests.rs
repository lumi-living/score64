//! The command's error kinds: the project's files are input, the index's own files are `io`.

use std::path::Path;

use super::*;
use crate::EngineConfig;

fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "score64-engine-index-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tracks/main")).unwrap();
    std::fs::write(root.join("score64.toml"), "schema = 1\n").unwrap();
    std::fs::write(root.join("tracks/main/track.toml"), "schema = 1\n").unwrap();
    root
}

fn run(project: &Path, rebuild: bool) -> Result<IndexOutput, CommandError> {
    Engine::new(EngineConfig::default()).run::<IndexCommand>(IndexArgs {
        project: project.to_path_buf(),
        rebuild,
    })
}

#[test]
fn a_bad_track_file_is_input_and_an_unwritable_index_is_io() {
    let root = project("kinds");
    assert_eq!(run(&root, false).unwrap().tracks, 1);

    std::fs::write(
        root.join("tracks/main/track.toml"),
        "schema = 1\ncolour = 1\n",
    )
    .unwrap();
    let e = run(&root, true).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput, "{}", e.message);
    assert!(e.message.contains("track.toml"), "{}", e.message);

    // `.score64` is a file, so nothing can be written under it.
    std::fs::write(root.join("tracks/main/track.toml"), "schema = 1\n").unwrap();
    std::fs::remove_dir_all(root.join(".score64")).unwrap();
    std::fs::write(root.join(".score64"), "in the way").unwrap();
    let e = run(&root, true).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{}", e.message);
    assert!(e.message.contains(".score64"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_cache_that_cant_be_listed_is_io() {
    let root = project("cache");
    // `.score64/cache` is a file, so it can't be listed.
    std::fs::create_dir_all(root.join(".score64")).unwrap();
    std::fs::write(root.join(".score64/cache"), "in the way").unwrap();
    let e = run(&root, true).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{}", e.message);
    assert!(e.message.contains("cache"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_damaged_index_is_io_and_says_how_to_fix_it() {
    let root = project("damaged");
    assert_eq!(run(&root, false).unwrap().bytes, 0);
    // A value no rebuild writes, which opening doesn't look at (the files are unchanged): the query reading it fails.
    let db = root.join(".score64/index.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "INSERT INTO artifacts (stage, key, name, hash, bytes) VALUES ('s', 'k', 'n', 'h', -5)",
            [],
        )
        .unwrap();
    let e = run(&root, false).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{}", e.message);
    assert!(
        e.message.contains("score64 index --rebuild"),
        "{}",
        e.message
    );
    // Which works.
    assert_eq!(run(&root, true).unwrap().bytes, 0);
    assert_eq!(run(&root, false).unwrap().artifacts, 0);
    let _ = std::fs::remove_dir_all(&root);
}
