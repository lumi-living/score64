//! Version identity: the same intent gives the same id however its files are written, and anything that can change
//! the output gives another.

use std::path::PathBuf;

use super::*;
use crate::project::Id;

const HASH: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

const PROJECT: &str = r#"schema = 1

[target]
video = "pal"

[[profile]]
name = "calm"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "effects" }
v3 = { usage = "effects" }

[[profile]]
name = "unused"
v1 = { usage = "music" }
v2 = { usage = "music" }
v3 = { usage = "effects" }

[pack]
player_page = 0x10
zero_page = 0xF8
sound_effects = true

[tools]
gt2reloc = { version = "2.73", reference = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12" }

[validate]
expendable_long_rows = 4
"#;

const TRACK: &str = r#"schema = 1
title = "Main theme"

[[source]]
id = "mix"
path = "audio/main-theme.wav"
blake3 = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12"
kind = "mix"

[[subtune]]
name = "calm"
profile = "calm"
from = "mix"
start = "0:12.500"
end = "1:14"
"#;

const DECISIONS: &str = r#"schema = 1

[[decision]]
id = "keep-bass"
subtune = "calm"
bar = 2
confidence = 1.0
reviewed = false

[decision.proposal]
change = "octave"
voice = 3
shift = -1

[decision.provenance]
by = "manual"

[[decision]]
id = "a-loop"
subtune = "calm"
bar = 4
confidence = 1.0
reviewed = false

[decision.proposal]
change = "loop_row"
row = 64

[decision.provenance]
by = "manual"
"#;

/// A project folder with `score64.toml` and track `main`; `decisions` too when given.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str, project: &str, track: &str, decisions: Option<&str>) -> Self {
        // A folder per call: the tests run in parallel.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("score64-version-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tracks/main")).unwrap();
        std::fs::write(root.join("score64.toml"), project).unwrap();
        std::fs::write(root.join("tracks/main/track.toml"), track).unwrap();
        if let Some(d) = decisions {
            std::fs::write(root.join("tracks/main/decisions.toml"), d).unwrap();
        }
        Folder(root)
    }

    fn id(&self, tool: &str) -> VersionId {
        let project = Project::open(&self.0).unwrap();
        let track = project.track(&Id::new("main").unwrap()).unwrap();
        version_id(&project, &track, tool).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn id_of(name: &str, project: &str, track: &str, decisions: Option<&str>) -> VersionId {
    Folder::new(name, project, track, decisions).id("0.1.0")
}

fn base() -> VersionId {
    id_of("base", PROJECT, TRACK, Some(DECISIONS))
}

#[test]
fn the_same_intent_has_the_same_id() {
    let f = Folder::new("same", PROJECT, TRACK, Some(DECISIONS));
    assert_eq!(f.id("0.1.0"), f.id("0.1.0"));
    assert_eq!(f.id("0.1.0"), base());
}

/// The scheme's own output for the fixture, so a change to what goes into the hash shows. If this fails on purpose,
/// bump [`SCHEME`].
#[test]
fn the_scheme_is_pinned() {
    assert_eq!(
        base().to_string(),
        "cad8e0883a30054328fc39da0d93e2cffa15920990191e6c83ca9f9ed02129e7",
        "the version scheme changed: bump SCHEME"
    );
}

#[test]
fn how_the_files_are_written_doesnt_matter() {
    // Comments, key order, hex or decimal, and the decisions in another order.
    let project = format!(
        "# The target.\n{}",
        PROJECT.replace("player_page = 0x10", "player_page = 16")
    );
    let track = TRACK
        .replace(
            "id = \"mix\"\npath = \"audio/main-theme.wav\"",
            "path = 'audio/main-theme.wav' # the mix\nid = \"mix\"",
        )
        .replace("schema = 1\n", "");
    let decisions = {
        let (a, b) = DECISIONS.split_at(DECISIONS.find("\n[[decision]]\nid = \"a-loop\"").unwrap());
        format!("schema = 1\n{}\n{}", b, &a["schema = 1\n".len()..])
    };
    assert_eq!(id_of("written", &project, &track, Some(&decisions)), base());
}

#[test]
fn an_override_counts_only_through_what_it_resolves_to() {
    // Overriding with the project's own values is no change.
    let same = format!("{TRACK}\n[target]\nvideo = \"pal\"\n\n[pack]\nzero_page = 0xF8\n");
    assert_eq!(
        id_of("override-same", PROJECT, &same, Some(DECISIONS)),
        base()
    );
    let other = format!("{TRACK}\n[pack]\nzero_page = 0xFB\n");
    assert_ne!(
        id_of("override-other", PROJECT, &other, Some(DECISIONS)),
        base()
    );
}

#[test]
fn a_profile_the_track_doesnt_use_doesnt_matter() {
    let unused = PROJECT.replace(
        "v2 = { usage = \"music\" }\nv3",
        "v2 = { usage = \"effects\" }\nv3",
    );
    assert_eq!(id_of("unused", &unused, TRACK, Some(DECISIONS)), base());
    let used = PROJECT.replace(
        "roles = [\"bed\"], filter = true",
        "roles = [\"bed\"], filter = false",
    );
    assert_ne!(id_of("used", &used, TRACK, Some(DECISIONS)), base());
}

#[test]
fn whatever_can_change_the_output_changes_the_id() {
    let other_hash = HASH.replace("ab12", "cd34");
    let cases: Vec<(&str, String, String, Option<String>)> = vec![
        (
            "source",
            PROJECT.into(),
            TRACK.replace(HASH, &other_hash),
            Some(DECISIONS.into()),
        ),
        (
            "range",
            PROJECT.into(),
            TRACK.replace("end = \"1:14\"", "end = \"1:15\""),
            Some(DECISIONS.into()),
        ),
        (
            "title",
            PROJECT.into(),
            TRACK.replace("Main theme", "Main theme 2"),
            Some(DECISIONS.into()),
        ),
        ("no-decisions", PROJECT.into(), TRACK.into(), None),
        (
            "decision",
            PROJECT.into(),
            TRACK.into(),
            Some(DECISIONS.replace("shift = -1", "shift = 1")),
        ),
        (
            "video",
            PROJECT.replace("video = \"pal\"", "video = \"ntsc\""),
            TRACK.into(),
            Some(DECISIONS.into()),
        ),
        (
            "pack",
            PROJECT.replace("zero_page = 0xF8", "zero_page = 0xFB"),
            TRACK.into(),
            Some(DECISIONS.into()),
        ),
        (
            "pin",
            PROJECT.replace("version = \"2.73\"", "version = \"2.74\""),
            TRACK.into(),
            Some(DECISIONS.into()),
        ),
        (
            "validate",
            PROJECT.replace("expendable_long_rows = 4", "expendable_long_rows = 2"),
            TRACK.into(),
            Some(DECISIONS.into()),
        ),
    ];
    let mut seen = vec![base()];
    for (name, project, track, decisions) in cases {
        let id = id_of(name, &project, &track, decisions.as_deref());
        assert!(!seen.contains(&id), "{name} didn't change the id");
        seen.push(id);
    }
    // And the tool that runs it.
    let f = Folder::new("tool", PROJECT, TRACK, Some(DECISIONS));
    assert_ne!(f.id("0.1.0"), f.id("0.2.0"));
}

#[test]
fn a_track_without_a_profile_uses_full() {
    let no_profile = TRACK.replace("profile = \"calm\"\n", "");
    let a = id_of("full-a", PROJECT, &no_profile, None);
    // `full` is built in: a change to a project profile it doesn't name leaves it alone.
    let b = id_of(
        "full-b",
        &PROJECT.replace("filter = true", "filter = false"),
        &no_profile,
        None,
    );
    assert_eq!(a, b);
}
