//! Adapters behind the core's ports: transcription providers, beat tracking, decision makers (rules, LLM, manual),
//! the GoatTracker packer, VICE, the C64 Ultimate and the optional Python sidecar (ADR-009).

pub mod basic_pitch;
pub mod sidecar;
