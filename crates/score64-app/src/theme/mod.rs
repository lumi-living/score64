//! Score64 Studio's look: the family visuals, spacing, text styles and fonts shared with Convert64 ([`family`]), the
//! Material Symbols icon font, and the app's semantic colours.
//!
//! Like Convert64, the theme follows the system until the user picks Light or Dark (View › Theme), and the choice is
//! kept between runs.

mod family;

pub use family::{apply_dark, apply_light, apply_spacing, fonts, text_styles};

use egui::{Color32, FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// Score64's semantic colours for one theme. For now the family's status and accent colours, which alias egui's own
/// (`warn_fg_color`, `error_fg_color`, `hyperlink_color`); voice and stage colours arrive with E10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Status: success (validator clean, stage fresh).
    pub ok: Color32,
    /// Status: warning (validator warnings).
    pub warn: Color32,
    /// Status: error (validator errors, failed stages).
    pub error: Color32,
    /// Accent.
    pub accent: Color32,
}

impl Theme {
    /// Light mode (the family's `status/*` and `accent`).
    pub const LIGHT: Theme = Theme {
        ok: Color32::from_rgb(0x16, 0xa3, 0x4a),
        warn: Color32::from_rgb(0xff, 0x64, 0x00),
        error: Color32::from_rgb(0xff, 0x00, 0x00),
        accent: Color32::from_rgb(0x00, 0x9b, 0xff),
    };

    /// Dark mode.
    pub const DARK: Theme = Theme {
        ok: Color32::from_rgb(0x4a, 0xde, 0x80),
        warn: Color32::from_rgb(0xff, 0x8f, 0x00),
        error: Color32::from_rgb(0xff, 0x00, 0x00),
        accent: Color32::from_rgb(0x5a, 0xaa, 0xff),
    };

    /// The semantic colours for `theme`.
    pub const fn of(theme: egui::Theme) -> &'static Self {
        match theme {
            egui::Theme::Light => &Self::LIGHT,
            egui::Theme::Dark => &Self::DARK,
        }
    }

    /// The semantic colours for the theme `ctx` is drawing with.
    pub fn current(ctx: &egui::Context) -> &'static Self {
        Self::of(ctx.theme())
    }
}

/// The named font family for Inter Medium (headings, emphasis): `FontFamily::Name(MEDIUM.into())`.
pub const MEDIUM: &str = "medium";

/// egui's font definitions with the bundled fonts first: Inter for `Proportional`, JetBrains Mono for `Monospace`,
/// and Inter Medium as the [`MEDIUM`] family. egui's own fonts stay behind them as fallbacks.
pub fn font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut put_first = |family: FontFamily, name: &str, data: &'static [u8]| {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(data)));
        defs.families
            .entry(family)
            .or_default()
            .insert(0, name.to_owned());
    };
    put_first(
        FontFamily::Proportional,
        fonts::PROPORTIONAL_NAME,
        fonts::PROPORTIONAL,
    );
    put_first(
        FontFamily::Monospace,
        fonts::MONOSPACE_NAME,
        fonts::MONOSPACE,
    );
    if let Some(medium) = fonts::PROPORTIONAL_MEDIUM {
        let name = format!("{} Medium", fonts::PROPORTIONAL_NAME);
        defs.font_data
            .insert(name.clone(), Arc::new(FontData::from_static(medium)));
        let mut chain = vec![name];
        chain.extend(
            defs.families
                .get(&FontFamily::Proportional)
                .cloned()
                .unwrap_or_default(),
        );
        defs.families.insert(FontFamily::Name(MEDIUM.into()), chain);
    }
    defs
}

/// Installs the look on `ctx`: fonts, then the Material Symbols icon font (after `set_fonts`, which would otherwise
/// drop it), then the light and dark styles. The theme follows the system until [`set_theme`] says otherwise. Call
/// once, at startup.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    egui_material_icons::initialize(ctx);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
    set_theme(ctx, egui::ThemePreference::System);
}

/// The full egui style for `theme`: egui's default with every family value applied.
pub fn style(theme: egui::Theme) -> egui::Style {
    let mut style = theme.default_style();
    match theme {
        egui::Theme::Light => apply_light(&mut style.visuals),
        egui::Theme::Dark => apply_dark(&mut style.visuals),
    }
    apply_spacing(&mut style.spacing);
    style.text_styles = text_styles();
    style
}

/// Switches between the system theme, light and dark.
pub fn set_theme(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_theme(preference);
}

/// How a theme choice is stored.
pub fn preference_to_str(preference: egui::ThemePreference) -> &'static str {
    match preference {
        egui::ThemePreference::System => "system",
        egui::ThemePreference::Light => "light",
        egui::ThemePreference::Dark => "dark",
    }
}

/// A stored theme choice, if it is one.
pub fn preference_from_str(s: &str) -> Option<egui::ThemePreference> {
    match s {
        "system" => Some(egui::ThemePreference::System),
        "light" => Some(egui::ThemePreference::Light),
        "dark" => Some(egui::ThemePreference::Dark),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
