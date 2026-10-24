//! Cache garbage collection (#29, ADR-004): what a recorded version refers to stays; the rest goes oldest-first once
//! the cache is over its size cap, along with what isn't a usable entry at all.
//!
//! - **Roots** are the cache entries something in the project's files refers to ([`Roots`]): every recorded run's
//!   `[[uses]]`, in every `tracks/*/history/` folder ([`RecordedVersions`]). Approvals and `audio.lock` (E10) can
//!   add theirs the same way. A root is never removed, even when the roots alone are over the cap; the report says by how
//!   much.
//! - **The cap** counts the bytes of whole entries' artifacts (what their manifests list), not manifests, leftovers or
//!   the file system's overhead. Whole entries no root names are removed oldest-first by `created` while the total is
//!   over the cap. With a cap of 0 every unrooted entry goes, empty ones included.
//! - **Broken entries:** a directory under a stage that isn't a whole entry of this layout (damaged, half-written, an
//!   older layout) can never be a hit, so it is removed once it is older than [`Gc::tmp_age`], whatever the cap. It
//!   is checked again just before: if a writer has put a whole entry there since it was listed, it stays. One from a
//!   newer Score64 is left alone.
//! - **Leftovers:** `.tmp-*` directories older than [`Gc::tmp_age`] are a crashed or racing writer's (or a crashed
//!   collector's), and are removed whatever the cap. Their age is the time in their name ([`crate::cache`] writes
//!   one), else, for a name without one or with a time still to come (a clock ahead of this one), their modification
//!   time. Younger ones may be a write, or another collection, in progress.
//! - **Links** are never followed: a stage or entry that is a symbolic link is left alone and not counted, and the
//!   size of a directory doesn't count what links in it point to.
//!
//! **Removal.** Everything removed, entries, broken directories and leftovers alike, is first renamed aside to a
//! fresh, uniquely named `.tmp-gc-t<seconds>-...` directory, then deleted there. Nothing is deleted in place: a writer
//! renaming its temporary copy onto its key and the collector renaming that copy aside can't both succeed, so the
//! collector never deletes files out of an entry that has just been put, and an entry is either whole under its key or
//! gone, never half there. What can't be moved (a file open in it, on Windows) is left as it is and reported in
//! [`Report::in_use`]. What vanished between listing and removal (a writer's rename, another collector) is not
//! reported. A copy moved aside but not deleted is reported as removed, and the error stops the collection.
//!
//! **Cost.** Listing reads and hashes every artifact once, as [`Cache::get`] does. The checks right before a removal
//! compare manifests and file sizes only; a broken entry whose sizes all match is hashed again, to tell it from one a
//! writer has just put. The engine's index rebuild afterwards hashes the kept entries once more.
//!
//! **Limits.**
//! - *Concurrency.* A collection may run while other processes `put` entries: an entry put while it runs is not
//!   removed. It must not run during a pipeline run, though. There is no project lock yet, so roots are read once at
//!   the start, and an entry a run is making (or has just looked up) but hasn't recorded yet is unrooted and may go.
//!   The run then makes it again.
//! - *Scope.* Only the histories in the current checkout are roots. Runs recorded only on other git branches, or in
//!   another clone, don't count, so their entries can go and are made again when needed. A track folder whose
//!   `track.toml` was removed still counts while its `history/` folder remains.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use score64_core::ContentHash;

use crate::cache::{
    Cache, Manifest, check_name, made_at, newer_layout, sized, temp_dir_named, temp_name, whole,
};
use crate::history::{CacheRef, History};
use crate::project::Project;

/// Why a source of roots could not be read: its own error.
pub type SourceError = Box<dyn std::error::Error + Send + Sync>;

/// Where roots come from: something in the project's files that refers to cache entries.
pub trait Roots {
    /// What these roots are, for the report and errors (`"recorded versions"`).
    fn name(&self) -> &str;
    /// The entries they refer to.
    fn refs(&self, project: &Project) -> Result<Vec<CacheRef>, SourceError>;
}

/// Every cache entry a recorded run uses, in every `tracks/*/history/` folder of the checkout: all of each history,
/// not only its latest version, and also a folder whose `track.toml` is gone. Histories on other git branches are not
/// seen (see the module's limits).
pub struct RecordedVersions;

impl Roots for RecordedVersions {
    fn name(&self) -> &str {
        "recorded versions"
    }

    fn refs(&self, project: &Project) -> Result<Vec<CacheRef>, SourceError> {
        let tracks = project.root().join("tracks");
        let listing = match std::fs::read_dir(&tracks) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("{}: {e}", tracks.display()).into()),
        };
        let mut out = Vec::new();
        for item in listing {
            let item = item.map_err(|e| format!("{}: {e}", tracks.display()))?;
            let dir = item.path().join("history");
            if !dir.is_dir() {
                continue;
            }
            for (_, entry) in History::read(&dir)?.entries() {
                out.extend(entry.uses.iter().cloned());
            }
        }
        Ok(out)
    }
}

