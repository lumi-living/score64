//! `gc`: clear the project's stage cache down to a size cap, keeping what recorded versions use (#29).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use score64_project::gc::{Gc, RecordedVersions};
use score64_project::{Index, Project};

use crate::{Command, CommandError, Effect, Engine, ErrorKind};

/// `gc`: remove cache entries no recorded version uses, oldest first, until the cache fits `cap_bytes`. Also removes
/// broken entries and old `.tmp-*` leftovers. Entries from a newer Score64 stay. Destructive: a removed entry's paid
/// stages run again (and cost again) if it is ever needed.
pub struct GcCommand;

/// Arguments of [`GcCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GcArgs {
    /// The project folder (the one holding `score64.toml`).
    pub project: PathBuf,
    /// The most bytes the cache may hold once unrooted entries are removed; 4 GiB by default.
    #[serde(default)]
    pub cap_bytes: Option<u64>,
    /// Report what would go, and remove nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// One removed cache directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GcRemoved {
    /// Its stage.
    pub stage: String,
    /// Its directory: the key, or a leftover's name.
    pub name: String,
    /// Bytes in it.
    pub bytes: u64,
    /// `unrooted`, `broken` or `leftover`.
    pub why: String,
}

/// Output of [`GcCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GcOutput {
    /// The cap applied, in bytes.
    pub cap_bytes: u64,
    /// Entries kept because a recorded version uses them.
    pub rooted: u64,
    /// Their bytes.
    pub rooted_bytes: u64,
    /// Entries kept without a root, under the cap.
    pub unrooted_kept: u64,
    /// Their bytes.
    pub unrooted_kept_bytes: u64,
    /// What was removed, or would be in a dry run.
    pub removed: Vec<GcRemoved>,
    /// Bytes the roots alone hold beyond the cap.
    pub roots_over_cap: u64,
    /// Roots naming entries the cache doesn't hold.
    pub missing_roots: u64,
    /// Entries from a newer Score64, left alone.
    pub newer: u64,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Whether the index was rebuilt afterwards (when one exists and something was removed).
    pub index_rebuilt: bool,
}

impl Command for GcCommand {
    const NAME: &'static str = "gc";
    const SUMMARY: &'static str = "Clear the stage cache down to a size cap, keeping what recorded versions use; also removes broken entries and old leftovers.";
    const EFFECT: Effect = Effect::Destructive;
    type Args = GcArgs;
    type Output = GcOutput;

    fn run(_: &mut Engine, args: GcArgs) -> Result<GcOutput, CommandError> {
        let invalid = |e: String| CommandError::new(ErrorKind::InvalidInput, e);
        let project = Project::open(&args.project).map_err(|e| invalid(e.to_string()))?;
        let gc = Gc {
            cap_bytes: args.cap_bytes.unwrap_or(Gc::default().cap_bytes),
            dry_run: args.dry_run,
            ..Gc::default()
        };
        let report = gc
            .collect(&project, &[&RecordedVersions])
            .map_err(|e| invalid(e.to_string()))?;
        // The index lists cache entries: keep it in step when something went.
        let index_rebuilt =
            !args.dry_run && !report.removed.is_empty() && Index::path(&project).exists();
        if index_rebuilt {
            Index::rebuild(&project).map_err(|e| invalid(e.to_string()))?;
        }
        Ok(GcOutput {
            cap_bytes: gc.cap_bytes,
            rooted: report.rooted.0,
            rooted_bytes: report.rooted.1,
            unrooted_kept: report.unrooted_kept.0,
            unrooted_kept_bytes: report.unrooted_kept.1,
            removed: report
                .removed
                .into_iter()
                .map(|r| GcRemoved {
                    stage: r.stage,
                    name: r.name,
                    bytes: r.bytes,
                    why: r.why.into(),
                })
                .collect(),
            roots_over_cap: report.roots_over_cap,
            missing_roots: report.missing_roots,
            newer: report.newer,
            dry_run: report.dry_run,
            index_rebuilt,
        })
    }
}
