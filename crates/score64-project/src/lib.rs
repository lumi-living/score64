//! Project state on disk: `track.toml`, `decisions.toml`, approvals, `audio.lock`, the content-addressed stage cache
//! and the rebuildable index (ADR-004). Files are the source of truth; nothing lives only in the index.

pub mod cache;

pub use cache::Cache;
