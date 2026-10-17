//! The `score64` command-line tool. Every user-visible action is a command in the shared engine (ADR-005); the CLI,
//! the MCP server and the app are thin front-ends over it.

use clap::Parser;

/// Turn music and sound into game-ready Commodore 64 SID assets.
#[derive(Parser)]
#[command(name = "score64", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    eprintln!("score64: no commands yet (milestone M1, see docs/github/EPICS.md)");
}
