# cpu-conformance

Measurement tool behind [ADR-010](../../../../docs/adr/ADR-010-6502-core.md). It runs every candidate 6502 core over the
[SingleStepTests/65x02](https://github.com/SingleStepTests/65x02) NMOS 6502 vectors (`6502/v1`, MIT, 10,000 cases per
opcode, 256 opcodes) and reports, per opcode and core:

- **state**: registers (B and bit 5 of P ignored) and every listed RAM byte match after one instruction,
- **cyc**: the instruction took the expected number of cycles,
- **bus**: the exact read/write sequence matches, dummy accesses included,
- **p**: the core panicked.

It is not part of the workspace or CI. It lives under `score64-emu` because it links zinc64 (GPL-3.0-or-later).

```sh
# about 1.1 GB of JSON
mkdir -p /tmp/sst && cd /tmp/sst
for i in $(seq 0 255); do printf "%02x\n" $i; done \
  | xargs -P16 -I{} curl -sfLO https://raw.githubusercontent.com/SingleStepTests/65x02/main/6502/v1/{}.json
cd -
cargo run --release -- /tmp/sst > results.tsv 2>/dev/null   # about 4 minutes on an M-series Mac
python3 summarise.py results.tsv
```

`results-2026-10-18.tsv` is the run quoted in the ADR.
