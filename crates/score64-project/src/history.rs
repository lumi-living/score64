//! A track's run history (ADR-014 §7, #27): one file per recorded run in `tracks/<id>/history/`, committed with the
//! track. A version id is content-addressed, so restoring an old intent gives its old id back; the history says which
//! version came from which, who made the change, and when.
//!
//! ```toml
//! schema = 1
//! version = "…"                 # the full version id (ADR-014 §6)
//! parent = "…"                  # the version that was latest when it was recorded; absent for the first
//! time = 2026-10-24T09:15:02Z   # UTC, whole seconds
//! restore = true                # only on a restore
//!
//! [provenance]
//! by = "manual"                 # "rules", "llm" (with `model`) or "manual" (with `name`)
//! name = "A. Person"
//!
//! [[uses]]                      # the cache entries the run's version was made from and into (#28)
//! stage = "transcribe"
//! key = "…"
//! ```
//!
//! A file is named by its run's time and the first 12 hex digits of its version,
//! `2026-10-24T091502Z-1a2b3c4d5e6f.toml`, with `-2`, `-3`, … for further runs in the same second (whatever their
//! versions), so entries order by time and then that number, and names hold no character Windows refuses. A run is recorded only when its version differs from the latest
//! recorded one, or when it restores an earlier one.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::value::{Date, Datetime, Offset, Time};

use crate::textfile::{Schema, line_col};
use crate::version::VersionId;

/// The layout version this build reads and writes.
pub const SCHEMA: u32 = crate::textfile::SCHEMA;

/// 9999-12-31T23:59:59Z, the last time a name can hold.
const LAST_SECOND: i64 = 253_402_300_799;

/// Characters of the time at the start of a file's name, `2026-10-24T091502Z`.
const TIME_CHARS: usize = 18;

/// How far ahead of a run's clock the latest entry may be before recording refuses, in seconds: one day. Within it,
/// the run is recorded at the latest entry's time, so the order holds; beyond it, the latest entry's clock was surely
/// wrong, and pinning every later run to its time would hide when they happened.
pub const MAX_AHEAD: i64 = 86_400;

/// Hex digits of the version in a file's name.
const NAME_DIGITS: usize = 12;

/// A moment in UTC, in whole seconds since 1970-01-01T00:00:00Z; written as a TOML datetime with offset `Z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Datetime", into = "Datetime")]
pub struct UtcTime(i64);

impl UtcTime {
    /// `seconds` after 1970-01-01T00:00:00Z, if the year stays within 1970–9999.
    pub fn from_unix(seconds: i64) -> Option<Self> {
        let t = UtcTime(seconds);
        (0..=LAST_SECOND).contains(&seconds).then_some(t)
    }

    /// The system clock, to the second.
    pub fn now() -> Self {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // A clock outside 1970–9999 is held at the edge, so the time always writes and reads back.
        UtcTime(i64::try_from(since).unwrap_or(i64::MAX).min(LAST_SECOND))
    }

    /// Seconds since 1970-01-01T00:00:00Z.
    pub fn unix(self) -> i64 {
        self.0
    }

    /// (year, month, day, hour, minute, second).
    fn civil(self) -> (i64, u8, u8, u8, u8, u8) {
        let (days, secs) = (self.0.div_euclid(86_400), self.0.rem_euclid(86_400));
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = yoe + era * 400 + i64::from(month <= 2);
        let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        (year, month, day, h as u8, m as u8, s as u8)
    }

    /// The UTC time of a civil date and time, if it is a real one.
    fn from_civil(year: i64, month: u8, day: u8, h: u8, m: u8, s: u8) -> Option<Self> {
        let days_in = |y: i64, mo: u8| match mo {
            2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if !(1..=12).contains(&month)
            || day == 0
            || day > days_in(year, month)
            || h > 23
            || m > 59
            || s > 59
        {
            return None;
        }
        // Howard Hinnant's days_from_civil.
        let y = year - i64::from(month <= 2);
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = i64::from((month + 9) % 12);
        let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        UtcTime::from_unix(days * 86_400 + i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s))
    }

    /// `2026-10-24T091502Z`: the time as it starts a file's name.
    fn name_part(self) -> String {
        let (y, mo, d, h, m, s) = self.civil();
        format!("{y:04}-{mo:02}-{d:02}T{h:02}{m:02}{s:02}Z")
    }
}

