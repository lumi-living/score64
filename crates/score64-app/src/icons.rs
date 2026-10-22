//! Every icon the app draws, by what it means. The rest of the app names these constants, never
//! `egui_material_icons::icons` directly, so a glyph change is one line here (the same rule as Convert64).

pub use egui_material_icons::MaterialIcon as Icon;
use egui_material_icons::icons as m;

/// The library of assets (`docs/UX.md` §3).
pub const LIBRARY: Icon = m::ICON_LIBRARY_MUSIC;
/// The sound-effect bench.
pub const EFFECTS: Icon = m::ICON_GRAPHIC_EQ;
/// Adaptive sets.
pub const ADAPTIVE: Icon = m::ICON_LAYERS;
/// Build: audio.lock, budgets, export.
pub const BUILD: Icon = m::ICON_BUILD;
