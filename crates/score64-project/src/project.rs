//! A project's files (ADR-014): `score64.toml`, what every track of the target shares, and each track's
//! `tracks/<id>/track.toml`, its sources, subtunes, adaptive sets and rule settings, with `decisions.toml` beside it.
//!
//! ```text
//! score64.toml
//! presets/
//! tracks/<track>/track.toml
//! tracks/<track>/decisions.toml
//! .score64/                      cache and index, not committed
//! ```
//!
//! Both files follow ADR-014 §4: `schema = 1` (absent means 1, newer is refused), unknown keys are errors, errors name
//! the file, line and column, writes are deterministic and atomic, and paths stay inside the project.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use score64_core::{AdaptiveSet, Profile, ProfileName, Profiles, SubtuneName, VideoStandard};

use score64_core::validate::RoleRanges;

use crate::decisions::DecisionsFile;
use crate::textfile::{Schema, headers, is_device_name, line_col, write_atomic};

/// The layout version these files are read and written in.
pub const SCHEMA: u32 = crate::textfile::SCHEMA;

/// The project file's name, at the project root.
pub const PROJECT_FILE: &str = "score64.toml";
/// A track's file, in `tracks/<id>/`.
pub const TRACK_FILE: &str = "track.toml";

/// Why a project or track file could not be read or written.
#[derive(Debug)]
pub enum Error {
    /// The file could not be read or written.
    Io {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// The file is not valid: bad TOML, an unknown key, a value out of range, or something that refers to what
    /// isn't there.
    Invalid {
        /// The file.
        path: PathBuf,
        /// The 1-based line and column, when known.
        at: Option<(usize, usize)>,
        /// The reason.
        message: String,
    },
    /// The track's `decisions.toml` could not be read.
    Decisions(crate::decisions::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Invalid { path, at, message } => match at {
                Some((line, col)) => write!(f, "{}:{line}:{col}: {message}", path.display()),
                None => write!(f, "{}: {message}", path.display()),
            },
            Error::Decisions(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A blake3 hash: 64 hex digits, written lowercase.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Blake3(String);

impl Blake3 {
    /// The hash of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Blake3(blake3::hash(bytes).to_hex().to_string())
    }

    /// The hash, lowercase hex.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Blake3 {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Blake3(s.to_ascii_lowercase()))
        } else {
            Err(format!("a blake3 hash is 64 hex digits, not {s:?}"))
        }
    }
}

impl From<Blake3> for String {
    fn from(h: Blake3) -> String {
        h.0
    }
}

/// A path inside the project, relative to its root, with `/` between parts: never absolute, never `..`, never empty,
/// and with parts every system can store (no Windows device names, no part ending in `.` or a space, no control or
/// bidirectional-override characters). Paths are compared as written, byte for byte; they are not Unicode-normalised.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectPath(String);

impl ProjectPath {
    /// The path under `root`.
    pub fn under(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(root.to_path_buf(), |p, part| p.join(part))
    }

    /// The path as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProjectPath {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let bad = |why: &str| Err(format!("path {s:?} {why}"));
        if s.is_empty() {
            return bad("is empty");
        }
        if s.starts_with('/') || s.contains('\\') || s.contains(':') {
            return bad("must be relative to the project root, with `/` between parts");
        }
        if s.split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return bad("must stay inside the project, with no empty, `.` or `..` parts");
        }
        if s.chars().any(|c| {
            c.is_control() || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        }) {
            return bad("has a control or bidirectional-override character");
        }
        if let Some(part) = s
            .split('/')
            .find(|p| p.ends_with('.') || p.ends_with(' ') || is_device_name(p))
        {
            return bad(&format!(
                "has a part, {part:?}, that Windows can't store (a device name, or one ending in `.` or a space)"
            ));
        }
        Ok(ProjectPath(s))
    }
}

impl From<ProjectPath> for String {
    fn from(p: ProjectPath) -> String {
        p.0
    }
}