impl std::str::FromStr for UtcTime {
    type Err = String;
    /// `2026-10-24T09:15:02Z`, as [`fmt::Display`] writes it.
    fn from_str(s: &str) -> Result<Self, String> {
        let dt: Datetime = s
            .parse()
            .map_err(|_| format!("{s:?} is not a UTC time like 2026-10-24T09:15:02Z"))?;
        UtcTime::try_from(dt)
    }
}

impl fmt::Display for UtcTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, mo, d, h, m, s) = self.civil();
        write!(f, "{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
    }
}

impl From<UtcTime> for Datetime {
    fn from(t: UtcTime) -> Datetime {
        let (y, mo, d, h, m, s) = t.civil();
        Datetime {
            date: Some(Date {
                year: y as u16,
                month: mo,
                day: d,
            }),
            time: Some(Time {
                hour: h,
                minute: m,
                second: Some(s),
                nanosecond: None,
            }),
            offset: Some(Offset::Z),
        }
    }
}

impl TryFrom<Datetime> for UtcTime {
    type Error = String;
    fn try_from(dt: Datetime) -> Result<Self, String> {
        let wanted = "a UTC time in whole seconds, like 2026-10-24T09:15:02Z";
        match dt {
            Datetime {
                date: Some(date),
                time:
                    Some(Time {
                        hour,
                        minute,
                        second: Some(second),
                        nanosecond: None | Some(0),
                    }),
                offset: Some(Offset::Z),
            } => UtcTime::from_civil(
                i64::from(date.year),
                date.month,
                date.day,
                hour,
                minute,
                second,
            )
            .ok_or_else(|| {
                format!("{dt} is not a real date and time from 1970 to 9999; expected {wanted}")
            }),
            _ => Err(format!("{dt} is not {wanted}")),
        }
    }
}

/// Who made a recorded run's change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunProvenance {
    /// A re-run that only picked up new tool or rule versions.
    Rules {},
    /// A model decider (ADR-008).
    Llm {
        /// The model id.
        model: String,
    },
    /// A person who edited the track or its decisions.
    Manual {
        /// Their git `user.name`.
        name: String,
    },
}

/// A cache entry a recorded version uses: its stage and key (`.score64/cache/<stage>/<key>/`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRef {
    /// The stage.
    pub stage: String,
    /// The entry's key.
    pub key: score64_core::ContentHash,
}

/// One recorded run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Layout version; absent means 1.
    #[serde(default)]
    schema: Schema,
    /// The run's version id.
    pub version: VersionId,
    /// The version that was latest when it was recorded (on this machine or branch); `None` for the first. Versions
    /// repeat with restores, so following parents can come back to a version: it is a history, not a tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<VersionId>,
    /// When it ran, or the latest entry's time if that was later (by under [`MAX_AHEAD`]), so entries keep their order.
    pub time: UtcTime,
    /// Whether it restored an earlier version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore: bool,
    /// Who made the change.
    pub provenance: RunProvenance,
    /// The cache entries its version was made from and into, sorted and without repeats: what the index lists for
    /// it, and what GC keeps (#28, #29).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<CacheRef>,
}

/// A run to record: its version, time and provenance, and whether it restores an earlier version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// Its version id.
    pub version: VersionId,
    /// When it ran.
    pub time: UtcTime,
    /// Who made the change.
    pub provenance: RunProvenance,
    /// Whether it restores an earlier version.
    pub restore: bool,
    /// The cache entries its version was made from and into, in any order.
    pub uses: Vec<CacheRef>,
}

/// A track's recorded runs, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    dir: PathBuf,
    entries: Vec<(String, Entry)>,
}

