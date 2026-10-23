use std::sync::OnceLock;

use super::*;

/// What the real gt2reloc printed packing the fixture song with `-D1 -H1 -W10 -ZF8`.
const REAL_REPORT: &str = "GoatTracker v2.73 Packer/Relocator
song file:       fixtures/gt2-original/sixteen-steps.sng
output file:     out.sid
output format:   sid
Player address:   $1000
Zeropage address: $00F8
packing results:
Playroutine:     1297 bytes
Songtable:       12 bytes
Total size:      2062 bytes
";

fn options() -> PackOptions {
    let mut o = PackOptions::new(0x10, ZeroPage::new(0xF8).unwrap());
    o.timing_marks = true;
    o
}

#[test]
fn options_render_every_flag_explicitly() {
    assert_eq!(
        options().args().unwrap(),
        ["-D1", "-H1", "-W10", "-ZF8", "-A0F00", "-P"]
    );
    let mut o = PackOptions::new(0x0C, ZeroPage::new(0x02).unwrap());
    o.sound_effects = false;
    o.hard_restart_adsr = 0x0A00;
    o.timing = Timing::Ntsc;
    // Two hex digits for the page: gt2reloc reads `-W1000` as $0000.
    assert_eq!(
        o.args().unwrap(),
        ["-D0", "-H0", "-W0C", "-Z02", "-A0A00", "-N"]
    );
}

#[test]
fn out_of_range_options_are_refused() {
    for bad in [0x00, 0x01, 0xFF] {
        assert!(
            matches!(ZeroPage::new(bad), Err(PackError::Option(_))),
            "{bad:#04x}"
        );
    }
    assert_eq!(ZeroPage::new(0xFE).unwrap().address(), 0xFE);
    let o = PackOptions::new(0x01, ZeroPage::new(0xF8).unwrap());
    assert!(matches!(o.args(), Err(PackError::Option(_))));
}

#[test]
fn the_format_comes_from_an_exact_lowercase_extension() {
    assert_eq!(Format::of(Path::new("a/tune.sid")).unwrap(), Format::Sid);
    assert_eq!(Format::of(Path::new("tune.prg")).unwrap(), Format::Prg);
    assert_eq!(Format::of(Path::new("tune.bin")).unwrap(), Format::Bin);
    for bad in ["tune.SID", "tune.Prg", "tune.txt", "tune", "tune.sid.bak"] {
        assert!(
            matches!(Format::of(Path::new(bad)), Err(PackError::Format(_))),
            "{bad}"
        );
    }
    // The working directory sits beside the output, so the result is renamed within one file system.
    let work = work_dir(Path::new("out/tune.sid"));
    assert_eq!(work.parent(), Some(Path::new("out")));
    assert_ne!(work, work_dir(Path::new("out/tune.sid")));
}

#[test]
fn the_report_is_read_and_checked() {
    let r = Report::parse(REAL_REPORT);
    assert_eq!(
        r,
        Report {
            version: Some("2.73".into()),
            song_loaded: true,
            player: Some(0x1000),
            zero_page: Some(0xF8),
            size: Some(2062),
        }
    );
    assert!(r.check(&options(), "2.73").is_ok());
    assert!(matches!(
        r.check(&options(), "2.77"),
        Err(PackError::Version { found: Some(v), .. }) if v == "2.73"
    ));
    let mut moved = options();
    moved.player_page = 0x20;
    assert!(matches!(
        r.check(&moved, "2.73"),
        Err(PackError::Mismatch(_))
    ));
    let mut zp = options();
    zp.zero_page = ZeroPage::new(0xFC).unwrap();
    assert!(matches!(r.check(&zp, "2.73"), Err(PackError::Mismatch(_))));
    assert!(matches!(
        Report::parse("").check(&options(), "2.73"),
        Err(PackError::Version { found: None, .. })
    ));
}

