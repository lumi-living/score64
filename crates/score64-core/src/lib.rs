//! Pure Rust, no I/O: the tracker-grid score model (notes on rows, voices with roles, subtunes), voice profiles,
//! reduction rules and the validator that enforces SID and player constraints (`docs/TAD.md §1`, ADR-006).
//!
//! This crate must stay free of `std::fs`, network access, threads-by-default and platform code.
//! Everything that talks to the outside world lives in `score64-adapters`.
