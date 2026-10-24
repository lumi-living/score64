//! The content-addressed stage cache (#26, ADR-004): `.score64/cache/<stage>/<key>/`, holding a stage's artifacts and
//! a `manifest.json` that says how they were made.
//!
//! Every stage is a pure function of its inputs, config and version, so its key, a blake3 hash of all of them, names
//! its output. An entry is written whole into a temporary directory and renamed into place, so readers never see half
//! of one, and two writers racing on one key both succeed with the same entry.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use score64_core::ContentHash;

/// The manifest layout version this build reads and writes: 2 added `created` (#28). An entry of another layout is a
/// miss, and is replaced when its key is put again.
pub const MANIFEST_SCHEMA: u32 = 2;

/// The cache under a project's `.score64/cache/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cache {
    root: PathBuf,
}

/// What a stage run recorded besides its artifacts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// The stage's own version, which changes when its output would.
    pub stage_version: String,
    /// The Score64 version that ran it.
    pub tool_version: String,
    /// The stage's config.
    pub config: Value,
    /// The input files' hashes.
    pub inputs: Vec<ContentHash>,
    /// How long the run took, in milliseconds.
    pub millis: u64,
    /// What it cost, for stages that call a paid service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
}

/// What a stage run cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    /// The amount.
    pub amount: f64,
    /// Its currency, as the provider bills it (ISO 4217, such as `"USD"`).
    pub currency: String,
}

/// One artifact in an entry: its file name, hash and size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// File name inside the entry's directory.
    pub name: String,
    /// blake3 of its bytes.
    pub hash: ContentHash,
    /// Its size, in bytes.
    pub bytes: u64,
}

/// An entry's `manifest.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Layout version.
    pub schema: u32,
    /// The stage.
    pub stage: String,
    /// The entry's key: [`Cache::key`] of the stage and its run.
    pub key: ContentHash,
    /// When it was written, in UTC. Not part of the key: the same work written later is the same entry.
    #[serde(with = "crate::history::as_string")]
    pub created: crate::UtcTime,
    /// How it was made.
    pub run: Run,
    /// What it holds, by name.
    pub artifacts: Vec<Artifact>,
}

/// A cache entry: where it is and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Its directory.
    pub dir: PathBuf,
    /// Its manifest.
    pub manifest: Manifest,
}

impl Entry {
    /// The path of the artifact called `name`, if the entry has one.
    pub fn artifact(&self, name: &str) -> Option<PathBuf> {
        self.manifest
            .artifacts
            .iter()
            .any(|a| a.name == name)
            .then(|| self.dir.join(name))
    }
}

