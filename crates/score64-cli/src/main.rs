//! The `score64` command-line tool. Every user-visible action is a command in the shared engine (ADR-005, ADR-012);
//! the CLI, the MCP server and the app are thin front-ends over it. Typed subcommands give the common commands
//! friendly flags; `score64 run` runs any registry command by name with JSON arguments.
//!
//! Exit codes: 0 when the command succeeded (for `validate`: no errors, though there may be warnings), 1 when
//! `validate` found errors, 2 when a command failed (bad arguments, an unreadable file, a helper failure).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use score64_engine::{
    CommandError, Engine, EngineConfig, NoArgs, REGISTRY, Validate, ValidateArgs, ValidateOutput,
    Version,
};
use serde_json::{Value, json};

/// Turn music and sound into game-ready Commodore 64 SID assets.
#[derive(Parser)]
#[command(name = "score64", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the Score64 version and the helper protocol it speaks.
    Version {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List every command, with argument and output schemas in JSON mode.
    Commands {
        /// Print JSON (names, summaries and schemas) instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Check a song against the SID, player and profile rules. Fails (exit 1) only on errors; warnings are printed.
    Validate {
        /// The song file (`.json` or `.toml`).
        song: PathBuf,
        /// A validation context file (`.json` or `.toml`): project profiles, adaptive sets, instrument facts.
        #[arg(long)]
        context: Option<PathBuf>,
        /// Print JSON (the problems, with the counts) instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Run any command by name with JSON arguments; prints its JSON output.
    Run {
        /// Command name (`score64 commands` lists them).
        name: String,
        /// Arguments as a JSON object.
        #[arg(default_value = "{}")]
        args: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut engine = Engine::new(EngineConfig::default());
    match cli.command {
        Cmd::Version { json } => match engine.run::<Version>(NoArgs {}) {
            Ok(v) if json => print_json(&serde_json::to_value(v).expect("serialisable")),
            Ok(v) => {
                println!(
                    "score64 {} (helper protocol {})",
                    v.score64, v.helper_protocol
                );
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        Cmd::Commands { json } if json => print_json(&Value::Array(
            REGISTRY
                .iter()
                .map(|e| {
                    json!({
                        "name": e.name,
                        "summary": e.summary,
                        "effect": e.effect,
                        "args": e.args_schema(),
                        "output": e.output_schema(),
                    })
                })
                .collect(),
        )),
        Cmd::Commands { .. } => {
            let width = REGISTRY.iter().map(|e| e.name.len()).max().unwrap_or(0);
            for e in REGISTRY {
                println!("{:width$}  {}", e.name, e.summary);
            }
            ExitCode::SUCCESS
        }
        Cmd::Validate {
            song,
            context,
            json,
        } => match engine.run::<Validate>(ValidateArgs { song, context }) {
            Ok(out) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&out).expect("serialisable")
                    );
                } else {
                    print_problems(&out);
                }
                if out.errors > 0 {
                    ExitCode::from(1)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(e) => fail(&e),
        },
        Cmd::Run { name, args } => {
            let args = match serde_json::from_str::<Value>(&args) {
                Ok(v) => v,
                Err(e) => {
                    return fail(&CommandError::new(
                        score64_engine::ErrorKind::InvalidArgs,
                        format!("arguments are not JSON: {e}"),
                    ));
                }
            };
            match engine.run_json(&name, args) {
                Ok(out) => print_json(&out),
                Err(e) => fail(&e),
            }
        }
    }
}

fn print_json(v: &Value) -> ExitCode {
    println!("{}", serde_json::to_string_pretty(v).expect("serialisable"));
    ExitCode::SUCCESS
}

/// Errors go to stderr as JSON (`{"error": {"kind", "message"}}`), so scripts can match on the kind; exit code 2.
fn fail(e: &CommandError) -> ExitCode {
    eprintln!("{}", json!({ "error": e }));
    ExitCode::from(2)
}

/// One line per problem, then a summary.
fn print_problems(out: &ValidateOutput) {
    for p in &out.problems {
        println!("{p}");
    }
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    println!(
        "{}, {}",
        plural(out.errors, "error"),
        plural(out.warnings, "warning")
    );
}
