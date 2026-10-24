//! Cache garbage collection (#29, ADR-004): what a recorded version refers to stays; the rest goes oldest-first once
//! the cache is over its size cap, along with what isn't a usable entry at all.
//!
//! - **Roots** are the cache entries something in the project's files refers to ([`Roots`]): every recorded run's
//!   `[[uses]]`, on every track ([`RecordedVersions`]). Approvals (E10) add theirs the same way. A root is never
//!   removed, even when the roots alone are over the cap; the report says by how much.
//! - **The cap.** Whole entries no root names are removed oldest-first by `created` until the cache's whole entries
//!   fit in the cap.
//! - **Broken entries:** a directory under a stage that isn't a whole entry of this layout (damaged, half-written, an
//!   older layout) can never be a hit, so it is removed. One from a newer Score64 is left alone.
//! - **Leftovers:** `.tmp-*` directories older than [`Gc::tmp_age`] are a crashed or racing writer's, and are
//!   removed. Younger ones may be a write in progress.
//!
//! A removed entry is first renamed aside, to a `.tmp-gc-*` name, so it is never half there under its key.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::cache::{Cache, check_name, newer_layout};
use crate::history::CacheRef;
use crate::project::Project;

/// Where roots come from: something in the project's files that refers to cache entries.
pub trait Roots {
    /// What these roots are, for the report (`"recorded versions"`).
    fn name(&self) -> &str;
    /// The entries they refer to.
    fn refs(&self, project: &Project) -> Result<Vec<CacheRef>, Error>;
}

/// Every cache entry a recorded run on any track uses: all of each track's history, not only its latest version.
pub struct RecordedVersions;

impl Roots for RecordedVersions {
    fn name(&self) -> &str {
        "recorded versions"
    }

    fn refs(&self, project: &Project) -> Result<Vec<CacheRef>, Error> {
        let mut out = Vec::new();
        for id in project
            .track_ids()
            .map_err(|e| Error::Roots(e.to_string()))?
        {
            let history = project
                .history(&id)
                .map_err(|e| Error::Roots(e.to_string()))?;
            for (_, entry) in history.entries() {
                out.extend(entry.uses.iter().cloned());
            }
        }
        Ok(out)
    }
}

/// A collection's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gc {
    /// The most bytes the cache's whole entries may hold once unrooted ones are removed.
    pub cap_bytes: u64,
    /// How old a `.tmp-*` directory must be to count as a leftover.
    pub tmp_age: Duration,
    /// Report what would go, and remove nothing.
    pub dry_run: bool,
}

impl Default for Gc {
    /// A 4 GiB cap, ten-minute-old leftovers, and removing for real.
    fn default() -> Self {
        Gc {
            cap_bytes: 4 << 30,
            tmp_age: Duration::from_secs(600),
            dry_run: false,
        }
    }
}

/// One removed (or, in a dry run, removable) cache directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Removed {
    /// Its stage.
    pub stage: String,
    /// Its directory's name: the key, or the leftover's name.
    pub name: String,
    /// Bytes in it.
    pub bytes: u64,
    /// Why: `unrooted` (over the cap), `broken` or `leftover`.
    pub why: &'static str,
}

/// What a collection did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Whole entries kept because a root names them, and their bytes.
    pub rooted: (u64, u64),
    /// Whole entries kept without a root, under the cap, and their bytes.
    pub unrooted_kept: (u64, u64),
    /// What was removed (or would be, in a dry run).
    pub removed: Vec<Removed>,
    /// Entries from a newer Score64, left alone.
    pub newer: u64,
    /// Bytes the roots alone hold beyond the cap; 0 when they fit.
    pub roots_over_cap: u64,
    /// Roots naming entries the cache doesn't have whole (they'll be made again when needed).
    pub missing_roots: u64,
    /// Whether nothing was removed because it was a dry run.
    pub dry_run: bool,
}