/// A short identifier in a file (a source id, a track id): letters, digits, `-` and `_`, starting with a letter or
/// digit, at most 64 characters, and not a Windows device name (`con`, `com1`, ...), so it is also a safe directory
/// name on every system (as the cache's names are, TAD §4).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Id(String);

impl Id {
    /// `s`, if it is a valid id.
    pub fn new(s: &str) -> Result<Self, String> {
        Self::try_from(s.to_string())
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Id {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            && !is_device_name(&s);
        if ok {
            Ok(Id(s))
        } else {
            Err(format!(
                "id {s:?} must be letters, digits, `-` and `_`, starting with a letter or digit, at most 64 long, and not a \
                 Windows device name"
            ))
        }
    }
}

impl From<Id> for String {
    fn from(i: Id) -> String {
        i.0
    }
}

/// A time in a source, exact to the microsecond, written `[h:]m:ss` with up to six decimals (`"0:12.500"`,
/// `"1:14"`, `"1:02:03.000250"`). Never a float, so a range is exact and diffs cleanly (ADR-014). Any `u64` of
/// microseconds is written and read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Timestamp(u64);

impl Timestamp {
    /// `us` microseconds from the start.
    pub fn from_micros(us: u64) -> Self {
        Timestamp(us)
    }

    /// Microseconds from the start.
    pub fn micros(self) -> u64 {
        self.0
    }
}

impl std::str::FromStr for Timestamp {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let bad =
            || format!("time {s:?} must be [h:]m:ss with up to six decimals, such as \"0:12.500\"");
        let parts: Vec<&str> = s.split(':').collect();
        let (hours, minutes, seconds) = match parts.as_slice() {
            [m, sec] => ("0", *m, *sec),
            [h, m, sec] => (*h, *m, *sec),
            _ => return Err(bad()),
        };
        let digits = |t: &str, max: usize| {
            !t.is_empty() && t.len() <= max && t.bytes().all(|b| b.is_ascii_digit())
        };
        let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
        if !digits(hours, 20)
            || !digits(minutes, 20)
            || whole.len() != 2
            || !digits(whole, 2)
            || (seconds.contains('.') && !digits(fraction, 6))
        {
            return Err(bad());
        }
        let (h, m, sec): (u64, u64, u64) = (
            hours.parse().map_err(|_| bad())?,
            minutes.parse().map_err(|_| bad())?,
            whole.parse().map_err(|_| bad())?,
        );
        if sec >= 60 || (parts.len() == 3 && (m >= 60 || minutes.len() != 2)) {
            return Err(bad());
        }
        let micros: u64 = format!("{fraction:0<6}").parse().map_err(|_| bad())?;
        h.checked_mul(60)
            .and_then(|t| t.checked_add(m))
            .and_then(|t| t.checked_mul(60))
            .and_then(|t| t.checked_add(sec))
            .and_then(|t| t.checked_mul(1_000_000))
            .and_then(|t| t.checked_add(micros))
            .map(Timestamp)
            .ok_or_else(|| format!("time {s:?} is too long"))
    }
}

impl fmt::Display for Timestamp {
    /// The canonical form: `m:ss`, or `h:mm:ss` from an hour on; a fraction only when there is one, with three
    /// digits when the time is whole milliseconds and six otherwise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (secs, micros) = (self.0 / 1_000_000, self.0 % 1_000_000);
        let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        if h > 0 {
            write!(f, "{h}:{m:02}:{s:02}")?;
        } else {
            write!(f, "{m}:{s:02}")?;
        }
        if micros > 0 {
            if micros % 1000 == 0 {
                write!(f, ".{:03}", micros / 1000)?;
            } else {
                write!(f, ".{micros:06}")?;
            }
        }
        Ok(())
    }
}

impl TryFrom<String> for Timestamp {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

impl From<Timestamp> for String {
    fn from(t: Timestamp) -> String {
        t.to_string()
    }
}

/// `[target]`: what the music plays on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// PAL (the default) or NTSC.
    #[serde(default)]
    pub video: VideoStandard,
}

impl Target {
    fn is_default(&self) -> bool {
        *self == Target::default()
    }
}

