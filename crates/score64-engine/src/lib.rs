//! The command engine (ADR-005, ADR-012). Every user-visible action is a [`Command`]: a stable name, serialisable
//! arguments and output with JSON Schemas, and a `run` against the [`Engine`]. The app, the CLI and the MCP server are
//! thin front-ends: they call commands by type, or by name with JSON through [`REGISTRY`].
//!
//! The engine also owns the long-lived resources commands share, starting with the emulator helper: started lazily,
//! watched with a timeout, and restarted after it breaks (see [`HelperConfig`]).

mod commands;
mod helper;

use schemars::{JsonSchema, Schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use commands::{HelperInfo, HelperInfoOutput, NoArgs, Version, VersionOutput};
pub use helper::HelperConfig;

/// One user-visible action.
pub trait Command {
    /// Stable snake_case name, used by `score64 run` and as the MCP tool name.
    const NAME: &'static str;
    /// One line for listings and tool descriptions.
    const SUMMARY: &'static str;
    /// Arguments; unknown fields are an error.
    type Args: DeserializeOwned + JsonSchema;
    /// Result, as data a script or an agent can read.
    type Output: Serialize + JsonSchema;

    /// Run the command.
    fn run(engine: &mut Engine, args: Self::Args) -> Result<Self::Output, CommandError>;
}

/// What kind of failure a [`CommandError`] is. Stable: front-ends and agents may match on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// No command has this name.
    UnknownCommand,
    /// The arguments do not match the command's schema.
    InvalidArgs,
    /// The emulator helper binary was not found.
    HelperMissing,
    /// The emulator helper crashed, closed its pipe or sent something invalid. The next command starts a new one.
    HelperFailed,
    /// The emulator helper did not answer in time and was killed. The next command starts a new one.
    HelperTimeout,
    /// The emulator helper refused a request (bad sequence, CPU jam, limit, …). It keeps running.
    HelperRefused,
    /// A bug in Score64.
    Internal,
}

/// A command that failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    /// What kind of failure.
    pub kind: ErrorKind,
    /// Details for people.
    pub message: String,
}

impl CommandError {
    /// A new error.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CommandError {}

/// A command in the [`REGISTRY`], callable by name with JSON.
pub struct Entry {
    /// [`Command::NAME`].
    pub name: &'static str,
    /// [`Command::SUMMARY`].
    pub summary: &'static str,
    args_schema: fn() -> Schema,
    output_schema: fn() -> Schema,
    run: fn(&mut Engine, Value) -> Result<Value, CommandError>,
}

impl Entry {
    /// JSON Schema of the arguments.
    pub fn args_schema(&self) -> Schema {
        (self.args_schema)()
    }

    /// JSON Schema of the output.
    pub fn output_schema(&self) -> Schema {
        (self.output_schema)()
    }

    /// Run with JSON arguments and get JSON output.
    pub fn run(&self, engine: &mut Engine, args: Value) -> Result<Value, CommandError> {
        (self.run)(engine, args)
    }
}

const fn entry<C: Command>() -> Entry {
    Entry {
        name: C::NAME,
        summary: C::SUMMARY,
        args_schema: schema::<C::Args>,
        output_schema: schema::<C::Output>,
        run: run_json::<C>,
    }
}

fn schema<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

fn run_json<C: Command>(engine: &mut Engine, args: Value) -> Result<Value, CommandError> {
    let args: C::Args = serde_json::from_value(args)
        .map_err(|e| CommandError::new(ErrorKind::InvalidArgs, format!("{}: {e}", C::NAME)))?;
    let output = C::run(engine, args)?;
    serde_json::to_value(output).map_err(|e| CommandError::new(ErrorKind::Internal, e.to_string()))
}

/// Every command, in listing order.
pub static REGISTRY: &[Entry] = &[entry::<Version>(), entry::<HelperInfo>()];

/// The registry entry called `name`.
pub fn find(name: &str) -> Option<&'static Entry> {
    REGISTRY.iter().find(|e| e.name == name)
}

/// Engine settings.
#[derive(Clone, Debug, Default)]
pub struct EngineConfig {
    /// The emulator helper.
    pub helper: HelperConfig,
}

/// What commands run against. Front-ends hold one for as long as they run.
pub struct Engine {
    helper: helper::HelperSlot,
}

impl Engine {
    /// An engine; nothing starts until a command needs it.
    pub fn new(config: EngineConfig) -> Self {
        Self {
            helper: helper::HelperSlot::new(config.helper),
        }
    }

    /// Run a command by type.
    pub fn run<C: Command>(&mut self, args: C::Args) -> Result<C::Output, CommandError> {
        C::run(self, args)
    }

    /// Run a command by name with JSON arguments.
    pub fn run_json(&mut self, name: &str, args: Value) -> Result<Value, CommandError> {
        let entry = find(name).ok_or_else(|| {
            CommandError::new(
                ErrorKind::UnknownCommand,
                format!("no command called {name:?}; `score64 commands` lists them"),
            )
        })?;
        entry.run(self, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_are_unique_snake_case_and_schemas_are_objects() {
        let mut names: Vec<_> = REGISTRY.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), REGISTRY.len(), "duplicate command name");
        for e in REGISTRY {
            assert!(
                e.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{}",
                e.name
            );
            assert!(!e.summary.is_empty(), "{}", e.name);
            assert_eq!(
                e.args_schema().get("type"),
                Some(&Value::from("object")),
                "{}",
                e.name
            );
            assert_eq!(
                e.output_schema().get("type"),
                Some(&Value::from("object")),
                "{}",
                e.name
            );
        }
    }

    #[test]
    fn json_and_typed_calls_agree() {
        let mut engine = Engine::new(EngineConfig::default());
        let typed = engine.run::<Version>(NoArgs {}).unwrap();
        let json = engine.run_json("version", serde_json::json!({})).unwrap();
        assert_eq!(json, serde_json::to_value(&typed).unwrap());
        assert_eq!(typed.score64, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn bad_names_and_arguments_are_errors_with_kinds() {
        let mut engine = Engine::new(EngineConfig::default());
        let e = engine.run_json("nope", serde_json::json!({})).unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnknownCommand);
        let e = engine
            .run_json("version", serde_json::json!({ "colour": "blue" }))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidArgs);
        assert_eq!(
            serde_json::to_value(&e).unwrap()["kind"],
            Value::from("invalid_args")
        );
    }
}