/// A collection's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gc {
    /// The most bytes the whole entries' artifacts may hold once unrooted entries are removed. 0 removes every
    /// unrooted entry.
    pub cap_bytes: u64,
    /// How old a `.tmp-*` or broken directory must be before it is removed.
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

/// One removed (or, in a dry run, removable) cache directory, or one that couldn't be moved.
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

/// How many distinct entries one source of roots names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RootCount {
    /// The source ([`Roots::name`]).
    pub source: String,
    /// The distinct entries it names, whether the cache holds them or not.
    pub refs: u64,
}

/// What a collection did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Each source of roots, in the order given.
    pub roots: Vec<RootCount>,
    /// Whole entries kept because a root names them, and their bytes.
    pub rooted: (u64, u64),
    /// Whole entries kept without a root, under the cap, and their bytes.
    pub unrooted_kept: (u64, u64),
    /// What was removed (or would be, in a dry run).
    pub removed: Vec<Removed>,
    /// Entries that should have gone but couldn't be moved aside (a file open in them, or no permission): left whole
    /// under their keys.
    pub in_use: Vec<Removed>,
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
    /// A directory could not be read, made, renamed or removed.
    Io {
        /// Where.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// A source of roots could not be read: the collection stops before removing anything it might name.
    Roots {
        /// The source ([`Roots::name`]).
        source: String,
        /// Its error.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Roots { source, message } => {
                write!(
                    f,
                    "reading the roots from {source}: {message}; nothing removed"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { error, .. } => Some(error),
            Error::Roots { .. } => None,
        }
    }
}

/// A collection that stopped on an error, with what it had done by then.
#[derive(Debug)]
pub struct Stopped {
    /// What was done before the error. Empty when it stopped before removing anything.
    pub report: Report,
    /// The error.
    pub error: Error,
}

