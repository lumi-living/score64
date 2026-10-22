//! Score64 Studio, native: `cargo run -p score64-app` (`docs/UX.md`, ADR-001). For now an empty window with the family
//! look (#6); the views arrive with E10.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window on Windows in release

use std::path::PathBuf;

use score64_app::shell::{Action, Section, Shell};
use score64_app::theme;

/// The storage key of the View › Theme choice (eframe's app storage).
const THEME_KEY: &str = "theme";

/// Top-level UI state.
#[derive(Debug, Default)]
struct App {
    /// The chrome.
    shell: Shell,
    /// `--screenshot`: save the window to a PNG after a few frames, then quit.
    screenshot: Option<Screenshot>,
}

impl App {
    /// The app with the family look installed and the theme restored from `cc`'s storage (or set by `--theme`).
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self {
            screenshot: args.screenshot,
            ..Self::default()
        };
        if let Some(section) = args.section {
            app.shell.section = section;
        }
        let stored = cc
            .storage
            .and_then(|s| s.get_string(THEME_KEY))
            .and_then(|s| theme::preference_from_str(&s));
        if let Some(preference) = args.theme.or(stored) {
            app.shell.theme = preference;
            theme::set_theme(&cc.egui_ctx, preference);
        }
        app
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for action in self.shell.show(ui) {
            match action {
                Action::SetTheme(preference) => theme::set_theme(ui.ctx(), preference),
                Action::Quit => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
        if let Some(shot) = &mut self.screenshot {
            shot.step(ui.ctx());
        }
    }

    fn persist_egui_memory(&self) -> bool {
        // A screenshot run leaves the user's window and panel sizes alone.
        self.screenshot.is_none()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_none() {
            storage.set_string(
                THEME_KEY,
                theme::preference_to_str(self.shell.theme).to_owned(),
            );
        }
    }
}

/// Frames to draw before taking a screenshot, so fonts and layout have settled.
const SETTLE_FRAMES: u32 = 5;

/// A pending `--screenshot`.
#[derive(Debug)]
struct Screenshot {
    path: PathBuf,
    frame: u32,
}

impl Screenshot {
    /// Asks for the screenshot once the frames have settled, writes it when it arrives and closes the window.
    fn step(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        if self.frame == SETTLE_FRAMES {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let written = image::save_buffer(
                &self.path,
                &rgba,
                w as u32,
                h as u32,
                image::ColorType::Rgba8,
            );
            match written {
                Ok(()) => println!("wrote {} ({w}×{h})", self.path.display()),
                Err(e) => eprintln!("score64-app: {}: {e}", self.path.display()),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Command-line options: `--screenshot <file.png> [--theme system|light|dark] [--section library|effects|adaptive|build]`.
#[derive(Debug, Default)]
struct Args {
    screenshot: Option<Screenshot>,
    theme: Option<egui::ThemePreference>,
    section: Option<Section>,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut out = Args::default();
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--screenshot" => {
                    out.screenshot = Some(Screenshot {
                        path: value()?.into(),
                        frame: 0,
                    })
                }
                "--theme" => {
                    let v = value()?;
                    out.theme =
                        Some(theme::preference_from_str(&v).ok_or(format!("unknown theme {v:?}"))?);
                }
                "--section" => {
                    let v = value()?;
                    out.section = Some(
                        Section::ALL
                            .into_iter()
                            .find(|s| s.key() == v)
                            .ok_or(format!("unknown section {v:?}"))?,
                    );
                }
                other => return Err(format!("unknown option {other:?}")),
            }
        }
        Ok(out)
    }
}

fn main() -> eframe::Result {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("score64-app: {e}");
            std::process::exit(2);
        }
    };
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Score64 Studio")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        // A screenshot run keeps the window size it asked for.
        persist_window: args.screenshot.is_none(),
        ..Default::default()
    };
    eframe::run_native(
        "Score64 Studio",
        native_options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, args)))),
    )
}
