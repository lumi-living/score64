//! `validate`: check a song against TAD §1.3 and report every problem with its severity (#136). A front-end fails only
//! when there are errors; warnings are reported, not failed on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use score64_core::validate::{Context, InstrumentFacts, Problem, RoleRanges, Severity};
use score64_core::{AdaptiveSet, PresetId, Profile, Profiles, Song};

use crate::{Command, CommandError, Effect, Engine, ErrorKind};

/// `validate`: every problem in a song, with its severity.
pub struct Validate;

/// Arguments of [`Validate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateArgs {
    /// The song file: JSON (`.json`) or TOML (`.toml`) in the score format (`docs/TAD.md` §3).
    pub song: PathBuf,
    /// A validation context file (JSON or TOML; see [`ContextFile`]): the project's own profiles, adaptive sets,
    /// instrument facts and role ranges. Without it, only the built-in profiles apply and no instrument has facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<PathBuf>,
}

/// Output of [`Validate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidateOutput {
    /// Every problem, in subtune, bar and voice order (the fields of `score64_core::validate::Problem`).
    pub problems: Vec<Problem>,
    /// How many are errors. A front-end fails when this is not zero.
    pub errors: usize,
    /// How many are warnings.
    pub warnings: usize,
}

/// What a project tells the validator besides the song, as a file. A stop-gap until the project files (E4) carry it:
/// the same shapes the project will use (TAD §3.1), so nothing here becomes a second format.
///
/// ```toml
/// expendable_long_rows = 4
///
/// [[profile]]
/// name = "calm"
/// v1 = { usage = "music", roles = ["bed"], filter = true }
/// v2 = { usage = "effects" }
/// v3 = { usage = "effects" }
///
/// [[adaptive]]
/// name = "score"
/// layers = ["calm", "tension", "peak"]
///
/// [instruments.lead]
/// filter = true
/// pulse_at_start = true
/// gate_timer = 2
/// hard_restart = true
/// gate_off = true
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextFile {
    /// Profiles the project defines, besides the built-in ones.
    #[serde(default, rename = "profile")]
    pub profiles: Vec<Profile>,
    /// Adaptive sets the project declares over the song.
    #[serde(default)]
    pub adaptive: Vec<AdaptiveSet>,
    /// Facts about every instrument preset the song uses.
    #[serde(default)]
    pub instruments: BTreeMap<PresetId, InstrumentFacts>,
    /// Role pitch ranges, if not the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranges: Option<RoleRanges>,
    /// The long-note warning threshold on expendable voices, in rows, if not the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expendable_long_rows: Option<u32>,
}

impl ContextFile {
    /// The validator's context: built-in profiles plus this file's.
    pub fn into_context(self) -> Result<Context, String> {
        let defaults = Context::default();
        Ok(Context {
            profiles: Profiles::with(self.profiles)?,
            adaptive: self.adaptive,
            instruments: self.instruments,
            ranges: self.ranges.unwrap_or(defaults.ranges),
            expendable_long_rows: self
                .expendable_long_rows
                .unwrap_or(defaults.expendable_long_rows),
        })
    }
}

/// Read a JSON or TOML file into `T`, by its extension.
fn read<T: DeserializeOwned>(path: &Path, what: &str) -> Result<T, CommandError> {
    let invalid = |e: String| {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!("{what} {}: {e}", path.display()),
        )
    };
    let text = std::fs::read_to_string(path).map_err(|e| invalid(e.to_string()))?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&text).map_err(|e| invalid(e.to_string())),
        Some("toml") => toml::from_str(&text).map_err(|e| invalid(e.to_string())),
        _ => Err(invalid("expected a .json or .toml file".into())),
    }
}

impl Command for Validate {
    const NAME: &'static str = "validate";
    const SUMMARY: &'static str = "Check a song against the SID, player and profile rules; report every problem with its severity.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Args = ValidateArgs;
    type Output = ValidateOutput;

    fn run(_: &mut Engine, args: ValidateArgs) -> Result<ValidateOutput, CommandError> {
        let song: Song = read(&args.song, "song")?;
        let context = match &args.context {
            Some(path) => read::<ContextFile>(path, "context")?
                .into_context()
                .map_err(|e| {
                    CommandError::new(
                        ErrorKind::InvalidInput,
                        format!("context {}: {e}", path.display()),
                    )
                })?,
            None => Context::default(),
        };
        let problems = score64_core::validate(&song, &context);
        let errors = problems.iter().filter(|p| p.is_error()).count();
        let warnings = problems
            .iter()
            .filter(|p| p.severity == Severity::Warning)
            .count();
        Ok(ValidateOutput {
            problems,
            errors,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