/// A stand-in gt2reloc, compiled once. It records its arguments in `args.txt` beside itself, its working directory
/// and the song it found there in `cwd.txt` and `song.txt`, and behaves as
/// `mode.txt` beside itself says: `ok`, `lie` (reports the player at $2000), `old` (banner version 2.72), `suffix`
/// (2.73b), `blank` (a blank `song file:`, as when it can't load the song), `nosize` (no total size), `quiet` (no
/// output file), `empty` (an empty one), `fail` or `hang`.
const FAKE: &str = r#"
use std::path::Path;
fn main() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::fs::write(dir.join("args.txt"), args.join("\n")).unwrap();
    let cwd = std::env::current_dir().unwrap();
    std::fs::write(dir.join("cwd.txt"), cwd.to_str().unwrap()).unwrap();
    std::fs::write(dir.join("song.txt"), std::fs::read(&args[0]).unwrap_or_default()).unwrap();
    let mode = std::fs::read_to_string(dir.join("mode.txt")).unwrap_or_default();
    match mode.trim() {
        "hang" => std::thread::sleep(std::time::Duration::from_secs(600)),
        "fail" => {
            eprintln!("fake gt2reloc failed");
            std::process::exit(3);
        }
        _ => {}
    }
    let version = match mode.trim() {
        "old" => "2.72",
        "suffix" => "2.73b",
        _ => "2.73",
    };
    println!("GoatTracker v{version} Packer/Relocator");
    let song = if mode.trim() == "blank" { "" } else { args[0].as_str() };
    println!("song file:       {song}");
    let opt = |p: &str, d: &str| {
        args.iter().find_map(|a| a.strip_prefix(p)).unwrap_or(d).to_string()
    };
    let page = if mode.trim() == "lie" { "20".to_string() } else { opt("-W", "10") };
    println!("Player address:   ${page}00");
    println!("Zeropage address: $00{}", opt("-Z", "FC"));
    if mode.trim() != "nosize" {
        println!("Total size:      123 bytes");
    }
    match mode.trim() {
        "quiet" => {}
        "empty" => std::fs::write(Path::new(&args[1]), b"").unwrap(),
        _ => std::fs::write(Path::new(&args[1]), b"packed").unwrap(),
    }
}
"#;

fn fake_exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("score64-fake-gt2reloc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("fake.rs");
        std::fs::write(&src, FAKE).unwrap();
        let exe = dir.join(format!("fake{}", std::env::consts::EXE_SUFFIX));
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let status = Command::new(rustc)
            .args(["--edition", "2021", "-O", "-o"])
            .arg(&exe)
            .arg(&src)
            .status()
            .unwrap();
        assert!(status.success(), "compiling the fake gt2reloc");
        exe
    })
}

/// A directory of its own with a copy of the fake in `mode`, and a song file.
struct Bench {
    dir: PathBuf,
    exe: PathBuf,
    song: PathBuf,
}

