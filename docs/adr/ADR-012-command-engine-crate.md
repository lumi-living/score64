# ADR-012: The command engine is its own crate and owns the emulator helper

**Status:** Proposed
**Date:** 2026-10-22
**Deciders:** @jahaa

## Context

ADR-005 puts every user-visible action behind one command engine, with the app, the CLI and the MCP server as thin front-ends. It does not say where the engine lives, what a command looks like, or who owns the long-lived resources commands need.

The first such resource is the emulator helper (#8). It is a separate GPL process (ADR-002) with state: a loaded player, a SID model, a play timeline. Live audition keeps one running and renders short chunks of the same timeline. A render job in the CLI starts one per run. If each front-end managed its own helper, each would also need its own start-up, restart and timeout handling, which is the drift ADR-005 exists to prevent.

The helper can also fail in ways its own cycle budgets do not cover. A bug outside the CPU loop, or a broken pipe, could leave a client blocked on a read forever. In the app that freezes the UI; in the MCP server it hangs a tool call.

Convert64 (ADR-005 there) keeps its registry in `convert64-core`, which works because its commands are pure edits of an in-memory document. Score64's commands start processes, read files and run threads, which `score64-core` must never do (CLAUDE.md rule 2).

## Decision

1. **A new crate, `score64-engine` (MIT),** holds the command trait, the registry and the `Engine` that commands run against. It depends on `score64-core`, `-formats`, `-project`, `-analysis` and `-adapters`. The CLI, the MCP server and the app depend on it and on nothing below it for anything user-visible.
2. **A command is a type implementing `Command`:** a stable snake_case `NAME`, a one-line `SUMMARY`, `Args` (serde-deserialisable, with a JSON Schema from `schemars`), `Output` (serde-serialisable, with a schema) and `run(&mut Engine, Args)`. `REGISTRY` lists every command as a name, a summary, both schemas and a JSON-in, JSON-out runner. Front-ends either call commands by type (the app, the CLI's typed subcommands) or by name with JSON (`score64 run`, the MCP server). Errors are data too: a `CommandError` with a stable kind and a message.
3. **The engine owns the helper.** Commands never see the raw client. The engine:
   - starts the helper lazily, on the first command that needs it;
   - runs it on a worker thread and waits for each job with a timeout. On timeout it kills the process, reports `HelperTimeout`, and starts a new helper on the next use;
   - after a `Malformed` reply, a broken pipe, a protocol error or a crash, drops the helper and starts a fresh one on the next use. A helper that answers with any other error keeps running;
   - counts starts, so a command that holds helper state (an audition session) can tell when it must load and init again.
4. **One helper per engine.** The app holds one engine for its lifetime. The CLI and the MCP server hold one engine per process.

## Options considered

### Option A: registry in `score64-core`
| Dimension | Assessment |
|---|---|
| Complexity | Lowest, as in Convert64 |
| Fit | Breaks rule 2: commands that spawn the helper or read audio cannot live in a pure crate. Splitting each command into a pure part in core and an effectful part elsewhere doubles the surface |

### Option B: a new `score64-engine` crate (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | One more crate; front-ends depend on it only |
| Performance | Nothing extra: commands are plain function calls in process |
| Maintenance | One place for command schemas, error kinds and helper lifecycle |
| Fit | Keeps core pure. The licence boundary is unchanged: the engine is MIT and reaches the helper only through `score64-adapters::emu` over a pipe |

### Option C: commands in `score64-project`
| Dimension | Assessment |
|---|---|
| Fit | Mixes the file model (track files, cache, index) with process management and adapters; the project crate would depend on adapters, which inverts the layering |

## Trade-off analysis

B costs one crate and gains a clean layering: core (pure) → formats, project, analysis → adapters (I/O, processes) → engine (commands, lifecycles) → front-ends. The watchdog adds a thread per helper and one channel hop per call, which is negligible next to rendering.

## Consequences
- Easier: parity between front-ends is structural; helper failures are handled once and tested once with scripted fakes; the MCP server can generate tool schemas from the registry.
- Harder: commands that keep helper state must check the start count and rebuild their state after a restart; command names and argument shapes are an interface and need care once released.
- Revisit when: a long-running job model (batch renders, progress, cancellation) is needed, or a second long-lived process joins the helper.

## Action items
- [ ] #7: the crate, `Command`, `REGISTRY`, `version` and `helper_info`, the helper lifecycle with tests, and the CLI's `version`, `commands` and `run`.
- [ ] E14: the MCP server lists `REGISTRY` as tools.
- [ ] M1 render and audition commands use the engine's helper.
