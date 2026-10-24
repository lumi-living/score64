# Emulator helper protocol, version 1

`score64-emu serve` speaks this protocol on stdin (requests) and stdout (replies); stderr is free-form log text. The codec lives in `score64_formats::emu_protocol` and is shared by the helper and its clients (`score64_adapters::emu`), which only ever meet across the process boundary (ADR-002). What the helper emulates is described in [TAD §5.1](../TAD.md#51-the-emulator-helper).

## Framing

Every message is a `u32` length followed by that many bytes. The length counts everything after itself, so it is never 0; the first byte after it is the message kind. Either side rejects a length above 64 MiB.

All integers are little-endian. A *string* is a `u16` byte count then UTF-8; an encoder cuts a longer string at the last whole character that fits, so it always decodes. A *blob* is a `u32` byte count then the bytes. A *flag* is one byte, 0 or 1; any other value is malformed. A message with bytes left over after its last field is malformed.

The client sends one request and reads exactly one reply before sending the next. When the client closes stdin, the helper exits.

## Session

1. `Hello` first. Before it, anything except `Quit` gets a `Sequence` error; a second `Hello` gets one too.
2. `Load` the player (and any effect data), as often as needed.
3. `Init` a subtune. This starts a new timeline at the next frame boundary; frame 0 is the first play call.
4. `Render` frames in chunks of any size. Chunks continue one timeline: five renders of 10 frames give the same audio and writes as one render of 50. `Poke` (for example a mailbox the player reads) and `Model` may come between chunks.
5. `Quit`.

To trace a timeline, send `Trace` before its `Init`, and `Report` whenever the client wants what has been collected. A trace stays armed for every following `Init` until another `Trace` replaces or disarms it.

The same build of the helper gives byte-identical replies to identical requests, in any process (#207; `crates/score64-adapters/tests/helper_determinism.rs`). Different platforms or compilers may still differ slightly in floating point.

## Requests (client → helper)

| Kind | Name | Fields after the kind byte | Reply |
|---|---|---|---|
| `0x01` | Hello | `u16` version, `u32` sample rate (8,000–192,000 Hz) | Ready |
| `0x02` | Load | `u16` address, blob data (must not run past `$FFFF`) | Ok |
| `0x03` | Init | `u16` init address, `u16` play address, `u8` subtune (0-based, passed in A) | Ok |
| `0x04` | Render | `u32` frames (1–30,000), flag "return SID writes" | Rendered |
| `0x05` | Poke | `u16` address, `u8` value | Ok |
| `0x06` | Model | `u8` chip: 0 = 6581, 1 = 8580 | Ok |
| `0x07` | Quit | — | Ok, then the helper exits |
| `0x08` | Trace | flag zero page, flag window, `u16` window first, `u16` window last, `u8` count *k* (0–16), *k* × `u16` watched address | Ok |
| `0x09` | Report | — | Traced |

- **Init** calls the init routine with a budget of one second of CPU time (985,248 cycles), then runs the machine idle to the next frame boundary. Audio and writes from init are not returned.
- **Render** calls play once per PAL frame (19,656 cycles), with a budget of one frame per call. If a play call fails, the timeline is dropped: the next `Render` gets `Sequence` until a new `Init`.
- **Render size rule:** an accepted `Render` never produces a reply over the 64 MiB limit. Before running anything, the helper bounds the reply from the frame count, the sample rate and whether writes were asked for. If the bound is over the limit, it answers `Limit`; nothing has run, so the timeline is where it was. The bound allows:
  - 2 bytes per sample, for the exact sample count plus 0.1% and 64 samples (resamplers run slightly fast);
  - when writes are asked for, 8 bytes for each of 6,552 writes per frame. A 6502 writes at most once every 3 cycles on average.
  
  `score64_formats::emu_protocol::max_render_frames` gives the largest accepted count. It is 30,000 frames up to 48 kHz, 8,759 at 192 kHz, and about 1,235 (25 s) with writes. Clients split longer renders into chunks of that size.
- **Poke** writes now, at the current cycle. Addresses `$D400`–`$D7FF` reach the SID (mirrored every 32 bytes); everything else is RAM.
- **Model** replaces the SID with a fresh one of the chosen model, started at the current cycle. RAM and the timeline are kept. The default is 6581.
- **Trace** arms a trace for every following `Init`, replacing any earlier one; each `Init` starts collecting afresh. With both flags off and no watched addresses it disarms. It does not change the timeline that is running. The window fields are read only when the window flag is set, and the window is inclusive. It answers `Limit` for:
  - more than 16 watched addresses;
  - an address watched twice;
  - a watched address in `$D400`–`$D7FF` (SID registers aren't RAM; their writes come with `Render`);
  - a window whose last address is before its first.
- **What a trace collects**, from the start of `Init`'s call to the init routine:
  - with the zero-page flag, every address in `$00`–`$FF` that the 6510 reads or writes;
  - with the window flag, the high byte of every address written outside the window, other than page 0, the stack page (`$01xx`) and the SID's registers `$D400`–`$D418`. A write to a SID mirror counts as written outside, because on a target with a second SID it would reach that SID;
  - for each watched address, its value after init, and each frame whose play call left it with a new value.

  Accesses are the CPU's own, at their effective addresses: opcode and operand fetches, pointer reads, and data reads and writes. The 6502 core makes no NMOS dummy reads. The host's `Load` and `Poke` aren't traced. A `Load` or `Poke` onto a watched address changes its value without counting as a change, so `start` plus the changes doesn't replay the host's writes; a client that writes a watched address tracks that itself.
- **Report** answers `Sequence` unless the latest `Init` succeeded with a trace armed: a failed `Init` leaves nothing to report, even after a traced one. After a failed play call, what was collected up to it can still be reported. The zero-page and outside sets are everything since `Init`. Watched changes are handed over once: each `Report` gives the changes since the previous one, and each address's `start` value is its value when that span began.
- **Report size rule:** an accepted `Render` never leaves more watched changes than one `Traced` reply can hold. Each watched address changes at most once per frame, so before running anything the helper bounds the next `Traced` from the frames since the later of the last `Init` and the last `Report`. If that bound is over the limit, it answers `Limit`, and the client sends `Report` first. With 16 addresses that allows about 838,000 frames (4.6 hours) between reports.

## Replies (helper → client)

| Kind | Name | Fields after the kind byte |
|---|---|---|
| `0x81` | Ready | `u16` version, `u32` sample rate, string helper name and version, string SID engine |
| `0x82` | Ok | — |
| `0x83` | Rendered | `u32` frames, `u32` longest play call in cycles, `u32` sample count *n*, *n* × `i16` mono samples, `u32` write count *m*, *m* × write |
| `0x84` | Traced | 32-byte zero-page map, 32-byte map of pages written outside, `u8` count *k*, *k* × watched |
| `0xFF` | Error | `u16` code, string message |

A *write* is 8 bytes: `u32` frame since `Init`, `u16` cycle within that frame (0–19,655, counted from the play call), `u8` SID register (`$00`–`$1C`), `u8` value. Writes come in the order they were made. The write list is empty unless the request asked for it.

A *map* is 256 bits: address or page *n* is bit *n* mod 8 of byte *n* / 8, where bit 0 is the least significant (value `$01`). A *watched* entry is `u16` address, `u8` start value, `u32` change count *c*, then *c* × (`u32` frame since `Init`, `u8` new value), in frame order. Watched entries come in the order `Trace` listed them.

The number of samples per chunk follows the PAL clock (985,248 Hz), not the 50 Hz frame count: 50 frames at 48 kHz are 0.9975 s, about 47,890 samples. The engine carries fractional samples across chunks.

## Errors

| Code | Name | When | Session afterwards |
|---|---|---|---|
| 1 | Malformed | The bytes are not a valid message (bad length, unknown kind, truncated or extra fields, bad flag or UTF-8) | The helper replies, then exits |
| 2 | Version | `Hello` asks for a version the helper does not speak | Continues; the client may send another `Hello` |
| 3 | Sequence | A request that is not allowed now | Continues |
| 4 | CpuJam | The CPU hit a JAM opcode in init or play | Continues; after a play failure, `Init` again |
| 5 | Timeout | Init or play did not return within its budget | Continues; after a play failure, `Init` again |
| 6 | Limit | A value is out of range: address overflow, frame count, sample rate, a `Render` whose reply (or the next `Traced`) could exceed the limit, or a `Trace` with too many, repeated or SID watched addresses or an inverted window | Continues; nothing ran |
| 7 | Internal | Anything else | Continues |

A client must accept codes it does not know; a newer helper may add some.

## Byte examples

Each example is the complete message, length prefix included. These are the codec's golden tests.

| Message | Bytes |
|---|---|
| Hello, version 1, 48,000 Hz | `07 00 00 00  01  01 00  80 BB 00 00` |
| Init, init `$1000`, play `$1003`, subtune 1 | `06 00 00 00  03  00 10  03 10  01` |
| Render, 50 frames, no writes | `06 00 00 00  04  32 00 00 00  00` |
| Quit | `01 00 00 00  07` |
| Ok | `01 00 00 00  82` |
| Error, Sequence, `"x"` | `06 00 00 00  FF  03 00  01 00 78` |
| Trace, zero page, window `$1000`–`$1FFF`, watch `$103F` | `0A 00 00 00  08  01 01  00 10  FF 1F  01  3F 10` |
| Report | `01 00 00 00  09` |
| Traced, zero page `$F8`/`$F9`, `$103F` from 0 to 1 at frame 7 | `4E 00 00 00  84`, 31 × `00`, `03`, 32 × `00`, `01  3F 10  00  01 00 00 00  07 00 00 00  01` |

## Versioning

The version in `Hello` and `Ready` changes whenever a message's layout or meaning changes, or a request or reply kind is added. New error codes do not bump it. Version 0 was the first. Version 1 adds `Trace`, `Report` and `Traced` (#51). Neither makes compatibility promises before M1, and the helper speaks only its own version.
