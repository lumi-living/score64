//! The look is installed on both themes, the bundled fonts lead their families, and theme choices round-trip.

use super::*;

/// Runs one frame so pending fonts and styles take effect.
fn frame(ctx: &egui::Context) {
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear(); // no renderer here
}

#[test]
fn install_applies_the_family_style_to_both_themes() {
    let ctx = egui::Context::default();
    install(&ctx);
    frame(&ctx);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        // Field by field: egui updates `visuals.text_options` from the frame's input.
        let installed = ctx.style_of(theme);
        let mut want = style(theme);
        want.visuals.text_options = installed.visuals.text_options;
        assert_eq!(installed.visuals, want.visuals, "{theme:?}");
        assert_eq!(installed.spacing, want.spacing, "{theme:?}");
        assert_eq!(installed.text_styles, text_styles());
    }
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::System
    );
}

#[test]
fn set_theme_switches_semantic_colours() {
    let ctx = egui::Context::default();
    install(&ctx);
    set_theme(&ctx, egui::ThemePreference::Dark);
    frame(&ctx);
    assert_eq!(Theme::current(&ctx), &Theme::DARK);
    set_theme(&ctx, egui::ThemePreference::Light);
    frame(&ctx);
    assert_eq!(Theme::current(&ctx), &Theme::LIGHT);
    assert!(!ctx.global_style().visuals.dark_mode);
}

#[test]
fn bundled_fonts_lead_their_families_and_icons_are_registered() {
    let ctx = egui::Context::default();
    install(&ctx);
    frame(&ctx);
    let defs = ctx.fonts(|f| f.definitions().clone());
    let first = |family: &FontFamily| defs.families[family].first().cloned();
    assert_eq!(
        first(&FontFamily::Proportional).as_deref(),
        Some(fonts::PROPORTIONAL_NAME)
    );
    assert_eq!(
        first(&FontFamily::Monospace).as_deref(),
        Some(fonts::MONOSPACE_NAME)
    );
    // The icon font survives `set_fonts` and is a fallback of the proportional family.
    assert!(
        defs.font_data
            .contains_key(egui_material_icons::FONT_FAMILY)
    );
    assert!(
        defs.families[&FontFamily::Proportional]
            .iter()
            .any(|f| f == egui_material_icons::FONT_FAMILY)
    );
    assert!(defs.families.contains_key(&FontFamily::Name(MEDIUM.into())));
}

/// The semantic status colours are the family's, and alias egui's own.
#[test]
fn status_colours_alias_egui_visuals() {
    for (theme, sem) in [
        (egui::Theme::Light, Theme::LIGHT),
        (egui::Theme::Dark, Theme::DARK),
    ] {
        let v = style(theme).visuals;
        assert_eq!(sem.warn, v.warn_fg_color, "{theme:?}");
        assert_eq!(sem.error, v.error_fg_color, "{theme:?}");
        assert_eq!(sem.accent, v.hyperlink_color, "{theme:?}");
    }
}

#[test]
fn theme_choices_round_trip_through_storage_strings() {
    for p in [
        egui::ThemePreference::System,
        egui::ThemePreference::Light,
        egui::ThemePreference::Dark,
    ] {
        assert_eq!(preference_from_str(preference_to_str(p)), Some(p));
    }
    assert_eq!(preference_from_str("sepia"), None);
}
