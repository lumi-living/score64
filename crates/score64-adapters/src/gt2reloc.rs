//! The packer: GoatTracker 2's `gt2reloc`, an external program (#50, `docs/reference/player.md`).
//!
//! `gt2reloc` checks little, so the adapter does it:
//! - it packs an empty tune and exits 0 when the song file is missing or isn't a song, so the song must exist, start
//!   with a `GTS` magic, and show in gt2reloc's report (`song file:` is blank when the magic is wrong). A damaged song
//!   with a good magic still packs, empty or partial: only the `.sng` reader (#47) can catch that, and the pack stage
//!   should read the song with it first;
//! - it ignores a malformed option and uses the default, so options are typed and rendered here, the zero page always
//!   explicitly, and the player and zero-page addresses it reports must be the ones asked for;
//! - it picks the format from the output's extension, case-sensitively, and writes a PRG for anything it doesn't
//!   know, so the extension must be exactly `sid`, `prg` or `bin`;
//! - it has no version flag, so the version is read from the banner it prints while packing and must be the pinned
//!   one.
//!
//! It also crashes on a song path over 59 bytes or an output path over 255 (fixed-size buffers). So it runs in a fresh
//! working directory beside the output, on a copy of the song called `song.sng`, writing `out.<ext>`, which is renamed
//! into place only once every check passes; the directory is removed either way. It runs with an argument list (never
//! a shell) and a time limit, killed with everything it started if it overruns (as the sidecar is).

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use crate::sidecar::{SidecarError, find_on_path, run_with_timeout};

/// Where the player keeps its two zero-page bytes: `address` and `address + 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ZeroPage(u8);

impl ZeroPage {
    /// The pair at `address` and `address + 1`, which must both be in zero page and clear of the 6510's port at
    /// `$00`–`$01`.
    pub fn new(address: u8) -> Result<Self, PackError> {
        if (0x02..=0xFE).contains(&address) {
            Ok(ZeroPage(address))
        } else {
            Err(PackError::Option(format!(
                "zero page ${address:02X}: the pair must lie in $02-$FF"
            )))
        }
    }

    /// The first address.
    pub fn address(self) -> u8 {
        self.0
    }
}

/// PAL or NTSC timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Timing {
    /// PAL (gt2reloc's default).
    Pal,
    /// NTSC.
    Ntsc,
}

/// How to pack. There is no default: the zero page always comes from the game's zero-page map.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackOptions {
    /// The player's start address, high byte (`-W`): `$10` puts it at `$1000`.
    pub player_page: u8,
    /// The player's zero-page pair (`-Z`).
    pub zero_page: ZeroPage,
    /// Sound-effect support (`-D`), which implies buffered SID writes.
    pub sound_effects: bool,
    /// Author info at start+$20…+$3F and timing marks (`-H`), for adaptive sets.
    pub timing_marks: bool,
    /// The hard-restart ADSR value (`-A`).
    pub hard_restart_adsr: u16,
    /// PAL or NTSC (`-P` / `-N`).
    pub timing: Timing,
}

impl PackOptions {
    /// Options for a game build: the player at page `player_page`, its zero-page pair at `zero_page`, sound effects
    /// on, timing marks off, gt2reloc's default hard-restart ADSR (`$0F00`) and PAL.
    pub fn new(player_page: u8, zero_page: ZeroPage) -> Self {
        PackOptions {
            player_page,
            zero_page,
            sound_effects: true,
            timing_marks: false,
            hard_restart_adsr: 0x0F00,
            timing: Timing::Pal,
        }
    }

    /// The options Score64 sets, as gt2reloc's arguments (the rest keep gt2reloc's defaults, fixed by the version
    /// pin), after checking that the player is clear of zero page and the stack.
    pub fn args(&self) -> Result<Vec<String>, PackError> {
        if self.player_page < 0x02 {
            return Err(PackError::Option(format!(
                "player at ${:02X}00: it must start at $0200 or above, clear of zero page and the stack",
                self.player_page
            )));
        }
        let flag = |on| if on { 1 } else { 0 };
        Ok(vec![
            format!("-D{}", flag(self.sound_effects)),
            format!("-H{}", flag(self.timing_marks)),
            format!("-W{:02X}", self.player_page),
            format!("-Z{:02X}", self.zero_page.0),
            format!("-A{:04X}", self.hard_restart_adsr),
            match self.timing {
                Timing::Pal => "-P",
                Timing::Ntsc => "-N",
            }
            .to_string(),
        ])
    }
}