/// Why a history could not be read or recorded.
#[derive(Debug)]
pub enum Error {
    /// A file or the folder could not be read or written.
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// A file is not a valid history entry, or doesn't fit with the others.
    Invalid {
        /// The file.
        path: PathBuf,
        /// The 1-based line and column, when known.
        at: Option<(usize, usize)>,
        /// The reason.
        message: String,
    },
    /// A restore of a version the history doesn't hold.
    NotRecorded(VersionId),
    /// A provenance that names no model or person.
    NoOne(&'static str),
    /// A `uses` stage the cache can't hold.
    BadStage(String),
    /// Every number for runs in this second (up to [`MAX_IN_SECOND`]) is taken.
    Crowded(String),
    /// The latest entry is dated more than [`MAX_AHEAD`] after the run: a clock was wrong when it was recorded.
    Ahead {
        /// The latest entry's file.
        path: PathBuf,
        /// Its time.
        time: UtcTime,
        /// The run's.
        run: UtcTime,
    },
}

/// The most runs one second can hold: names number them `-2` to `-9999`.
pub const MAX_IN_SECOND: u32 = 9999;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Invalid { path, at, message } => match at {
                Some((line, col)) => write!(f, "{}:{line}:{col}: {message}", path.display()),
                None => write!(f, "{}: {message}", path.display()),
            },
            Error::NotRecorded(v) => {
                write!(f, "version {v} was never recorded, so it can't be restored")
            }
            Error::NoOne(why) => f.write_str(why),
            Error::BadStage(stage) => {
                write!(f, "`uses` names a stage `{stage}` the cache can't hold")
            }
            Error::Crowded(second) => write!(f, "{second} already holds {MAX_IN_SECOND} runs"),
            Error::Ahead { path, time, run } => write!(
                f,
                "{}: the latest run is dated {time}, more than a day after this run ({run}); a clock was wrong when it \
                 was recorded. Fix that entry's time and name, or this machine's clock",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl History {
    /// The history in `dir` (a track's `history/` folder): empty when the folder doesn't exist.
    ///
    /// Every `.toml` file in it must be an entry named for its time and version. Files whose names start with `.`
    /// (`.DS_Store`, editors' swap files) and files that don't end in `.toml` (`Thumbs.db`, writers' `.part` files,
    /// `x.toml~`) are skipped. The entries form a graph, not a single chain, because two machines or two git branches
    /// can each add runs after the same version:
    /// - each entry's parent, if it has one, is a version some entry in the folder records;
    /// - no entry is its own parent;
    /// - a restore's version is one another entry records.
    ///
    /// The entries are ordered by time, then by same-second number, then by name. The latest is the last of them.
    pub fn read(dir: &Path) -> Result<Self, Error> {
        let listing = match std::fs::read_dir(dir) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(History {
                    dir: dir.to_path_buf(),
                    entries: vec![],
                });
            }
            Err(error) => {
                return Err(Error::Io {
                    path: dir.to_path_buf(),
                    error,
                });
            }
        };
        let mut names = Vec::new();
        for item in listing {
            let item = item.map_err(|error| Error::Io {
                path: dir.to_path_buf(),
                error,
            })?;
            let raw = item.file_name();
            let bytes = raw.as_encoded_bytes();
            if bytes.starts_with(b".") || !bytes.ends_with(b".toml") {
                continue;
            }
            let Some(name) = raw.to_str() else {
                return Err(Error::Invalid {
                    path: item.path(),
                    at: None,
                    message: "not a history entry: the name isn't valid UTF-8".into(),
                });
            };
            if item.file_type().is_ok_and(|t| t.is_symlink()) {
                return Err(Error::Invalid {
                    path: item.path(),
                    at: None,
                    message: "a symbolic link, where a history entry belongs".into(),
                });
            }
            names.push(name.to_owned());
        }
        names.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)).then(a.cmp(b)));
        let mut entries: Vec<(String, Entry)> = Vec::with_capacity(names.len());
        let mut texts = Vec::with_capacity(names.len());
        for name in names {
            let path = dir.join(&name);
            let invalid = |at, message: String| Error::Invalid {
                path: path.clone(),
                at,
                message,
            };
            let Some((time, prefix)) = parse_name(&name) else {
                return Err(invalid(
                    None,
                    "not a history entry: entries are named like 2026-10-24T091502Z-1a2b3c4d5e6f.toml, or with -2 to \
                     -9999 for further runs in that second"
                        .into(),
                ));
            };
            if path.is_dir() {
                return Err(invalid(
                    None,
                    "a folder, where a history entry belongs".into(),
                ));
            }
            let text = std::fs::read_to_string(&path).map_err(|error| Error::Io {
                path: path.clone(),
                error,
            })?;
            let entry: Entry = toml::from_str(&text).map_err(|e| {
                invalid(
                    e.span().map(|s| line_col(&text, s.start)),
                    e.message().to_string(),
                )
            })?;
            entry
                .provenance
                .check()
                .map_err(|why| invalid(key_line(&text, "[provenance]"), why.into()))?;
            if entry.uses.windows(2).any(|w| w[0] >= w[1]) {
                return Err(invalid(
                    key_line(&text, "[[uses]]"),
                    "`uses` must be sorted by stage and key, without repeats".into(),
                ));
            }
            if let Some(bad) = entry
                .uses
                .iter()
                .find(|u| crate::cache::check_name(&u.stage).is_err())
            {
                return Err(invalid(
                    key_line(&text, "[[uses]]"),
                    format!("`uses` names a stage `{}` the cache can't hold", bad.stage),
                ));
            }
            if entry.time.name_part() != time || !entry.version.to_string().starts_with(&prefix) {
                return Err(invalid(
                    None,
                    format!(
                        "the name doesn't match the entry: it holds {} and version {}",
                        entry.time, entry.version
                    ),
                ));
            }
            texts.push(text);
            entries.push((name, entry));
        }
        let recorded: std::collections::BTreeSet<VersionId> =
            entries.iter().map(|(_, e)| e.version).collect();
        for ((name, entry), text) in entries.iter().zip(&texts) {
            let invalid = |key: &str, message: String| Error::Invalid {
                path: dir.join(name),
                at: key_line(text, key),
                message,
            };
            if let Some(parent) = entry.parent {
                if parent == entry.version {
                    return Err(invalid(
                        "parent",
                        "it names its own version as its parent".into(),
                    ));
                }
                if !recorded.contains(&parent) {
                    return Err(invalid(
                        "parent",
                        format!("its parent {parent} isn't a version any entry here records"),
                    ));
                }
            }
            if entry.restore
                && entries
                    .iter()
                    .filter(|(_, e)| e.version == entry.version)
                    .count()
                    < 2
            {
                return Err(invalid(
                    "restore",
                    "it restores a version no other entry here records".into(),
                ));
            }
        }
        Ok(History {
            dir: dir.to_path_buf(),
            entries,
        })
    }

    /// The recorded runs, oldest first, with their file names.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Entry)> {
        self.entries.iter().map(|(n, e)| (n.as_str(), e))
    }

    /// The latest recorded run.
    pub fn latest(&self) -> Option<&Entry> {
        self.entries.last().map(|(_, e)| e)
    }

    /// Record `run`, unless it changes nothing. First the folder is read again, so runs other writers recorded
    /// meanwhile count.
    ///
    /// A run whose version is the latest recorded one writes nothing (a restore of it included) and returns `None`.
    /// Otherwise its entry is created in the folder (made if needed), never replacing a file, and its path returned:
    /// - the entry's parent is the latest version;
    /// - its time is the run's, or the latest entry's if that is later (by at most [`MAX_AHEAD`]), so a clock a little
    ///   behind another machine's never puts a new run before the latest;
    /// - its number in its second is one past any already there.
    ///
    /// If another writer takes the name first, the folder is read again and the run decided afresh. Under a race the
    /// history this returns can lag behind the folder; read it again for the current state.
    ///
    /// Refused:
    /// - a restore of a version never recorded;
    /// - a provenance with an empty model or name;
    /// - a `uses` stage the cache can't hold;
    /// - a second already holding [`MAX_IN_SECOND`] runs;
    /// - a latest entry dated more than [`MAX_AHEAD`] after the run ([`Error::Ahead`]).
    pub fn record(&mut self, run: Run) -> Result<Option<PathBuf>, Error> {
        run.provenance.check().map_err(Error::NoOne)?;
        if let Some(bad) = run
            .uses
            .iter()
            .find(|u| crate::cache::check_name(&u.stage).is_err())
        {
            return Err(Error::BadStage(bad.stage.clone()));
        }
        let mut uses = run.uses;
        uses.sort();
        uses.dedup();
        loop {
            *self = History::read(&self.dir)?;
            let latest = self.latest();
            if latest.is_some_and(|l| l.version == run.version) {
                return Ok(None);
            }
            if run.restore && !self.entries.iter().any(|(_, e)| e.version == run.version) {
                return Err(Error::NotRecorded(run.version));
            }
            if let Some((name, l)) = self.entries.last()
                && l.time.unix() - run.time.unix() > MAX_AHEAD
            {
                return Err(Error::Ahead {
                    path: self.dir.join(name),
                    time: l.time,
                    run: run.time,
                });
            }
            let entry = Entry {
                schema: Schema,
                version: run.version,
                parent: latest.map(|l| l.version),
                time: latest.map_or(run.time, |l| run.time.max(l.time)),
                restore: run.restore,
                provenance: run.provenance.clone(),
                uses: uses.clone(),
            };
            let text = toml::to_string(&entry).expect("an entry serialises");
            std::fs::create_dir_all(&self.dir).map_err(|error| Error::Io {
                path: self.dir.clone(),
                error,
            })?;
            let time = entry.time.name_part();
            let n = self
                .entries
                .iter()
                .map(|(name, _)| sort_key(name))
                .filter(|(t, _)| *t == time)
                .map(|(_, n)| n + 1)
                .max()
                .unwrap_or(1);
            if n > MAX_IN_SECOND {
                return Err(Error::Crowded(entry.time.to_string()));
            }
            let stem = format!("{time}-{}", &entry.version.to_string()[..NAME_DIGITS]);
            let name = if n == 1 {
                format!("{stem}.toml")
            } else {
                format!("{stem}-{n}.toml")
            };
            let path = self.dir.join(&name);
            match crate::textfile::create_atomic(&path, &text) {
                Ok(()) => {
                    self.entries.push((name, entry));
                    return Ok(Some(path));
                }
                // Another writer got this name first: see what they recorded, and decide again.
                Err((_, e)) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err((path, error)) => return Err(Error::Io { path, error }),
            }
        }
    }
}

