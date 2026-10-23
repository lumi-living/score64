//! Project state on disk: `score64.toml`, `track.toml`, `decisions.toml`, approvals, `audio.lock`, the
//! content-addressed stage cache and the rebuildable index (ADR-004, ADR-014). Files are the source of truth; nothing
//! lives only in the index.

pub mod cache;
pub mod decisions;
pub mod project;
mod textfile;

pub use cache::Cache;
pub use decisions::DecisionsFile;
pub use project::{Project, ProjectFile, SourceProblem, Timestamp, Track, TrackFile};
