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
        /// The lines of the two `[[decision]]` headers, when read from text written that way.
        lines: Option<(usize, usize)>,
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
            Error::DuplicateId {
                path,
                subtune,
                id,
                lines,
            } => {
                write!(
                    f,
                    "{}: subtune {:?} has two decisions with the id {:?}",
                    file(path),
                    subtune.as_str(),
                    id.as_str()
                )?;
                match lines {
                    Some((a, b)) => write!(f, " (lines {a} and {b})"),
                    None => Ok(()),
                }
            }
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
        let invalid = |at, message: String| Error::Invalid {
            path: path.map(Path::to_path_buf),
            at,
            message,
        };
        // Errors inside a `proposal` or `provenance` table point at its header: those tables are read whole to find
        // their kind, which loses the positions within them.
        let file: DecisionsFile = toml::from_str(text).map_err(|e| {
            invalid(
                e.span().map(|s| line_col(text, s.start)),
                e.message().to_string(),
            )
        })?;
        // A provenance with no fields of its own (`by = "manual"`) must have no others: serde would drop them.
        let table: toml::Table = toml::from_str(text).expect("parsed above");
        let headers = decision_headers(text);
        if let Some(toml::Value::Array(decisions)) = table.get("decision") {
            for (i, d) in decisions.iter().enumerate() {
                let Some(toml::Value::Table(p)) = d.get("provenance") else {
                    continue;
                };
                if p.get("by").and_then(toml::Value::as_str) == Some("manual") && p.len() > 1 {
                    let extra: Vec<&str> = p
                        .keys()
                        .map(String::as_str)
                        .filter(|k| *k != "by")
                        .collect();
                    let at = headers.get(i).map(|&line| (line, 1));
                    return Err(invalid(
                        at,
                        format!(
                            "unknown field `{}` in a manual provenance",
                            extra.join("`, `")
                        ),
                    ));
                }
            }
        }
        file.check(path, Some(&headers))?;
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

    /// The file's text: decisions sorted by subtune and then id, each one's plain fields in their declared order and
    /// then its `proposal` and `provenance` tables (TOML puts tables last), so the same decisions always give the same
    /// bytes.
    pub fn to_toml(&self) -> Result<String, Error> {
        self.check(None, None)?;
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

    /// Write the file at `path`: to a temporary file beside it (`<name>.<pid>-<n>.part`, so two writers never share
    /// one), synced to disk, then renamed over the target. A crash or power loss leaves the old file or the new one,
    /// never half of one; on Windows the rename fails while another process holds the target open. A temporary file
    /// is removed when anything fails.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let text = self.to_toml()?;
        crate::textfile::write_atomic(path, &text)
            .map_err(|(path, error)| Error::Io { path, error })
    }

    /// Two decisions with one id in one subtune are an error: notes and overrides name decisions by id. `headers`
    /// are the lines of the `[[decision]]` headers, to say where both are.
    fn check(&self, path: Option<&Path>, headers: Option<&[usize]>) -> Result<(), Error> {
        let mut seen = std::collections::BTreeMap::new();
        for (i, d) in self.decision.iter().enumerate() {
            if let Some(first) = seen.insert((&d.subtune, &d.id), i) {
                let line = |n: usize| headers.and_then(|h| h.get(n).copied());
                return Err(Error::DuplicateId {
                    path: path.map(Path::to_path_buf),
                    subtune: d.subtune.clone(),
                    id: d.id.clone(),
                    lines: line(first).zip(line(i)),
                });
            }
        }
        Ok(())
    }
}

/// The lines of the `[[decision]]` headers, in order: the n-th decision's header is the n-th.
fn decision_headers(text: &str) -> Vec<usize> {
    crate::textfile::headers(text, "[[decision]]")
}

use crate::textfile::line_col;

#[cfg(test)]
mod tests;
