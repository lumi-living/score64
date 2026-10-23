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
fn columns_count_characters() {
    let (at, _) = error(TrackFile::parse("title = \"ääää\" x\n", p("x")));
    assert_eq!(at, Some((1, 16)));
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
    // With no start, the range starts at the source's start, so an end at 0:00 is empty too.
    let text = track_text("").replace("start = \"0:12.500\"\nend = \"1:14\"", "end = \"0:00\"");
    assert!(
        error(TrackFile::parse(&text, p("x")))
            .1
            .contains("not after its start 0:00")
    );
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
    // Two adaptive sets with one name.
    let text = format!(
        "{}\n[[adaptive]]\nname = \"score\"\nlayers = [\"calm\"]\n",
        track_text("")
    );
    let (at, m) = error(TrackFile::parse(&text, p("x")));
    assert!(m.contains("adaptive set `score` is declared twice"), "{m}");
    assert_eq!(
        at.unwrap().0,
        text.lines()
            .collect::<Vec<_>>()
            .iter()
            .rposition(|l| *l == "[[adaptive]]")
            .unwrap()
            + 1
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
    // A repeated profile is reported at its own header, not the first.
    let text = format!(
        "{PROJECT}\n[[profile]]\nname = \"calm\"\nv1 = {{ usage = \"music\" }}\nv2 = {{ usage = \"music\" }}\nv3 = {{ usage = \"music\" }}\n"
    );
    let (at, m) = error(ProjectFile::parse(&text, p("x")));
    assert_eq!(
        at.unwrap().0,
        text.lines()
            .collect::<Vec<_>>()
            .iter()
            .rposition(|l| *l == "[[profile]]")
            .unwrap()
            + 1,
        "{m}"
    );
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
        "audio/CON",
        "aux.wav",
        "audio/lpt1.txt",
        "audio./a.wav",
        "audio/a.wav ",
        "a\\u0000b.wav",
        "a\\tb.wav",
        "a\\u202Eb.wav",
        "a?.wav",
        "a*.wav",
        "a<b.wav",
        "a>b.wav",
        "a|b.wav",
        "a\\\"b.wav",
        "audio/conin$",
        "CONOUT$.txt",
        "COM¹.wav",
        "lpt³",
    ] {
        let text = track_text("").replace("audio/main-theme.wav", bad);
        let (_, m) = error(TrackFile::parse(&text, p("x")));
        assert!(m.contains("path"), "{bad}: {m}");
    }
    // Names that merely start like a device are fine.
    assert!(ProjectPath::try_from("audio/console.wav".to_string()).is_ok());
    assert!(ProjectPath::try_from("audio/com10.wav".to_string()).is_ok());
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
        ("0:12.000250", 12_000_250, "0:12.000250"),
        ("0:12.25", 12_250_000, "0:12.250"),
        ("0:12.0005", 12_000_500, "0:12.000500"),
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
        "99999999999999999999:00:00",
    ] {
        assert!(bad.parse::<Timestamp>().is_err(), "{bad}");
    }
}

