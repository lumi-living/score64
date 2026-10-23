//! Project and track files: the ADR's examples, errors with their positions, timestamps, paths, overrides and
//! source checks.

use proptest::prelude::*;

use super::*;

const HASH: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

const PROJECT: &str = r#"schema = 1

[target]
video = "pal"

[[profile]]
name = "calm"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "effects" }
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

fn track_text(extra: &str) -> String {
    format!(
        r#"schema = 1
title = "Main theme"

[[source]]
id = "mix"
path = "audio/main-theme.wav"
blake3 = "{HASH}"
kind = "mix"

[[subtune]]
name = "calm"
profile = "calm"
from = "mix"
start = "0:12.500"
end = "1:14"

[[subtune]]
name = "tension"
from = "mix"

[[adaptive]]
name = "score"
layers = ["calm", "tension"]
switch = {{ every_bars = 2 }}

[rules.reduce]
min_note_rows = 1
{extra}"#
    )
}

fn p(s: &str) -> &Path {
    Path::new(s)
}

fn error(r: Result<impl fmt::Debug, Error>) -> (Option<(usize, usize)>, String) {
    match r {
        Err(Error::Invalid { at, message, .. }) => (at, message),
        other => panic!("expected an invalid-file error, got {other:?}"),
    }
}

#[test]
fn the_adr_examples_read_and_write_back() {
    let project = ProjectFile::parse(PROJECT, p("score64.toml")).unwrap();
    assert_eq!(project.target.video, VideoStandard::Pal);
    assert_eq!(project.pack.as_ref().unwrap().zero_page, 0xF8);
    assert_eq!(project.tools.gt2reloc.as_ref().unwrap().version, "2.73");
    assert!(
        project
            .profiles()
            .unwrap()
            .get(&ProfileName::new("calm").unwrap())
            .is_some()
    );
    let again = project.to_toml().unwrap();
    assert_eq!(
        ProjectFile::parse(&again, p("score64.toml")).unwrap(),
        project
    );
    assert_eq!(
        ProjectFile::parse(&again, p("x"))
            .unwrap()
            .to_toml()
            .unwrap(),
        again,
        "deterministic"
    );

    let track = TrackFile::parse(&track_text(""), p("track.toml")).unwrap();
    assert_eq!(
        track.subtunes[0].start,
        Some(Timestamp::from_micros(12_500_000))
    );
    assert_eq!(
        track.subtunes[0].end,
        Some(Timestamp::from_micros(74_000_000))
    );
    assert_eq!(track.adaptive[0].layers().len(), 2);
    assert_eq!(track.rules["reduce"]["min_note_rows"].as_integer(), Some(1));
    let again = track.to_toml().unwrap();
    assert!(again.contains("start = \"0:12.500\""), "{again}");
    assert_eq!(TrackFile::parse(&again, p("track.toml")).unwrap(), track);
    // An empty file is a valid, empty project.
    assert_eq!(
        ProjectFile::parse("", p("x")).unwrap(),
        ProjectFile::default()
    );
}

#[test]
fn syntax_errors_and_unknown_keys_give_line_and_column() {
    let (at, m) = error(ProjectFile::parse(
        "schema = 1\n[target]\nvideo = \"pal\"\ncolour = 1\n",
        p("x"),
    ));
    assert_eq!(at.map(|a| a.0), Some(4), "{m}");
    assert!(m.contains("unknown field `colour`"), "{m}");
    let (at, m) = error(ProjectFile::parse("schema = 2\n", p("x")));
    assert_eq!(at.map(|a| a.0), Some(1));
    assert!(m.contains("schema 2 is not supported"), "{m}");
    // A float where a time goes is refused.
    let text = track_text("").replace("start = \"0:12.500\"", "start = 12.5");
    let (at, m) = error(TrackFile::parse(&text, p("x")));
    assert!(at.is_some(), "{m}");
    assert!(m.contains("string"), "{m}");
}

