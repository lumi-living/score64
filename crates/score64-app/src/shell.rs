//! The window's chrome: menu bar, navigation dock, status bar and an empty central view (#6). The sections follow
//! `docs/UX.md` §3; each view arrives with its epic.

use egui::{Button, RichText, Ui};

use crate::icons::{self, Icon};

/// A top-level section of the app (`docs/UX.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    /// Assets and their versions.
    #[default]
    Library,
    /// The sound-effect bench.
    Effects,
    /// Adaptive sets: layers and audition.
    Adaptive,
    /// Build: audio.lock, budgets, export.
    Build,
}

impl Section {
    /// Every section, in dock order.
    pub const ALL: [Section; 4] = [
        Section::Library,
        Section::Effects,
        Section::Adaptive,
        Section::Build,
    ];

    /// Its name in the dock.
    pub const fn label(self) -> &'static str {
        match self {
            Section::Library => "Library",
            Section::Effects => "Effects bench",
            Section::Adaptive => "Adaptive sets",
            Section::Build => "Build",
        }
    }

    /// Its key on the command line (`--section`).
    pub const fn key(self) -> &'static str {
        match self {
            Section::Library => "library",
            Section::Effects => "effects",
            Section::Adaptive => "adaptive",
            Section::Build => "build",
        }
    }

    /// Its icon.
    pub const fn icon(self) -> Icon {
        match self {
            Section::Library => icons::LIBRARY,
            Section::Effects => icons::EFFECTS,
            Section::Adaptive => icons::ADAPTIVE,
            Section::Build => icons::BUILD,
        }
    }
}

/// Something the user asked for that the app, not the chrome, carries out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// View › Theme.
    SetTheme(egui::ThemePreference),
    /// File › Quit.
    Quit,
}

/// The chrome's state.
#[derive(Debug, Clone, Default)]
pub struct Shell {
    /// The section on screen.
    pub section: Section,
    /// The theme choice (System until the user picks one).
    pub theme: egui::ThemePreference,
}

/// The navigation dock's width, in points.
const DOCK_WIDTH: f32 = 200.0;

impl Shell {
    /// Draws one frame of the chrome and returns what the user asked for.
    pub fn show(&mut self, ui: &mut Ui) -> Vec<Action> {
        let mut actions = Vec::new();
        egui::Panel::top("menu_bar").show(ui, |ui| self.menu(ui, &mut actions));
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Score64 Studio {}", env!("CARGO_PKG_VERSION"))).small(),
                );
            });
        });
        egui::Panel::left("dock")
            .resizable(true)
            .default_size(DOCK_WIDTH)
            .show(ui, |ui| self.dock(ui));
        egui::CentralPanel::default().show(ui, |ui| self.empty_view(ui));
        actions
    }

    fn menu(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if cfg!(not(target_arch = "wasm32")) && ui.button("Quit").clicked() {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("Theme", |ui| {
                    for (preference, label) in [
                        (egui::ThemePreference::System, "System"),
                        (egui::ThemePreference::Light, "Light"),
                        (egui::ThemePreference::Dark, "Dark"),
                    ] {
                        if ui
                            .add(Button::selectable(self.theme == preference, label))
                            .clicked()
                        {
                            self.theme = preference;
                            actions.push(Action::SetTheme(preference));
                        }
                    }
                });
            });
        });
    }

    fn dock(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        for section in Section::ALL {
            // The icon in its own font: Inter has glyphs of its own in the private-use range the icons use.
            let text = (section.icon().rich_text(), section.label());
            if ui
                .add(Button::selectable(self.section == section, text).frame_when_inactive(false))
                .clicked()
            {
                self.section = section;
            }
        }
    }

    fn empty_view(&self, ui: &mut Ui) {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(format!("{}: nothing here yet", self.section.label())).weak());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame of the shell on a headless context with the app's look, and returns the actions.
    fn frame(ctx: &egui::Context, shell: &mut Shell) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| actions = shell.show(ui));
        output.textures_delta.clear(); // no renderer here
        actions
    }

    #[test]
    fn the_shell_draws_in_both_themes() {
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            crate::theme::set_theme(&ctx, preference);
            let mut shell = Shell::default();
            assert_eq!(frame(&ctx, &mut shell), []);
            assert_eq!(frame(&ctx, &mut shell), [], "no actions without input");
        }
    }

    #[test]
    fn sections_follow_the_ux_document() {
        let labels: Vec<_> = Section::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(
            labels,
            ["Library", "Effects bench", "Adaptive sets", "Build"]
        );
        assert_eq!(Shell::default().section, Section::Library);
        assert_eq!(Shell::default().theme, egui::ThemePreference::System);
    }
}