/// `[pack]`: how gt2reloc packs the player (#50). Where it sits and its zero page come from the target's memory map,
/// so they have no defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    /// The player's start address, high byte (`$10` puts it at `$1000`).
    pub player_page: u8,
    /// The first of the player's two zero-page bytes.
    pub zero_page: u8,
    /// Sound-effect support (`-D1`).
    pub sound_effects: bool,
    /// The hard-restart ADSR value; gt2reloc's default (`$0F00`) when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_restart_adsr: Option<u16>,
}

impl Pack {
    fn check(&self) -> Result<(), String> {
        if self.player_page < 0x02 {
            return Err(format!(
                "[pack] player_page ${:02X}: the player must start at $0200 or above, clear of zero page and the stack",
                self.player_page
            ));
        }
        if !(0x02..=0xFE).contains(&self.zero_page) {
            return Err(format!(
                "[pack] zero_page ${:02X}: the pair must start in $02–$FE, clear of the 6510 port at $00–$01 and with both \
                 bytes in zero page",
                self.zero_page
            ));
        }
        Ok(())
    }
}

/// A track's `[pack]`: any of the project's keys, overridden.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackOverride {
    /// Overrides `player_page`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_page: Option<u8>,
    /// Overrides `zero_page`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero_page: Option<u8>,
    /// Overrides `sound_effects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_effects: Option<bool>,
    /// Overrides `hard_restart_adsr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_restart_adsr: Option<u16>,
}

/// A track's `[target]`: the project's, overridden.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetOverride {
    /// Overrides `video`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoStandard>,
}

/// The packer pin (ADR-014): gt2reloc's banner version, and the blake3 of packing Score64's reference fixture with
/// the reference options, which is the same on every platform for the same packer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gt2relocPin {
    /// The version its banner names.
    pub version: String,
    /// The reference pack's blake3.
    pub reference: Blake3,
}

/// `[tools]`: the pinned external programs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    /// The packer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gt2reloc: Option<Gt2relocPin>,
}

impl Tools {
    fn is_empty(&self) -> bool {
        self.gt2reloc.is_none()
    }
}

/// `[validate]`: settings for the validator's rules (TAD §1.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateSettings {
    /// Notes on an expendable voice longer than this many rows get a warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expendable_long_rows: Option<u32>,
    /// Role pitch ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranges: Option<RoleRanges>,
}

impl ValidateSettings {
    fn is_empty(&self) -> bool {
        self.expendable_long_rows.is_none() && self.ranges.is_none()
    }
}

/// `score64.toml`: what every track of the target shares.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    #[serde(default)]
    schema: Schema,
    /// The target.
    #[serde(default, skip_serializing_if = "Target::is_default")]
    pub target: Target,
    /// The target's voice plan: profiles added to the built-in ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile: Vec<Profile>,
    /// How to pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<Pack>,
    /// Pinned tools.
    #[serde(default, skip_serializing_if = "Tools::is_empty")]
    pub tools: Tools,
    /// Validator settings.
    #[serde(default, skip_serializing_if = "ValidateSettings::is_empty")]
    pub validate: ValidateSettings,
}

/// What a source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A full mix.
    Mix,
    /// One stem.
    Stem,
    /// A MIDI file.
    Midi,
}

/// `[[source]]`: a file the track is made from, found by path and identified by its content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Its id in this track.
    pub id: Id,
    /// Where it is, under the project root.
    pub path: ProjectPath,
    /// The blake3 of the content the track was made from.
    pub blake3: Blake3,
    /// What it is.
    pub kind: SourceKind,
}

/// `[[subtune]]`: one subtune of the song, made from (a range of) a source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubtuneSpec {
    /// The subtune's name in the song.
    pub name: SubtuneName,
    /// Its profile, when not the track's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileName>,
    /// The source it is made from.
    pub from: Id,
    /// Where in the source it starts; the source's start when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Timestamp>,
    /// Where it ends; the source's end when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Timestamp>,
}

