//! `index`: the project's rebuildable index (#28, ADR-004), `.score64/index.db`.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use score64_project::index::Error as IndexError;
use score64_project::{Index, Project};

use crate::{Command, CommandError, Effect, Engine, ErrorKind};

/// `index`: open the project's index, rebuilding it from the files when asked, or when it is missing, damaged, of
/// another layout or made from files that have changed since ([`Index::open`]), and report what it holds. The index
/// only ever holds what the files say, so this reads the project; it changes nothing a rebuild couldn't make again.
pub struct IndexCommand;

/// Arguments of [`IndexCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexArgs {
    /// The project folder (the one holding `score64.toml`).
    pub project: PathBuf,
    /// Rebuild from the files even if the index is current.
    #[serde(default)]
    pub rebuild: bool,
}

/// Output of [`IndexCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexOutput {
    /// The index database.
    pub path: String,
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

impl Command for IndexCommand {
    const NAME: &'static str = "index";
    const SUMMARY: &'static str = "Open the project's index (rebuilding it from the files when asked, missing or outdated) and show what it holds.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Args = IndexArgs;
    type Output = IndexOutput;

    fn run(_: &mut Engine, args: IndexArgs) -> Result<IndexOutput, CommandError> {
        let project = Project::open(&args.project)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        let index = if args.rebuild {
            Index::rebuild(&project)
        } else {
            Index::open(&project)
        }
        .map_err(error)?;
        let s = index.summary().map_err(error)?;
        Ok(IndexOutput {
            path: Index::path(&project).display().to_string(),
            tracks: s.tracks,
            versions: s.versions,
            entries: s.entries,
            artifacts: s.artifacts,
            bytes: s.bytes,
        })
    }
}

/// The kind of an index failure: the project's own files are input; the index, its temporary files and the cache are
/// Score64's.
fn error(e: IndexError) -> CommandError {
    let kind = match &e {
        IndexError::Project(_) | IndexError::History(_) => ErrorKind::InvalidInput,
        IndexError::Io { .. }
        | IndexError::Sql { .. }
        | IndexError::Cache(_)
        | IndexError::TooLarge { .. }
        | IndexError::Damaged { .. } => ErrorKind::Io,
    };
    CommandError::new(kind, e.to_string())
}

#[cfg(test)]
mod tests;