impl Bench {
    fn new(name: &str, mode: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("score64-gt2reloc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(fake_exe().file_name().unwrap());
        // A hard link, not a copy: copying holds the new executable open for writing, and a process another test
        // forks meanwhile inherits that handle, so running it fails with "text file busy" on Linux.
        std::fs::hard_link(fake_exe(), &exe).unwrap();
        std::fs::write(dir.join("mode.txt"), mode).unwrap();
        // A name far over gt2reloc's 59-byte limit for the song path, starting with a dash.
        let song = dir.join(format!("-{}.sng", "long".repeat(20)));
        std::fs::write(&song, b"GTS5").unwrap();
        Bench { dir, exe, song }
    }

    fn pack(&self, out: &str) -> Result<Packed, PackError> {
        Gt2Reloc::at(&self.exe, "2.73")
            .with_timeout(Duration::from_secs(20))
            .pack(&self.song, &self.dir.join(out), &options())
    }

    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("args.txt"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Files left in the directory besides the bench's own.
    fn leftovers(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !["mode.txt", "args.txt", "cwd.txt", "song.txt"].contains(&n.as_str()))
            .filter(|n| !n.ends_with(".sng"))
            .filter(|n| !n.starts_with("fake"))
            .collect();
        names.sort();
        names
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn packing_passes_the_song_a_temporary_output_and_every_option() {
    let b = Bench::new("ok", "ok");
    let packed = b.pack("tune.sid").unwrap();
    assert_eq!(packed.path, b.dir.join("tune.sid"));
    assert_eq!((packed.format, packed.size), (Format::Sid, 123));
    assert_eq!(packed.version, "2.73");
    assert_eq!(std::fs::read(&packed.path).unwrap(), b"packed");
    let args = b.args();
    assert_eq!(args[..2], ["song.sng", "out.sid"]);
    assert_eq!(args[2..], ["-D1", "-H1", "-W10", "-ZF8", "-A0F00", "-P"]);
    // In a working directory of its own beside the output, on a copy of the song.
    let cwd = PathBuf::from(std::fs::read_to_string(b.dir.join("cwd.txt")).unwrap());
    assert_eq!(
        cwd.parent().map(|p| p.canonicalize().unwrap()),
        Some(b.dir.canonicalize().unwrap())
    );
    assert!(
        cwd.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".tune."),
        "{cwd:?}"
    );
    assert_eq!(std::fs::read(b.dir.join("song.txt")).unwrap(), b"GTS5");
    assert_eq!(b.leftovers(), ["tune.sid"]);
}

#[test]
fn every_failure_leaves_no_output() {
    for (mode, want) in [
        ("lie", "Mismatch"),
        ("old", "Version"),
        ("suffix", "Version"),
        ("blank", "Mismatch"),
        ("nosize", "Mismatch"),
        ("quiet", "Mismatch"),
        ("empty", "Mismatch"),
        ("fail", "Failed"),
    ] {
        let b = Bench::new(mode, mode);
        let err = b.pack("tune.sid").unwrap_err();
        assert!(format!("{err:?}").starts_with(want), "{mode}: {err:?}");
        assert!(b.leftovers().is_empty(), "{mode}: {:?}", b.leftovers());
    }
    let b = Bench::new("fail-stderr", "fail");
    match b.pack("tune.sid") {
        Err(PackError::Failed { stderr, .. }) => assert!(stderr.contains("fake gt2reloc failed")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_hung_packer_is_killed() {
    let b = Bench::new("hang", "hang");
    let started = std::time::Instant::now();
    let err = Gt2Reloc::at(&b.exe, "2.73")
        .with_timeout(Duration::from_millis(500))
        .pack(&b.song, &b.dir.join("tune.sid"), &options())
        .unwrap_err();
    assert!(matches!(err, PackError::Timeout(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(b.leftovers().is_empty());
}

#[test]
fn nothing_runs_without_a_song_or_with_a_bad_extension() {
    let b = Bench::new("refuse", "ok");
    let g = Gt2Reloc::at(&b.exe, "2.73");
    let missing = b.dir.join("missing.sng");
    assert!(matches!(
        g.pack(&missing, &b.dir.join("tune.sid"), &options()),
        Err(PackError::NoSong(p)) if p == missing
    ));
    assert!(matches!(
        g.pack(&b.song, &b.dir.join("tune.SID"), &options()),
        Err(PackError::Format(_))
    ));
    // Not a song: gt2reloc would pack an empty tune, or crash.
    let junk = b.dir.join("junk.sng");
    for bytes in [&b"\x00\x01junk"[..], b"GTS?", b"GT"] {
        std::fs::write(&junk, bytes).unwrap();
        assert!(matches!(
            g.pack(&junk, &b.dir.join("tune.sid"), &options()),
            Err(PackError::NotASong(p)) if p == junk
        ));
    }
    let gone = b.dir.join("no-such-dir");
    assert!(matches!(
        g.pack(&b.song, &gone.join("tune.sid"), &options()),
        Err(PackError::NoOutputDir(p)) if p == gone
    ));
    assert!(!b.dir.join("args.txt").exists(), "gt2reloc was not started");
    assert!(b.leftovers().is_empty());
}

#[test]
fn a_relative_program_path_is_made_absolute() {
    // It runs in a working directory of its own, so a path relative to ours must not be resolved from there.
    let g = Gt2Reloc::at("tools/gt2reloc", "2.73");
    assert!(g.program.is_absolute(), "{:?}", g.program);
    assert!(g.program.ends_with("tools/gt2reloc"));
    // A bare name is looked up on PATH, so it stays as it is.
    assert_eq!(
        Gt2Reloc::at("gt2reloc", "2.73").program,
        Path::new("gt2reloc")
    );
}

/// The real gt2reloc, if it is installed (CI does not install it): the fixture song packs to the size the #20 spike
/// setup reports, as a PSID file.
#[test]
fn the_real_gt2reloc_packs_the_fixture_song() {
    let Ok(g) = Gt2Reloc::locate("2.73") else {
        eprintln!("skipped: gt2reloc is not installed");
        return;
    };
    let song =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gt2-original/sixteen-steps.sng");
    let dir = std::env::temp_dir().join(format!("score64-real-gt2reloc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("sixteen-steps.sid");
    let packed = g.pack(&song, &out, &options());
    let bytes = std::fs::read(&out);
    let _ = std::fs::remove_dir_all(&dir);
    let packed = packed.unwrap();
    assert_eq!(packed.size, 2062);
    assert_eq!(&bytes.unwrap()[..4], b"PSID");
}