/// `tracks/<id>/track.toml`: one track's intent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackFile {
    #[serde(default)]
    schema: Schema,
    /// A title for people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The song's default profile; `full` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileName>,
    /// The sources, in file order.
    #[serde(default, rename = "source", skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// The subtunes, in song order.
    #[serde(default, rename = "subtune", skip_serializing_if = "Vec::is_empty")]
    pub subtunes: Vec<SubtuneSpec>,
    /// Adaptive sets over the subtunes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adaptive: Vec<AdaptiveSet>,
    /// Settings for each stage, by stage name; each stage checks its own keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, toml::Table>,
    /// Overrides of the project's `[target]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetOverride>,
    /// Overrides of the project's `[pack]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<PackOverride>,
}

/// Parse `text` as `T`, with errors at their line and column in `path`.
fn parse<T: for<'de> Deserialize<'de>>(text: &str, path: &Path) -> Result<T, Error> {
    toml::from_str(text).map_err(|e| Error::Invalid {
        path: path.to_path_buf(),
        at: e.span().map(|s| line_col(text, s.start)),
        message: e.message().to_string(),
    })
}

/// The line of the first `[table]` header or `key =` line, for errors about that table or key.
fn line_of(text: &str, start: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim_start();
            l.starts_with(start)
                && (start.starts_with('[') || l[start.len()..].trim_start().starts_with('='))
        })
        .map(|i| i + 1)
}

fn invalid(path: &Path, line: Option<usize>, message: String) -> Error {
    Error::Invalid {
        path: path.to_path_buf(),
        at: line.map(|l| (l, 1)),
        message,
    }
}

fn to_toml<T: Serialize>(value: &T, path: &Path) -> Result<String, Error> {
    toml::to_string(value).map_err(|e| invalid(path, None, e.to_string()))
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |error| Error::Io {
        path: path.to_path_buf(),
        error,
    }
}

fn write(path: &Path, text: &str) -> Result<(), Error> {
    write_atomic(path, text).map_err(|(path, error)| Error::Io { path, error })
}

impl ProjectFile {
    /// Parse a `score64.toml`'s text; `path` names it in errors.
    pub fn parse(text: &str, path: &Path) -> Result<Self, Error> {
        let file: ProjectFile = parse(text, path)?;
        if let Some(pack) = &file.pack {
            pack.check()
                .map_err(|m| invalid(path, line_of(text, "[pack]"), m))?;
        }
        let lines = headers(text, "[[profile]]");
        file.profiles()
            .map_err(|m| invalid(path, lines.first().copied(), m))?;
        Ok(file)
    }

    /// The built-in profiles plus this project's.
    pub fn profiles(&self) -> Result<Profiles, String> {
        Profiles::with(self.profile.iter().cloned())
    }

    /// The file's text, for a new file. It passes the same checks as [`ProjectFile::parse`]. Comments and number
    /// bases aren't kept: an existing file is edited in place instead.
    pub fn to_toml(&self) -> Result<String, Error> {
        let path = Path::new(PROJECT_FILE);
        let text = to_toml(self, path)?;
        Self::parse(&text, path)?;
        Ok(text)
    }
}

