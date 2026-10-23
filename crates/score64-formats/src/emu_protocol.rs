//! The emulator helper's stdin/stdout protocol, version 1 (`docs/reference/emu-protocol.md`).
//!
//! Shared by the GPL helper (`score64-emu`, server) and its MIT clients (`score64-adapters`), so both sides encode and
//! decode the same way; they still only meet across a process boundary (ADR-002). Pure: encoding and decoding work
//! on byte slices and `Read`/`Write`, with no process handling here.
//!
//! Framing: every message is `u32 length` (little-endian, the number of bytes that follow) then that many bytes, the
//! first of which is the message kind. All integers are little-endian. Strings are `u16 length` + UTF-8; byte blobs
//! are `u32 length` + bytes. The client sends one request and reads exactly one reply before the next.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};

/// The protocol version this code speaks.
pub const VERSION: u16 = 1;

/// The largest message either side accepts (the length field's value).
pub const MAX_MESSAGE: u32 = 64 << 20;

/// The most frames one `Render` may ask for (10 minutes at 50 Hz). A render whose reply could exceed
/// [`MAX_MESSAGE`] is refused even below this; see [`max_render_frames`].
pub const MAX_RENDER_FRAMES: u32 = 50 * 60 * 10;

/// PAL CPU clock in Hz, which with the frame length fixes how many samples a frame produces.
pub const PAL_CLOCK_HZ: u32 = 985_248;

/// CPU cycles per PAL frame.
pub const PAL_CYCLES_PER_FRAME: u32 = 19_656;

/// The most SID writes one frame can hold: on average a 6502 writes at most once every 3 cycles (read-modify-write
/// instructions write twice in 6).
pub const MAX_WRITES_PER_FRAME: u32 = PAL_CYCLES_PER_FRAME / 3;

/// The most samples `frames` frames can produce at `sample_rate`. Resamplers may run slightly fast (reSIDfp's emits
/// about 0.03% more than `cycles * rate / clock`), so the bound allows 0.1% plus 64 samples over the exact ratio.
pub fn max_samples(frames: u32, sample_rate: u32) -> u64 {
    let exact = (u64::from(frames) * u64::from(PAL_CYCLES_PER_FRAME) * u64::from(sample_rate))
        .div_ceil(u64::from(PAL_CLOCK_HZ));
    exact + exact / 1000 + 64
}

/// The length field of a `Rendered` reply holding `samples` samples and `writes` writes.
pub fn rendered_size(samples: u64, writes: u64) -> u64 {
    // kind, frames, max_play_cycles, sample count, samples, write count, writes
    1 + 4 + 4 + 4 + 2 * samples + 4 + 8 * writes
}

/// The largest `Rendered` reply a render of `frames` frames could produce, with `writes` asked for or not.
pub fn render_reply_bound(frames: u32, sample_rate: u32, writes: bool) -> u64 {
    let w = if writes {
        u64::from(frames) * u64::from(MAX_WRITES_PER_FRAME)
    } else {
        0
    };
    rendered_size(max_samples(frames, sample_rate), w)
}

/// The most frames one `Render` may ask for at `sample_rate`: the largest count whose reply bound fits in
/// [`MAX_MESSAGE`], capped at [`MAX_RENDER_FRAMES`]. Clients split longer renders into chunks of this size.
pub fn max_render_frames(sample_rate: u32, writes: bool) -> u32 {
    let fits = |f: u32| render_reply_bound(f, sample_rate, writes) <= u64::from(MAX_MESSAGE);
    let (mut lo, mut hi) = (0, MAX_RENDER_FRAMES);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) { lo = mid } else { hi = mid - 1 }
    }
    lo
}

/// The most addresses one [`Request::Trace`] may watch.
pub const MAX_WATCH: usize = 16;

/// The length field of a [`Reply::Traced`] for `watch` watched addresses and `changes` changes in all.
pub fn traced_size(watch: usize, changes: u64) -> u64 {
    // kind, two bitmaps, watch count, per address (address, start value, change count), changes
    1 + 32 + 32 + 1 + 7 * watch as u64 + 5 * changes
}

