//! `gc`: clear the project's stage cache down to a size cap, keeping what recorded versions use (#29).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use score64_project::gc::{self, Gc, RecordedVersions, Report};
use score64_project::{Index, Project};

use crate::{Command, CommandError, Effect, Engine, ErrorKind};

/// `gc`: remove cache entries no recorded version uses, oldest first, until the cache's artifacts fit `cap_bytes`.
/// Also removes broken entries and `.tmp-*` leftovers older than ten minutes, whatever the cap. Entries from a newer
/// Score64, entries put while it runs, and links stay. An existing index is rebuilt afterwards; if that fails, the
/// output carries a `warning`, and the next `Index::open` rebuilds it since the cache it lists has changed. A
/// collection an error stops part-way still reports what it removed, with a `warning`, and fails.
///
/// Destructive: a removed entry's paid stages run again (and cost again) if it is ever needed. The effect is the
/// command's, so a `dry_run`, which removes nothing, is still labelled destructive.
///
/// Don't run it during a pipeline run: there is no project lock yet, and an entry a run has made but not yet recorded
/// is unrooted. Only the histories in the current checkout count, not those on other git branches.
pub struct GcCommand;

/// Arguments of [`GcCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GcArgs {
    /// The project folder (the one holding `score64.toml`).
    pub project: PathBuf,
    /// The most bytes the cache's artifacts may hold once unrooted entries are removed; 4 GiB by default. 0 removes
    /// every unrooted entry.
    #[serde(default)]
    pub cap_bytes: Option<u64>,
    /// Report what would go, and remove nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// One cache directory removed, or that would be in a dry run, or that couldn't be.
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

/// How many distinct cache entries one source of roots names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GcRoots {
    /// The source: `recorded versions`, later approvals and `audio.lock`.
    pub source: String,
    /// The distinct entries it names, whether the cache holds them or not.
    pub refs: u64,
}

/// Output of [`GcCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GcOutput {
    /// The cap applied, in bytes.
    pub cap_bytes: u64,
    /// Each source of roots.
    pub roots: Vec<GcRoots>,
    /// Entries kept because a root names them.
    pub rooted: u64,
    /// Their bytes.
    pub rooted_bytes: u64,
    /// Entries kept without a root, under the cap.
    pub unrooted_kept: u64,
    /// Their bytes.
    pub unrooted_kept_bytes: u64,
    /// What was removed, or would be in a dry run.
    pub removed: Vec<GcRemoved>,
    /// Entries that should have gone but couldn't be moved aside (a file open in them): left whole.
    pub in_use: Vec<GcRemoved>,
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
    /// False when an error stopped the collection part-way; `warning` says which. A failure (exit 1).
    pub complete: bool,
    /// What went wrong after something was removed: the collection stopping part-way, or the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn items(list: Vec<gc::Removed>) -> Vec<GcRemoved> {
    list.into_iter()
        .map(|r| GcRemoved {
            stage: r.stage,
            name: r.name,
            bytes: r.bytes,
            why: r.why.into(),
        })
        .collect()
}

fn kind(e: &gc::Error) -> ErrorKind {
    match e {
        gc::Error::Io { .. } => ErrorKind::Io,
        gc::Error::Roots { .. } => ErrorKind::InvalidInput,
    }
}

impl Command for GcCommand {
    const NAME: &'static str = "gc";
    const SUMMARY: &'static str = "Clear the stage cache down to a size cap, keeping what recorded versions use; also removes broken entries and old leftovers. Not during a pipeline run.";
    const EFFECT: Effect = Effect::Destructive;
    type Args = GcArgs;
    type Output = GcOutput;

    fn run(_: &mut Engine, args: GcArgs) -> Result<GcOutput, CommandError> {
        let project = Project::open(&args.project)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        let gc = Gc {
            cap_bytes: args.cap_bytes.unwrap_or(Gc::default().cap_bytes),
            dry_run: args.dry_run,
            ..Gc::default()
        };
        let (report, stopped): (Report, _) = match gc.collect(&project, &[&RecordedVersions]) {
            Ok(report) => (report, None),
            // Nothing changed: a plain error.
            Err(s) if !s.changed() => return Err(CommandError::new(kind(&s.error), s.to_string())),
            Err(s) => {
                let s = *s;
                (s.report, Some(s.error))
            }
        };
        let mut warnings = Vec::new();
        if let Some(e) = &stopped {
            warnings.push(format!("stopped part-way: {e}"));
        }
        // The index lists cache entries: keep it in step when something went. If that fails it is left as it is; its
        // fingerprint of the cache's folders no longer matches, so the next `Index::open` rebuilds it (#28). Deleting
        // it instead would leave a reader on Unix holding the deleted file.
        let changed = !report.dry_run && !report.removed.is_empty();
        let mut index_rebuilt = false;
        if changed && Index::path(&project).exists() {
            match Index::rebuild(&project) {
                Ok(_) => index_rebuilt = true,
                Err(e) => warnings.push(format!(
                    "the index could not be rebuilt ({e}); it is out of date until the next open rebuilds it"
                )),
            }
        }
        Ok(GcOutput {
            cap_bytes: gc.cap_bytes,
            roots: report
                .roots
                .into_iter()
                .map(|r| GcRoots {
                    source: r.source,
                    refs: r.refs,
                })
                .collect(),
            rooted: report.rooted.0,
            rooted_bytes: report.rooted.1,
            unrooted_kept: report.unrooted_kept.0,
            unrooted_kept_bytes: report.unrooted_kept.1,
            removed: items(report.removed),
            in_use: items(report.in_use),
            roots_over_cap: report.roots_over_cap,
            missing_roots: report.missing_roots,
            newer: report.newer,
            dry_run: report.dry_run,
            index_rebuilt,
            complete: stopped.is_none(),
            warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
        })
    }

    /// A collection an error stopped part-way is a failure, though what it removed is reported. Index trouble alone
    /// is a warning: the index is rebuildable.
    fn fails(output: &GcOutput) -> bool {
        !output.complete
    }
}

#[cfg(test)]
mod tests;
