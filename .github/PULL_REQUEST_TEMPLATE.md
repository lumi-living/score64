## What

<!-- one or two sentences -->

## Why / issue

Closes #

## Checks

- [ ] Output still valid for the player (validator passes; packed file played in VICE or sidplayfp if a format or the packer changed)
- [ ] `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace` pass
- [ ] No crate other than the `score64-emu` binary depends on GPL code (ADR-002)
- [ ] Golden or snapshot files updated with a reason if output changed
- [ ] ADR added/updated if architecture, format or scope changed
