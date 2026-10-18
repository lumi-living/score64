"""Wrap sfx-psid.asm's binary (loaded at $0E00) in a PSID v2 header: init $0E00, play $0E20, PAL, 6581 (#18).

    python3 make_sfx_psid.py wrapper.bin out.sid "<name>" <subtunes>
"""

import struct
import sys

LOAD, INIT, PLAY = 0x0E00, 0x0E00, 0x0E20


def main():
    data, out, name, songs = open(sys.argv[1], "rb").read(), sys.argv[2], sys.argv[3], int(sys.argv[4])
    header = struct.pack(">4sHHHHHHHI", b"PSID", 2, 0x7C, 0, INIT, PLAY, songs, 1, 0)
    for text in (name, "Lumi Living", "2026 Lumi Living, MIT"):
        header += text.encode("ascii")[:32].ljust(32, b"\0")
    header += struct.pack(">HBBBB", 0x0014, 0, 0, 0, 0)  # flags: PAL, 6581; no relocation or extra SIDs
    assert len(header) == 0x7C
    with open(out, "wb") as f:
        f.write(header + struct.pack("<H", LOAD) + data)


if __name__ == "__main__":
    main()
