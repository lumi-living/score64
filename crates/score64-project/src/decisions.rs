//! `decisions.toml` (#25): every decision about a track, as text in git (ADR-004, `docs/TAD.md` §3.3 and §4).
//!
//! ```toml
//! schema = 1
//!
//! [[decision]]
//! id = "loop"
//! subtune = "main"
//! bar = 5
//! confidence = 0.9
//! reviewed = true
//!
//! [decision.proposal]
//! change = "loop_row"
//! row = 80
//!
//! [decision.provenance]
//! by = "manual"
//! ```

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use score64_core::{Decision, DecisionId, SubtuneName};

/// The layout version this build reads and writes.
pub const SCHEMA: u32 = 1;

/// A `decisions.toml`: its decisions, sorted by subtune and then id when written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionsFile {
    /// Layout version; absent means 1.
    #[serde(default)]
    schema: Schema,
    /// The decisions.
    #[serde(default)]
    pub decision: Vec<Decision>,
}

/// The layout version, 1: a newer file is an error rather than a partial read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
struct Schema;

impl TryFrom<u32> for Schema {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        if v == SCHEMA {
            Ok(Schema)
        } else {
            Err(format!(
                "decisions schema {v} is not supported; this build reads {SCHEMA}"
            ))
        }
    }
}

impl From<Schema> for u32 {
    fn from(_: Schema) -> u32 {
        SCHEMA
    }
}

/// Why a `decisions.toml` could not be read or written.
#[derive(Debug)]
pub enum Error {
    /// The file could not be read or written.
    Io {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// The file is not a valid `decisions.toml`: bad TOML, an unknown field, a value out of range.
    Invalid {
        /// The file, if it was read from one.
        path: Option<PathBuf>,
        /// The 1-based line and column, when the parser knows them.
        at: Option<(usize, usize)>,
        /// The reason.
        message: String,
    },
    /// Two decisions in one subtune share an id.
    DuplicateId {
        /// The file, if it was read from one.
        path: Option<PathBuf>,
        /// The subtune.
        subtune: SubtuneName,
        /// The id.
        id: DecisionId,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = |p: &Option<PathBuf>| {
            p.as_ref()
                .map_or("decisions".to_string(), |p| p.display().to_string())
        };
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Invalid { path, at, message } => match at {
                Some((line, col)) => write!(f, "{}:{line}:{col}: {message}", file(path)),
                None => write!(f, "{}: {message}", file(path)),
            },
            Error::DuplicateId { path, subtune, id } => write!(
                f,
                "{}: subtune {:?} has two decisions with the id {:?}",
                file(path),
                subtune.as_str(),
                id.as_str()
            ),
        }
    }
}

impl std::error::Error for Error {}

impl DecisionsFile {
    /// A file holding `decisions`.
    pub fn new(decision: Vec<Decision>) -> Self {
        DecisionsFile {
            schema: Schema,
            decision,
        }
    }

    /// Parse a `decisions.toml`'s text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::parse_from(text, None)
    }

    fn parse_from(text: &str, path: Option<&Path>) -> Result<Self, Error> {
        let file: DecisionsFile = toml::from_str(text).map_err(|e| Error::Invalid {
            path: path.map(Path::to_path_buf),
            at: e.span().map(|s| line_col(text, s.start)),
            message: e.message().to_string(),
        })?;
        file.check(path)?;
        Ok(file)
    }

    /// Read a `decisions.toml`.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|error| Error::Io {
            path: path.to_path_buf(),
            error,
        })?;
        Self::parse_from(&text, Some(path))
    }

    /// The file's text: decisions sorted by subtune and then id, fields in their declared order, so the same
    /// decisions always give the same bytes.
    pub fn to_toml(&self) -> Result<String, Error> {
        self.check(None)?;
        let mut sorted = self.clone();
        sorted
            .decision
            .sort_by(|a, b| (&a.subtune, &a.id).cmp(&(&b.subtune, &b.id)));
        toml::to_string(&sorted).map_err(|e| Error::Invalid {
            path: None,
            at: None,
            message: e.to_string(),
        })
    }

    /// Write the file at `path`: to a temporary file beside it first, then renamed into place, so a crash never
    /// leaves half a file.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let text = self.to_toml()?;
        let io = |error| Error::Io {
            path: path.to_path_buf(),
            error,
        };
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        std::fs::write(&part, text).map_err(io)?;
        std::fs::rename(&part, path).map_err(io)
    }

    /// Two decisions with one id in one subtune are an error: notes and overrides name decisions by id.
    fn check(&self, path: Option<&Path>) -> Result<(), Error> {
        let mut seen = std::collections::BTreeSet::new();
        for d in &self.decision {
            if !seen.insert((&d.subtune, &d.id)) {
                return Err(Error::DuplicateId {
                    path: path.map(Path::to_path_buf),
                    subtune: d.subtune.clone(),
                    id: d.id.clone(),
                });
            }
        }
        Ok(())
    }
}

/// The 1-based line and column of byte `offset` in `text`.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rfind('\n')
        .map_or(before.len(), |i| before.len() - i - 1)
        + 1;
    (line, col)
}

#[cfg(test)]
mod tests;
