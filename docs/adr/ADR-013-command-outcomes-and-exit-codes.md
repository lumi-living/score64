# ADR-013: Commands decide failure; CLI exit codes; a provisional validation context file

**Status:** Proposed
**Date:** 2026-10-23
**Deciders:** @jahaa

## Context

ADR-012 defines a command as a name, a summary, an effect, JSON-Schema'd arguments and output, and a JSON-in, JSON-out runner. Errors are `CommandError`s. The `validate` command (#136) is the first command that can *succeed* with an output that should still fail a build: a song with validator errors. Its warnings must never fail one.

The first implementation put "exit 1 when there are errors" in the CLI's typed `validate` subcommand. `score64 run validate` then exited 0 on the same song, and the MCP server (E14) would have had to copy the rule a third time. That is the front-end drift ADR-005 exists to prevent. The validator also needs project context (profiles, adaptive sets, instrument facts) before the project files (E4) exist.

## Decision

1. **A command decides whether its output is a failure.** `Command::fails(&Output) -> bool` defaults to `false`, and `validate` returns `errors > 0`. Running by name returns `Outcome { output, failed }`. Every front-end reads that flag: the CLI's typed subcommand and `score64 run`, the MCP server's tool error flag, and the app's status. A front-end that calls a command by type calls `C::fails` itself.
2. **CLI exit codes:**
   - **0:** success, warnings included.
   - **1:** the command ran, but `fails` says its output is a failure.
   - **2:** the command failed (a `CommandError`, printed as JSON on stderr), or the command line is wrong (clap's usage message, plain text).

   This replaces "exit 1 on any failure", a breaking change made before any release.
3. **A provisional validation context file**, until E4's project files carry the same data:
   - JSON or TOML by extension (any case);
   - `schema = 1`, with newer versions refused;
   - `[[profile]]` and `[[adaptive]]` in the shapes of TAD §3.1, `[instruments.<preset>]` facts, and optional `ranges` and `expendable_long_rows`;
   - unknown keys are errors.

   E4 replaces it. Instrument facts move to the presets (E7), so there is one source of truth (ADR-004).

## Options considered

### Option A: the command decides (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | One default method and one small struct |
| Fit | One rule, read by every front-end; no drift |

### Option B: each front-end inspects the output
| Dimension | Assessment |
|---|---|
| Fit | Rejected: the CLI's by-name path already disagreed with its typed path, which is the drift ADR-005 forbids |

### Option C: make validation errors a `CommandError`
| Dimension | Assessment |
|---|---|
| Fit | Rejected: the problem list is the useful output, and an error would lose it or bury it in a message; warnings-only runs would also need a separate path |

## Consequences
- Easier: scripts and CI can rely on exit 1 meaning "the song is wrong", and 2 meaning "the tool or the input failed". The MCP server gets its error flag for free.
- Harder: every command author decides `fails` deliberately. The context file is a second, temporary place for project data until E4.
- Revisit when: E4's project files land (retire the context file), or a command needs more than pass/fail (for example a warnings-as-errors mode).

## Action items
- [x] #136 / PR #151: `Command::fails`, `Outcome`, exit codes 0/1/2, the context file, tests on both CLI paths.
- [ ] E14: the MCP server maps `Outcome::failed` to the tool result's error flag.
- [ ] E4: replace the context file with the project files; keep `schema` handling for old files or remove the format.
