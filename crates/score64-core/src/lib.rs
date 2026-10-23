//! Pure Rust, no I/O: the tracker-grid score model (notes on rows, voices with roles, subtunes), voice profiles,
//! reduction rules and the validator that enforces SID and player constraints (`docs/TAD.md §1`, ADR-006).
//!
//! This crate must stay free of `std::fs`, network access, threads-by-default and platform code.
//! Everything that talks to the outside world lives in `score64-adapters`.
//!
//! The model serialises deterministically, so files diff cleanly in git (ADR-004): fields in declaration order,
//! lists instead of maps, absent optional fields left out. Unknown fields are errors, so a typo in a file is caught
//! instead of silently dropped.
//!
//! Features: `schema` derives JSON Schemas (for engine commands whose arguments embed model types); `proptest`
//! exposes [`strategies`] for property tests.

pub mod adaptive;
pub mod decide;
pub mod grid;
pub mod profile;
pub mod provenance;
pub mod reduce;
#[cfg(feature = "schema")]
mod schema;
pub mod score;
#[cfg(any(test, feature = "proptest"))]
pub mod strategies;
pub mod transcription;
pub mod validate;

pub use adaptive::{AdaptiveSet, FIRST_MARK_VALUE, LayerProblem, Switch, TimingMark, TimingMarks};
pub use decide::{Checked, Rank, Resolution, Slot, Unapplied, check, resolve};
pub use grid::{BeatGrid, Quantised, TempoSnap, VideoStandard, snap_tempo};
pub use profile::{Profile, ProfileName, Profiles, VoiceRule, VoiceUse, builtin};
pub use provenance::{
    Confidence, ContentHash, Decision, DecisionId, DropReason, Proposal, Provenance, SourceRef,
};
pub use score::{
    ArpEvent, Attack, FramesPerRow, Note, Pitch, PresetId, Role, RoleSpan, Roles, SCHEMA_VERSION,
    Schema, Song, Subtune, SubtuneName, TempoChange, Voice, VoiceLine,
};
pub use transcription::{
    DroppedNote, QuantiseReport, QuantisedPart, SourceNote, TranscribedPart, Unplaced,
};
pub use validate::{
    Context, InstrumentFacts, PackBudget, PackFacts, Problem, Rule, check_pack, validate,
};

#[cfg(test)]
mod properties;
#[cfg(test)]
mod tests;