impl RunProvenance {
    fn check(&self) -> Result<(), &'static str> {
        match self {
            RunProvenance::Llm { model } if model.trim().is_empty() => {
                Err("an llm run names no model")
            }
            RunProvenance::Manual { name } if name.trim().is_empty() => {
                Err("a manual run names no one")
            }
            _ => Ok(()),
        }
    }
}

/// The line of the first line in `text` that starts with `key` (a table header, or a key followed by ` =`).
fn key_line(text: &str, key: &str) -> Option<(usize, usize)> {
    text.lines()
        .position(|l| {
            let l = l.trim_start();
            if key.starts_with('[') {
                l.starts_with(key)
            } else {
                l.strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with('='))
            }
        })
        .map(|i| (i + 1, 1))
}

/// The time part and version prefix of an entry's file name, if it is one: `<time>-<12 lowercase hex digits>`, then
/// nothing or `-2` to `-9999` (digits only, no leading zero), then `.toml`.
fn parse_name(name: &str) -> Option<(String, String)> {
    let stem = name.strip_suffix(".toml")?;
    let (time, rest) = stem.split_at_checked(TIME_CHARS)?;
    let rest = rest.strip_prefix('-')?;
    let (prefix, suffix) = rest.split_at_checked(NAME_DIGITS)?;
    let time_ok = time.char_indices().all(|(i, c)| match i {
        4 | 7 => c == '-',
        10 => c == 'T',
        17 => c == 'Z',
        _ => c.is_ascii_digit(),
    });
    let prefix_ok = prefix
        .bytes()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
    let suffix_ok = suffix.is_empty() || number(suffix).is_some();
    (time_ok && prefix_ok && suffix_ok).then(|| (time.to_string(), prefix.to_string()))
}

/// The number in a `-2` … `-9999` suffix.
fn number(suffix: &str) -> Option<u32> {
    let digits = suffix.strip_prefix('-')?;
    let ok = (1..=4).contains(&digits.len())
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    let n: u32 = digits.parse().ok().filter(|_| ok)?;
    (n >= 2).then_some(n)
}

/// Entries sort by time, then by their same-second number: `…-2.toml` after the plain name, `-10` after `-9`.
fn sort_key(name: &str) -> (String, u32) {
    let stem = name.strip_suffix(".toml").unwrap_or(name);
    let n = stem
        .get(TIME_CHARS + 1 + NAME_DIGITS..)
        .and_then(number)
        .unwrap_or(1);
    (stem.get(..TIME_CHARS).unwrap_or(stem).to_string(), n)
}

#[cfg(test)]
mod tests;