/// Why the cache could not do something.
#[derive(Debug)]
pub enum Error {
    /// A file or directory could not be read or written.
    Io {
        /// Where.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// A stage or artifact name that would not stay inside its directory on every system, or a reserved one.
    BadName(String),
    /// A manifest that could not be written.
    Manifest(String),
    /// An entry written by a newer Score64 is in the way: left alone rather than replaced.
    Newer(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::BadName(name) => write!(
                f,
                "{name:?} is not a valid stage or artifact name (letters, digits, '-', '_' and '.', not starting or \
                 ending with '.', not a device name, not twice)"
            ),
            Error::Manifest(m) => write!(f, "manifest: {m}"),
            Error::Newer(dir) => write!(
                f,
                "{}: written by a newer Score64 (manifest schema above {MANIFEST_SCHEMA}); left as it is",
                dir.display()
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

const MANIFEST: &str = "manifest.json";

/// Names that stay inside their directory on every system: letters, digits, `-`, `_` and `.`; not starting with `.`
/// (which also keeps clear of the temporary directories) or ending with one; not a Windows device name (`con`,
/// `nul.json`, `com1`, ...); not the manifest.
pub(crate) fn check_name(name: &str) -> Result<(), Error> {
    let device = crate::textfile::is_device_name(name);
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !device
        && !name.eq_ignore_ascii_case(MANIFEST)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if ok {
        Ok(())
    } else {
        Err(Error::BadName(name.to_owned()))
    }
}

/// `v` with every object's keys sorted, whatever order `serde_json` keeps them in.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> =
                map.iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Flush a file or directory to disk. Directories can't be opened for syncing on every system, so failures are left
/// alone: the hash checks still keep a half-flushed entry from ever being a hit.
fn sync(path: &Path) {
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.sync_all();
    }
}

impl Cache {
    /// The cache at `root` (a project's `.score64/cache`); nothing is created until something is put.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Cache { root: root.into() }
    }

    /// The key of a stage run: blake3 of the stage, its version, its config and its inputs, as canonical JSON (every
    /// object's keys sorted), so the same work always has the same key and any change gives another. Numbers are
    /// taken as written: `1` and `1.0`, or `0.0` and `-0.0`, give different keys (a needless miss, never a wrong
    /// hit).
    pub fn key(
        stage: &str,
        stage_version: &str,
        config: &Value,
        inputs: &[ContentHash],
    ) -> ContentHash {
        let canonical = canonical(&serde_json::json!({
            "stage": stage,
            "stage_version": stage_version,
            "config": config,
            "inputs": inputs,
        }));
        let bytes = serde_json::to_vec(&canonical).expect("JSON values serialise");
        ContentHash(*blake3::hash(&bytes).as_bytes())
    }

    fn entry_dir(&self, stage: &str, key: ContentHash) -> PathBuf {
        self.root.join(stage).join(key.to_string())
    }

    /// The entry for `key`, if the cache has it whole: its manifest reads, names this stage and key, and every
    /// artifact's bytes still hash to what it says. Anything less is a miss.
    pub fn get(&self, stage: &str, key: ContentHash) -> Option<Entry> {
        check_name(stage).ok()?;
        let dir = self.entry_dir(stage, key);
        let manifest = whole(&dir, stage, key)?;
        Some(Entry { dir, manifest })
    }

    /// Put a stage run's `artifacts` (name and bytes) with its manifest, under the key its run gives
    /// ([`Cache::key`]), so an entry can't claim inputs or config it wasn't made from. Written whole and synced into a
    /// temporary directory, then renamed into place. If another writer got there first, its entry is kept and
    /// returned; an entry already there but not whole (damaged, or from an older layout) is replaced, and one from a
    /// newer Score64 is left alone ([`Error::Newer`]).
    pub fn put(&self, stage: &str, artifacts: &[(&str, &[u8])], run: Run) -> Result<Entry, Error> {
        check_name(stage)?;
        let mut names: Vec<String> = Vec::with_capacity(artifacts.len());
        for (name, _) in artifacts {
            check_name(name)?;
            // Names that differ only by case are one file on macOS and Windows.
            let folded = name.to_ascii_lowercase();
            if names.contains(&folded) {
                return Err(Error::BadName((*name).to_owned()));
            }
            names.push(folded);
        }
        let key = Self::key(stage, &run.stage_version, &run.config, &run.inputs);
        if let Some(entry) = self.get(stage, key) {
            return Ok(entry);
        }
        // One from a newer Score64 stays, so don't write a copy only to throw it away (#29).
        let dir = self.entry_dir(stage, key);
        if newer_layout(&dir) {
            return Err(Error::Newer(dir));
        }
        let stage_dir = self.root.join(stage);
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |error| Error::Io { path, error }
        };
        std::fs::create_dir_all(&stage_dir).map_err(io(&stage_dir))?;
        let tmp = temp_dir(&stage_dir)?;
        let written = (|| {
            let mut listed = Vec::with_capacity(artifacts.len());
            for (name, bytes) in artifacts {
                let path = tmp.join(name);
                std::fs::write(&path, bytes).map_err(io(&path))?;
                sync(&path);
                listed.push(Artifact {
                    name: (*name).to_owned(),
                    hash: ContentHash(*blake3::hash(bytes).as_bytes()),
                    bytes: bytes.len() as u64,
                });
            }
            listed.sort_by(|a, b| a.name.cmp(&b.name));
            let manifest = Manifest {
                schema: MANIFEST_SCHEMA,
                stage: stage.to_owned(),
                key,
                created: crate::UtcTime::now(),
                run,
                artifacts: listed,
            };
            let text =
                serde_json::to_vec_pretty(&manifest).map_err(|e| Error::Manifest(e.to_string()))?;
            let path = tmp.join(MANIFEST);
            std::fs::write(&path, text).map_err(io(&path))?;
            sync(&path);
            sync(&tmp);
            Ok::<Manifest, Error>(manifest)
        })();
        let manifest = match written {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e);
            }
        };
        let dir = self.entry_dir(stage, key);
        let mut result = std::fs::rename(&tmp, &dir);
        if result.is_err()
            && self.get(stage, key).is_none()
            && std::fs::symlink_metadata(&dir).is_ok()
        {
            // Something that isn't a whole entry is in the way. One from a newer Score64 is left alone.
            // It may have appeared since the check above.
            if newer_layout(&dir) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(Error::Newer(dir));
            }
            // Move it aside. If what was moved turns out whole, another writer replaced it first: put theirs back and
            // keep it. Otherwise clear it and try once more.
            if let Ok(aside) = temp_dir(&stage_dir) {
                let _ = std::fs::remove_dir(&aside);
                if std::fs::rename(&dir, &aside).is_ok() {
                    if whole(&aside, stage, key).is_some() {
                        let _ = std::fs::rename(&aside, &dir);
                        let _ = std::fs::remove_dir_all(&tmp);
                        return self.get(stage, key).ok_or(Error::Io {
                            path: dir,
                            error: std::io::Error::other(
                                "another writer's entry moved while replacing",
                            ),
                        });
                    }
                    if std::fs::remove_dir_all(&aside).is_err() {
                        let _ = std::fs::remove_file(&aside);
                    }
                }
            }
            result = std::fs::rename(&tmp, &dir);
        }
        match result {
            Ok(()) => {
                sync(&stage_dir);
                Ok(Entry { dir, manifest })
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&tmp);
                // Another writer's entry won the race: keep it, since a pure stage gives the same output.
                self.get(stage, key).ok_or(Error::Io { path: dir, error })
            }
        }
    }
}