impl TrackFile {
    /// Parse a `track.toml`'s text; `path` names it in errors. Checks everything the track can check alone: unique
    /// source ids and subtune names, each subtune's source and range, adaptive layers that are the track's subtunes.
    pub fn parse(text: &str, path: &Path) -> Result<Self, Error> {
        let file: TrackFile = parse(text, path)?;
        let (sources, subtunes, sets) = (
            headers(text, "[[source]]"),
            headers(text, "[[subtune]]"),
            headers(text, "[[adaptive]]"),
        );
        let at = |lines: &[usize], i: usize| lines.get(i).copied();
        let mut ids = BTreeSet::new();
        for (i, s) in file.sources.iter().enumerate() {
            if !ids.insert(&s.id) {
                return Err(invalid(
                    path,
                    at(&sources, i),
                    format!("source `{}` is declared twice", s.id.as_str()),
                ));
            }
        }
        let mut names = BTreeSet::new();
        for (i, s) in file.subtunes.iter().enumerate() {
            let line = at(&subtunes, i);
            if !names.insert(&s.name) {
                return Err(invalid(
                    path,
                    line,
                    format!("subtune `{}` is declared twice", s.name.as_str()),
                ));
            }
            if !ids.contains(&s.from) {
                return Err(invalid(
                    path,
                    line,
                    format!(
                        "subtune `{}` is made from `{}`, which no [[source]] declares",
                        s.name.as_str(),
                        s.from.as_str()
                    ),
                ));
            }
            if let (Some(a), Some(b)) = (s.start, s.end)
                && b <= a
            {
                return Err(invalid(
                    path,
                    line,
                    format!(
                        "subtune `{}` ends at {b}, not after its start {a}",
                        s.name.as_str()
                    ),
                ));
            }
        }
        let mut set_names = BTreeSet::new();
        for (i, set) in file.adaptive.iter().enumerate() {
            if !set_names.insert(set.name()) {
                return Err(invalid(
                    path,
                    at(&sets, i),
                    format!("adaptive set `{}` is declared twice", set.name()),
                ));
            }
            if let Some(l) = set.layers().iter().find(|l| !names.contains(l)) {
                return Err(invalid(
                    path,
                    at(&sets, i),
                    format!(
                        "adaptive set `{}` names `{}`, which isn't one of this track's subtunes",
                        set.name(),
                        l.as_str()
                    ),
                ));
            }
        }
        Ok(file)
    }

    /// The file's text, for a new file. It passes the same checks as [`TrackFile::parse`]. Comments and number
    /// bases aren't kept: an existing file is edited in place instead ([`Project::record_hash`]).
    pub fn to_toml(&self) -> Result<String, Error> {
        let path = Path::new(TRACK_FILE);
        let text = to_toml(self, path)?;
        Self::parse(&text, path)?;
        Ok(text)
    }
}

/// `text` with source `id`'s `blake3` value replaced by `hash`, and nothing else changed; `None` if no source has
/// that id.
fn replace_hash(text: &str, path: &Path, id: &Id, hash: &Blake3) -> Result<Option<String>, Error> {
    #[derive(Deserialize)]
    struct Sources {
        #[serde(default)]
        source: Vec<SourceSpan>,
    }
    #[derive(Deserialize)]
    struct SourceSpan {
        id: Id,
        blake3: toml::Spanned<String>,
    }
    let sources: Sources = parse(text, path)?;
    Ok(sources.source.iter().find(|s| &s.id == id).map(|s| {
        let span = s.blake3.span();
        format!(
            "{}\"{}\"{}",
            &text[..span.start],
            hash.as_str(),
            &text[span.end..]
        )
    }))
}

/// A source that isn't as the track says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProblem {
    /// Its file isn't there.
    Missing {
        /// The source.
        id: Id,
        /// Its path.
        path: ProjectPath,
    },
    /// Its file is there but can't be read: a folder, no permission, or a path that leads out of the project
    /// through a symbolic link.
    Unreadable {
        /// The source.
        id: Id,
        /// Its path.
        path: ProjectPath,
        /// What went wrong.
        reason: String,
    },
    /// Its file's content has changed since the track was made.
    Changed {
        /// The source.
        id: Id,
        /// Its path.
        path: ProjectPath,
        /// The hash `track.toml` records.
        expected: Blake3,
        /// The file's hash now.
        found: Blake3,
    },
}

impl fmt::Display for SourceProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceProblem::Missing { id, path } => {
                write!(
                    f,
                    "source `{}`: `{}` is missing",
                    id.as_str(),
                    path.as_str()
                )
            }
            SourceProblem::Unreadable { id, path, reason } => write!(
                f,
                "source `{}`: `{}` can't be read: {reason}",
                id.as_str(),
                path.as_str()
            ),
            SourceProblem::Changed {
                id,
                path,
                expected,
                found,
            } => write!(
                f,
                "source `{}`: `{}` changed since the track was made: expected `{}`, found `{}`",
                id.as_str(),
                path.as_str(),
                expected.as_str(),
                found.as_str()
            ),
        }
    }
}

