# Initial plan (2026-10-18)

1. **M0 Scaffold.** Land the workspace, CI and community files (done in the initial commit), then the score model and validator (E2), fixtures and the spike (E3). The spike decides the emulator stack (`resid-rs` vs FFI reSIDfp, 6502 core) and confirms ONNX transcription before M1 builds on them.
2. **M1 Pipeline MVP.** CLI end to end on one fixture track: transcribe → grid → reduce → `.sng` → pack → render → score → HTML report. The HTML report is the first review surface and remains the share format later.
3. **M2 Studio & sound effects.** Score64 Studio over the same command engine, LLM decisions with evaluation, the sound-effect bench, MCP server.
4. **M3 Adaptive & integrations.** Mission sets with live audition, the VICE hardware harness, C64 Ultimate and DAWproject, then release 0.1.

Background and earlier design notes (pipeline research, UI and stack decisions) informed ADR-001 to ADR-009; the ADRs are now the source of truth.