proptest! {
    #[test]
    fn any_time_reads_back_from_its_text(us in any::<u64>()) {
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
    let (at, m) = error(pr.track(&id("main")));
    assert!(m.contains("zero_page $FF"), "{m}");
    let text = track_text("\n[pack]\nzero_page = 0xFF\n");
    assert_eq!(
        at.unwrap().0,
        text.lines().position(|l| l == "[pack]").unwrap() + 1
    );
    // hard_restart_adsr: the track's, the project's, or none.
    std::fs::write(
        root.join("tracks/main/track.toml"),
        track_text("\n[pack]\nhard_restart_adsr = 0x0A00\n"),
    )
    .unwrap();
    assert_eq!(
        pr.track(&id("main"))
            .unwrap()
            .pack
            .unwrap()
            .hard_restart_adsr,
        Some(0x0A00)
    );
    std::fs::write(
        root.join("score64.toml"),
        PROJECT.replace(
            "sound_effects = true",
            "sound_effects = true\nhard_restart_adsr = 0x0F00",
        ),
    )
    .unwrap();
    let pr = Project::open(&root).unwrap();
    assert_eq!(
        pr.track(&id("main"))
            .unwrap()
            .pack
            .unwrap()
            .hard_restart_adsr,
        Some(0x0A00)
    );
    std::fs::write(
        root.join("tracks/main/track.toml"),
        track_text("\n[pack]\nzero_page = 0xFB\n"),
    )
    .unwrap();
    assert_eq!(
        pr.track(&id("main"))
            .unwrap()
            .pack
            .unwrap()
            .hard_restart_adsr,
        Some(0x0F00)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_track_overrides_only_its_target_video_and_pack() {
    for extra in [
        "\n[validate]\nexpendable_long_rows = 2\n",
        "\n[tools]\ngt2reloc = { version = \"2.73\", reference = \"x\" }\n",
        "\n[[profile]]\nname = \"mine\"\n",
        "\n[target]\nvideo = \"ntsc\"\nchip = \"8580\"\n",
    ] {
        let (at, m) = error(TrackFile::parse(&track_text(extra), p("x")));
        // `profile` is the track's default profile name, so a [[profile]] table is the wrong type rather than unknown.
        assert!(
            m.contains("unknown field") || m.contains("invalid type"),
            "{extra}: {m}"
        );
        assert!(at.is_some());
    }
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
    // The track's own default profile points at its key.
    let text = track_text("")
        .replace(
            "title = \"Main theme\"",
            "title = \"Main theme\"\nprofile = \"busy\"",
        )
        .replace("profile = \"calm\"\n", "");
    std::fs::write(root.join("tracks/main/track.toml"), &text).unwrap();
    let (at, m) = error(pr.track(&id("main")));
    assert!(m.contains("profile `busy`"), "{m}");
    assert_eq!(at.unwrap().0, 3);
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
    let mix = || ProjectPath::try_from("audio/main-theme.wav".to_string()).unwrap();
    assert_eq!(
        pr.check_sources(&t),
        [SourceProblem::Missing {
            id: id("mix"),
            path: mix(),
        }]
    );
    assert_eq!(
        pr.check_sources(&t)[0].to_string(),
        "source `mix`: `audio/main-theme.wav` is missing"
    );
    // A folder where the file should be is not "missing".
    std::fs::create_dir_all(root.join("audio/main-theme.wav")).unwrap();
    match &pr.check_sources(&t)[..] {
        [p @ SourceProblem::Unreadable { .. }] => {
            assert!(
                p.to_string()
                    .starts_with("source `mix`: `audio/main-theme.wav` can't be read: "),
                "{p}"
            )
        }
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir(root.join("audio/main-theme.wav")).unwrap();
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
    // Recording the new hash, on purpose, changes only that value: comments and hex stay.
    let path = root.join("tracks/main/track.toml");
    let before = format!(
        "# Main theme, from the April mix.\n{}\n[pack]\nzero_page = 0xFB # the target's pair\n",
        track_text("")
    );
    std::fs::write(&path, &before).unwrap();
    let mut t2 = pr.track(&id("main")).unwrap();
    assert!(pr.record_hash(&mut t2, &id("mix"), found.clone()).unwrap());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before.replace(HASH, found.as_str())
    );
    assert_eq!(t2.file.sources[0].blake3, found);
    assert!(pr.check_sources(&pr.track(&id("main")).unwrap()).is_empty());
    assert!(
        !pr.record_hash(&mut t, &id("stems"), Blake3::of(b""))
            .unwrap()
    );
    // Two sources, the hash written as a literal and as a multi-line string: only the named one changes.
    let other = HASH.replace("ab12", "cd34");
    for (written, quoted) in [
        (format!("'{other}'"), format!("'{other}'")),
        (
            format!("\"\"\"{other}\"\"\""),
            format!("\"\"\"{other}\"\"\""),
        ),
    ] {
        let before = format!(
            "{}\n[[source]]\nid = \"stem\"\npath = \"audio/stem.wav\"   # bass\nblake3 = {written}\nkind = \"stem\"\n",
            track_text("")
        );
        std::fs::write(&path, &before).unwrap();
        let mut t = pr.track(&id("main")).unwrap();
        assert!(pr.record_hash(&mut t, &id("stem"), found.clone()).unwrap());
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            before.replace(&quoted, &format!("\"{}\"", found.as_str())),
            "{written}"
        );
        assert!(after.contains(HASH), "the first source is untouched");
        assert_eq!(t.file.sources[1].blake3, found);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_source_may_not_leave_the_project_through_a_link() {
    let root = project("links", &track_text(""));
    let outside = root.with_extension("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("main-theme.wav"), b"sound").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("audio")).unwrap();
    let pr = Project::open(&root).unwrap();
    let t = pr.track(&id("main")).unwrap();
    match &pr.check_sources(&t)[..] {
        [p @ SourceProblem::Unreadable { .. }] => {
            assert!(p.to_string().contains("leads out of the project"), "{p}")
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn new_files_are_written_deterministically_and_never_over_old_ones() {
    let root = std::env::temp_dir().join(format!("score64-project-create-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = ProjectFile::parse(PROJECT, p("x")).unwrap();
    let pr = Project::create(&root, file.clone()).unwrap();
    assert!(matches!(
        Project::create(&root, file),
        Err(Error::Io { .. })
    ));
    let track = TrackFile::parse(&track_text(""), p("x")).unwrap();
    let t = pr.create_track(&id("main"), &track).unwrap();
    assert_eq!(t.file, track);
    let written = std::fs::read_to_string(root.join("tracks/main/track.toml")).unwrap();
    assert_eq!(written, track.to_toml().unwrap());
    assert_eq!(
        TrackFile::parse(&written, p("x"))
            .unwrap()
            .to_toml()
            .unwrap(),
        written,
        "deterministic"
    );
    assert!(matches!(
        pr.create_track(&id("main"), &track),
        Err(Error::Io { .. })
    ));
    // A file the readers would refuse is never written.
    let mut bad = track.clone();
    bad.sources.push(bad.sources[0].clone());
    assert!(error(bad.to_toml()).1.contains("declared twice"));
    assert!(pr.create_track(&id("other"), &bad).is_err());
    assert!(
        !root.join("tracks/other").exists(),
        "nothing is left behind"
    );
    // Checked against the project too: an unknown profile is refused before anything is written.
    let mut unknown = track.clone();
    unknown.subtunes[0].profile = Some(ProfileName::new("busy").unwrap());
    assert!(
        error(pr.create_track(&id("other"), &unknown))
            .1
            .contains("profile `busy`")
    );
    assert!(!root.join("tracks/other").exists());
    // And never beside a track whose id differs only in case.
    assert!(
        error(pr.create_track(&id("Main"), &track))
            .1
            .contains("only in case")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn track_folders_must_be_track_ids() {
    let root = project("ids", &track_text(""));
    let pr = Project::open(&root).unwrap();
    // A folder without a track.toml is not a track.
    std::fs::create_dir_all(root.join("tracks/notes and drafts")).unwrap();
    assert_eq!(pr.track_ids().unwrap(), [id("main")]);
    std::fs::write(
        root.join("tracks/notes and drafts/track.toml"),
        track_text(""),
    )
    .unwrap();
    let (_, m) = error(pr.track_ids());
    assert!(
        m.contains("\"notes and drafts\" isn't a valid track id"),
        "{m}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn track_ids_that_differ_only_in_case_are_an_error() {
    let root = project("case", &track_text(""));
    let pr = Project::open(&root).unwrap();
    std::fs::create_dir(root.join("tracks/Main")).ok();
    if std::fs::read_dir(root.join("tracks")).unwrap().count() < 2 {
        // A case-insensitive file system: there can't be two such folders.
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    std::fs::write(root.join("tracks/Main/track.toml"), track_text("")).unwrap();
    assert!(error(pr.track_ids()).1.contains("differ only in case"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ids_are_safe_folder_names() {
    for bad in [
        "con",
        "COM1",
        "nul",
        "Lpt9",
        "",
        "-a",
        "a b",
        "ä",
        &"a".repeat(65),
    ] {
        assert!(Id::new(bad).is_err(), "{bad}");
    }
    for ok in ["main", "com10", "console", "a-b_c", "1", &"a".repeat(64)] {
        assert!(Id::new(ok).is_ok(), "{ok}");
    }
}

/// What a new file looks like: the ADR's examples written back. Numbers are written in decimal and comments aren't
/// kept, which is why existing files are edited in place.
#[test]
fn new_files_look_like_this() {
    let project = ProjectFile::parse(PROJECT, p("x"))
        .unwrap()
        .to_toml()
        .unwrap();
    assert_eq!(project, PROJECT_WRITTEN);
    let track = TrackFile::parse(&track_text(""), p("x"))
        .unwrap()
        .to_toml()
        .unwrap();
    assert_eq!(track, TRACK_WRITTEN);
}

const PROJECT_WRITTEN: &str = r#"schema = 1

[[profile]]
name = "calm"

[profile.v1]
usage = "music"
roles = ["bed"]
filter = true

[profile.v2]
usage = "effects"
filter = false

[profile.v3]
usage = "effects"
filter = false

[pack]
player_page = 16
zero_page = 248
sound_effects = true

[tools.gt2reloc]
version = "2.73"
reference = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12"

[validate]
expendable_long_rows = 4
"#;

const TRACK_WRITTEN: &str = r#"schema = 1
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

[[subtune]]
name = "tension"
from = "mix"

[[adaptive]]
name = "score"
layers = ["calm", "tension"]

[adaptive.switch]
every_bars = 2

[rules.reduce]
min_note_rows = 1
"#;
