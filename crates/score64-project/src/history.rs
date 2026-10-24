//! A track's run history (ADR-014 §7, #27): one file per recorded run in `tracks/<id>/history/`, committed with the
//! track. A version id is content-addressed, so restoring an old intent gives its old id back; the history says which
//! version came from which, who made the change, and when.
//!
//! ```toml
//! schema = 1
//! version = "…"                 # the full version id (ADR-014 §6)
//! parent = "…"                  # the version recorded before it; absent for the first
//! time = 2026-10-24T09:15:02Z   # UTC, whole seconds
//! restore = true                # only on a restore
//!
//! [provenance]
//! by = "manual"                 # "rules", "llm" (with `model`) or "manual" (with `name`)
//! name = "A. Person"
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
        (seconds >= 0 && t.civil().0 <= 9999).then_some(t)
    }

    /// The system clock, to the second.
    pub fn now() -> Self {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        UtcTime(i64::try_from(since).unwrap_or(0))
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
            .ok_or_else(|| format!("{dt} is not a real time from 1970 on; expected {wanted}")),
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

/// One recorded run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Layout version; absent means 1.
    #[serde(default)]
    schema: Schema,
    /// The run's version id.
    pub version: VersionId,
    /// The version recorded before it on this track; `None` for the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<VersionId>,
    /// When it ran.
    pub time: UtcTime,
    /// Whether it restored an earlier version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore: bool,
    /// Who made the change.
    pub provenance: RunProvenance,
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
    /// A file is not a valid history entry, or the entries don't form one history.
    Invalid {
        /// The file.
        path: PathBuf,
        /// The 1-based line and column, when the parser knows them.
        at: Option<(usize, usize)>,
        /// The reason.
        message: String,
    },
    /// A run to record can't be: its time is before the latest run's, it restores a version that was never recorded,
    /// or its provenance names no one.
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Invalid { path, at, message } => match at {
                Some((line, col)) => write!(f, "{}:{line}:{col}: {message}", path.display()),
                None => write!(f, "{}: {message}", path.display()),
            },
            Error::Refused(why) => f.write_str(why),
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
    /// The history in `dir` (a track's `history/` folder): empty when the folder doesn't exist. Every `.toml` file in
    /// it must be an entry named for its time and version, and the entries in name order must chain: each one's
    /// parent is the version before it, and a restore's version was recorded before. Writers' temporary `.part` files
    /// are skipped; anything else is an error.
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
            let name = item.file_name().to_string_lossy().into_owned();
            if name.ends_with(".part") {
                continue;
            }
            names.push(name);
        }
        names.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)).then(a.cmp(b)));
        let mut entries: Vec<(String, Entry)> = Vec::with_capacity(names.len());
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
                    "not a history entry: entries are named like 2026-10-24T091502Z-1a2b3c4d5e6f.toml".into(),
                ));
            };
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
            entry.provenance.check().map_err(|why| invalid(None, why))?;
            if entry.time.name_part() != time || !entry.version.to_string().starts_with(&prefix) {
                return Err(invalid(
                    None,
                    format!(
                        "the name doesn't match the entry: it holds {} and version {}",
                        entry.time, entry.version
                    ),
                ));
            }
            if entries
                .last()
                .is_some_and(|(n, _)| sort_key(n) == sort_key(&name))
            {
                return Err(invalid(
                    None,
                    "another entry has the same time and number; a second run in one second is numbered -2, -3, …".into(),
                ));
            }
            let before = entries.last().map(|(_, e)| e);
            if entry.parent.as_ref() != before.map(|e| &e.version) {
                return Err(invalid(
                    None,
                    match before {
                        Some(b) => {
                            format!("its parent should be {}, the version before it", b.version)
                        }
                        None => "the first entry has no parent".into(),
                    },
                ));
            }
            if before.is_some_and(|b| entry.time < b.time) {
                return Err(invalid(None, "it is older than the entry before it".into()));
            }
            if entry.restore && !entries.iter().any(|(_, e)| e.version == entry.version) {
                return Err(invalid(
                    None,
                    "it restores a version that was never recorded before it".into(),
                ));
            }
            if !entry.restore && before.is_some_and(|b| b.version == entry.version) {
                return Err(invalid(
                    None,
                    "it records the same version as the entry before it, and isn't a restore"
                        .into(),
                ));
            }
            entries.push((name, entry));
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

    /// Record `run`, unless it changes nothing: a run whose version is the latest recorded one writes nothing (a
    /// restore of it included) and returns `None`. Otherwise its entry, with the latest version as parent, is created
    /// in the folder (made if needed), never replacing a file, and its path returned. Refused: a time before the
    /// latest run's (the clock went back; the history would no longer read in order), a restore of a version that
    /// was never recorded, and a provenance with an empty model or name.
    pub fn record(&mut self, run: Run) -> Result<Option<PathBuf>, Error> {
        run.provenance.check().map_err(Error::Refused)?;
        let latest = self.latest();
        if latest.is_some_and(|l| l.version == run.version) {
            return Ok(None);
        }
        if let Some(l) = latest
            && run.time < l.time
        {
            return Err(Error::Refused(format!(
                "the run's time {} is before the latest recorded run's, {}",
                run.time, l.time
            )));
        }
        if run.restore && !self.entries.iter().any(|(_, e)| e.version == run.version) {
            return Err(Error::Refused(format!(
                "version {} was never recorded, so it can't be restored",
                run.version
            )));
        }
        let entry = Entry {
            schema: Schema,
            version: run.version,
            parent: latest.map(|l| l.version),
            time: run.time,
            restore: run.restore,
            provenance: run.provenance,
        };
        let text = toml::to_string(&entry).map_err(|e| Error::Refused(e.to_string()))?;
        std::fs::create_dir_all(&self.dir).map_err(|error| Error::Io {
            path: self.dir.clone(),
            error,
        })?;
        let time = entry.time.name_part();
        let stem = format!("{time}-{}", &entry.version.to_string()[..NAME_DIGITS]);
        // Runs in one second are numbered across versions, so they keep their order.
        let first = self
            .entries
            .iter()
            .map(|(n, _)| sort_key(n))
            .filter(|(t, _)| *t == time)
            .map(|(_, n)| n + 1)
            .max()
            .unwrap_or(1);
        for n in first.. {
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
                Err((_, e)) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err((path, error)) => return Err(Error::Io { path, error }),
            }
        }
        unreachable!("some suffix is free")
    }
}