/// SID model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    /// MOS 6581.
    Mos6581,
    /// MOS 8580.
    Mos8580,
}

/// Client → helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Must come first. The helper answers [`Reply::Ready`] or an [`ErrorCode::Version`] error.
    Hello {
        /// Protocol version the client speaks.
        version: u16,
        /// Output sample rate in Hz (8,000–192,000).
        sample_rate: u32,
    },
    /// Copy bytes into C64 RAM at `address` (for example a packed player, or effect data). May repeat.
    Load {
        /// Start address.
        address: u16,
        /// The bytes; `address + data.len()` must not pass `$FFFF`.
        data: Vec<u8>,
    },
    /// Call `init` with A = `subtune` and remember `play` for [`Request::Render`]. Starts a new timeline.
    Init {
        /// Init routine.
        init: u16,
        /// Play routine, called once per frame.
        play: u16,
        /// Subtune, 0-based (passed in A).
        subtune: u8,
    },
    /// Run `frames` PAL frames (play once per frame) and return the audio they produced.
    Render {
        /// Number of frames, 1–[`MAX_RENDER_FRAMES`].
        frames: u32,
        /// Also return every SID register write made in these frames.
        writes: bool,
    },
    /// Write one byte to C64 memory now (RAM, or a SID register if the address is in `$D400`–`$D7FF`).
    Poke {
        /// Address.
        address: u16,
        /// Value.
        value: u8,
    },
    /// Select the SID model. Replaces the SID (its state is reset); RAM and the timeline are kept.
    Model {
        /// The model.
        chip: Chip,
    },
    /// End the session. The helper answers [`Reply::Ok`] and exits.
    Quit,
    /// Arm a trace for the next [`Request::Init`], replacing any earlier one; everything off disarms it.
    Trace {
        /// Collect the zero-page addresses the 6510 reads or writes.
        zero_page: bool,
        /// Collect the pages written outside this window (first and last address, inclusive).
        outside: Option<(u16, u16)>,
        /// RAM addresses whose changes to report: at most [`MAX_WATCH`], distinct, outside `$D400`–`$D7FF`.
        watch: Vec<u16>,
    },
    /// Hand over what the trace has collected since `Init`, and the watched changes since the last `Report`.
    Report,
}

/// What a trace collected, as reported by [`Reply::Traced`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Traced {
    /// Every zero-page address the 6510 read or wrote since `Init` (empty unless asked for).
    pub zero_page: BTreeSet<u8>,
    /// The high byte of every address written since `Init` outside the window, other than zero page, the stack
    /// page and the SID's registers `$D400`–`$D418` (empty unless asked for).
    pub pages_written_outside: BTreeSet<u8>,
    /// Each watched address, in the order asked for.
    pub watched: Vec<Watched>,
}

/// One watched address's changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watched {
    /// The address.
    pub address: u16,
    /// Its value when this span began: after `Init` for the first `Report`, at the previous `Report` after that.
    pub start: u8,
    /// Each frame (since `Init`, 0 = the first play call) whose play call left a new value, with that value.
    pub changes: Vec<(u32, u8)>,
}

/// One SID register write, as reported by [`Reply::Rendered`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidWrite {
    /// Frame number since [`Request::Init`] (0 = the first play call).
    pub frame: u32,
    /// CPU cycle within that frame (0–19,655), counted from the play call.
    pub cycle: u16,
    /// SID register `$00`–`$1C`.
    pub register: u8,
    /// Value written.
    pub value: u8,
}

/// Helper → client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Answer to `Hello`.
    Ready {
        /// Protocol version (equal to the client's).
        version: u16,
        /// Sample rate in use.
        sample_rate: u32,
        /// Helper name and version, for logs.
        helper: String,
        /// SID engine, for logs (for example `"residfp 1.2.3"`).
        engine: String,
    },
    /// Answer to `Load`, `Init`, `Poke`, `Model`, `Trace` and `Quit`.
    Ok,
    /// Answer to `Render`.
    Rendered {
        /// Frames rendered.
        frames: u32,
        /// The longest play call among them, in CPU cycles.
        max_play_cycles: u32,
        /// Mono 16-bit samples produced during these frames.
        samples: Vec<i16>,
        /// SID writes in these frames, in order (empty unless asked for).
        writes: Vec<SidWrite>,
    },
    /// Answer to `Report`.
    Traced(Traced),
    /// A request failed. After [`ErrorCode::Malformed`] the helper exits; after any other code the session goes on.
    Error {
        /// What went wrong.
        code: ErrorCode,
        /// Details for people.
        message: String,
    },
}

