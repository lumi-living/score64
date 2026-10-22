# Emulator helper protocol, version 0

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

- **Init** calls the init routine with a budget of one second of CPU time (985,248 cycles), then runs the machine idle to the next frame boundary. Audio and writes from init are not returned.
- **Render** calls play once per PAL frame (19,656 cycles), with a budget of one frame per call. If a play call fails, the timeline is dropped: the next `Render` gets `Sequence` until a new `Init`.
- **Render size rule:** an accepted `Render` never produces a reply over the 64 MiB limit. Before running anything, the helper bounds the reply from the frame count, the sample rate and whether writes were asked for. If the bound is over the limit, it answers `Limit`; nothing has run, so the timeline is where it was. The bound allows:
  - 2 bytes per sample, for the exact sample count plus 0.1% and 64 samples (resamplers run slightly fast);
  - when writes are asked for, 8 bytes for each of 6,552 writes per frame. A 6502 writes at most once every 3 cycles on average.
  
  `score64_formats::emu_protocol::max_render_frames` gives the largest accepted count. It is 30,000 frames up to 48 kHz, 8,759 at 192 kHz, and about 1,235 (25 s) with writes. Clients split longer renders into chunks of that size.
- **Poke** writes now, at the current cycle. Addresses `$D400`–`$D7FF` reach the SID (mirrored every 32 bytes); everything else is RAM.
- **Model** replaces the SID with a fresh one of the chosen model, started at the current cycle. RAM and the timeline are kept. The default is 6581.

## Replies (helper → client)

| Kind | Name | Fields after the kind byte |
|---|---|---|
| `0x81` | Ready | `u16` version, `u32` sample rate, string helper name and version, string SID engine |
| `0x82` | Ok | — |
| `0x83` | Rendered | `u32` frames, `u32` longest play call in cycles, `u32` sample count *n*, *n* × `i16` mono samples, `u32` write count *m*, *m* × write |
| `0xFF` | Error | `u16` code, string message |

A *write* is 8 bytes: `u32` frame since `Init`, `u16` cycle within that frame (0–19,655, counted from the play call), `u8` SID register (`$00`–`$1C`), `u8` value. Writes come in the order they were made. The write list is empty unless the request asked for it.

The number of samples per chunk follows the PAL clock (985,248 Hz), not the 50 Hz frame count: 50 frames at 48 kHz are 0.9975 s, about 47,890 samples. The engine carries fractional samples across chunks.

## Errors

| Code | Name | When | Session afterwards |
|---|---|---|---|
| 1 | Malformed | The bytes are not a valid message (bad length, unknown kind, truncated or extra fields, bad flag or UTF-8) | The helper replies, then exits |
| 2 | Version | `Hello` asks for a version the helper does not speak | Continues; the client may send another `Hello` |
| 3 | Sequence | A request that is not allowed now | Continues |
| 4 | CpuJam | The CPU hit a JAM opcode in init or play | Continues; after a play failure, `Init` again |
| 5 | Timeout | Init or play did not return within its budget | Continues; after a play failure, `Init` again |
| 6 | Limit | A value is out of range: address overflow, frame count, sample rate, or a `Render` whose reply could exceed the limit | Continues; nothing ran |
| 7 | Internal | Anything else | Continues |

A client must accept codes it does not know; a newer helper may add some.

## Byte examples

Each example is the complete message, length prefix included. These are the codec's golden tests.

| Message | Bytes |
|---|---|
| Hello, version 0, 48,000 Hz | `07 00 00 00  01  00 00  80 BB 00 00` |
| Init, init `$1000`, play `$1003`, subtune 1 | `06 00 00 00  03  00 10  03 10  01` |
| Render, 50 frames, no writes | `06 00 00 00  04  32 00 00 00  00` |
| Quit | `01 00 00 00  07` |
| Ok | `01 00 00 00  82` |
| Error, Sequence, `"x"` | `06 00 00 00  FF  03 00  01 00 78` |

## Versioning

The version in `Hello` and `Ready` changes whenever a message's layout or meaning changes, or a request or reply kind is added. New error codes do not bump it. Version 0 is the first, and it makes no compatibility promises before M1.