impl Stopped {
    /// Whether anything was removed before it stopped: an index of the cache is then out of date.
    pub fn changed(&self) -> bool {
        !self.report.dry_run && !self.report.removed.is_empty()
    }
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for Stopped {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// A whole entry, as listed.
struct Listed {
    r: CacheRef,
    manifest: Manifest,
    bytes: u64,
    dir: PathBuf,
}

/// Any other directory under a stage, as listed.
struct Other {
    stage: String,
    stage_dir: PathBuf,
    name: String,
    dir: PathBuf,
}

/// What became of a directory GC tried to remove.
enum Gone {
    /// Removed (or, in a dry run, would be).
    Removed,
    /// Already gone: another collector, or a writer replacing it.
    Vanished,
    /// It couldn't be moved aside, so it stays where it is.
    InUse,
    /// Moved aside, it turned out whole: a writer had just put it. It went back.
    Whole,
    /// Moved off its key, but the copy aside couldn't be deleted: a leftover for a later collection.
    Aside(Error),
    /// Nothing could be done: it is as it was.
    Failed(Error),
}

impl Gc {
    /// Collect the project's cache, keeping everything `roots` name. Roots are all read first: if any source fails,
    /// nothing is removed. An error after something was removed comes with the report so far ([`Stopped`]).
    pub fn collect(&self, project: &Project, roots: &[&dyn Roots]) -> Result<Report, Box<Stopped>> {
        self.collect_at(project, roots, SystemTime::now())
    }

    /// [`Gc::collect`] with `now` as the time the ages of leftovers and broken entries are measured from.
    fn collect_at(
        &self,
        project: &Project,
        roots: &[&dyn Roots],
        now: SystemTime,
    ) -> Result<Report, Box<Stopped>> {
        let mut report = Report {
            dry_run: self.dry_run,
            ..Report::default()
        };
        let mut rooted = BTreeSet::new();
        for source in roots {
            let refs: BTreeSet<CacheRef> = match source.refs(project) {
                Ok(refs) => refs.into_iter().collect(),
                Err(e) => {
                    let error = Error::Roots {
                        source: source.name().to_owned(),
                        message: e.to_string(),
                    };
                    return Err(Box::new(Stopped { report, error }));
                }
            };
            report.roots.push(RootCount {
                source: source.name().to_owned(),
                refs: refs.len() as u64,
            });
            rooted.extend(refs);
        }
        let cache_dir = project.cache_dir();
        let cache = Cache::new(&cache_dir);
        let listed = list(&cache, &cache_dir);
        let result = listed.and_then(|(whole, other)| {
            self.sweep_unrooted(&rooted, whole, &mut report)?;
            self.sweep_others(&cache, other, now, &mut report)
        });
        match result {
            Ok(()) => Ok(report),
            Err(error) => Err(Box::new(Stopped { report, error })),
        }
    }

    /// Keep the roots, and remove unrooted entries oldest-first while the cache is over the cap.
    fn sweep_unrooted(
        &self,
        rooted: &BTreeSet<CacheRef>,
        whole: Vec<Listed>,
        report: &mut Report,
    ) -> Result<(), Error> {
        let mut present = BTreeSet::new();
        let mut unrooted = Vec::new();
        let mut kept_bytes = 0u64;
        let mut rooted_bytes = 0u64;
        for entry in whole {
            present.insert(entry.r.clone());
            kept_bytes = kept_bytes.saturating_add(entry.bytes);
            if rooted.contains(&entry.r) {
                report.rooted.0 += 1;
                report.rooted.1 = report.rooted.1.saturating_add(entry.bytes);
                rooted_bytes = rooted_bytes.saturating_add(entry.bytes);
            } else {
                unrooted.push(entry);
            }
        }
        report.missing_roots = rooted.difference(&present).count() as u64;
        report.roots_over_cap = rooted_bytes.saturating_sub(self.cap_bytes);
        // Oldest first, then by stage and key, so the same cache always loses the same entries.
        unrooted.sort_by(|a, b| (a.manifest.created, &a.r).cmp(&(b.manifest.created, &b.r)));
        for entry in unrooted {
            if kept_bytes <= self.cap_bytes && self.cap_bytes != 0 {
                report.unrooted_kept.0 += 1;
                report.unrooted_kept.1 = report.unrooted_kept.1.saturating_add(entry.bytes);
                continue;
            }
            // Check again, by manifest and sizes (listing just hashed it): damaged, replaced or gone since it was
            // listed, it is no longer the whole entry to count or remove.
            if sized(&entry.dir, &entry.r.stage, entry.r.key).as_ref() != Some(&entry.manifest) {
                kept_bytes = kept_bytes.saturating_sub(entry.bytes);
                continue;
            }
            let item = Removed {
                stage: entry.r.stage.clone(),
                name: entry.r.key.to_string(),
                bytes: entry.bytes,
                why: "unrooted",
            };
            let stage_dir = entry.dir.parent().expect("under a stage");
            match self.remove(stage_dir, &entry.dir, None) {
                Gone::Removed => {
                    kept_bytes = kept_bytes.saturating_sub(entry.bytes);
                    report.removed.push(item);
                }
                Gone::Vanished | Gone::Whole => kept_bytes = kept_bytes.saturating_sub(entry.bytes),
                Gone::InUse => report.in_use.push(item),
                Gone::Aside(error) => {
                    report.removed.push(item);
                    return Err(error);
                }
                Gone::Failed(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Remove old leftovers and old broken directories; count newer entries.
    fn sweep_others(
        &self,
        cache: &Cache,
        other: Vec<Other>,
        now: SystemTime,
        report: &mut Report,
    ) -> Result<(), Error> {
        for o in other {
            let leftover = o.name.starts_with(".tmp-");
            if leftover {
                // The time in the name, which a rename aside doesn't change; else, with none or one still to come (a
                // clock ahead of this one), when it was last changed.
                let made = made_at(&o.name)
                    .filter(|t| *t <= now)
                    .or_else(|| modified(&o.dir));
                if !older_than(made, now, self.tmp_age) {
                    continue;
                }
            } else if newer_layout(&o.dir) {
                report.newer += 1;
                continue;
            } else if !older_than(modified(&o.dir), now, self.tmp_age) {
                // Maybe a write in progress, or a copy being made by hand.
                continue;
            }
            let item = Removed {
                stage: o.stage.clone(),
                name: o.name.clone(),
                bytes: size(&o.dir),
                why: if leftover { "leftover" } else { "broken" },
            };
            let key = (!leftover).then(|| key_named(&o.name)).flatten();
            if let Some(key) = key
                && sized(&o.dir, &o.stage, key).is_some()
                && cache.get(&o.stage, key).is_some()
            {
                // A writer has put a whole entry here since it was listed: whole means keep. Only one that looks
                // whole by its sizes is hashed again.
                continue;
            }
            match self.remove(&o.stage_dir, &o.dir, key.map(|k| (o.stage.as_str(), k))) {
                Gone::Removed => report.removed.push(item),
                Gone::Vanished | Gone::Whole => {}
                Gone::InUse => report.in_use.push(item),
                Gone::Aside(error) => {
                    report.removed.push(item);
                    return Err(error);
                }
                Gone::Failed(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Remove `dir`, a directory under a stage: renamed aside to a fresh `.tmp-gc-*` directory first, then deleted
    /// there, so it is never half there under its own name and never deleted while a writer renames it into place. If
    /// it can't be moved it stays as it was. With `key` (its stage and key) a copy that turns out whole once moved was
    /// just put by a writer, and goes back. A dry run does nothing.
    fn remove(&self, stage_dir: &Path, dir: &Path, key: Option<(&str, ContentHash)>) -> Gone {
        self.remove_with(
            stage_dir,
            dir,
            key,
            |from, to| std::fs::rename(from, to),
            |d| std::fs::remove_dir_all(d),
        )
    }

    /// [`Gc::remove`], moving it aside with `rename` and deleting the copy with `delete`.
    fn remove_with(
        &self,
        stage_dir: &Path,
        dir: &Path,
        key: Option<(&str, ContentHash)>,
        rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
        delete: impl Fn(&Path) -> std::io::Result<()>,
    ) -> Gone {
        if self.dry_run {
            return Gone::Removed;
        }
        // A name no other writer or collector has: made, then freed for the rename (which can't replace a directory
        // on every system).
        let aside = match temp_dir_named(stage_dir, || temp_name(".tmp-gc-")) {
            Ok(aside) => aside,
            Err(crate::cache::Error::Io { path, error }) => {
                return Gone::Failed(Error::Io { path, error });
            }
            Err(e) => {
                return Gone::Failed(Error::Io {
                    path: stage_dir.to_path_buf(),
                    error: std::io::Error::other(e.to_string()),
                });
            }
        };
        let _ = std::fs::remove_dir(&aside);
        match rename(dir, &aside) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Gone::Vanished,
            Err(_) => return Gone::InUse,
        }
        if let Some((stage, key)) = key
            && whole(&aside, stage, key).is_some()
        {
            // If another entry has taken its place meanwhile, that one stays and this copy goes.
            if std::fs::rename(&aside, dir).is_err() {
                let _ = std::fs::remove_dir_all(&aside);
            }
            return Gone::Whole;
        }
        match delete(&aside) {
            Ok(()) => Gone::Removed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Gone::Removed,
            Err(error) => Gone::Aside(Error::Io { path: aside, error }),
        }
    }
}

/// Every directory under the cache's stages, sorted: the whole entries, and the rest. Stage folders with names a
/// stage can't have, files, and symbolic links (at the stage or the entry level) are skipped.
fn list(cache: &Cache, root: &Path) -> Result<(Vec<Listed>, Vec<Other>), Error> {
    let mut whole = Vec::new();
    let mut other = Vec::new();
    for stage in names(root)? {
        let stage_dir = root.join(&stage);
        if check_name(&stage).is_err() || !real_dir(&stage_dir) {
            continue;
        }
        for name in names(&stage_dir)? {
            let dir = stage_dir.join(&name);
            if !real_dir(&dir) {
                continue;
            }
            if let Some(key) = key_named(&name)
                && let Some(entry) = cache.get(&stage, key)
            {
                whole.push(Listed {
                    r: CacheRef {
                        stage: stage.clone(),
                        key,
                    },
                    bytes: entry
                        .manifest
                        .artifacts
                        .iter()
                        .fold(0u64, |sum, a| sum.saturating_add(a.bytes)),
                    manifest: entry.manifest,
                    dir,
                });
            } else {
                other.push(Other {
                    stage: stage.clone(),
                    stage_dir: stage_dir.clone(),
                    name,
                    dir,
                });
            }
        }
    }
    Ok((whole, other))
}

/// The key a directory named `name` would hold, if it is named like one.
fn key_named(name: &str) -> Option<ContentHash> {
    ContentHash::from_hex(name).filter(|k| k.to_string() == name)
}

/// Whether `path` is a directory itself, not a link to one.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// When `dir` was last changed, if that can be read.
fn modified(dir: &Path) -> Option<SystemTime> {
    std::fs::symlink_metadata(dir)
        .and_then(|m| m.modified())
        .ok()
}

/// Whether `made` is more than `age` before `now` (a time after `now`, or none, counts as young).
fn older_than(made: Option<SystemTime>, now: SystemTime, age: Duration) -> bool {
    made.and_then(|t| now.duration_since(t).ok())
        .is_some_and(|d| d > age)
}

/// The bytes in the files under `dir`, through its folders but never through a link.
fn size(dir: &Path) -> u64 {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return 0;
    };
    listing.flatten().fold(0u64, |sum, e| {
        let bytes = match e.file_type() {
            Ok(t) if t.is_dir() => size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        };
        sum.saturating_add(bytes)
    })
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