/// Error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The bytes are not a valid message. Fatal: the helper exits after replying.
    Malformed,
    /// The client's protocol version is not supported.
    Version,
    /// The request is not allowed now (nothing before `Hello`; `Render` before `Init`).
    Sequence,
    /// The CPU hit a JAM opcode.
    CpuJam,
    /// A routine did not return within its cycle budget.
    Timeout,
    /// A value is out of range (address overflow, too many frames, bad sample rate).
    Limit,
    /// Anything else.
    Internal,
    /// A code this side does not know (a newer peer).
    Unknown(u16),
}

impl ErrorCode {
    fn to_u16(self) -> u16 {
        match self {
            ErrorCode::Malformed => 1,
            ErrorCode::Version => 2,
            ErrorCode::Sequence => 3,
            ErrorCode::CpuJam => 4,
            ErrorCode::Timeout => 5,
            ErrorCode::Limit => 6,
            ErrorCode::Internal => 7,
            ErrorCode::Unknown(c) => c,
        }
    }

    fn from_u16(c: u16) -> Self {
        match c {
            1 => ErrorCode::Malformed,
            2 => ErrorCode::Version,
            3 => ErrorCode::Sequence,
            4 => ErrorCode::CpuJam,
            5 => ErrorCode::Timeout,
            6 => ErrorCode::Limit,
            7 => ErrorCode::Internal,
            c => ErrorCode::Unknown(c),
        }
    }
}

/// Why bytes could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed message: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

// Message kinds.
const HELLO: u8 = 0x01;
const LOAD: u8 = 0x02;
const INIT: u8 = 0x03;
const RENDER: u8 = 0x04;
const POKE: u8 = 0x05;
const MODEL: u8 = 0x06;
const QUIT: u8 = 0x07;
const TRACE: u8 = 0x08;
const REPORT: u8 = 0x09;
const READY: u8 = 0x81;
const OK: u8 = 0x82;
const RENDERED: u8 = 0x83;
const TRACED: u8 = 0x84;
const ERROR: u8 = 0xFF;

struct Out(Vec<u8>);

impl Out {
    fn new(kind: u8) -> Self {
        Self(vec![kind])
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn str(&mut self, s: &str) -> &mut Self {
        // Cut over-long strings on a character boundary, so the other side still reads valid UTF-8.
        let b = &s.as_bytes()[..s.floor_char_boundary(usize::from(u16::MAX))];
        self.u16(b.len() as u16);
        self.0.extend_from_slice(b);
        self
    }
    fn blob(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
        self
    }
    /// A set of bytes as a 256-bit map: bit `n % 8` of byte `n / 8`.
    fn bitmap(&mut self, set: &BTreeSet<u8>) -> &mut Self {
        let mut map = [0u8; 32];
        for &n in set {
            map[usize::from(n / 8)] |= 1 << (n % 8);
        }
        self.0.extend_from_slice(&map);
        self
    }
    /// The body with its length prefix.
    fn frame(&self) -> Vec<u8> {
        let mut f = (self.0.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(&self.0);
        f
    }
}

struct In<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len());
        let end =
            end.ok_or_else(|| DecodeError(format!("ends inside a field at byte {}", self.pos)))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let n = usize::from(self.u16()?);
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| DecodeError("string is not UTF-8".into()))
    }
    fn blob(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            v => Err(DecodeError(format!("flag must be 0 or 1, not {v}"))),
        }
    }
    fn bitmap(&mut self) -> Result<BTreeSet<u8>, DecodeError> {
        let map = self.take(32)?;
        Ok((0..=255u8)
            .filter(|&n| map[usize::from(n / 8)] & (1 << (n % 8)) != 0)
            .collect())
    }
    fn done(&self) -> Result<(), DecodeError> {
        if self.pos == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError(format!(
                "{} bytes left over",
                self.b.len() - self.pos
            )))
        }
    }
}

