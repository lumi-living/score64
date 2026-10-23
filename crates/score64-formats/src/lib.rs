//! Byte-exact readers and writers: GoatTracker 2 `.sng` / `.ins`, GoatTracker sound-effect data, MIDI import and WAV.
//!
//! Every writer matches `docs/reference/` byte for byte and has a round-trip test (ADR-007).

pub mod annotate;
pub mod emu_protocol;
pub mod gt2;
pub mod packed;
pub mod psid;
