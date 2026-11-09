//! For tests that need an outside tool (#240): the emulator helper or gt2reloc. Without the tool a test skips
//! visibly (`SKIPPED <test>: <why>` on stderr) and passes, which is right on a contributor's machine. With the tool
//! named in `SCORE64_REQUIRE` (comma-separated: `helper`, `gt2reloc`), a missing tool fails the test instead, so a CI
//! job that sets it can't pass by skipping. The sidecar's tests follow the same rule with `SCORE64_SIDECAR`.
//!
//! Public, and hidden from the docs, only so that the tests of this crate, the engine and the CLI can share it.

use std::io::Write as _;

/// The variable that turns a missing tool from a skip into a failure.
pub const VAR: &str = "SCORE64_REQUIRE";

/// A tool a test can need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// The `score64-emu` helper (`emu::Helper::locate`).
    Helper,
    /// gt2reloc (`gt2reloc::Gt2Reloc::locate`).
    Gt2reloc,
}

impl Tool {
    /// Its name in `SCORE64_REQUIRE`.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Helper => "helper",
            Tool::Gt2reloc => "gt2reloc",
        }
    }
}

/// The tools of `found` that weren't found: `(tool, whether it was found)`.
pub fn missing<const N: usize>(found: [(Tool, bool); N]) -> Vec<Tool> {
    found
        .into_iter()
        .filter(|&(_, here)| !here)
        .map(|(tool, _)| tool)
        .collect()
}

/// Skip `test`, which can't run because `missing` aren't here, for `why`: print `SKIPPED <test>: <why>`, or panic if
/// `SCORE64_REQUIRE` names one of them.
pub fn skip(test: &str, missing: &[Tool], why: &str) {
    match verdict(std::env::var(VAR).ok().as_deref(), test, missing, why) {
        // Straight to the process's stderr: libtest captures eprintln! of passing tests, and a skip must show.
        Ok(line) => {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
        Err(failure) => panic!("{failure}"),
    }
}

/// [`skip`] with `SCORE64_REQUIRE`'s value explicit (`None` when unset): the skip line, or why the test fails.
fn verdict(
    require: Option<&str>,
    test: &str,
    missing: &[Tool],
    why: &str,
) -> Result<String, String> {
    const KNOWN: [Tool; 2] = [Tool::Helper, Tool::Gt2reloc];
    let mut named = Vec::new();
    for name in require
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        // A misspelt name would otherwise turn the requirement off without a word.
        let Some(tool) = KNOWN.into_iter().find(|t| t.name() == name) else {
            let known: Vec<&str> = KNOWN.iter().map(|t| t.name()).collect();
            return Err(format!(
                "{test}: {VAR} names unknown tool `{name}` (known: {})",
                known.join(", ")
            ));
        };
        named.push(tool);
    }
    let required: Vec<&str> = missing
        .iter()
        .filter(|t| named.contains(t))
        .map(|t| t.name())
        .collect();
    if required.is_empty() {
        Ok(format!("SKIPPED {test}: {why}"))
    } else {
        Err(format!(
            "{test}: {VAR} names {}, which isn't here: {why}",
            required.join(" and ")
        ))
    }
}

#[cfg(test)]
mod tests;
