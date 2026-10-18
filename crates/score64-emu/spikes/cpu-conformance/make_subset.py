"""Write the committed SingleStepTests subset used by score64-emu's CPU conformance test (ADR-010).

    python3 make_subset.py /path/to/SingleStepTests/65x02/6502/v1 > ../../testdata/sst-6502-subset.txt

Takes the first 16 cases of every documented and stable undocumented opcode (JAM and the seven unstable opcodes
are left out) and writes one case per line:

    <initial> | <final> | <cycles>
    state = pc s a x y p addr:value addr:value ...   (hex)
"""

import json
import sys
from pathlib import Path

PER_OPCODE = 16
JAM = {0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2}
UNSTABLE = {0x8B, 0xAB, 0x93, 0x9F, 0x9C, 0x9E, 0x9B}


def state(s):
    regs = f"{s['pc']:04x} {s['s']:02x} {s['a']:02x} {s['x']:02x} {s['y']:02x} {s['p']:02x}"
    ram = " ".join(f"{a:04x}:{v:02x}" for a, v in s["ram"])
    return f"{regs} {ram}"


src = Path(sys.argv[1])
print("# SingleStepTests/65x02 6502/v1 (MIT), first 16 cases per opcode; see crates/score64-emu/spikes/cpu-conformance")
for op in range(256):
    if op in JAM or op in UNSTABLE:
        continue
    for case in json.loads((src / f"{op:02x}.json").read_text())[:PER_OPCODE]:
        print(f"{state(case['initial'])} | {state(case['final'])} | {len(case['cycles'])}")
