# Score64 — UX foundations

**Status:** draft for M0. Same family conventions as Convert64: desktop-first, light and dark themes, Material Symbols (the unfilled Rounded cut, via `egui_material_icons`' `outline` feature), keyboard-first, commercial-editor polish.

## 1. Who it is for and what they do

Expert users in long sessions. The jobs:

| # | Job |
|---|---|
| J1 | Hear whether the SID version still sounds like the source |
| J2 | See where each part went and why (which voice, which arpeggio, what was dropped) |
| J3 | Fix a bar: override a decision and re-run only what depends on it |
| J4 | Compare two versions by ear, notes, settings, decisions and metrics |
| J5 | Approve a version for the game build |
| J6 | Check fit: size, raster time, both chips; push to real hardware |
| J7 | Feel an adaptive set: layer changes and simulated effects over the real packed set |
| J8 | Make an effect that sounds like a reference recording |

## 2. Design principles

1. **Review and decide, never edit notes** (ADR-003). Note edits round-trip through a tracker.
2. **Hear the difference, don't remember it.** All sources share one playhead; switching is instant.
3. **Every decision shows its provenance** (rules, LLM, manual) and confidence. LLM decisions stay "unreviewed" until a person accepts them.
4. **Recalculation is explicit.** An override marks downstream stages stale; nothing re-runs (or spends money) until asked.
5. **Hardware truth is always on screen**: budgets, chip warnings and validator results next to the thing they judge.
6. **Every view is linkable** (`score64://asset/<id>/history?a=v7&b=v9&bar=33`).

## 3. Information architecture

```
Library ─ Asset ─┬─ Listen     (source / stems / MIDI / SID 6581 / SID 8580 / earlier versions)
                 ├─ Map        (source parts → SID voices per bar; decisions; overrides)
                 ├─ Pipeline   (stage graph, cache state, cost, logs)
                 ├─ History    (versions, diffs, approvals, comments)
                 └─ Budget     (size, raster, chips, licence and provenance)
Effects bench    (reference ↔ matched effect, bytes, variants, priority order)
Adaptive sets    (soundscape, layers, audition)
Build            (audio.lock, all budgets, push to C64 Ultimate, export)
```

Command palette: ⌘K / Ctrl+K for any asset, version, stage or source.

## 4. Key views

- **Listen:** number keys 1–9 switch the audible source without stopping playback (~10 ms crossfade); L loops the selection; [ and ] move by bar. Piano roll draws SID voices in voice colours over ghosted source notes, with dropped notes outlined; a tracker view shows the same data as pattern rows; a spectrogram helps with ambience and effects.
- **Map:** source-part lanes above, SID-voice lanes below, per bar. Selecting a bar lists its decisions with provenance, confidence and an override action.
- **Pipeline:** node graph of stages, coloured by state (cached, fresh, stale, running, failed); paid stages show cost and ask before forced refresh.
- **History:** version list with status, metrics and pins; compare any two by audio, notes, config, decisions and metrics; approve, restore as new version, comment at a bar.
- **Effects bench:** reference vs matched waveform and per-frame loudness and brightness curves, parameters, annotated bytes with format checks, pitch variants played as a sequence, audition over ambience or music. The priority list is the memory layout: drag to reorder.
- **Adaptive sets:** layer buttons that switch on the next timing mark, effect triggers, a voice scope and a timeline; record a scenario for a reference render.

## 5. Words

| Use | Not |
|---|---|
| asset | file, item |
| version | revision, take |
| decision | choice, setting |
| override | edit |
| voice (V1–V3) | channel, track |
| role (lead, bass, arp, drums, drone, ostinato, bed, texture) | part (except for source parts) |
| profile | mode |
| approve / approved | publish, lock |
| sound effect / effect | SFX in UI copy (fine in code) |

## 6. Visual language

- Voice colours come from the C64 palette (V1 cyan, V2 yellow, V3 light red as the starting point), tuned per theme for ≥ 3:1 contrast against the background. Voices are never identified by colour alone: labels and line patterns too.
- Monospace for hex, tracker rows and byte views. Theme tokens follow the shared family tokens used by Convert64.

## 7. States and feedback

Empty, loading, running (with stage progress), stale, failed (with log tail and retry), over budget, chip divergence, unreviewed LLM decisions. Every long operation is cancellable.

## 8. Accessibility

Keyboard reachability for every action; visible focus; AccessKit integration through eframe; reduced motion respected in the playhead and transitions; colour never the only cue.

## 9. What we deliberately do not build

A note or pattern editor; a mixer; plug-in hosting; cloud sync or accounts.
