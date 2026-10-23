//! Repository maintenance tasks, run as `cargo xtask <command>`.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod listen;

#[derive(Parser)]
#[command(name = "cargo xtask", about = "Repository maintenance tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a listening set for a PR: export scores through the GoatTracker song export, pack them with the pinned
    /// gt2reloc (with and without sound-effect support), render 6581 and 8580 WAVs with the emulator helper and
    /// sidplayfp (with and without effects), and write LISTEN.md. Output goes to the git-ignored `scratch/<pr>-listen/`.
    Listen(listen::Args),
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Listen(args) => listen::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}