/// An open project: its root and `score64.toml`.
#[derive(Clone, Debug)]
pub struct Project {
    root: PathBuf,
    /// The project file.
    pub file: ProjectFile,
}

/// A track, loaded with its decisions and the project's values resolved.
#[derive(Clone, Debug)]
pub struct Track {
    /// Its id: its directory's name under `tracks/`.
    pub id: Id,
    /// `track.toml`.
    pub file: TrackFile,
    /// `decisions.toml`, empty when there is none yet.
    pub decisions: DecisionsFile,
    /// The video standard: the track's override, or the project's.
    pub video: VideoStandard,
    /// The pack options: the project's `[pack]` with the track's overrides, if the result is complete.
    pub pack: Option<Pack>,
}

fn read(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|error| Error::Io {
        path: path.to_path_buf(),
        error,
    })
}

impl Project {
    /// Open the project at `root` (the folder holding `score64.toml`).
    pub fn open(root: &Path) -> Result<Self, Error> {
        let path = root.join(PROJECT_FILE);
        let file = ProjectFile::parse(&read(&path)?, &path)?;
        Ok(Project {
            root: root.to_path_buf(),
            file,
        })
    }

    /// The project's root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The ids of the tracks: the folders under `tracks/` that hold a `track.toml`, sorted.
    pub fn track_ids(&self) -> Result<Vec<Id>, Error> {
        let dir = self.root.join("tracks");
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(Error::Io { path: dir, error }),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io(&dir))?;
            if !entry.path().join(TRACK_FILE).is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = Id::new(&name).map_err(|m| {
                invalid(
                    &entry.path(),
                    None,
                    format!("track folder {name:?} isn't a valid track id: {m}"),
                )
            })?;
            ids.push(id);
        }
        ids.sort();
        let mut folded = BTreeMap::new();
        for id in &ids {
            if let Some(other) = folded.insert(id.as_str().to_ascii_lowercase(), id) {
                return Err(invalid(
                    &dir,
                    None,
                    format!(
                        "tracks `{}` and `{}` differ only in case, which some systems can't tell apart",
                        other.as_str(),
                        id.as_str()
                    ),
                ));
            }
        }
        Ok(ids)
    }

    /// Load track `id`: `tracks/<id>/track.toml`, its `decisions.toml` if there is one, and its resolved settings.
    /// Its profiles must be the project's or built-in.
    pub fn track(&self, id: &Id) -> Result<Track, Error> {
        let dir = self.track_dir(id);
        let path = dir.join(TRACK_FILE);
        let text = read(&path)?;
        let file = TrackFile::parse(&text, &path)?;
        let profiles = self
            .file
            .profiles()
            .map_err(|m| invalid(&self.root.join(PROJECT_FILE), None, m))?;
        let subtune_lines = headers(&text, "[[subtune]]");
        let named = file
            .profile
            .iter()
            .map(|p| (p, line_of(&text, "profile")))
            .chain(file.subtunes.iter().enumerate().filter_map(|(i, s)| {
                s.profile
                    .as_ref()
                    .map(|p| (p, subtune_lines.get(i).copied()))
            }));
        for (p, line) in named {
            if profiles.get(p).is_none() {
                return Err(invalid(
                    &path,
                    line,
                    format!("profile `{p}` isn't built in or defined in {PROJECT_FILE}"),
                ));
            }
        }
        let decisions_path = dir.join("decisions.toml");
        let decisions = if decisions_path.is_file() {
            DecisionsFile::read(&decisions_path).map_err(Error::Decisions)?
        } else {
            DecisionsFile::default()
        };
        let video = file
            .target
            .as_ref()
            .and_then(|t| t.video)
            .unwrap_or(self.file.target.video);
        let pack = resolve_pack(self.file.pack.as_ref(), file.pack.as_ref())
            .map_err(|m| invalid(&path, line_of(&text, "[pack]"), m))?;
        Ok(Track {
            id: id.clone(),
            file,
            decisions,
            video,
            pack,
        })
    }

    /// `tracks/<id>/`.
    pub fn track_dir(&self, id: &Id) -> PathBuf {
        self.root.join("tracks").join(id.as_str())
    }

    /// Every source of `track` that is missing, unreadable, or whose content changed since the track recorded its
    /// hash. Each file is hashed as it is read, so a long recording isn't held in memory.
    pub fn check_sources(&self, track: &Track) -> Vec<SourceProblem> {
        track
            .file
            .sources
            .iter()
            .filter_map(|s| {
                let (id, path) = (s.id.clone(), s.path.clone());
                match self.hash_source(&s.path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Some(SourceProblem::Missing { id, path })
                    }
                    Err(e) => Some(SourceProblem::Unreadable {
                        id,
                        path,
                        reason: e.to_string(),
                    }),
                    Ok(found) => (found != s.blake3).then(|| SourceProblem::Changed {
                        id,
                        path,
                        expected: s.blake3.clone(),
                        found,
                    }),
                }
            })
            .collect()
    }

    /// The blake3 of the file at `path`, which must resolve to a file inside the project.
    fn hash_source(&self, path: &ProjectPath) -> std::io::Result<Blake3> {
        let file = path.under(&self.root);
        let (real, root) = (file.canonicalize()?, self.root.canonicalize()?);
        if !real.starts_with(&root) {
            return Err(std::io::Error::other(format!(
                "it leads out of the project, to {}",
                real.display()
            )));
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(std::fs::File::open(&real)?)?;
        Ok(Blake3(hasher.finalize().to_hex().to_string()))
    }

    /// Record `hash` as source `id`'s content in `track`, after its file changed on purpose: only that value changes
    /// in `track.toml`, so its comments and layout stay. False if the track has no such source.
    pub fn record_hash(&self, track: &mut Track, id: &Id, hash: Blake3) -> Result<bool, Error> {
        let path = self.track_dir(&track.id).join(TRACK_FILE);
        let text = read(&path)?;
        let Some(new) = replace_hash(&text, &path, id, &hash)? else {
            return Ok(false);
        };
        let file = TrackFile::parse(&new, &path)?;
        write(&path, &new)?;
        track.file = file;
        Ok(true)
    }

    /// Create a project at `root` with `file` as its `score64.toml`; an error if there is one already.
    pub fn create(root: &Path, file: ProjectFile) -> Result<Self, Error> {
        let path = root.join(PROJECT_FILE);
        create_new(&path, &file.to_toml()?)?;
        Ok(Project {
            root: root.to_path_buf(),
            file,
        })
    }

    /// Create track `id` with `file` as its `track.toml`; an error if it exists already.
    pub fn create_track(&self, id: &Id, file: &TrackFile) -> Result<Track, Error> {
        let dir = self.track_dir(id);
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        create_new(&dir.join(TRACK_FILE), &file.to_toml()?)?;
        self.track(id)
    }
}

