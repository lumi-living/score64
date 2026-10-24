//! The index (#28, ADR-004, `docs/TAD.md` §4): `.score64/index.db`, an SQLite database the app and commands query
//! for tracks, their recorded versions and the cache entries those versions use. It is never the truth: everything in
//! it is read from files (`track.toml`, `tracks/<id>/history/`, the cache's manifests), and [`Index::rebuild`] makes
//! it again from them at any time. Nothing writes to it except a rebuild.
//!
//! Tables (`PRAGMA user_version` is [`INDEX_SCHEMA`]; another value means rebuild):
//! - `tracks(id, title)`;
//! - `versions(track, seq, file, version, parent, parent_run, time, unix, restore, by, who)`: each track's history in
//!   graph order, `parent` being the version of the run it came after (`parent_run`),
//!   `who` being the model for `llm` and the name for `manual`;
//! - `uses(track, seq, stage, key)`: the cache entries each recorded run's version uses;
//! - `entries(stage, key, created, unix, stage_version, tool_version, millis, cost, currency, bytes)`: every whole
//!   cache entry, `bytes` being its artifacts' total;
//! - `artifacts(stage, key, name, hash, bytes)`;
//! - `inputs(path, kind, bytes, mtime)`: the files and folders the index was made from, as they were when the rebuild
//!   started (path under the project root, `file`, `folder` or `unreadable`, size, modification time in nanoseconds),
//!   which [`Index::open`] compares with the files now. It is the file system's own metadata, so it too is in the
//!   files.
//!
//! The journal mode is `DELETE` (a rollback journal beside the database while a rebuild copies in); WAL isn't used.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, ErrorCode, OpenFlags, params};

use score64_core::ContentHash;

use crate::cache::{Cache, Cost, check_name, unique_name};
use crate::history::{CacheRef, RunProvenance};
use crate::project::{Id, Project, TRACK_FILE, TrackFile};
use crate::version::VersionId;

/// The index layout this build writes; an index of another layout is rebuilt.
pub const INDEX_SCHEMA: i64 = 1;

/// How long a reader waits for a rebuild's copy, or a rebuild for readers, before giving up.
const WAIT: Duration = Duration::from_secs(10);

