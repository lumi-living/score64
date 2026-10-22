//! Score64 Studio, the egui/eframe desktop app: library, listen and compare, voice map, pipeline, history,
//! sound-effect bench and adaptive-set audition (`docs/UX.md`, ADR-001). The UI is a thin front-end over the command
//! engine (ADR-005); UI state lives here, never in the core.

pub mod args;
pub mod icons;
pub mod shell;
pub mod theme;