/// Write `text` at `path`, which must not exist yet.
fn create_new(path: &Path, text: &str) -> Result<(), Error> {
    if path.exists() {
        return Err(io(path)(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "already exists",
        )));
    }
    write(path, text)
}

/// The project's `[pack]` with a track's overrides: none if neither has one; an error if the track overrides keys
/// but the result lacks a required one.
fn resolve_pack(
    project: Option<&Pack>,
    track: Option<&PackOverride>,
) -> Result<Option<Pack>, String> {
    let Some(o) = track else {
        return Ok(project.cloned());
    };
    let pack = Pack {
        player_page: o
            .player_page
            .or(project.map(|p| p.player_page))
            .ok_or("[pack] needs player_page: the project has no [pack] to take it from")?,
        zero_page: o
            .zero_page
            .or(project.map(|p| p.zero_page))
            .ok_or("[pack] needs zero_page: the project has no [pack] to take it from")?,
        sound_effects: o
            .sound_effects
            .or(project.map(|p| p.sound_effects))
            .ok_or("[pack] needs sound_effects: the project has no [pack] to take it from")?,
        hard_restart_adsr: o
            .hard_restart_adsr
            .or(project.and_then(|p| p.hard_restart_adsr)),
    };
    pack.check()?;
    Ok(Some(pack))
}

#[cfg(test)]
mod tests;