impl Cache {
    /// Every whole entry, by stage and then key: what the index lists (#28). Temporary directories (`.tmp-*`) and
    /// anything that isn't a whole entry of its stage and key (damaged, half-written, another layout) are left out, as
    /// [`Cache::get`] would miss them. No cache yet is no entries.
    pub fn entries(&self) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        for stage in sorted_names(&self.root)? {
            if check_name(&stage).is_err() {
                continue;
            }
            let stage_dir = self.root.join(&stage);
            if !stage_dir.is_dir() {
                continue;
            }
            for name in sorted_names(&stage_dir)? {
                let Some(key) = ContentHash::from_hex(&name).filter(|k| k.to_string() == name)
                else {
                    continue;
                };
                if let Some(entry) = self.get(&stage, key) {
                    out.push(entry);
                }
            }
        }
        Ok(out)
    }
}

/// The names in `dir`, sorted; none when it doesn't exist.
fn sorted_names(dir: &Path) -> Result<Vec<String>, Error> {
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
    let mut names = Vec::new();
    for item in listing {
        let item = item.map_err(|error| Error::Io {
            path: dir.to_path_buf(),
            error,
        })?;
        if let Some(name) = item.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Whether the entry in `dir` says it is of a newer layout than this build's: it is left alone, never replaced or
/// collected.
pub(crate) fn newer_layout(dir: &Path) -> bool {
    std::fs::read(dir.join(MANIFEST))
        .ok()
        .and_then(|t| serde_json::from_slice::<Value>(&t).ok())
        .and_then(|v| v.get("schema").and_then(Value::as_u64))
        .is_some_and(|v| v > u64::from(MANIFEST_SCHEMA))
}

/// The manifest of the entry in `dir`, if it is whole: it reads, is this layout, names `stage` and `key`, and every
/// artifact's bytes still hash to what it says.
fn whole(dir: &Path, stage: &str, key: ContentHash) -> Option<Manifest> {
    let text = std::fs::read(dir.join(MANIFEST)).ok()?;
    let manifest: Manifest = serde_json::from_slice(&text).ok()?;
    if manifest.schema != MANIFEST_SCHEMA || manifest.stage != stage || manifest.key != key {
        return None;
    }
    for a in &manifest.artifacts {
        check_name(&a.name).ok()?;
        let bytes = std::fs::read(dir.join(&a.name)).ok()?;
        if bytes.len() as u64 != a.bytes || ContentHash(*blake3::hash(&bytes).as_bytes()) != a.hash
        {
            return None;
        }
    }
    Some(manifest)
}

/// A new, empty temporary directory under `stage_dir`, named so no other writer (another process, another container
/// with the same pid, a crashed run's leftovers) is using it.
fn temp_dir(stage_dir: &Path) -> Result<PathBuf, Error> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    temp_dir_named(stage_dir, || {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        format!(
            ".tmp-{}-{}-{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    })
}

/// [`temp_dir`] with its names from `name`: a taken name is skipped for the next, up to 64 tries.
fn temp_dir_named(stage_dir: &Path, mut name: impl FnMut() -> String) -> Result<PathBuf, Error> {
    for _ in 0..64 {
        let path = stage_dir.join(name());
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::Io { path, error }),
        }
    }
    Err(Error::Io {
        path: stage_dir.to_path_buf(),
        error: std::io::Error::other("no free temporary directory name"),
    })
}

#[cfg(test)]
mod tests;
