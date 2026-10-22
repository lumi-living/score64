//! The content-addressed stage cache (#26, ADR-004): `.score64/cache/<stage>/<key>/`, holding a stage's artifacts and
//! a `manifest.json` that says how they were made.
//!
//! Every stage is a pure function of its inputs, config and version, so its key, a blake3 hash of all of them, names
//! its output. An entry is written whole into a temporary directory and renamed into place, so readers never see half
//! of one, and two writers racing on one key both succeed with the same entry.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use score64_core::ContentHash;

/// The manifest layout version this build reads and writes.
pub const MANIFEST_SCHEMA: u32 = 1;

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
    /// The stage's config, as given to [`Cache::key`].
    pub config: Value,
    /// The input files' hashes, as given to [`Cache::key`].
    pub inputs: Vec<ContentHash>,
    /// How long the run took, in milliseconds.
    pub millis: u64,
    /// What it cost, for stages that call a paid service, in the provider's currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
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
    /// The entry's key.
    pub key: ContentHash,
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
    /// A stage or artifact name that would not stay inside its directory, or a reserved one.
    BadName(String),
    /// A manifest that could not be written.
    Manifest(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::BadName(name) => write!(
                f,
                "{name:?} is not a valid stage or artifact name (letters, digits, '-', '_' and '.', not starting \
                 with '.')"
            ),
            Error::Manifest(m) => write!(f, "manifest: {m}"),
        }
    }
}

impl std::error::Error for Error {}

const MANIFEST: &str = "manifest.json";

/// Names that stay inside their directory: letters, digits, `-`, `_` and `.`, not starting with `.` (which also keeps
/// clear of the temporary directories), and not the manifest.
fn check_name(name: &str) -> Result<(), Error> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name != MANIFEST
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if ok {
        Ok(())
    } else {
        Err(Error::BadName(name.to_owned()))
    }
}

impl Cache {
    /// The cache at `root` (a project's `.score64/cache`); nothing is created until something is put.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Cache { root: root.into() }
    }

    /// The key of a stage run: blake3 of the stage, its version, its config and its inputs, as canonical JSON (keys
    /// sorted), so the same work always has the same key and any change gives another.
    pub fn key(
        stage: &str,
        stage_version: &str,
        config: &Value,
        inputs: &[ContentHash],
    ) -> ContentHash {
        let canonical = serde_json::json!({
            "stage": stage,
            "stage_version": stage_version,
            "config": config,
            "inputs": inputs,
        });
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
        let text = std::fs::read(dir.join(MANIFEST)).ok()?;
        let manifest: Manifest = serde_json::from_slice(&text).ok()?;
        if manifest.schema != MANIFEST_SCHEMA || manifest.stage != stage || manifest.key != key {
            return None;
        }
        for a in &manifest.artifacts {
            check_name(&a.name).ok()?;
            let bytes = std::fs::read(dir.join(&a.name)).ok()?;
            if bytes.len() as u64 != a.bytes
                || ContentHash(*blake3::hash(&bytes).as_bytes()) != a.hash
            {
                return None;
            }
        }
        Some(Entry { dir, manifest })
    }

    /// Put a stage run's `artifacts` (name and bytes) under `key`, with its manifest. Written whole into a temporary
    /// directory and renamed into place; if another writer got there first, its entry is kept and returned.
    pub fn put(
        &self,
        stage: &str,
        key: ContentHash,
        artifacts: &[(&str, &[u8])],
        run: Run,
    ) -> Result<Entry, Error> {
        check_name(stage)?;
        let mut names: Vec<&str> = Vec::with_capacity(artifacts.len());
        for (name, _) in artifacts {
            check_name(name)?;
            if names.contains(name) {
                return Err(Error::BadName((*name).to_owned()));
            }
            names.push(name);
        }
        if let Some(entry) = self.get(stage, key) {
            return Ok(entry);
        }
        let stage_dir = self.root.join(stage);
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |error| Error::Io { path, error }
        };
        std::fs::create_dir_all(&stage_dir).map_err(io(&stage_dir))?;
        let tmp = stage_dir.join(temp_name());
        std::fs::create_dir(&tmp).map_err(io(&tmp))?;
        let written = (|| {
            let mut listed = Vec::with_capacity(artifacts.len());
            for (name, bytes) in artifacts {
                let path = tmp.join(name);
                std::fs::write(&path, bytes).map_err(io(&path))?;
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
                run,
                artifacts: listed,
            };
            let text =
                serde_json::to_vec_pretty(&manifest).map_err(|e| Error::Manifest(e.to_string()))?;
            let path = tmp.join(MANIFEST);
            std::fs::write(&path, text).map_err(io(&path))?;
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
        match std::fs::rename(&tmp, &dir) {
            Ok(()) => Ok(Entry { dir, manifest }),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&tmp);
                // Another writer's entry won the race: keep it, since a pure stage gives the same output.
                self.get(stage, key).ok_or(Error::Io { path: dir, error })
            }
        }
    }
}

/// A temporary directory name unique to this process and call: `.tmp-<pid>-<n>`.
fn temp_name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests;