impl RunProvenance {
    fn check(&self) -> Result<(), String> {
        match self {
            RunProvenance::Llm { model } if model.trim().is_empty() => {
                Err("an llm run names no model".into())
            }
            RunProvenance::Manual { name } if name.trim().is_empty() => {
                Err("a manual run names no one".into())
            }
            _ => Ok(()),
        }
    }
}

/// The time part and version prefix of an entry's file name, if it is one.
fn parse_name(name: &str) -> Option<(String, String)> {
    let stem = name.strip_suffix(".toml")?;
    let (time, rest) = stem.split_at_checked(18)?;
    let rest = rest.strip_prefix('-')?;
    let (prefix, suffix) = rest.split_at_checked(NAME_DIGITS)?;
    let time_ok = time.len() == 18
        && time.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            10 => c == 'T',
            17 => c == 'Z',
            _ => c.is_ascii_digit(),
        });
    let prefix_ok = prefix
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    let suffix_ok = suffix.is_empty()
        || suffix
            .strip_prefix('-')
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n >= 2 && !suffix.starts_with("-0"));
    (time_ok && prefix_ok && suffix_ok).then(|| (time.to_string(), prefix.to_string()))
}

/// Entries sort by time, then by their same-second number: `…-2.toml` after the plain name, `-10` after `-9`.
fn sort_key(name: &str) -> (String, u32) {
    let stem = name.strip_suffix(".toml").unwrap_or(name);
    let base_len = 18 + 1 + NAME_DIGITS;
    let n = stem
        .get(base_len..)
        .and_then(|s| s.strip_prefix('-'))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    (stem.get(..18).unwrap_or(stem).to_string(), n)
}

#[cfg(test)]
mod tests;