#[test]
fn track_checks_point_at_the_item() {
    let line_of =
        |text: &str, needle: &str| text.lines().position(|l| l.contains(needle)).unwrap() + 1;
    // A second source with the same id.
    let text = track_text(&format!(
        "\n[[source]]\nid = \"mix\"\npath = \"b.wav\"\nblake3 = \"{HASH}\"\nkind = \"stem\"\n"
    ));
    let (at, m) = error(TrackFile::parse(&text, p("x")));
    assert!(m.contains("source `mix` is declared twice"), "{m}");
    assert_eq!(
        at.unwrap().0,
        text.lines()
            .enumerate()
            .filter(|(_, l)| l.starts_with("[[source]]"))
            .nth(1)
            .unwrap()
            .0
            + 1
    );
    // A subtune from an unknown source, an empty range, a repeated name.
    let text = track_text("").replace(
        "name = \"tension\"\nfrom = \"mix\"",
        "name = \"tension\"\nfrom = \"stems\"",
    );
    let (at, m) = error(TrackFile::parse(&text, p("x")));
    assert!(m.contains("`tension` is made from `stems`"), "{m}");
    assert_eq!(at.unwrap().0, line_of(&text, "name = \"tension\"") - 1);
    let text = track_text("").replace("end = \"1:14\"", "end = \"0:12.500\"");
    assert!(
        error(TrackFile::parse(&text, p("x")))
            .1
            .contains("not after its start")
    );
    let text = track_text("")
        .replace("name = \"tension\"", "name = \"calm\"")
        .replace("layers = [\"calm\", \"tension\"]", "layers = [\"calm\"]");
    assert!(
        error(TrackFile::parse(&text, p("x")))
            .1
            .contains("subtune `calm` is declared twice")
    );
    // An adaptive layer that isn't a subtune here.
    let text = track_text("").replace(
        "layers = [\"calm\", \"tension\"]",
        "layers = [\"calm\", \"peak\"]",
    );
    let (at, m) = error(TrackFile::parse(&text, p("x")));
    assert!(m.contains("names `peak`"), "{m}");
    assert_eq!(at.unwrap().0, line_of(&text, "[[adaptive]]"));
}

#[test]
fn values_out_of_range_are_refused() {
    for (from, to, why) in [
        ("zero_page = 0xF8", "zero_page = 0xFF", "zero_page $FF"),
        ("zero_page = 0xF8", "zero_page = 0x01", "zero_page $01"),
        (
            "player_page = 0x10",
            "player_page = 0x01",
            "player_page $01",
        ),
    ] {
        let (_, m) = error(ProjectFile::parse(&PROJECT.replace(from, to), p("x")));
        assert!(m.contains(why), "{m}");
    }
    // A profile redefining a built-in.
    let text = PROJECT.replace("name = \"calm\"", "name = \"full\"");
    assert!(
        error(ProjectFile::parse(&text, p("x")))
            .1
            .contains("redefines a built-in")
    );
    // A hash that isn't one; uppercase is fine and written lowercase.
    let text = track_text("").replace(HASH, "abc");
    assert!(
        error(TrackFile::parse(&text, p("x")))
            .1
            .contains("64 hex digits")
    );
    let t = TrackFile::parse(&track_text("").replace(HASH, &HASH.to_uppercase()), p("x")).unwrap();
    assert_eq!(t.sources[0].blake3.as_str(), HASH);
}

#[test]
fn paths_stay_inside_the_project() {
    for bad in [
        "../audio.wav",
        "/abs/a.wav",
        "a//b.wav",
        "a/./b.wav",
        "a\\\\b.wav", // TOML `a\\b.wav`, which reads as `a\b.wav`
        "C:/a.wav",
        "",
    ] {
        let text = track_text("").replace("audio/main-theme.wav", bad);
        let (_, m) = error(TrackFile::parse(&text, p("x")));
        assert!(m.contains("path"), "{bad}: {m}");
    }
    let ok = ProjectPath::try_from("audio/a b.wav".to_string()).unwrap();
    assert_eq!(
        ok.under(Path::new("/root")),
        Path::new("/root/audio/a b.wav")
    );
}

#[test]
fn times_are_exact_and_written_canonically() {
    for (text, us, canonical) in [
        ("0:12.500", 12_500_000, "0:12.500"),
        ("1:14", 74_000_000, "1:14"),
        ("0:00", 0, "0:00"),
        ("0:12.5", 12_500_000, "0:12.500"),
        ("0:12.000250", 12_000_250, "0:12.00025"),
        ("12:34.000001", 754_000_001, "12:34.000001"),
        ("1:02:03.25", 3_723_250_000, "1:02:03.250"),
        ("75:00", 4_500_000_000, "1:15:00"),
    ] {
        let t: Timestamp = text.parse().unwrap();
        assert_eq!(t.micros(), us, "{text}");
        assert_eq!(t.to_string(), canonical, "{text}");
    }
    for bad in [
        "12.5",
        "0:5",
        "0:60",
        "1:60:00",
        "0:12.1234567",
        "a:bc",
        "0:12.",
        "1:2:3:4",
        "-0:01",
        "0:1x",
    ] {
        assert!(bad.parse::<Timestamp>().is_err(), "{bad}");
    }
}