#[cfg(test)]
thread_local! {
    /// A shorter [`WAIT`] for tests on this thread.
    static TEST_WAIT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

/// [`WAIT`], or a test's own on this thread.
fn wait() -> Duration {
    #[cfg(test)]
    if let Some(w) = TEST_WAIT.get() {
        return w;
    }
    WAIT
}

/// How old a temporary database must be before a rebuild takes it for a crashed one's and removes it.
const LEFTOVER_AGE: Duration = Duration::from_secs(10 * 60);

const TABLES: &str = "
    CREATE TABLE tracks (id TEXT PRIMARY KEY, title TEXT);
    CREATE TABLE versions (
        track TEXT NOT NULL, seq INTEGER NOT NULL, file TEXT NOT NULL, version TEXT NOT NULL, parent TEXT, parent_run TEXT,
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
    CREATE TABLE inputs (path TEXT PRIMARY KEY, kind TEXT NOT NULL, bytes INTEGER, mtime INTEGER);
";

/// The tables a rebuild makes, sorted by name.
const TABLE_NAMES: [&str; 6] = [
    "artifacts",
    "entries",
    "inputs",
    "tracks",
    "uses",
    "versions",
];

/// An open index.
pub struct Index {
    conn: Connection,
    path: PathBuf,
}

/// How much an index holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// The version of the run it came after.
    pub parent: Option<VersionId>,
    /// The run it came after, by its id.
    pub parent_run: Option<String>,
    /// When it ran.
    pub time: crate::UtcTime,
    /// Whether it restored an earlier version.
    pub restore: bool,
    /// Who made the change.
    pub provenance: RunProvenance,
    /// Its history file's name.
    pub file: String,
}

/// One whole cache entry, as the index lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct CacheEntry {
    /// Its stage and key.
    pub entry: CacheRef,
    /// When it was written.
    pub created: crate::UtcTime,
    /// The stage's version.
    pub stage_version: String,
    /// The Score64 version that ran it.
    pub tool_version: String,
    /// How long the run took, in milliseconds.
    pub millis: u64,
    /// What it cost, if anything.
    pub cost: Option<Cost>,
    /// Bytes in its artifacts.
    pub bytes: u64,
    /// How many artifacts it holds.
    pub artifacts: u64,
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
    /// SQLite failed while building the index or copying it into place.
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
    /// A number in a file is too large for the index (SQLite integers are signed 64-bit).
    TooLarge {
        /// The file.
        path: PathBuf,
        /// What the number is.
        what: &'static str,
    },
    /// The index can't be read, or holds something no rebuild writes: `score64 index --rebuild` makes it again.
    Damaged {
        /// The index.
        path: PathBuf,
        /// SQLite's error, when a query failed.
        error: Option<rusqlite::Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Sql { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Project(e) => e.fmt(f),
            Error::History(e) => e.fmt(f),
            Error::Cache(e) => e.fmt(f),
            Error::TooLarge { path, what } => write!(
                f,
                "{}: {what} is too large for the index (over {})",
                path.display(),
                i64::MAX
            ),
            Error::Damaged {
                path,
                error: Some(error),
            } => write!(
                f,
                "{}: the index can't be read ({error}); run `score64 index --rebuild`",
                path.display()
            ),
            Error::Damaged { path, error: None } => write!(
                f,
                "{}: the index holds something no rebuild writes; run `score64 index --rebuild`",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { error, .. } => Some(error),
            Error::Sql { error, .. }
            | Error::Damaged {
                error: Some(error), ..
            } => Some(error),
            _ => None,
        }
    }
}

impl Index {
    /// Where a project's index lives.
    pub fn path(project: &Project) -> PathBuf {
        project.root().join(".score64").join("index.db")
    }

    /// The project's index, rebuilt first ([`Index::rebuild`]) when it is missing, can't be read, is of another
    /// layout ([`INDEX_SCHEMA`]) or lacks a table, or was made from files that have changed since.
    ///
    /// The check is cheap: it lists the files the index is made from (every `track.toml`, every `history/*.toml` the
    /// history reads, the cache's stage and key folders, and each key folder's `manifest.json`) and compares each
    /// one's name, size and modification time (a key folder's time changes when an artifact in it is deleted) with
    /// what the last rebuild recorded. It reads no file's contents and hashes nothing, so it can miss an edit that
    /// keeps a file's size and lands within the file system's timestamp resolution, and an artifact rewritten in
    /// place. That is enough for showing the project. Anything
    /// that deletes or keeps data on the index's word, such as cache GC (#29), calls [`Index::rebuild`] instead.
    pub fn open(project: &Project) -> Result<Self, Error> {
        let path = Self::path(project);
        if let Some(index) = Self::current(project, &path) {
            return Ok(index);
        }
        Self::rebuild(project)
    }

    /// The index at `path`, if it is this layout with every table, and was made from the files as they are now.
    fn current(project: &Project, path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        let index = Self::reader(path).ok()?;
        let version: i64 = index
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .ok()?;
        if version != INDEX_SCHEMA {
            return None;
        }
        let tables = index
            .strings("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .ok()?;
        if tables != TABLE_NAMES {
            return None;
        }
        let recorded = index.inputs().ok()?;
        (recorded == inputs(project)).then_some(index)
    }

    /// A connection to the index at `path` that only reads, waiting on a rebuild's copy rather than failing at once.
    ///
    /// It is opened read-write (never creating the file) with `query_only` on, not read-only: after a rebuild crashed
    /// midway through its copy, the database is half-written and its rollback journal is hot, and only a connection
    /// that may write can roll it back. So whichever reader comes first, including one opened before the crash,
    /// restores the old index, where a read-only one would fail ("attempt to write a readonly database"). A file the
    /// system won't let us write is opened read-only by SQLite itself.
    fn reader(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        conn.busy_timeout(wait())?;
        Ok(Index {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Make the index again from the project's files, whatever is there now.
    ///
    /// It is written whole into a new temporary database beside it (`index.db.<pid>-<n>-<nanos>.part`, created
    /// exclusively), then copied into `index.db` with SQLite's online backup API in one step, which is a single write
    /// transaction there. So on every system a reader sees the old index or the new one, never a mix; an [`Index`]
    /// held open across a rebuild sees the new one from its next query; and a crash midway leaves the old one, which
    /// the next reader restores from SQLite's rollback journal, since readers may write. The copy waits for readers
    /// in the middle of a query or a read transaction for up to ten seconds; past that the rebuild fails
    /// ([`Error::Sql`]), leaving the old index and no temporary files. Temporary databases a crashed rebuild left,
    /// over ten minutes old, are removed first.
    ///
    /// Fails on a track file, history or cache that can't be read, naming it; the old index stays as it was.
    pub fn rebuild(project: &Project) -> Result<Self, Error> {
        let path = Self::path(project);
        let dir = path.parent().expect("under .score64").to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|error| Error::Io {
            path: dir.clone(),
            error,
        })?;
        remove_leftovers(&dir);
        let tmp = create_temp(&dir)?;
        let built = build(project, &tmp).and_then(|source| copy_into(&source, &path));
        remove_temp(&tmp);
        built?;
        Self::reader(&path).map_err(|e| damaged(&path, e))
    }

    fn damaged(&self) -> impl Fn(rusqlite::Error) -> Error + '_ {
        move |e| damaged(&self.path, e)
    }

    fn corrupt(&self) -> Error {
        Error::Damaged {
            path: self.path.clone(),
            error: None,
        }
    }

    /// The first column of every row `sql` gives, as text.
    fn strings(&self, sql: &str) -> rusqlite::Result<Vec<String>> {
        let mut q = self.conn.prepare(sql)?;
        q.query_map([], |r| r.get(0))?.collect()
    }

    /// The files the index was made from, as the rebuild recorded them.
    fn inputs(&self) -> rusqlite::Result<Vec<Input>> {
        let mut q = self
            .conn
            .prepare("SELECT path, kind, bytes, mtime FROM inputs ORDER BY path")?;
        q.query_map([], |r| {
            Ok(Input {
                path: r.get(0)?,
                kind: r.get(1)?,
                bytes: r.get(2)?,
                mtime: r.get(3)?,
            })
        })?
        .collect()
    }

    /// How much it holds.
    pub fn summary(&self) -> Result<Summary, Error> {
        let count = |sql: &str| -> Result<u64, Error> {
            let n: i64 = self
                .conn
                .query_row(sql, [], |r| r.get(0))
                .map_err(self.damaged())?;
            u64::try_from(n).map_err(|_| self.corrupt())
        };
        Ok(Summary {
            tracks: count("SELECT COUNT(*) FROM tracks")?,
            versions: count("SELECT COUNT(*) FROM versions")?,
            entries: count("SELECT COUNT(*) FROM entries")?,
            artifacts: count("SELECT COUNT(*) FROM artifacts")?,
            bytes: count("SELECT COALESCE(SUM(bytes), 0) FROM artifacts")?,
        })
    }

    /// Every track, by id, with its title.
    pub fn tracks(&self) -> Result<Vec<(Id, Option<String>)>, Error> {
        let mut q = self
            .conn
            .prepare("SELECT id, title FROM tracks ORDER BY id")
            .map_err(self.damaged())?;
        let rows = q
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(self.damaged())?;
        let mut out = Vec::new();
        for row in rows {
            let (id, title) = row.map_err(self.damaged())?;
            out.push((Id::new(&id).map_err(|_| self.corrupt())?, title));
        }
        Ok(out)
    }

    /// Track `id`'s recorded runs, oldest first.
    pub fn versions(&self, id: &Id) -> Result<Vec<Version>, Error> {
        let mut q = self
            .conn
            .prepare(
                "SELECT version, parent, time, unix, restore, by, who, file, parent_run FROM versions WHERE track = ?1 \
                 ORDER BY seq",
            )
            .map_err(self.damaged())?;
        let rows = q
            .query_map(params![id.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(self.damaged())?;
        let mut out = Vec::new();
        for row in rows {
            let (version, parent, text, unix, restore, by, who, file, parent_run) =
                row.map_err(self.damaged())?;
            let hash = |s: &str| VersionId::from_hex(s).ok_or_else(|| self.corrupt());
            let provenance = match (by.as_str(), who) {
                ("rules", None) => RunProvenance::Rules {},
                ("llm", Some(model)) => RunProvenance::Llm { model },
                ("manual", Some(name)) => RunProvenance::Manual { name },
                _ => return Err(self.corrupt()),
            };
            out.push(Version {
                version: hash(&version)?,
                parent: parent.as_deref().map(hash).transpose()?,
                parent_run,
                time: self.time(unix, &text)?,
                restore,
                provenance,
                file,
            });
        }
        Ok(out)
    }

    /// The time in a row's `unix` column, which its text column must spell.
    fn time(&self, unix: i64, text: &str) -> Result<crate::UtcTime, Error> {
        crate::UtcTime::from_unix(unix)
            .filter(|t| t.to_string() == text)
            .ok_or_else(|| self.corrupt())
    }

    /// Every whole cache entry, by stage and then key.
    pub fn entries(&self) -> Result<Vec<CacheEntry>, Error> {
        let mut q = self
            .conn
            .prepare(
                "SELECT e.stage, e.key, e.created, e.unix, e.stage_version, e.tool_version, e.millis, e.cost, \
                 e.currency, e.bytes, (SELECT COUNT(*) FROM artifacts a WHERE a.stage = e.stage AND a.key = e.key) \
                 FROM entries e ORDER BY e.stage, e.key",
            )
            .map_err(self.damaged())?;
        type Row = (
            String,
            String,
            String,
            i64,
            String,
            String,
            i64,
            Option<f64>,
            Option<String>,
            i64,
            i64,
        );
        let rows = q
            .query_map([], |r| -> rusqlite::Result<Row> {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            })
            .map_err(self.damaged())?;
        let mut out = Vec::new();
        for row in rows {
            let (
                stage,
                key,
                created,
                unix,
                stage_version,
                tool_version,
                millis,
                amount,
                currency,
                bytes,
                artifacts,
            ) = row.map_err(self.damaged())?;
            let unsigned = |n: i64| u64::try_from(n).map_err(|_| self.corrupt());
            let cost = match (amount, currency) {
                (Some(amount), Some(currency)) => Some(Cost { amount, currency }),
                (None, None) => None,
                _ => return Err(self.corrupt()),
            };
            out.push(CacheEntry {
                entry: CacheRef {
                    stage,
                    key: ContentHash::from_hex(&key).ok_or_else(|| self.corrupt())?,
                },
                created: self.time(unix, &created)?,
                stage_version,
                tool_version,
                millis: unsigned(millis)?,
                cost,
                bytes: unsigned(bytes)?,
                artifacts: unsigned(artifacts)?,
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
            .map_err(self.damaged())?;
        let rows = q
            .query_map(params![version.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(self.damaged())?;
        let mut out = Vec::new();
        for row in rows {
            let (stage, key) = row.map_err(self.damaged())?;
            let key = ContentHash::from_hex(&key).ok_or_else(|| self.corrupt())?;
            out.push(CacheRef { stage, key });
        }
        Ok(out)
    }

    /// The tracks and versions that use a cache entry, sorted and without repeats.
    pub fn used_by(&self, entry: &CacheRef) -> Result<Vec<(Id, VersionId)>, Error> {
        let mut q = self
            .conn
            .prepare(
                "SELECT DISTINCT v.track, v.version FROM uses u JOIN versions v ON u.track = v.track AND u.seq = v.seq \
                 WHERE u.stage = ?1 AND u.key = ?2 ORDER BY v.track, v.version",
            )
            .map_err(self.damaged())?;
        let rows = q
            .query_map(params![entry.stage, entry.key.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(self.damaged())?;
        let mut out = Vec::new();
        for row in rows {
            let (track, version) = row.map_err(self.damaged())?;
            let track = Id::new(&track).map_err(|_| self.corrupt())?;
            let v = VersionId::from_hex(&version).ok_or_else(|| self.corrupt())?;
            out.push((track, v));
        }
        Ok(out)
    }
}

fn damaged(path: &Path, error: rusqlite::Error) -> Error {
    Error::Damaged {
        path: path.to_path_buf(),
        error: Some(error),
    }
}

/// One file or folder the index is made from, as [`Index::open`] compares it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Input {
    /// Its path under the project root, with `/`; a folder's ends in `/`.
    path: String,
    /// `file`, `folder` (a stage folder counts by its listing only; a key folder by its listing and its time, which
    /// changes when a file inside is added or removed) or `unreadable` (it couldn't be listed or looked at, which no
    /// rebuild that succeeded saw).
    kind: String,
    /// A file's size.
    bytes: Option<i64>,
    /// A file's or key folder's modification time, in nanoseconds since 1970, where the system gives one.
    mtime: Option<i64>,
}

impl Input {
    /// A folder that counts by its listing alone (a stage folder: putting an entry makes and renames a temporary
    /// directory in it, which changes its time without changing what the index holds).
    fn folder(path: String) -> Self {
        Input {
            path,
            kind: "folder".into(),
            bytes: None,
            mtime: None,
        }
    }

    /// A folder that counts by its time too: a key folder, whose time changes when an artifact in it is deleted or
    /// added, so the entry is no longer whole.
    fn timed_folder(path: String, meta: std::io::Result<std::fs::Metadata>) -> Self {
        let mut folder = Input::folder(path);
        folder.mtime = meta.ok().and_then(|m| nanos(m.modified()));
        folder
    }

    fn unreadable(path: String) -> Self {
        Input {
            path,
            kind: "unreadable".into(),
            bytes: None,
            mtime: None,
        }
    }

    fn file(path: String, meta: std::io::Result<std::fs::Metadata>) -> Self {
        let Ok(meta) = meta else {
            return Input::unreadable(path);
        };
        Input {
            path,
            kind: "file".into(),
            bytes: i64::try_from(meta.len()).ok(),
            mtime: nanos(meta.modified()),
        }
    }
}

/// A modification time in nanoseconds since 1970 (negative before), if the system gives one that fits.
fn nanos(time: std::io::Result<SystemTime>) -> Option<i64> {
    match time.ok()?.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).ok(),
        Err(before) => i64::try_from(before.duration().as_nanos()).ok().map(|n| -n),
    }
}

/// The names in `dir`, sorted: `Some(vec![])` when it doesn't exist, `None` when it can't be listed.
fn listing(dir: &Path) -> Option<Vec<String>> {
    let items = match std::fs::read_dir(dir) {
        Ok(items) => items,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(vec![]),
        Err(_) => return None,
    };
    let mut names = Vec::new();
    for item in items {
        names.push(item.ok()?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Some(names)
}

/// The files and folders an index of `project` is made from, as they are now, sorted by path: each track's
/// `track.toml` and the `history/*.toml` files [`crate::History::read`] reads, and the cache's stage folders, key
/// folders and manifests that [`Cache::entries`] looks at. Only listings and metadata are read.
fn inputs(project: &Project) -> Vec<Input> {
    let root = project.root();
    let mut out = Vec::new();
    match listing(&root.join("tracks")) {
        None => out.push(Input::unreadable("tracks/".into())),
        Some(tracks) => {
            for name in tracks {
                let dir = root.join("tracks").join(&name);
                let file = dir.join(TRACK_FILE);
                // As `Project::track_ids`: a track is a folder holding a `track.toml` file.
                if !file.is_file() {
                    continue;
                }
                out.push(Input::file(
                    format!("tracks/{name}/{TRACK_FILE}"),
                    std::fs::metadata(&file),
                ));
                let history = format!("tracks/{name}/history/");
                match listing(&dir.join("history")) {
                    None => out.push(Input::unreadable(history)),
                    Some(files) => {
                        for f in files {
                            // As `History::read`: hidden files and anything not `.toml` are skipped.
                            if f.starts_with('.') || !f.ends_with(".toml") {
                                continue;
                            }
                            let meta = std::fs::symlink_metadata(dir.join("history").join(&f));
                            out.push(Input::file(format!("{history}{f}"), meta));
                        }
                    }
                }
            }
        }
    }
    let cache = root.join(".score64").join("cache");
    match listing(&cache) {
        None => out.push(Input::unreadable(".score64/cache/".into())),
        Some(stages) => {
            for stage in stages {
                let stage_dir = cache.join(&stage);
                // As `Cache::entries`: only valid stage names and lowercase-hex keys.
                if check_name(&stage).is_err() || !stage_dir.is_dir() {
                    continue;
                }
                let at = format!(".score64/cache/{stage}/");
                let Some(keys) = listing(&stage_dir) else {
                    out.push(Input::unreadable(at));
                    continue;
                };
                out.push(Input::folder(at.clone()));
                for key in keys {
                    if ContentHash::from_hex(&key).is_none_or(|k| k.to_string() != key) {
                        continue;
                    }
                    let key_dir = stage_dir.join(&key);
                    out.push(Input::timed_folder(
                        format!("{at}{key}/"),
                        std::fs::metadata(&key_dir),
                    ));
                    let manifest = key_dir.join("manifest.json");
                    if let Ok(meta) = std::fs::metadata(&manifest) {
                        out.push(Input::file(format!("{at}{key}/manifest.json"), Ok(meta)));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// A new, empty temporary database file in `dir`, created exclusively.
fn create_temp(dir: &Path) -> Result<PathBuf, Error> {
    for _ in 0..64 {
        let path = dir.join(unique_name("index.db.", ".part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::Io { path, error }),
        }
    }
    Err(Error::Io {
        path: dir.to_path_buf(),
        error: std::io::Error::other("no free temporary database name"),
    })
}

/// SQLite's rollback journal for the database at `path`.
fn journal(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push("-journal");
    PathBuf::from(name)
}

/// Remove a temporary database and its journal, if they are there.
fn remove_temp(tmp: &Path) {
    let _ = std::fs::remove_file(tmp);
    let _ = std::fs::remove_file(journal(tmp));
}

/// Remove the temporary databases (and journals) in `dir` that a crashed rebuild left: over [`LEFTOVER_AGE`] old, so
/// never one a running rebuild is still writing.
fn remove_leftovers(dir: &Path) {
    let Ok(items) = std::fs::read_dir(dir) else {
        return;
    };
    for item in items.flatten() {
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("index.db.")
            || !(name.ends_with(".part") || name.ends_with(".part-journal"))
        {
            continue;
        }
        let old = item
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > LEFTOVER_AGE);
        if old {
            let _ = std::fs::remove_file(item.path());
        }
    }
}

/// Build the index of `project` into the empty database at `tmp`, and give the open connection to it.
fn build(project: &Project, tmp: &Path) -> Result<Connection, Error> {
    // Before anything is read, so a file that changes during the rebuild makes the next `open` rebuild again.
    let inputs = inputs(project);
    let sql = |error| Error::Sql {
        path: tmp.to_path_buf(),
        error,
    };
    let mut conn = Connection::open(tmp).map_err(sql)?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))
        .map_err(sql)?;
    debug_assert_eq!(mode, "delete");
    fill(&mut conn, project, &inputs).map_err(|e| match e {
        Fill::Sql(error) => sql(error),
        Fill::Other(e) => e,
    })?;
    Ok(conn)
}

/// Copy the database `source` holds into `path` in one backup step: one write transaction there, so readers see it
/// whole or not at all. A file at `path` that isn't an SQLite database at all is replaced.
fn copy_into(source: &Connection, path: &Path) -> Result<(), Error> {
    let not_a_database = |e: &rusqlite::Error| {
        matches!(
            e.sqlite_error_code(),
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
        )
    };
    let result = match backup(source, path) {
        Err(e) if not_a_database(&e) => {
            // Nothing a reader could be using: start afresh.
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(Error::Io {
                    path: path.to_path_buf(),
                    error,
                });
            }
            let _ = std::fs::remove_file(journal(path));
            backup(source, path)
        }
        result => result,
    };
    result.map_err(|error| Error::Sql {
        path: path.to_path_buf(),
        error,
    })
}

/// One backup of `source` into `path`, retried while readers hold the index, for up to [`WAIT`]. This loop is the only
/// waiting: `dest`'s busy handler is off (rusqlite's default is five seconds), so a busy step returns at once.
fn backup(source: &Connection, path: &Path) -> rusqlite::Result<()> {
    let mut dest = Connection::open(path)?;
    dest.busy_timeout(Duration::ZERO)?;
    let backup = Backup::new(source, &mut dest)?;
    let start = Instant::now();
    loop {
        match backup.step(-1)? {
            StepResult::Done => return Ok(()),
            // Readers are in the middle of a query or transaction; their next one sees the new index.
            _ if start.elapsed() < wait() => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    Some("readers held the index for too long".into()),
                ));
            }
        }
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

impl From<Error> for Fill {
    fn from(e: Error) -> Self {
        Fill::Other(e)
    }
}

/// `n` as an SQLite integer, or [`Error::TooLarge`] naming `path` and `what`.
fn int(n: u64, path: &Path, what: &'static str) -> Result<i64, Fill> {
    i64::try_from(n).map_err(|_| {
        Fill::Other(Error::TooLarge {
            path: path.to_path_buf(),
            what,
        })
    })
}

/// Fill a new database from the project's files, in one transaction.
fn fill(conn: &mut Connection, project: &Project, inputs: &[Input]) -> Result<(), Fill> {
    let tx = conn.transaction()?;
    tx.execute_batch(TABLES)?;
    for i in inputs {
        tx.execute(
            "INSERT INTO inputs (path, kind, bytes, mtime) VALUES (?1, ?2, ?3, ?4)",
            params![i.path, i.kind, i.bytes, i.mtime],
        )?;
    }
    for id in project.track_ids().map_err(Error::Project)? {
        // Only `track.toml`, through its own parse: the index needs the title, not the decisions or the project's
        // profiles, so a bad `decisions.toml` doesn't block it.
        let path = project.track_dir(&id).join(TRACK_FILE);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            Error::Project(crate::project::Error::Io {
                path: path.clone(),
                error,
            })
        })?;
        let track = TrackFile::parse(&text, &path).map_err(Error::Project)?;
        tx.execute(
            "INSERT INTO tracks (id, title) VALUES (?1, ?2)",
            params![id.as_str(), track.title],
        )?;
        let history = project.history(&id).map_err(Error::History)?;
        let dir = project.track_dir(&id).join("history");
        for (seq, (file, entry)) in history.entries().enumerate() {
            let seq = int(
                u64::try_from(seq).unwrap_or(u64::MAX),
                &dir,
                "the number of runs",
            )?;
            let (by, who) = match &entry.provenance {
                RunProvenance::Rules {} => ("rules", None),
                RunProvenance::Llm { model } => ("llm", Some(model.as_str())),
                RunProvenance::Manual { name } => ("manual", Some(name.as_str())),
            };
            tx.execute(
                "INSERT INTO versions (track, seq, file, version, parent, parent_run, time, unix, restore, by, who) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id.as_str(),
                    seq,
                    file,
                    entry.version.to_string(),
                    entry
                        .parent
                        .as_deref()
                        .and_then(|p| history.run(p))
                        .map(|p| p.version.to_string()),
                    entry.parent,
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
                    params![id.as_str(), seq, u.stage, u.key.to_string()],
                )?;
            }
        }
    }
    let cache = Cache::new(project.root().join(".score64").join("cache"));
    for entry in cache.entries().map_err(Error::Cache)? {
        let m = &entry.manifest;
        let manifest = entry.dir.join("manifest.json");
        let key = m.key.to_string();
        let total = m
            .artifacts
            .iter()
            .try_fold(0u64, |t, a| t.checked_add(a.bytes))
            .unwrap_or(u64::MAX);
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
                int(m.run.millis, &manifest, "`run.millis`")?,
                m.run.cost.as_ref().map(|c| c.amount),
                m.run.cost.as_ref().map(|c| c.currency.as_str()),
                int(total, &manifest, "the artifacts' total size")?
            ],
        )?;
        for a in &m.artifacts {
            tx.execute(
                "INSERT INTO artifacts (stage, key, name, hash, bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    m.stage,
                    key,
                    a.name,
                    a.hash.to_string(),
                    int(a.bytes, &manifest, "an artifact's `bytes`")?
                ],
            )?;
        }
    }
    tx.pragma_update(None, "user_version", INDEX_SCHEMA)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests;