impl Request {
    /// The framed bytes of this request.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Request::Hello {
                version,
                sample_rate,
            } => Out::new(HELLO).u16(*version).u32(*sample_rate).frame(),
            Request::Load { address, data } => Out::new(LOAD).u16(*address).blob(data).frame(),
            Request::Init {
                init,
                play,
                subtune,
            } => Out::new(INIT).u16(*init).u16(*play).u8(*subtune).frame(),
            Request::Render { frames, writes } => {
                Out::new(RENDER).u32(*frames).u8(u8::from(*writes)).frame()
            }
            Request::Poke { address, value } => Out::new(POKE).u16(*address).u8(*value).frame(),
            Request::Model { chip } => Out::new(MODEL)
                .u8(match chip {
                    Chip::Mos6581 => 0,
                    Chip::Mos8580 => 1,
                })
                .frame(),
            Request::Quit => Out::new(QUIT).frame(),
            Request::Trace {
                zero_page,
                outside,
                watch,
            } => {
                let (first, last) = outside.unwrap_or((0, 0));
                // The count is a byte: a longer list is cut here. The helper takes at most MAX_WATCH anyway, and the
                // client refuses more before sending.
                debug_assert!(watch.len() <= 255, "{} watched addresses", watch.len());
                let mut o = Out::new(TRACE);
                o.u8(u8::from(*zero_page))
                    .u8(u8::from(outside.is_some()))
                    .u16(first)
                    .u16(last)
                    .u8(watch.len().min(255) as u8);
                for &a in watch.iter().take(255) {
                    o.u16(a);
                }
                o.frame()
            }
            Request::Report => Out::new(REPORT).frame(),
        }
    }

    /// Decode a message body (without its length prefix).
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = In { b: body, pos: 0 };
        let req = match r.u8()? {
            HELLO => Request::Hello {
                version: r.u16()?,
                sample_rate: r.u32()?,
            },
            LOAD => Request::Load {
                address: r.u16()?,
                data: r.blob()?.to_vec(),
            },
            INIT => Request::Init {
                init: r.u16()?,
                play: r.u16()?,
                subtune: r.u8()?,
            },
            RENDER => Request::Render {
                frames: r.u32()?,
                writes: r.bool()?,
            },
            POKE => Request::Poke {
                address: r.u16()?,
                value: r.u8()?,
            },
            MODEL => Request::Model {
                chip: match r.u8()? {
                    0 => Chip::Mos6581,
                    1 => Chip::Mos8580,
                    v => return Err(DecodeError(format!("unknown SID model {v}"))),
                },
            },
            QUIT => Request::Quit,
            TRACE => {
                let zero_page = r.bool()?;
                let has_window = r.bool()?;
                let (first, last) = (r.u16()?, r.u16()?);
                let n = r.u8()?;
                let watch = (0..n).map(|_| r.u16()).collect::<Result<_, _>>()?;
                Request::Trace {
                    zero_page,
                    outside: has_window.then_some((first, last)),
                    watch,
                }
            }
            REPORT => Request::Report,
            k => return Err(DecodeError(format!("unknown request kind ${k:02X}"))),
        };
        r.done()?;
        Ok(req)
    }
}

