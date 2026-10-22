//! Command-line options: `--screenshot <file.png> [--theme system|light|dark] [--section library|effects|adaptive|build]`.
//!
//! `--theme` and `--section` only make sense with `--screenshot`, which draws from defaults and saves nothing; a normal
//! run takes no options, so it never overwrites the theme the user chose. macOS may pass `-psn_…` when launching an
//! app bundle; it is ignored.

use std::path::PathBuf;

use crate::shell::Section;
use crate::theme;

/// What the command line asked for.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// `--screenshot`: save the window to this PNG after a few frames, then quit.
    pub screenshot: Option<PathBuf>,
    /// `--theme`, for a screenshot.
    pub theme: Option<egui::ThemePreference>,
    /// `--section`, for a screenshot.
    pub section: Option<Section>,
}

impl Args {
    /// Parses the arguments after the program name.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Args::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg.starts_with("-psn_") {
                continue;
            }
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--screenshot" => out.screenshot = Some(value()?.into()),
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
        if out.screenshot.is_none() && (out.theme.is_some() || out.section.is_some()) {
            return Err("--theme and --section go with --screenshot".into());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn options() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        let a = parse(&[
            "--screenshot",
            "out.png",
            "--theme",
            "dark",
            "--section",
            "build",
        ])
        .unwrap();
        assert_eq!(
            a.screenshot.as_deref(),
            Some(std::path::Path::new("out.png"))
        );
        assert_eq!(a.theme, Some(egui::ThemePreference::Dark));
        assert_eq!(a.section, Some(Section::Build));
        // A macOS bundle launch argument is ignored.
        assert_eq!(parse(&["-psn_0_12345"]).unwrap(), Args::default());
    }

    #[test]
    fn mistakes_are_errors() {
        assert!(
            parse(&["--theme", "dark"])
                .unwrap_err()
                .contains("go with --screenshot")
        );
        assert!(parse(&["--section", "library"]).is_err());
        assert!(
            parse(&["--screenshot"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            parse(&["--screenshot", "x.png", "--theme", "sepia"])
                .unwrap_err()
                .contains("unknown theme")
        );
        assert!(
            parse(&["--screenshot", "x.png", "--section", "mixer"])
                .unwrap_err()
                .contains("unknown section")
        );
        assert!(
            parse(&["--verbose"])
                .unwrap_err()
                .contains("unknown option")
        );
    }
}
