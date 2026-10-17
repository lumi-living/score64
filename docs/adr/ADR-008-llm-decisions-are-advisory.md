# ADR-008: LLM decisions are advisory, structured and overridable

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Some arrangement calls (which part is the melody, chord names, arpeggio choice, loop points) benefit from judgement. An LLM can help, but it is non-deterministic and can be confidently wrong.

## Decision

LLM help is optional and shaped as classification: one structured-output request per decision point, validated by the core against the invariants, recorded with provenance and confidence, cached by input hash for reproducibility, and overridable in `decisions.toml`. LLM decisions stay unreviewed until a person accepts them; approval of an asset is blocked while low-confidence unreviewed decisions remain. The LLM never writes song bytes. A golden set of decision cases guards prompt or model changes.

## Consequences
- Easier: better first drafts without losing control or reproducibility.
- Harder: a provider adapter and an evaluation set to maintain.
