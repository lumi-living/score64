//! The index (#28, ADR-004, `docs/TAD.md` §4): `.score64/index.db`, an SQLite database the app and commands query
//! for tracks, their recorded versions and the cache entries those versions use. It is never the truth: everything in
//! it is read from files (`track.toml`, `tracks/<id>/history/`, the cache's manifests), and [`Index::rebuild`] makes
//! it again from them at any time. Nothing writes to it except a rebuild.
//!
//! Tables (`PRAGMA user_version` is [`INDEX_SCHEMA`]; another value means rebuild):
//! - `tracks(id, title)`;
//! - `versions(track, seq, file, version, parent, time, unix, restore, by, who)`: each track's history in order,
//!   `who` being the model for `llm` and the name for `manual`;
//! - `uses(track, seq, stage, key)`: the cache entries each recorded run's version uses;
//! - `entries(stage, key, created, unix, stage_version, tool_version, millis, cost, currency, bytes)`: every whole
//!   cache entry, `bytes` being its artifacts' total;
//! - `artifacts(stage, key, name, hash, bytes)`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;

use crate::cache::Cache;
use crate::history::{CacheRef, RunProvenance};
use crate::project::{Id, Project};
use crate::version::VersionId;

/// The index layout this build writes; an index of another layout is rebuilt.
pub const INDEX_SCHEMA: i64 = 1;

const TABLES: &str = "
    CREATE TABLE tracks (id TEXT PRIMARY KEY, title TEXT);
    CREATE TABLE versions (
        track TEXT NOT NULL, seq INTEGER NOT NULL, file TEXT NOT NULL, version TEXT NOT NULL, parent TEXT,
        time TEXT NOT NULL, unix INTEGER NOT NULL, restore INTEGER NOT NULL, by TEXT NOT NULL, who TEXT,
        PRIMARY KEY (track, seq)
    );
    CREATE INDEX versions_by_version ON versions (version);
    CREATE TABLE uses (
        track TEXT NOT NULL, seq INTEGER NOT NULL, stage TEXT NOT NULL, key TEXT NOT NULL,
        PRIMARY KEY (track, seq, stage, key)
    );
    CREATE INDEX uses_by_entry ON uses (stage, key);
    CREATE TABLE entries (
        stage TEXT NOT NULL, key TEXT NOT NULL, created TEXT NOT NULL, unix INTEGER NOT NULL,
        stage_version TEXT NOT NULL, tool_version TEXT NOT NULL, millis INTEGER NOT NULL, cost REAL, currency TEXT,
        bytes INTEGER NOT NULL,
        PRIMARY KEY (stage, key)
    );
    CREATE TABLE artifacts (
        stage TEXT NOT NULL, key TEXT NOT NULL, name TEXT NOT NULL, hash TEXT NOT NULL, bytes INTEGER NOT NULL,
        PRIMARY KEY (stage, key, name)
    );
";

/// An open index.
pub struct Index {
    conn: Connection,
    path: PathBuf,
}

/// How much an index holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Tracks.
    pub tracks: u64,
    /// Recorded runs, over every track.
    pub versions: u64,
    /// Whole cache entries.
    pub entries: u64,
    /// Their artifacts.
    pub artifacts: u64,
    /// Bytes in those artifacts.
    pub bytes: u64,
}

/// One recorded run, as the index lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// Its version id.
    pub version: VersionId,
    /// The version before it.
    pub parent: Option<VersionId>,
    /// When it ran.
    pub time: crate::UtcTime,
    /// Whether it restored an earlier version.
    pub restore: bool,
    /// Who made the change.
    pub provenance: RunProvenance,
    /// Its history file's name.
    pub file: String,
}

