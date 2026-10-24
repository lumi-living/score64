# Family design: update notifications and updates

**Applies to:** Convert64 and Score64 (Lumi Living's "64" family). The same text is kept in both repos as `docs/family/updates.md`; change both together.
**Decided by:** Convert64 ADR-013 and Score64 ADR-017. This note is the shared specification those ADRs point to.
**Status:** Draft, 2026-10-25.

## 1. Goal

Tell people when a new version exists and make updating easy, in a way that behaves the same in both tools:
- **Nothing surprising:** no network call the user hasn't agreed to, and no data about the user or their projects.
- **Nothing blocking:** never a pop-up, never during an export or a running job, silent when offline.
- **Honest about how it's installed:** the right update path for an archive, an installer, Homebrew or a package manager.
- **One implementation:** a shared crate, so both apps show the same notice and both CLIs answer the same way.

## 2. Phases

| Phase | What the user gets | Depends on |
|---|---|---|
| **1. Notify** | A notice with release notes and a download link; a manual "Check for updates"; a CLI check | Nothing new: today's plain release archives |
| **2. Update now** | One click downloads, verifies and installs the new version, then offers a restart | Signed and notarised releases, and cargo-dist installers (Score64 E17 #109; Convert64's release epic) |

Phase 1 ships first and stays: phase 2 adds a button, it doesn't replace the notice.

## 3. The shared crates

A new repository, **`lumi-living/family64`** (MIT; name open), holding two crates:

| Crate | Contents | Used by |
|---|---|---|
| `family64-update` | Parsing releases, version comparison, channel rules, install detection, the cache file, the consent setting, all pure. The HTTPS transport sits behind a feature and is injected by the front-end. No UI and no async runtime | Both apps, both CLIs, both MCP servers (through each app's command engine) |
| `family64-update-egui` | The notice, the "What's new" window and the Settings › Updates panel, using the family theme | Both apps |

Both apps pin the same egui release already (Score64 #141), so one UI crate fits both. The family design tokens (Score64 #67) can live in the same repository later.

**The engine owns the update state; the front-end injects the transport** (the pattern PhotoCraft uses, §10). Each app's command engine holds the last result and answers `check_update` and `update_status` from it, so the CLI, the MCP server and the app see the same state. The engine itself has no network dependency: the desktop app and the CLI hand it a transport, and a front-end without one (a test, the web build) reports "update checks are unavailable here".

Each app depends on a tagged version (`git` dependency with `tag = "vX.Y.Z"`, or crates.io once published), so neither moves the other.

### 3.1 API sketch (`family64-update`)

```rust
pub struct App {
    pub name: &'static str,          // "Score64", "Convert64": for messages and the User-Agent
    pub owner: &'static str,         // "lumi-living"
    pub repo: &'static str,          // "score64", "convert64"
    pub current: semver::Version,    // env!("CARGO_PKG_VERSION")
    pub assets: &'static [&'static str], // archive name prefixes for this app, e.g. ["score64-cli", "score64-app", "score64-emu"]
}

pub enum Channel { Stable, PreRelease }

pub enum Install {
    Archive,                          // extracted release archive (today's only kind)
    DistInstaller { receipt: PathBuf }, // cargo-dist installer receipt found (phase 2)
    Homebrew,                         // running from a Homebrew prefix
    Managed,                          // built without the `update-check` feature, e.g. by a distribution
}

pub enum Outcome {
    UpToDate { checked_at: Timestamp },
    Available(Release),               // newer than `current`, not skipped
    Skipped(Release),                 // newer, but the user chose "Skip this version"
    Unavailable { reason: String },   // offline, rate-limited, bad response: never an error dialog
}

pub struct Release {
    pub version: semver::Version,
    pub tag: String,
    pub prerelease: bool,
    pub published_at: Timestamp,
    pub notes: String,                // the release body, untrusted text (§6)
    pub page: Url,                    // the release's GitHub page
    pub assets: Vec<Asset>,           // only this app's archives for this target
}

pub struct Asset { pub name: String, pub url: Url, pub size: u64, pub sha256: Option<[u8; 32]> }

pub fn detect_install() -> Install;

/// Pure: interpret GitHub's answer. Builds every URL itself from the validated tag (§6).
pub fn parse_releases(app: &App, channel: Channel, body: &[u8]) -> Result<Option<Release>, String>;

/// The network side, behind the `transport` feature, started by the front-end on a worker thread.
pub trait Transport: Send { fn fetch(&self, etag: Option<&str>) -> Fetched; }
pub fn check(app: &App, channel: Channel, state: &mut State, transport: &dyn Transport, force: bool) -> Outcome;
```

`State` is the small cache file of §4.3. Phase 2 adds `install(release, …)` behind a feature, wrapping `axoupdater`.

## 4. Behaviour

### 4.1 Consent
Both tools promise in `SECURITY.md` to connect only to services the user configures. Automatic checks therefore start **off** and are turned on by a choice:
- **First launch of an app with this feature:** a non-modal card in the start view says: *"Check for new versions automatically? {App} asks GitHub once a day. Nothing about you or your projects is sent."* with **Check automatically** and **Not now**. Either answer is remembered, and the card doesn't return.
- **Settings › Updates:** "Check automatically" (on or off), "Channel" (Stable or Pre-release), and "Last checked: …".
- **An explicit action is consent for that one check:** Help › Check for updates…, `score64 version --check`, `convert64 version --check` and the MCP tool all check immediately, whatever the setting.
- **CLIs and MCP servers never check on their own.** Scripts, CI and agents get no surprise network calls.
- **`{APP}_NO_UPDATE_CHECK=1`** (`SCORE64_…`, `CONVERT64_…`) turns off every automatic check, and the explicit ones report that checks are disabled.
- **Distributors** can build without the `update-check` cargo feature; the app then reports `Install::Managed` and points to the package manager.

### 4.2 When the app checks
- At startup, after the first frame, on a worker thread, and then every 24 hours while the app stays open. Never more than once per 24 hours across restarts, unless forced.
- Offline, timed out (5 s), rate-limited or a bad answer: `Unavailable`, logged quietly, retried at the next scheduled time. No dialog.
- Not on the web build (Convert64's wasm): a web page is always the latest version.

### 4.3 What it requests
One HTTPS request: `GET https://api.github.com/repos/{owner}/{repo}/releases?per_page=20`, with:
- `User-Agent: {App}-update-check` (GitHub's API requires one; it carries no version, so the request doesn't reveal which version someone runs);
- `Accept: application/vnd.github+json`;
- `If-None-Match` with the last `ETag`, so an unchanged answer costs almost nothing and doesn't count against GitHub's rate limit.

Nothing else is sent: no IDs, no version, no project names, no operating-system details. GitHub sees the IP address, as with any visit.

**Answers:**
- **404:** the repository has no matching release yet, which counts as up to date.
- **304:** unchanged since the last check; the cached answer stands.
- **403 or 429:** "GitHub is limiting update checks. Try again later." The next automatic check waits at least until the next day, or until `Retry-After` if it's later.
- **Any redirect:** refused.

**Choosing the release:**
- Ignore drafts.
- Accept only tags of the form `v{semver}`.
- **Stable** ignores releases marked pre-release and versions with a pre-release part (`-alpha.1`). **Pre-release** accepts both.
- Pick the highest version above `current`.
- Keep only this app's assets for this target (by the `assets` prefixes and the target triple in the archive name), and take each asset's SHA-256 from the API's `digest` field when present.

**The cache file** (`updates.json` in the app's config directory, beside eframe's storage):
`{ "schema": 1, "last_checked": "...", "etag": "...", "latest": { … }, "skipped": "0.3.0", "auto": true | false | null, "channel": "stable" }`.
`auto: null` means the user hasn't been asked yet.

### 4.4 What the user sees
- **Status bar:** a small "Update {version}" item while an update is available. Clicking it opens "What's new".
- **Banner, once per version:** *"{App} {new} is available (you have {current}). What's new · Download · Skip this version · ×"*. Dismissing it hides it until the next day; "Skip this version" hides it until a newer one appears.
  - Never while a job or export runs (Score64 ADR-012 / #132; Convert64's export): it waits until the app is idle.
- **"What's new" window:** the release notes, the downloads for this platform with their sizes, and the install hint for this install kind:

  | Install kind | Hint |
  |---|---|
  | `Archive` | **Download** opens the asset (or the release page when there are several archives to fetch) |
  | `Homebrew` | Shows `brew upgrade {formula or cask}`, with a copy button |
  | `DistInstaller` (phase 2) | **Update now** |
  | `Managed` | "Update through the package manager you installed {App} with." |

- **Help › Check for updates…:** checks now and answers inline: up to date, available, or "Couldn't reach GitHub. Try again later."

### 4.5 CLI and MCP
- `score64 version --check` / `convert64 version --check` print the current version and the result; `--json` gives the `Outcome` as JSON. Exit code 0 in every case, including "update available" and "unreachable", so scripts never fail because of the check.
- Each app's command engine gets a `check_update` command (read-only), which the CLI and MCP server call. Because it reaches the network, its registry entry is marked as such, and the MCP server sets the tool's `openWorldHint`.

## 5. Release-side rules (both repositories)
1. Tags are `v{semver}` (`v0.3.0`, `v0.4.0-alpha.1`), and pre-releases are marked as such on GitHub.
2. Release notes start with a two-line summary, then **What's new**, **Fixes** and **Compatibility**: whether project files from older versions open, and whether files saved by this version still open in older ones. The app shows the summary in the banner tooltip and the whole body in "What's new".
3. Every archive is published with its `.sha256`, as cargo-dist does today.
4. **Score64 only:** a release always contains all three programs (`score64-cli`, `score64-app`, `score64-emu`) at the same version, because the helper protocol checks versions at the handshake (#128).

## 6. Security
- HTTPS only, through `ureq` with rustls, **trusting the operating system's certificate store** (`rustls-platform-verifier`), so checks work behind company proxies that install their own root certificate, while chains and host names are still verified.
- Requests go only to `api.github.com`, with redirects refused. Downloads (phase 2) follow GitHub's redirect to its release-asset hosts only.
- **Never trust a URL from the response.** The release page is built from the validated tag (`https://github.com/{owner}/{repo}/releases/tag/{tag}`), and a tag must be short and plain (letters, digits, `.`, `-`, `+`). An asset URL is accepted only if it points at this repository's own release downloads.
- Responses are capped at 1 MB and parsed strictly.
- **Release notes are untrusted text.** They're rendered as plain Markdown without HTML or images. Links show their full URL, and open in the browser only when clicked.
- **Phase 1 never runs or unpacks anything it downloads**; "Download" hands the URL to the browser.
- **Phase 2 verifies before installing:**
  - the SHA-256 against both the release's `.sha256` file and the API's digest;
  - the GitHub artifact attestation once releases carry them (cargo-dist's `github-attestations`);
  - and it keeps the previous version until the new one has started once, for rollback.

## 7. Phase 2 outline (decided with the signing work)
Two routes, to be chosen with E17:
- **(a) cargo-dist's updater.** Turn on cargo-dist installers: Homebrew (formula for the CLIs, cask for the apps), shell and PowerShell scripts, later MSI. They write the install receipts `axoupdater` uses, and "Update now" wraps `axoupdater` (MIT/Apache-2.0) behind a feature.
- **(b) Swap in place by install kind, as the ArtCraft Launcher does (§10).** Download, verify, then replace the copy the way it was installed. A small helper process waits for the app to exit, finishes what can't happen while it runs (Windows locks a running `.exe`), and restarts it:

  | Installed as | Swap |
  |---|---|
  | macOS `.app` | Copy the new bundle from the signed DMG over this one, reopen it |
  | Windows MSI | After exit, run the new MSI (it reinstalls into the same folder), restart |
  | Windows portable zip | After exit, unzip over the folder (settings stay) |
  | Linux AppImage | Rename the new AppImage over this one, restart |
  | Linux `.deb` / `.rpm` / Flatpak | Through the system package manager, with its password prompt |
  | Anything else (a development build, Homebrew) | The download link or the `brew upgrade` hint |

  Route (b) needs no cargo-dist installers and suits the DMG, MSI and AppImage packaging E17 will produce. The ArtCraft Launcher's code for it is MIT OR Apache-2.0, so it can be adapted with attribution.

Either way:
- Score64 updates its three programs as one set and refuses a partial update.
- Homebrew installs show the `brew upgrade` hint instead of replacing files themselves.
- macOS app bundles update through the signed bundle: the Homebrew cask, or a Sparkle-style updater, chosen with notarisation.

## 8. Testing
- Recorded GitHub API responses as fixtures: drafts, pre-releases, malformed tags, missing digests, a 304 Not Modified, a 403 rate limit, an oversized body.
- A local fake server for the HTTP path; CI never calls GitHub. Cover hostile answers too: a redirect to another host, an `html_url` pointing elsewhere (ignored), an oversized body, a timeout.
- `{APP}_UPDATE_PRETEND_VERSION=0.0.1` makes a build believe it's older for one run, to try the whole flow against the real latest release (the ArtCraft Launcher's trick).
- Property tests for version and channel selection.
- Round-trip tests for the cache file, including an older `schema`.
- An app test that the banner never appears while a job runs, and that the first-launch card appears once.

## 9. Open questions
0. **Opt-in or opt-out.** This note makes automatic checks opt-in, to keep `SECURITY.md`'s promise. PhotoCraft turns them on by default, with a setting to turn them off, which reaches more people. Opt-out would need `SECURITY.md` reworded to say so plainly.
1. The shared repository's and crates' names (`family64`, or a Lumi Living name).
2. Release notes in "What's new": render Markdown in the app (for example with `egui_commonmark`), or show the summary and link to the page.
3. Whether Convert64 should ship phase 1 before its Windows and Linux targets exist, or together with them.

## 10. Prior art: ArtCraft (PhotoCraft and the ArtCraft Launcher)

ArtCraft's Craft apps are a close match for this family: Rust, egui, a command registry, an MCP server, several apps sharing libraries. They're MIT OR Apache-2.0. Checked on 2026-10-25 at [storytold/photocraft](https://github.com/storytold/photocraft) `486afd8` and [storytold/craft-launcher](https://github.com/storytold/craft-launcher) `c0fa1c6`.

**Each app (PhotoCraft): notify only.**
- One check per launch, at startup, if "Automatically Check for Updates" is on (the default), plus Help › Check for Updates….
- It asks `releases/latest` (stable only) with a generic `User-Agent: PhotoCraft-update-check`, no redirects, a 1 MB cap, a 15 s timeout and the OS certificate store. A 404 means up to date, and a 403 or 429 gives a friendly "GitHub is limiting update checks".
- It builds the release-page link from the validated tag and never trusts `html_url`.
- **The engine owns the state and the desktop injects the transport**, so the CLI and MCP (`help.checkForUpdates`, `updates.status`) share it, and a front-end without a transport says checks are unavailable. A repeated request never starts a second worker.
- A small non-modal window opens once per new version: "PhotoCraft {version} is available. Open Download Page · Check Again". No in-app install.

**The suite (ArtCraft Launcher): installs and updates every app.**
- One small app (with a tray or menu-bar icon) lists the suite, shows installed and latest versions, installs the right build for the OS and CPU, and checks each download's SHA-256 (from the API's `digest`, falling back to the release's `SHA256SUMS.txt`). It never replaces a running app, and it finds apps installed by hand.
- It updates itself by swapping in place per install kind, through a helper process that waits for it to exit (the table in §7).
- A server of theirs polls GitHub for every app and serves one feed, so a million launchers don't hit GitHub's 60-requests-an-hour limit. GitHub stays the fallback. The client throttles itself hard: backoff, jitter, `Retry-After`, persisted across restarts. **The family doesn't need this** at one request per app per day; it's noted in case a family launcher ever exists.

**What this design took from it:** the injected transport, the generic User-Agent, never trusting URLs from the response, the OS certificate store, the answer handling, the pretend-version test, and phase 2 route (b).

