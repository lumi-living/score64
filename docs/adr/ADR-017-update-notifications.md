# ADR-017: Update notifications now, one-click updates with signed releases, shared with Convert64

**Status:** Proposed
**Date:** 2026-10-25
**Deciders:** @jahaa

## Context

Score64 has no way to tell people a new version exists. Releases are plain archives built by cargo-dist (#150): one each for `score64-cli`, `score64-app` and `score64-emu`, unsigned, with no installers. Someone who downloaded 0.1 has no reason to learn that 0.2 fixed the bug they hit.

Four things shape the answer:
- **`SECURITY.md` promises** that Score64 connects only to services the user configures. An update check is a new network call, so it has to be something the user turns on.
- **Three programs must move together.** The app and CLI talk to `score64-emu` over a versioned protocol (#128), so a newer app with an older helper fails at the handshake.
- **Unsigned macOS apps can't safely replace themselves.** Gatekeeper quarantines downloaded binaries, and signing and notarisation are E17 (#109).
- **Convert64 needs the same thing.** It has no update mechanism either, and the two tools are meant to behave as one family.

## Decision

1. **Two phases**, specified in the shared design note `docs/family/updates.md` (kept identical in both repositories):
   - **Phase 1, now: notify.**
     - Check GitHub Releases for a newer version.
     - Show a non-modal notice with the release notes and a download link, never during a running job.
     - Add Help › Check for updates….
     - Add `score64 version --check` (with `--json`; always exit 0).
   - **Phase 2, with E17's signed releases: "Update now".** It uses cargo-dist installers and `axoupdater`, verifies every download, and keeps the previous version for rollback.
2. **Automatic checks are opt-in.** On first launch a card asks once ("Check automatically" / "Not now"), and Settings › Updates can change it.
   - An explicit check (the Help menu, `version --check`, the MCP tool) always runs and counts as consent for that one check.
   - The CLI and the MCP server never check on their own.
   - `SCORE64_NO_UPDATE_CHECK=1` turns off every check.
   - A build without the `update-check` feature reports itself as managed by a package manager.
3. **One request, no user data:** at most once a day, `GET api.github.com/repos/lumi-living/score64/releases`, conditional on the last `ETag`, with `User-Agent: Score64/{version}` and nothing else. Offline or failed checks are silent.
4. **Channels:** Stable by default, Pre-release as an option. Releases are tagged `v{semver}`.
5. **Score64 updates as a set.** A release always contains all three programs at one version. The notice lists the three archives, and phase 2 installs all three or none.
6. **An engine command, per ADR-005 and ADR-012:** `check_update`, `Effect::ReadOnly`. This ADR adds a `network` flag to command registry entries; the MCP server maps it to the tool's `openWorldHint`.
7. **Shared code.** The logic lives in `family64-update` and the egui notice in `family64-update-egui`, in a new MIT repository under lumi-living. Score64 and Convert64 both depend on it by tag.
8. **`SECURITY.md`** gains: "…and, if you turn it on, a once-a-day check of GitHub Releases for new versions."

## Options considered

### Option A: notify now, install later, shared crate (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | Low for phase 1: one HTTPS request, a small cache file, a notice |
| Performance | Negligible: one conditional request a day, off the UI thread |
| Maintenance | One implementation for two apps; phase 2 reuses cargo-dist's own updater |
| Fit | Works with today's unsigned archives; keeps the `SECURITY.md` promise through opt-in |

### Option B: full self-update now with `axoupdater`
| Dimension | Assessment |
|---|---|
| Complexity | Needs cargo-dist installers, since the updater relies on their install receipts |
| Fit | Replaces unsigned binaries on macOS, where Gatekeeper interferes; premature before E17 |

### Option C: rely on package managers only (Homebrew, winget)
| Dimension | Assessment |
|---|---|
| Complexity | Low in the app |
| Fit | People who downloaded an archive are never told; Score64 isn't in any package manager yet |

### Option D: each app builds its own
| Dimension | Assessment |
|---|---|
| Fit | Two implementations that drift apart in behaviour and look, against the family's purpose |

## Trade-off analysis

Option A delivers the important part, knowing a new version exists, now and safely. It defers the risky part, replacing binaries, until signing makes it safe. Opt-in costs some reach: people who answer "Not now" won't hear about updates unless they check by hand. It is the only way to keep `SECURITY.md`'s promise, and the first-launch card makes the choice visible rather than buried in settings. A shared crate costs a third repository, which is cheaper than keeping two implementations in step by hand.

## Consequences
- **Easier:**
  - people learn about fixes;
  - release notes reach the app;
  - the MCP server can answer "is this the latest version?";
  - both family apps behave the same.
- **Harder:**
  - a third repository and its releases;
  - release notes must follow the shared format (summary, What's new, Fixes, Compatibility);
  - the registry gains a `network` flag that future network commands must set.
- **Revisit when:**
  - E17 lands signed releases (phase 2);
  - GitHub's API or rate limits change;
  - Score64 is published through a package manager that handles updates itself.

## Action items
- [ ] Create `lumi-living/family64` with `family64-update` and `family64-update-egui` (phase 1), tests per the design note §8.
- [ ] Engine: `check_update` command; `network` flag on registry entries; MCP `openWorldHint`.
- [ ] CLI: `score64 version --check [--json]`.
- [ ] App: first-launch card, Settings › Updates, status-bar item, banner (idle only), "What's new" window, Help › Check for updates….
- [ ] `SECURITY.md` and TAD: the update check.
- [ ] Release workflow: the release-notes template; confirm each release carries all three archives with `.sha256`.
- [ ] Phase 2 (E17): cargo-dist installers, "Update now" through `axoupdater`, attestation checks, set-wise install, rollback.
