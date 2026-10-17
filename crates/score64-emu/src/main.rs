//! GPL-3.0-or-later helper process: a 6502 core plus SID emulation that runs packed C64 music players, renders them
//! to PCM and drives live audition. It is a separate binary so the rest of Score64 stays MIT (ADR-002); other
//! crates must never depend on it as a library.

fn main() {
    eprintln!("score64-emu: no emulation yet (epic E3 spike, see docs/github/EPICS.md)");
}