impl Reply {
    /// The framed bytes of this reply.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Reply::Ready {
                version,
                sample_rate,
                helper,
                engine,
            } => Out::new(READY)
                .u16(*version)
                .u32(*sample_rate)
                .str(helper)
                .str(engine)
                .frame(),
            Reply::Ok => Out::new(OK).frame(),
            Reply::Rendered {
                frames,
                max_play_cycles,
                samples,
                writes,
            } => {
                let mut o = Out::new(RENDERED);
                o.u32(*frames)
                    .u32(*max_play_cycles)
                    .u32(samples.len() as u32);
                for s in samples {
                    o.0.extend_from_slice(&s.to_le_bytes());
                }
                o.u32(writes.len() as u32);
                for w in writes {
                    o.u32(w.frame).u16(w.cycle).u8(w.register).u8(w.value);
                }
                o.frame()
            }
            Reply::Traced(t) => {
                let mut o = Out::new(TRACED);
                o.bitmap(&t.zero_page)
                    .bitmap(&t.pages_written_outside)
                    .u8(t.watched.len().min(255) as u8);
                for w in t.watched.iter().take(255) {
                    o.u16(w.address).u8(w.start).u32(w.changes.len() as u32);
                    for &(frame, value) in &w.changes {
                        o.u32(frame).u8(value);
                    }
                }
                o.frame()
            }
            Reply::Error { code, message } => {
                Out::new(ERROR).u16(code.to_u16()).str(message).frame()
            }
        }
    }

    /// Decode a message body (without its length prefix).
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = In { b: body, pos: 0 };
        let reply = match r.u8()? {
            READY => Reply::Ready {
                version: r.u16()?,
                sample_rate: r.u32()?,
                helper: r.str()?,
                engine: r.str()?,
            },
            OK => Reply::Ok,
            RENDERED => {
                let frames = r.u32()?;
                let max_play_cycles = r.u32()?;
                let n = r.u32()? as usize;
                let samples = r.take(
                    n.checked_mul(2)
                        .ok_or_else(|| DecodeError("sample count overflows".into()))?,
                )?;
                let samples = samples
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&b| i16::from_le_bytes(b))
                    .collect();
                let m = r.u32()? as usize;
                let mut writes = Vec::with_capacity(m.min(1 << 20));
                for _ in 0..m {
                    writes.push(SidWrite {
                        frame: r.u32()?,
                        cycle: r.u16()?,
                        register: r.u8()?,
                        value: r.u8()?,
                    });
                }
                Reply::Rendered {
                    frames,
                    max_play_cycles,
                    samples,
                    writes,
                }
            }
            TRACED => {
                let zero_page = r.bitmap()?;
                let pages_written_outside = r.bitmap()?;
                let n = r.u8()?;
                let mut watched = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    let address = r.u16()?;
                    let start = r.u8()?;
                    let m = r.u32()? as usize;
                    // Each change is 5 bytes: never reserve more than the message can hold.
                    let mut changes = Vec::with_capacity(m.min((r.b.len() - r.pos) / 5));
                    for _ in 0..m {
                        changes.push((r.u32()?, r.u8()?));
                    }
                    watched.push(Watched {
                        address,
                        start,
                        changes,
                    });
                }
                Reply::Traced(Traced {
                    zero_page,
                    pages_written_outside,
                    watched,
                })
            }
            ERROR => Reply::Error {
                code: ErrorCode::from_u16(r.u16()?),
                message: r.str()?,
            },
            k => return Err(DecodeError(format!("unknown reply kind ${k:02X}"))),
        };
        r.done()?;
        Ok(reply)
    }
}

/// Read one message body. `Ok(None)` at a clean end of stream (no bytes of a new message).
pub fn read_body(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..])? {
            0 if got == 0 => return Ok(None),
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream ends inside a length prefix",
                ));
            }
            n => got += n,
        }
    }
    let len = u32::from_le_bytes(len);
    if len == 0 || len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message length {len} out of range"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Write one framed message and flush.