proptest! {
    #[test]
    fn any_time_reads_back_from_its_text(us in 0u64..100 * 3_600_000_000) {
        let t = Timestamp::from_micros(us);
        prop_assert_eq!(t.to_string().parse::<Timestamp>().unwrap(), t);
    }
}

/// A project folder with `score64.toml` and one track, `main`.
fn project(name: &str, track: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("score64-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tracks/main")).unwrap();
    std::fs::write(root.join("score64.toml"), PROJECT).unwrap();
    std::fs::write(root.join("tracks/main/track.toml"), track).unwrap();
    root
}

fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}

#[test]
fn a_track_loads_with_the_projects_values_and_its_overrides() {
    let root = project(
        "overrides",
        &track_text("\n[target]\nvideo = \"ntsc\"\n\n[pack]\nzero_page = 0xFB\n"),
    );
    let pr = Project::open(&root).unwrap();
    assert_eq!(pr.track_ids().unwrap(), [id("main")]);
    let t = pr.track(&id("main")).unwrap();
    assert_eq!(t.video, VideoStandard::Ntsc);
    let pack = t.pack.unwrap();
    assert_eq!(
        (pack.player_page, pack.zero_page, pack.sound_effects),
        (0x10, 0xFB, true)
    );
    assert!(t.decisions.decision.is_empty(), "no decisions.toml yet");
    // Without overrides, the project's values.
    std::fs::write(root.join("tracks/main/track.toml"), track_text("")).unwrap();
    let t = pr.track(&id("main")).unwrap();
    assert_eq!(
        (t.video, t.pack.unwrap().zero_page),
        (VideoStandard::Pal, 0xF8)
    );
    // An override that leaves the pack out of range.
    std::fs::write(
        root.join("tracks/main/track.toml"),
        track_text("\n[pack]\nzero_page = 0xFF\n"),
    )
    .unwrap();
    assert!(error(pr.track(&id("main"))).1.contains("zero_page $FF"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_track_needs_known_profiles_and_a_complete_pack() {
    let root = project(
        "profiles",
        &track_text("").replace("profile = \"calm\"", "profile = \"busy\""),
    );
    let pr = Project::open(&root).unwrap();
    let (at, m) = error(pr.track(&id("main")));
    assert!(m.contains("profile `busy`"), "{m}");
    assert!(at.is_some());
    // No project [pack]: a track override must give every required key.
    std::fs::write(root.join("score64.toml"), "schema = 1\n").unwrap();
    std::fs::write(
        root.join("tracks/main/track.toml"),
        track_text("\n[pack]\nzero_page = 0xFB\n").replace("profile = \"calm\"\n", ""),
    )
    .unwrap();
    let pr = Project::open(&root).unwrap();
    assert!(error(pr.track(&id("main"))).1.contains("needs player_page"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn sources_are_checked_against_their_hashes() {
    let root = project("sources", &track_text(""));
    let pr = Project::open(&root).unwrap();
    let mut t = pr.track(&id("main")).unwrap();
    assert_eq!(
        pr.check_sources(&t),
        [SourceProblem::Missing {
            id: id("mix"),
            path: ProjectPath::try_from("audio/main-theme.wav".to_string()).unwrap(),
        }]
    );
    assert_eq!(
        pr.check_sources(&t)[0].to_string(),
        "source `mix`: `audio/main-theme.wav` is missing"
    );
    std::fs::create_dir_all(root.join("audio")).unwrap();
    std::fs::write(root.join("audio/main-theme.wav"), b"sound").unwrap();
    let found = Blake3::of(b"sound");
    match &pr.check_sources(&t)[..] {
        [
            p @ SourceProblem::Changed {
                expected, found: f, ..
            },
        ] => {
            assert_eq!((expected.as_str(), f), (HASH, &found));
            assert!(
                p.to_string()
                    .contains(&format!("expected `{HASH}`, found `{}`", found.as_str()))
            );
        }
        other => panic!("{other:?}"),
    }
    // Recording the new hash, on purpose, and writing the track.
    assert!(t.file.record_hash(&id("mix"), found));
    assert!(!t.file.record_hash(&id("stems"), Blake3::of(b"")));
    pr.write_track(&t).unwrap();
    let t = pr.track(&id("main")).unwrap();
    assert!(pr.check_sources(&t).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
