//! The commands. Each is a unit type implementing [`Command`]; add new ones to [`crate::REGISTRY`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Command, CommandError, Effect, Engine};

/// Arguments of a command that takes none: `{}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// `version`: what this build is. Touches nothing.
pub struct Version;

/// Output of [`Version`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VersionOutput {
    /// Score64's version.
    pub score64: String,
    /// The emulator helper protocol version this build speaks (`docs/reference/emu-protocol.md`).
    pub helper_protocol: u16,
}

impl Command for Version {
    const NAME: &'static str = "version";
    const SUMMARY: &'static str = "Show the Score64 version and the helper protocol it speaks.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Args = NoArgs;
    type Output = VersionOutput;

    fn run(_: &mut Engine, _: NoArgs) -> Result<VersionOutput, CommandError> {
        Ok(VersionOutput {
            score64: env!("CARGO_PKG_VERSION").into(),
            helper_protocol: score64_formats::emu_protocol::VERSION,
        })
    }
}

/// `helper_info`: start the emulator helper if needed and report what it is.
pub struct HelperInfo;

/// Output of [`HelperInfo`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelperInfoOutput {
    /// The helper binary in use.
    pub binary: String,
    /// Helper name and version.
    pub helper: String,
    /// SID engine.
    pub engine: String,
    /// Protocol version.
    pub protocol: u16,
    /// Sample rate of the session.
    pub sample_rate: u32,
    /// How many helper processes this engine has started, counting restarts.
    pub starts: u32,
}

impl Command for HelperInfo {
    const NAME: &'static str = "helper_info";
    const SUMMARY: &'static str =
        "Start the emulator helper if it is not running, and show its version and engine.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Args = NoArgs;
    type Output = HelperInfoOutput;

    fn run(engine: &mut Engine, _: NoArgs) -> Result<HelperInfoOutput, CommandError> {
        let info = engine.helper.call(|h| Ok(h.info().clone()))?;
        Ok(HelperInfoOutput {
            binary: engine.helper.binary().unwrap_or_default(),
            helper: info.helper,
            engine: info.engine,
            protocol: info.version,
            sample_rate: info.sample_rate,
            starts: engine.helper.starts(),
        })
    }
}
