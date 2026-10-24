//! The `score64` command-line tool. Every user-visible action is a command in the shared engine (ADR-005, ADR-012);
//! the CLI, the MCP server and the app are thin front-ends over it. Typed subcommands give the common commands
//! friendly flags; `score64 run` runs any registry command by name with JSON arguments.
//!
//! Exit codes: 0 when the command succeeded (for `validate`: no errors, though there may be warnings); 1 when the
//! command ran but its output is a failure (`Command::fails`: `validate` found errors), by its typed subcommand and by
//! `score64 run` alike; 2 when a command failed (bad arguments, an unreadable file, a helper failure; JSON error on
//! stderr) or the command line itself is wrong (clap's usage message, plain text).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use score64_engine::{
    Command, CommandError, Engine, EngineConfig, GcArgs, GcCommand, IndexArgs, IndexCommand,
    NoArgs, REGISTRY, Validate, ValidateArgs, ValidateOutput, Version,
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
    /// Open the project's index, rebuilding it from the files when asked, missing or outdated; show what it holds.
    Index {
        /// The project folder (holding `score64.toml`).
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Rebuild from the files even if the index is current.
        #[arg(long)]
        rebuild: bool,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Clear the stage cache down to a size cap, keeping what recorded versions use. Removes broken entries and old
    /// leftovers too.
    Gc {
        /// The project folder (holding `score64.toml`).
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// The most bytes the cache may hold once unrooted entries are removed (default 4 GiB).
        #[arg(long)]
        cap_bytes: Option<u64>,
        /// Report what would go, and remove nothing.
        #[arg(long)]
        dry_run: bool,
        /// Print JSON instead of text.
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
                if Validate::fails(&out) {
                    ExitCode::from(1)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(e) => fail(&e),
        },
        Cmd::Index {
            project,
            rebuild,
            json,
        } => match engine.run::<IndexCommand>(IndexArgs { project, rebuild }) {
            Ok(out) if json => print_json(&serde_json::to_value(out).expect("serialisable")),
            Ok(out) => {
                println!(
                    "{}: {} tracks, {} recorded runs, {} cache entries ({} artifacts, {} bytes)",
                    out.path, out.tracks, out.versions, out.entries, out.artifacts, out.bytes
                );
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        Cmd::Gc {
            project,
            cap_bytes,
            dry_run,
            json,
        } => match engine.run::<GcCommand>(GcArgs {
            project,
            cap_bytes,
            dry_run,
        }) {
            Ok(out) if json => print_json(&serde_json::to_value(out).expect("serialisable")),
            Ok(out) => {
                let verb = if out.dry_run {
                    "would remove"
                } else {
                    "removed"
                };
                for r in &out.removed {
                    println!(
                        "{verb} {}/{} ({}, {} bytes)",
                        r.stage, r.name, r.why, r.bytes
                    );
                }
                let gone: u64 = out.removed.iter().map(|r| r.bytes).sum();
                println!(
                    "{verb} {} ({gone} bytes); kept {} used by recorded versions ({} bytes) and {} others ({} bytes) under a {}-byte cap",
                    out.removed.len(),
                    out.rooted,
                    out.rooted_bytes,
                    out.unrooted_kept,
                    out.unrooted_kept_bytes,
                    out.cap_bytes
                );
                if out.roots_over_cap > 0 {
                    println!(
                        "the entries recorded versions use are {} bytes over the cap; they stay",
                        out.roots_over_cap
                    );
                }
                ExitCode::SUCCESS
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
                Ok(out) => {
                    print_json(&out.output);
                    // The command decides whether its output is a failure (`validate` with errors), on every path.
                    if out.failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
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