/// Why the index could not be built or read.
#[derive(Debug)]
pub enum Error {
    /// A file or folder could not be read or written.
    Io {
        /// Where.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// SQLite failed.
    Sql {
        /// The database.
        path: PathBuf,
        /// What went wrong.
        error: rusqlite::Error,
    },
    /// A project file couldn't be read.
    Project(crate::project::Error),
    /// A track's history couldn't be read.
    History(crate::history::Error),
    /// The cache couldn't be listed.
    Cache(crate::cache::Error),
    /// The index holds something no rebuild writes: rebuild it.
    Corrupt(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Sql { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Project(e) => e.fmt(f),
            Error::History(e) => e.fmt(f),
            Error::Cache(e) => e.fmt(f),
            Error::Corrupt(path) => write!(
                f,
                "{}: not an index this build wrote; rebuild it",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

impl Index {
    /// Where a project's index lives.
    pub fn path(project: &Project) -> PathBuf {
        project.root().join(".score64").join("index.db")
    }

    /// The project's index, rebuilt first when it is missing, unreadable or of another layout ([`INDEX_SCHEMA`]).
    pub fn open(project: &Project) -> Result<Self, Error> {
        let path = Self::path(project);
        if path.exists()
            && let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            && conn
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .is_ok_and(|v| v == INDEX_SCHEMA)
        {
            return Ok(Index { conn, path });
        }
        Self::rebuild(project)
    }

    /// Make the index again from the project's files, whatever is there now. It is written whole into a temporary
    /// database beside it, then renamed over the old one, so a reader sees the old index or the new one. Fails on a
    /// track file, history or cache that can't be read, naming it; the old index stays as it was.
    pub fn rebuild(project: &Project) -> Result<Self, Error> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = Self::path(project);
        let dir = path.parent().expect("under .score64").to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|error| Error::Io {
            path: dir.clone(),
            error,
        })?;
        let tmp = dir.join(format!(
            "index.db.{}-{}.part",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&tmp);
        let built = (|| {
            let mut conn = Connection::open(&tmp).map_err(sql(&tmp))?;
            fill(&mut conn, project).map_err(|e| match e {
                Fill::Sql(error) => Error::Sql {
                    path: tmp.clone(),
                    error,
                },
                Fill::Other(e) => e,
            })?;
            conn.close().map_err(|(_, error)| Error::Sql {
                path: tmp.clone(),
                error,
            })
        })();
        if let Err(e) = built {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(error) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::Io { path, error });
        }
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sql(&path))?;
        Ok(Index { conn, path })
    }

    /// How much it holds.
    pub fn summary(&self) -> Result<Summary, Error> {
        let count = |table: &str| -> Result<u64, Error> {
            self.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .map(|n| n as u64)
                .map_err(sql(&self.path))
        };
        let bytes: i64 = self
            .conn
            .query_row("SELECT COALESCE(SUM(bytes), 0) FROM artifacts", [], |r| {
                r.get(0)
            })
            .map_err(sql(&self.path))?;
        Ok(Summary {
            tracks: count("tracks")?,
            versions: count("versions")?,
            entries: count("entries")?,
            artifacts: count("artifacts")?,
            bytes: bytes as u64,
        })
    }

    /// Track `id`'s recorded runs, oldest first.
    pub fn versions(&self, id: &Id) -> Result<Vec<Version>, Error> {
        let mut q = self
            .conn
            .prepare(
                "SELECT version, parent, unix, restore, by, who, file FROM versions WHERE track = ?1 ORDER BY seq",
            )
            .map_err(sql(&self.path))?;
        let rows = q
            .query_map(params![id.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(sql(&self.path))?;
        let mut out = Vec::new();
        for row in rows {
            let (version, parent, unix, restore, by, who, file) = row.map_err(sql(&self.path))?;
            let corrupt = || Error::Corrupt(self.path.clone());
            let hash = |s: &str| VersionId::from_hex(s).ok_or_else(corrupt);
            let provenance = match (by.as_str(), who) {
                ("rules", None) => RunProvenance::Rules {},
                ("llm", Some(model)) => RunProvenance::Llm { model },
                ("manual", Some(name)) => RunProvenance::Manual { name },
                _ => return Err(corrupt()),
            };
            out.push(Version {
                version: hash(&version)?,
                parent: parent.as_deref().map(hash).transpose()?,
                time: crate::UtcTime::from_unix(unix).ok_or_else(corrupt)?,
                restore,
                provenance,
                file,
            });
        }
        Ok(out)
    }

    /// The cache entries a recorded version uses, on any track, sorted.
    pub fn uses(&self, version: &VersionId) -> Result<Vec<CacheRef>, Error> {
        let mut q = self
            .conn
            .prepare(
                "SELECT DISTINCT u.stage, u.key FROM uses u JOIN versions v ON u.track = v.track AND u.seq = v.seq \
                 WHERE v.version = ?1 ORDER BY u.stage, u.key",
            )
            .map_err(sql(&self.path))?;
        let rows = q
            .query_map(params![version.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(sql(&self.path))?;
        let mut out = Vec::new();
        for row in rows {
            let (stage, key) = row.map_err(sql(&self.path))?;
            let key = score64_core::ContentHash::from_hex(&key)
                .ok_or_else(|| Error::Corrupt(self.path.clone()))?;
            out.push(CacheRef { stage, key });
        }
        Ok(out)
    }

    /// The tracks and versions that use a cache entry, sorted and without repeats.
    pub fn used_by(&self, entry: &CacheRef) -> Result<Vec<(String, VersionId)>, Error> {
        let mut q = self
            .conn
            .prepare(
                "SELECT DISTINCT v.track, v.version FROM uses u JOIN versions v ON u.track = v.track AND u.seq = v.seq \
                 WHERE u.stage = ?1 AND u.key = ?2 ORDER BY v.track, v.version",
            )
            .map_err(sql(&self.path))?;
        let rows = q
            .query_map(params![entry.stage, entry.key.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(sql(&self.path))?;
        let mut out = Vec::new();
        for row in rows {
            let (track, version) = row.map_err(sql(&self.path))?;
            let v =
                VersionId::from_hex(&version).ok_or_else(|| Error::Corrupt(self.path.clone()))?;
            out.push((track, v));
        }
        Ok(out)
    }
}

enum Fill {
    Sql(rusqlite::Error),
    Other(Error),
}

impl From<rusqlite::Error> for Fill {
    fn from(e: rusqlite::Error) -> Self {
        Fill::Sql(e)
    }
}

/// Fill a new database from the project's files, in one transaction.
fn fill(conn: &mut Connection, project: &Project) -> Result<(), Fill> {
    let tx = conn.transaction()?;
    tx.execute_batch(TABLES)?;
    for id in project
        .track_ids()
        .map_err(|e| Fill::Other(Error::Project(e)))?
    {
        let track = project
            .track(&id)
            .map_err(|e| Fill::Other(Error::Project(e)))?;
        tx.execute(
            "INSERT INTO tracks (id, title) VALUES (?1, ?2)",
            params![id.as_str(), track.file.title],
        )?;
        let history = project
            .history(&id)
            .map_err(|e| Fill::Other(Error::History(e)))?;
        for (seq, (file, entry)) in history.entries().enumerate() {
            let (by, who) = match &entry.provenance {
                RunProvenance::Rules {} => ("rules", None),
                RunProvenance::Llm { model } => ("llm", Some(model.as_str())),
                RunProvenance::Manual { name } => ("manual", Some(name.as_str())),
            };
            tx.execute(
                "INSERT INTO versions (track, seq, file, version, parent, time, unix, restore, by, who) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id.as_str(),
                    seq as i64,
                    file,
                    entry.version.to_string(),
                    entry.parent.map(|p| p.to_string()),
                    entry.time.to_string(),
                    entry.time.unix(),
                    entry.restore,
                    by,
                    who
                ],
            )?;
            for u in &entry.uses {
                tx.execute(
                    "INSERT INTO uses (track, seq, stage, key) VALUES (?1, ?2, ?3, ?4)",
                    params![id.as_str(), seq as i64, u.stage, u.key.to_string()],
                )?;
            }
        }
    }
    let cache = Cache::new(project.root().join(".score64").join("cache"));
    for entry in cache.entries().map_err(|e| Fill::Other(Error::Cache(e)))? {
        let m = &entry.manifest;
        let key = m.key.to_string();
        let total: u64 = m.artifacts.iter().map(|a| a.bytes).sum();
        tx.execute(
            "INSERT INTO entries (stage, key, created, unix, stage_version, tool_version, millis, cost, currency, bytes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                m.stage,
                key,
                m.created.to_string(),
                m.created.unix(),
                m.run.stage_version,
                m.run.tool_version,
                m.run.millis as i64,
                m.run.cost.as_ref().map(|c| c.amount),
                m.run.cost.as_ref().map(|c| c.currency.as_str()),
                total as i64
            ],
        )?;
        for a in &m.artifacts {
            tx.execute(
                "INSERT INTO artifacts (stage, key, name, hash, bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![m.stage, key, a.name, a.hash.to_string(), a.bytes as i64],
            )?;
        }
    }
    tx.pragma_update(None, "user_version", INDEX_SCHEMA)?;
    tx.commit()?;
    Ok(())
}

fn sql(path: &Path) -> impl Fn(rusqlite::Error) -> Error + '_ {
    move |error| Error::Sql {
        path: path.to_path_buf(),
        error,
    }
}

#[cfg(test)]
mod tests;