/// What gt2reloc writes, from the output's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// A PSID file (`.sid`).
    Sid,
    /// A C64 program with its load address (`.prg`).
    Prg,
    /// Raw bytes (`.bin`).
    Bin,
}

impl Format {
    /// The format for `path`'s extension, which must be exactly `sid`, `prg` or `bin`: gt2reloc writes a PRG for
    /// anything else, `.SID` included.
    pub fn of(path: &Path) -> Result<Self, PackError> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("sid") => Ok(Format::Sid),
            Some("prg") => Ok(Format::Prg),
            Some("bin") => Ok(Format::Bin),
            _ => Err(PackError::Format(path.to_path_buf())),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Sid => "sid",
            Format::Prg => "prg",
            Format::Bin => "bin",
        }
    }
}

/// A packed tune.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    /// The file written.
    pub path: PathBuf,
    /// Its format.
    pub format: Format,
    /// The packed size gt2reloc reports (player and data, without a file header), for the size budget.
    pub size: u32,
    /// The version from gt2reloc's banner.
    pub version: String,
    /// The blake3 of the gt2reloc program that packed it, lowercase hex, for provenance.
    pub program_blake3: String,
}

/// Why packing failed.
#[derive(Debug)]
pub enum PackError {
    /// gt2reloc isn't on `PATH` and `SCORE64_GT2RELOC` doesn't name it.
    NotFound,
    /// An option is out of range.
    Option(String),
    /// The song file doesn't exist (gt2reloc would pack an empty tune).
    NoSong(PathBuf),
    /// The song file doesn't start with a GoatTracker song magic (`GTS` and a digit).
    NotASong(PathBuf),
    /// The output's directory doesn't exist.
    NoOutputDir(PathBuf),
    /// The output's extension isn't exactly `sid`, `prg` or `bin`.
    Format(PathBuf),
    /// gt2reloc couldn't be started, or the output couldn't be moved into place.
    Io(std::io::Error),
    /// gt2reloc exited with an error.
    Failed {
        /// Exit status.
        status: ExitStatus,
        /// The end of its stderr.
        stderr: String,
    },
    /// gt2reloc ran longer than allowed and was killed.
    Timeout(Duration),
    /// The banner names another version than the pinned one, or none.
    Version {
        /// The version found, if any.
        found: Option<String>,
        /// The pinned version.
        pinned: String,
    },
    /// gt2reloc reported something other than what was asked for, or wrote nothing.
    Mismatch(String),
    /// The gt2reloc program isn't the pinned build: its blake3 differs.
    Binary {
        /// The program's blake3.
        found: String,
        /// The pinned blake3.
        pinned: String,
    },
    /// The packed file couldn't replace the output, for example because another program has it open.
    Replace {
        /// The output.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::NotFound => write!(
                f,
                "gt2reloc is not on PATH and SCORE64_GT2RELOC is not set (see docs/TOOLING.md)"
            ),
            PackError::Option(why) => write!(f, "pack options: {why}"),
            PackError::NoSong(p) => write!(f, "song {} does not exist", p.display()),
            PackError::NotASong(p) => write!(f, "{} is not a GoatTracker song", p.display()),
            PackError::NoOutputDir(p) => {
                write!(f, "output directory {} does not exist", p.display())
            }
            PackError::Format(p) => write!(
                f,
                "{}: the output must end in .sid, .prg or .bin (lowercase)",
                p.display()
            ),
            PackError::Io(e) => write!(f, "gt2reloc: {e}"),
            PackError::Failed { status, stderr } => {
                write!(f, "gt2reloc failed ({status}):\n{stderr}")
            }
            PackError::Timeout(after) => {
                write!(f, "gt2reloc killed after {:.0} s", after.as_secs_f64())
            }
            PackError::Version { found, pinned } => match found {
                Some(v) => write!(f, "gt2reloc is version {v}; the project pins {pinned}"),
                None => write!(
                    f,
                    "gt2reloc printed no version banner; the project pins {pinned}"
                ),
            },
            PackError::Mismatch(why) => write!(f, "gt2reloc: {why}"),
            PackError::Binary { found, pinned } => write!(
                f,
                "gt2reloc's blake3 is {found}; the project pins {pinned} (another build, or another version)"
            ),
            PackError::Replace { path, error } => write!(
                f,
                "could not replace {}: {error} (is it open in another program, such as an emulator?)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PackError {}

impl From<SidecarError> for PackError {
    fn from(e: SidecarError) -> Self {
        match e {
            SidecarError::Spawn(e) => PackError::Io(e),
            SidecarError::Failed { status, stderr } => PackError::Failed { status, stderr },
            SidecarError::Timeout { after, .. } => PackError::Timeout(after),
            other => PackError::Mismatch(other.to_string()),
        }
    }
}

/// A located gt2reloc, checked against a pinned version on every run.
#[derive(Clone, Debug)]
pub struct Gt2Reloc {
    program: PathBuf,
    pinned: String,
    binary: Option<String>,
    timeout: Duration,
}

impl Gt2Reloc {
    /// Find gt2reloc (`SCORE64_GT2RELOC`, else on `PATH`). `pinned` is the version its banner must name, as the
    /// project records it (for example `2.73`).
    pub fn locate(pinned: &str) -> Result<Self, PackError> {
        let program = match std::env::var_os("SCORE64_GT2RELOC") {
            Some(p) => PathBuf::from(p),
            None => find_on_path("gt2reloc").ok_or(PackError::NotFound)?,
        };
        Ok(Self::at(program, pinned))
    }

    /// gt2reloc at `program`.
    pub fn at(program: impl Into<PathBuf>, pinned: &str) -> Self {
        let program = program.into();
        // It runs in a working directory of its own, so a relative path must not be resolved from there.
        let program = if program.components().count() > 1 {
            std::path::absolute(&program).unwrap_or(program)
        } else {
            program
        };
        Gt2Reloc {
            program,
            pinned: pinned.to_string(),
            binary: None,
            timeout: Duration::from_secs(60),
        }
    }

    /// Also pin the program itself by its blake3 (hex, any case). The banner can't tell builds apart (the 2.77 source
    /// prints v2.73), so this is the stronger pin; every pack checks it before running.
    pub fn with_blake3(mut self, hex: &str) -> Self {
        self.binary = Some(hex.to_ascii_lowercase());
        self
    }

    /// The program's blake3, lowercase hex. A bare name is looked up on `PATH`.
    pub fn program_blake3(&self) -> Result<String, PackError> {
        let path = if self.program.components().count() > 1 {
            self.program.clone()
        } else {
            find_on_path(&self.program.to_string_lossy()).ok_or(PackError::NotFound)?
        };
        let bytes = std::fs::read(&path).map_err(PackError::Io)?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    /// Change the time limit (default 60 seconds; packing takes well under one).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Pack `song` (a `.sng`) into `out`, in the format its extension names.
    pub fn pack(
        &self,
        song: &Path,
        out: &Path,
        options: &PackOptions,
    ) -> Result<Packed, PackError> {
        let args = options.args()?;
        let format = Format::of(out)?;
        if !song.is_file() {
            return Err(PackError::NoSong(song.to_path_buf()));
        }
        let mut magic = [0; 4];
        let is_song = std::fs::File::open(song)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic))
            .is_ok()
            && magic.starts_with(b"GTS")
            && magic[3].is_ascii_digit();
        if !is_song {
            return Err(PackError::NotASong(song.to_path_buf()));
        }
        let dir = match out.parent() {
            Some(d) if d.as_os_str().is_empty() => Path::new("."),
            Some(d) => d,
            None => Path::new("."),
        };
        if !dir.is_dir() {
            return Err(PackError::NoOutputDir(dir.to_path_buf()));
        }
        let program_blake3 = self.program_blake3()?;
        if let Some(pinned) = &self.binary
            && *pinned != program_blake3
        {
            return Err(PackError::Binary {
                found: program_blake3,
                pinned: pinned.clone(),
            });
        }
        // A working directory of its own beside the output, with short fixed names: gt2reloc crashes on a song path
        // over 59 bytes or an output path over 255. Removed when this returns, panics included.
        let guard = WorkDir::create(out)?;
        let work = guard.0.as_path();
        let name = format!("out.{}", format.extension());
        let packed = work.join(&name);
        std::fs::copy(song, work.join("song.sng"))
            .map_err(PackError::Io)
            .and_then(|_| self.run(work, &name, &args))
            .and_then(|stdout| {
                let report = Report::parse(&String::from_utf8_lossy(&stdout));
                report.check(options, &self.pinned)?;
                let size = report
                    .size
                    .ok_or_else(|| PackError::Mismatch("no total size reported".into()))?;
                match std::fs::metadata(&packed) {
                    Ok(m) if m.len() > 0 => {}
                    _ => return Err(PackError::Mismatch("no output written".into())),
                }
                std::fs::rename(&packed, out).map_err(|error| PackError::Replace {
                    path: out.to_path_buf(),
                    error,
                })?;
                Ok(Packed {
                    path: out.to_path_buf(),
                    format,
                    size,
                    version: report.version.expect("checked"),
                    program_blake3,
                })
            })
    }

    fn run(&self, work: &Path, out: &str, args: &[String]) -> Result<Vec<u8>, PackError> {
        let mut cmd = Command::new(&self.program);
        cmd.current_dir(work)
            .args(["song.sng", out])
            .args(args)
            .stdin(Stdio::null());
        Ok(run_with_timeout(cmd, self.timeout, true)?)
    }
}

/// The working directory, removed when dropped.
struct WorkDir(PathBuf);

impl WorkDir {
    fn create(out: &Path) -> Result<Self, PackError> {
        let path = work_dir(out);
        std::fs::create_dir(&path).map_err(PackError::Io)?;
        Ok(WorkDir(path))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh directory name beside `out`, unique to this process and call.
fn work_dir(out: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = std::ffi::OsString::from(".");
    name.push(out.file_stem().unwrap_or_default());
    name.push(format!(
        ".{}-{}.gt2reloc",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    out.with_file_name(name)
}

/// What gt2reloc printed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    version: Option<String>,
    song_loaded: bool,
    player: Option<u16>,
    zero_page: Option<u16>,
    size: Option<u32>,
}

impl Report {
    /// Read the banner (`GoatTracker v2.73 Packer/Relocator`), `song file:       song.sng` (blank when it could not
    /// load the song), `Player address:   $1000`,
    /// `Zeropage address: $00F8` and `Total size:      2062 bytes`.
    fn parse(stdout: &str) -> Self {
        let mut r = Report::default();
        let hex = |v: &str| u16::from_str_radix(v.trim().trim_start_matches('$'), 16).ok();
        for line in stdout.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("GoatTracker v")
                && let Some(version) = rest.strip_suffix(" Packer/Relocator")
            {
                r.version = Some(version.trim().to_string());
            } else if let Some(v) = line.strip_prefix("song file:") {
                r.song_loaded = !v.trim().is_empty();
            } else if let Some(v) = line.strip_prefix("Player address:") {
                r.player = hex(v);
            } else if let Some(v) = line.strip_prefix("Zeropage address:") {
                r.zero_page = hex(v);
            } else if let Some(v) = line.strip_prefix("Total size:") {
                r.size = v
                    .trim()
                    .strip_suffix("bytes")
                    .and_then(|n| n.trim().parse().ok());
            }
        }
        r
    }

    fn check(&self, options: &PackOptions, pinned: &str) -> Result<(), PackError> {
        if self.version.as_deref() != Some(pinned) {
            return Err(PackError::Version {
                found: self.version.clone(),
                pinned: pinned.to_string(),
            });
        }
        if !self.song_loaded {
            return Err(PackError::Mismatch(
                "it did not load the song (blank `song file:`)".into(),
            ));
        }
        let player = u16::from(options.player_page) << 8;
        if self.player != Some(player) {
            return Err(PackError::Mismatch(format!(
                "player at {}, not ${player:04X} as asked",
                shown(self.player)
            )));
        }
        let zero_page = u16::from(options.zero_page.0);
        if self.zero_page != Some(zero_page) {
            return Err(PackError::Mismatch(format!(
                "zero page at {}, not ${zero_page:04X} as asked",
                shown(self.zero_page)
            )));
        }
        Ok(())
    }
}

fn shown(address: Option<u16>) -> String {
    address.map_or("an unreported address".into(), |a| format!("${a:04X}"))
}

#[cfg(test)]
mod tests;
