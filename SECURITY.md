# Security policy

Score64 is a desktop/CLI tool. It reads audio, MIDI and project files you give it and writes Commodore 64 music and sound-effect files. It makes network connections only to services you configure: a transcription provider, an LLM provider for optional arrangement suggestions, and the optional C64 Ultimate push to a host you name. API keys stay on your machine.

## Supported versions

Only the latest release and the `main` branch receive fixes. There are no long-term support branches.

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

Use GitHub's private vulnerability reporting instead:

1. Go to <https://github.com/lumi-living/score64/security/advisories/new>.
2. Describe the problem, how to reproduce it, and which version or commit you tested.
3. Attach a minimal input file if the issue is triggered by a malformed audio, MIDI, project or C64 file.

You will get an acknowledgement within 7 days. Once a fix is ready we publish an advisory, credit you unless you prefer otherwise, and release a patched version.

## What counts

- Crashes, memory-safety bugs or unbounded allocation triggered by crafted input files (audio, MIDI, `.sng`, `.ins`, `.sid`, `.prg`, project files).
- Path traversal or arbitrary file writes through project files, decision files or CLI arguments.
- Leaking API keys into logs, caches, reports or project files.
- Command injection through file names passed to external tools (gt2reloc, VICE, the Python sidecar).
- Supply-chain problems in our build or release process.

Not security issues: output that sounds wrong or does not fit the budget (that is a bug, open a normal issue).
