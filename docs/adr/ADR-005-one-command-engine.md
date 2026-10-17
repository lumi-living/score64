# ADR-005: One command engine behind app, CLI and MCP

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Convert64 put every user-visible action behind one command registry so the app, CLI and MCP server stay in parity. Score64 has the same three front-ends.

## Decision

Every user-visible action is a command in a shared engine. The CLI, the MCP server and Score64 Studio are thin front-ends. Anything the app can do, `score64` and the MCP server can do headlessly.

## Consequences
- Easier: automation from Makefiles and AI agents; one place to test behaviour.
- Harder: commands need stable, serialisable inputs and outputs from the start.