pub fn write_frame(w: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    w.write_all(frame)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(frame: &[u8]) -> &[u8] {
        assert_eq!(
            u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize,
            frame.len() - 4
        );
        &frame[4..]
    }

    #[test]
    fn requests_round_trip() {
        for req in [
            Request::Hello {
                version: 0,
                sample_rate: 48_000,
            },
            Request::Load {
                address: 0x1000,
                data: vec![0x4C, 0x00, 0x10],
            },
            Request::Init {
                init: 0x1000,
                play: 0x1003,
                subtune: 1,
            },
            Request::Render {
                frames: 50,
                writes: true,
            },
            Request::Poke {
                address: 0x0340,
                value: 0x7F,
            },
            Request::Model {
                chip: Chip::Mos8580,
            },
            Request::Quit,
            Request::Trace {
                zero_page: true,
                outside: Some((0x1000, 0x1FFF)),
                watch: vec![0x103F, 0x0340],
            },
            Request::Trace {
                zero_page: false,
                outside: None,
                watch: vec![],
            },
            Request::Report,
        ] {
            let f = req.encode();
            assert_eq!(Request::decode(body(&f)).unwrap(), req);
        }
    }

    #[test]
    fn replies_round_trip() {
        for reply in [
            Reply::Ready {
                version: 0,
                sample_rate: 44_100,
                helper: "score64-emu 0.0.0".into(),
                engine: "residfp 1.2.3".into(),
            },
            Reply::Ok,
            Reply::Rendered {
                frames: 2,
                max_play_cycles: 1027,
                samples: vec![0, -1, 32767, -32768],
                writes: vec![SidWrite {
                    frame: 1,
                    cycle: 435,
                    register: 0x18,
                    value: 0x0F,
                }],
            },
            Reply::Error {
                code: ErrorCode::Sequence,
                message: "render before init".into(),
            },
            Reply::Error {
                code: ErrorCode::Unknown(99),
                message: String::new(),
            },
            Reply::Traced(Traced {
                zero_page: [0x00, 0x07, 0x08, 0xF8, 0xF9, 0xFF].into(),
                pages_written_outside: [0x03, 0xC0].into(),
                watched: vec![
                    Watched {
                        address: 0x103F,
                        start: 0x00,
                        changes: vec![(7, 0x01), (71, 0x02)],
                    },
                    Watched {
                        address: 0x0340,
                        start: 0x55,
                        changes: vec![],
                    },
                ],
            }),
            Reply::Traced(Traced::default()),
        ] {
            let f = reply.encode();
            assert_eq!(Reply::decode(body(&f)).unwrap(), reply);
        }
    }

    /// The byte layouts documented in docs/reference/emu-protocol.md.
    #[test]
    fn golden_bytes() {
        assert_eq!(
            Request::Hello {
                version: 1,
                sample_rate: 48_000
            }
            .encode(),
            [0x07, 0, 0, 0, 0x01, 0x01, 0x00, 0x80, 0xBB, 0x00, 0x00]
        );
        assert_eq!(
            Request::Init {
                init: 0x1000,
                play: 0x1003,
                subtune: 1
            }
            .encode(),
            [0x06, 0, 0, 0, 0x03, 0x00, 0x10, 0x03, 0x10, 0x01]
        );
        assert_eq!(
            Request::Render {
                frames: 50,
                writes: false
            }
            .encode(),
            [0x06, 0, 0, 0, 0x04, 0x32, 0, 0, 0, 0x00]
        );
        assert_eq!(Request::Quit.encode(), [0x01, 0, 0, 0, 0x07]);
        assert_eq!(
            Request::Trace {
                zero_page: true,
                outside: Some((0x1000, 0x1FFF)),
                watch: vec![0x103F]
            }
            .encode(),
            [
                0x0A, 0, 0, 0, 0x08, 0x01, 0x01, 0x00, 0x10, 0xFF, 0x1F, 0x01, 0x3F, 0x10
            ]
        );
        assert_eq!(Request::Report.encode(), [0x01, 0, 0, 0, 0x09]);
        let traced = Reply::Traced(Traced {
            zero_page: [0xF8, 0xF9].into(),
            pages_written_outside: [].into(),
            watched: vec![Watched {
                address: 0x103F,
                start: 0,
                changes: vec![(7, 1)],
            }],
        })
        .encode();
        let mut want = vec![0x4E, 0, 0, 0, 0x84];
        want.extend_from_slice(&[0; 32]);
        want[5 + 31] = 0x03; // $F8 and $F9: bits 0 and 1 of byte 31
        want.extend_from_slice(&[0; 32]);
        want.extend_from_slice(&[0x01, 0x3F, 0x10, 0x00, 0x01, 0, 0, 0, 0x07, 0, 0, 0, 0x01]);
        assert_eq!(traced, want);
        assert_eq!(traced.len() as u64 - 4, traced_size(1, 1));
        assert_eq!(Reply::Ok.encode(), [0x01, 0, 0, 0, 0x82]);
        assert_eq!(
            Reply::Error {
                code: ErrorCode::Sequence,
                message: "x".into()
            }
            .encode(),
            [0x06, 0, 0, 0, 0xFF, 0x03, 0x00, 0x01, 0x00, b'x']
        );
    }

    #[test]
    fn rejects_bad_bodies() {
        assert!(Request::decode(&[0x55]).is_err(), "unknown kind");
        assert!(Request::decode(&[0x07, 0x00]).is_err(), "trailing byte");
        assert!(Request::decode(&[0x05, 0x40]).is_err(), "truncated");
        assert!(
            Request::decode(&[0x04, 1, 0, 0, 0, 2]).is_err(),
            "flag not 0/1"
        );
        assert!(Request::decode(&[0x06, 2]).is_err(), "unknown model");
        assert!(
            Request::decode(&[0x08, 1, 0, 0, 0, 0, 0, 1, 0x3F]).is_err(),
            "watch list truncated"
        );
        assert!(
            Request::decode(&[0x08, 2, 0, 0, 0, 0, 0, 0]).is_err(),
            "flag not 0/1"
        );
        assert!(Reply::decode(&[0x84; 10]).is_err(), "bitmaps truncated");
        // A Traced cut off inside a watched entry, and one claiming more changes than it holds.
        let mut cut = vec![TRACED];
        cut.extend_from_slice(&[0; 64]);
        cut.extend_from_slice(&[0x01, 0x3F, 0x10]);
        assert!(Reply::decode(&cut).is_err(), "watched entry truncated");
        let mut huge = vec![TRACED];
        huge.extend_from_slice(&[0; 64]);
        huge.extend_from_slice(&[
            0x01, 0x3F, 0x10, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x07, 0, 0, 0, 0x01,
        ]);
        assert!(Reply::decode(&huge).is_err(), "change count past the end");
        assert!(
            Reply::decode(&[0x83, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x7F]).is_err(),
            "sample count past the end"
        );
    }

    #[test]
    fn long_strings_are_cut_on_a_character_boundary() {
        // 65,534 ASCII bytes then a 2-byte character that would straddle the 65,535-byte limit.
        let message = format!("{}é", "a".repeat(65_534));
        let f = Reply::Error {
            code: ErrorCode::Internal,
            message,
        }
        .encode();
        match Reply::decode(body(&f)).unwrap() {
            Reply::Error { message, .. } => assert_eq!(message.len(), 65_534),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn render_size_limits() {
        // 10 minutes at 192 kHz is about 230 MB of samples: refused well below MAX_RENDER_FRAMES.
        assert!(render_reply_bound(MAX_RENDER_FRAMES, 192_000, false) > u64::from(MAX_MESSAGE));
        for (rate, writes) in [
            (8_000, false),
            (48_000, false),
            (192_000, false),
            (48_000, true),
            (192_000, true),
        ] {
            let max = max_render_frames(rate, writes);
            assert!(max >= 1);
            assert!(render_reply_bound(max, rate, writes) <= u64::from(MAX_MESSAGE));
            if max < MAX_RENDER_FRAMES {
                assert!(render_reply_bound(max + 1, rate, writes) > u64::from(MAX_MESSAGE));
            }
        }
        assert_eq!(max_render_frames(8_000, false), MAX_RENDER_FRAMES);
        // An actual reply never exceeds its bound: one frame at 48 kHz is about 958 samples.
        assert!(max_samples(1, 48_000) >= 958);
    }

    #[test]
    fn framing() {
        let mut stream: &[u8] = &[Request::Quit.encode(), Request::Quit.encode()].concat();
        assert_eq!(read_body(&mut stream).unwrap(), Some(vec![0x07]));
        assert_eq!(read_body(&mut stream).unwrap(), Some(vec![0x07]));
        assert_eq!(read_body(&mut stream).unwrap(), None);
        let mut truncated: &[u8] = &[0x05, 0, 0, 0, 0x07];
        assert!(read_body(&mut truncated).is_err());
        let mut huge: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF];
        assert!(read_body(&mut huge).is_err());
        let mut empty: &[u8] = &[0, 0, 0, 0];
        assert!(read_body(&mut empty).is_err());
    }
}