/// Why a collection failed.
#[derive(Debug)]
pub enum Error {
    /// A directory could not be read, renamed or removed.
    Io {
        /// Where.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// A source of roots could not be read: the collection stops rather than remove what it might name.
    Roots(String),
    /// The cache could not be listed.
    Cache(crate::cache::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Roots(why) => write!(f, "roots: {why}; nothing removed"),
            Error::Cache(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl Gc {
    /// Collect the project's cache, keeping everything `roots` name. Roots are all read first: if any source fails,
    /// nothing is removed.
    pub fn collect(&self, project: &Project, roots: &[&dyn Roots]) -> Result<Report, Error> {
        let mut rooted = BTreeSet::new();
        for source in roots {
            rooted.extend(source.refs(project)?);
        }
        let root_dir = project.root().join(".score64").join("cache");
        let cache = Cache::new(&root_dir);
        let mut report = Report {
            dry_run: self.dry_run,
            ..Report::default()
        };
        let whole = cache.entries().map_err(Error::Cache)?;
        let mut present = BTreeSet::new();
        let mut unrooted = Vec::new();
        let mut kept_bytes = 0u64;
        let mut rooted_bytes = 0u64;
        for entry in &whole {
            let m = &entry.manifest;
            let r = CacheRef {
                stage: m.stage.clone(),
                key: m.key,
            };
            let bytes: u64 = m.artifacts.iter().map(|a| a.bytes).sum();
            present.insert(r.clone());
            if rooted.contains(&r) {
                report.rooted.0 += 1;
                report.rooted.1 += bytes;
                rooted_bytes += bytes;
            } else {
                unrooted.push((m.created, r, bytes, entry.dir.clone()));
            }
            kept_bytes += bytes;
        }
        report.missing_roots = rooted.difference(&present).count() as u64;
        report.roots_over_cap = rooted_bytes.saturating_sub(self.cap_bytes);
        // Oldest first, then by stage and key, so the same cache always loses the same entries.
        unrooted.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        for (_, r, bytes, dir) in unrooted {
            if kept_bytes > self.cap_bytes {
                self.remove(&dir)?;
                kept_bytes -= bytes;
                report.removed.push(Removed {
                    stage: r.stage,
                    name: r.key.to_string(),
                    bytes,
                    why: "unrooted",
                });
            } else {
                report.unrooted_kept.0 += 1;
                report.unrooted_kept.1 += bytes;
            }
        }
        // Broken entries and leftovers, stage by stage.
        let whole_dirs: BTreeSet<PathBuf> = whole.iter().map(|e| e.dir.clone()).collect();
        for stage in names(&root_dir)? {
            let stage_dir = root_dir.join(&stage);
            if check_name(&stage).is_err() || !stage_dir.is_dir() {
                continue;
            }
            for name in names(&stage_dir)? {
                let dir = stage_dir.join(&name);
                if whole_dirs.contains(&dir) || !dir.is_dir() {
                    continue;
                }
                let why = if name.starts_with(".tmp-") {
                    if !older_than(&dir, self.tmp_age) {
                        continue;
                    }
                    "leftover"
                } else if newer_layout(&dir) {
                    report.newer += 1;
                    continue;
                } else {
                    "broken"
                };
                let bytes = size(&dir);
                self.remove(&dir)?;
                report.removed.push(Removed {
                    stage: stage.clone(),
                    name,
                    bytes,
                    why,
                });
            }
        }
        Ok(report)
    }

    /// Remove `dir`: renamed aside first, so it is never half there under its own name, then deleted. A dry run does
    /// nothing.
    fn remove(&self, dir: &Path) -> Result<(), Error> {
        if self.dry_run {
            return Ok(());
        }
        let parent = dir.parent().expect("under a stage");
        let aside = parent.join(format!(
            ".tmp-gc-{}-{}",
            std::process::id(),
            dir.file_name().and_then(|n| n.to_str()).unwrap_or("x")
        ));
        let target = match std::fs::rename(dir, &aside) {
            Ok(()) => aside,
            // Already moved (another collector), or a leftover we can't rename: delete it where it is.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => dir.to_path_buf(),
        };
        match std::fs::remove_dir_all(&target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Io {
                path: target,
                error,
            }),
        }
    }
}

/// Whether `dir` was last changed more than `age` ago (a clock in the past, or an unreadable time, counts as young).
fn older_than(dir: &Path, age: Duration) -> bool {
    std::fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|d| d > age)
}

/// The bytes in the files under `dir`.
fn size(dir: &Path) -> u64 {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return 0;
    };
    listing
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

/// The names in `dir`, sorted; none when it doesn't exist.
fn names(dir: &Path) -> Result<Vec<String>, Error> {
    let listing = match std::fs::read_dir(dir) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => {
            return Err(Error::Io {
                path: dir.to_path_buf(),
                error,
            });
        }
    };
    let mut out: Vec<String> = listing
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests;
