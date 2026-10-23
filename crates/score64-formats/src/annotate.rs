//! Annotated dumps of GoatTracker 2 song (`.sng`), instrument (`.ins`) and sound-effect bytes, for golden tests (#22).
//!
//! One line per header field, orderlist, instrument, table entry, pattern row and effect frame, each with its file offset,
//! raw bytes and meaning, so a snapshot diff names exactly what changed. Every byte of the input is accounted for: a
//! dump fails if the data ends early or has bytes left over. Layouts follow the GoatTracker 2.77 readme §6 and
//! `docs/reference/goattracker-sfx.md`.

use std::fmt::Write as _;

/// Why bytes could not be annotated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotateError {
    /// The magic bytes are wrong.
    BadMagic {
        /// What was expected.
        expected: &'static str,
    },
    /// The data ended inside a structure.
    Truncated {
        /// Where.
        offset: usize,
        /// What was being read.
        what: &'static str,
    },
    /// Bytes are left after the last structure.
    Trailing {
        /// Where they start.
        offset: usize,
        /// How many.
        count: usize,
    },
}

impl std::fmt::Display for AnnotateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnnotateError::BadMagic { expected } => write!(f, "not a {expected} file"),
            AnnotateError::Truncated { offset, what } => {
                write!(f, "data ends at ${offset:04X} inside {what}")
            }
            AnnotateError::Trailing { offset, count } => {
                write!(f, "{count} unexpected bytes from ${offset:04X}")
            }
        }
    }
}

impl std::error::Error for AnnotateError {}

/// A cursor that writes one annotated line per structure.
struct Dump<'a> {
    data: &'a [u8],
    pos: usize,
    out: String,
}

/// Width of the hex column (bytes shown before `..`).
const HEX_BYTES: usize = 8;

impl<'a> Dump<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            out: String::new(),
        }
    }

    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], AnnotateError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or(AnnotateError::Truncated {
            offset: self.pos,
            what,
        })?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn byte(&mut self, what: &'static str) -> Result<u8, AnnotateError> {
        Ok(self.take(1, what)?[0])
    }

    /// Consume `n` bytes and write a line for them.
    fn line(
        &mut self,
        n: usize,
        what: &'static str,
        meaning: impl FnOnce(&[u8]) -> String,
    ) -> Result<&'a [u8], AnnotateError> {
        let start = self.pos;
        let bytes = self.take(n, what)?;
        let mut hex: String = bytes
            .iter()
            .take(HEX_BYTES)
            .map(|b| format!("{b:02X} "))
            .collect();
        if bytes.len() > HEX_BYTES {
            hex.push_str(".. ");
        }
        let _ = writeln!(self.out, "{start:04X}  {hex:<27} {}", meaning(bytes));
        Ok(bytes)
    }

    fn heading(&mut self, text: &str) {
        let _ = writeln!(self.out, "-- {text}");
    }

    fn finish(self) -> Result<String, AnnotateError> {
        if self.pos != self.data.len() {
            return Err(AnnotateError::Trailing {
                offset: self.pos,
                count: self.data.len() - self.pos,
            });
        }
        Ok(self.out)
    }
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let s: String = bytes[..end]
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                char::from(b)
            } else {
                '?'
            }
        })
        .collect();
    format!("{s:?}")
}

const NOTE_NAMES: [&str; 12] = [
    "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
];

/// A semitone counted from C-0, as GoatTracker names it ("A-4").
fn note_name(semitone: u8) -> String {
    format!(
        "{}{}",
        NOTE_NAMES[usize::from(semitone % 12)],
        semitone / 12
    )
}

fn waveform(v: u8) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [
        (0x80, "noise"),
        (0x40, "pulse"),
        (0x20, "saw"),
        (0x10, "tri"),
        (0x08, "test"),
        (0x04, "ring"),
        (0x02, "sync"),
        (0x01, "gate"),
    ] {
        if v & bit != 0 {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join("+")
    }
}

const COMMANDS: [&str; 16] = [
    "nop",
    "porta up",
    "porta down",
    "toneporta",
    "vibrato",
    "set AD",
    "set SR",
    "set wave",
    "wave ptr",
    "pulse ptr",
    "filter ptr",
    "filter ctrl",
    "cutoff",
    "volume/mark",
    "funktempo",
    "tempo",
];

fn command(cmd: u8, data: u8) -> String {
    match cmd {
        0 => String::new(),
        0x0F => format!(
            "  F{data:02X} tempo {}{}",
            data & 0x7F,
            if data & 0x80 != 0 {
                " (this channel)"
            } else {
                ""
            }
        ),
        c => format!("  {c:X}{data:02X} {}", COMMANDS[usize::from(c & 0x0F)]),
    }
}

fn wave_row(l: u8, r: u8) -> String {
    let left = match l {
        0x00 => "no wave change".to_string(),
        0x01..=0x0F => format!("delay {l}"),
        0x10..=0xDF => format!("wave ${l:02X} {}", waveform(l)),
        0xE0..=0xEF => format!("wave ${:02X} {} (inaudible)", l - 0xE0, waveform(l - 0xE0)),
        0xF0..=0xFE => {
            return format!(
                "command {:X}{r:02X} {}",
                l - 0xF0,
                COMMANDS[usize::from(l - 0xF0)]
            );
        }
        0xFF => {
            return if r == 0 {
                "jump: stop".into()
            } else {
                format!("jump to {r:02X}")
            };
        }
    };
    let right = match r {
        0x00..=0x5F => format!("note +{r}"),
        0x60..=0x7F => format!("note -{}", 0x80 - u16::from(r)),
        0x80 => "keep frequency".into(),
        _ => format!("note {} (absolute)", note_name(r - 0x80)),
    };
    format!("{left}, {right}")
}

fn pulse_row(l: u8, r: u8) -> String {
    match l {
        0x00 => "(unused)".into(),
        0x01..=0x7F => format!("for {l} ticks, pulse speed {:+}", r as i8),
        0x80..=0xFE => format!(
            "set pulse width ${:03X}",
            (u16::from(l & 0x0F) << 8) | u16::from(r)
        ),
        0xFF => {
            if r == 0 {
                "jump: stop".into()
            } else {
                format!("jump to {r:02X}")
            }
        }
    }
}

fn filter_row(l: u8, r: u8) -> String {
    match l {
        0x00 => format!("set cutoff ${r:02X}"),
        0x01..=0x7F => format!("for {l} ticks, cutoff speed {:+}", r as i8),
        0x80..=0xFE => {
            let mut pass = Vec::new();
            for (bit, name) in [(0x10, "low"), (0x20, "band"), (0x40, "high")] {
                if l & bit != 0 {
                    pass.push(name);
                }
            }
            let voices: Vec<String> = (0..3)
                .filter(|v| r & (1 << v) != 0)
                .map(|v| (v + 1).to_string())
                .collect();
            format!(
                "filter {}-pass, resonance {}, voices {}",
                if pass.is_empty() {
                    "no".to_string()
                } else {
                    pass.join("+")
                },
                r >> 4,
                if voices.is_empty() {
                    "none".to_string()
                } else {
                    voices.join(",")
                }
            )
        }
        0xFF => {
            if r == 0 {
                "jump: stop".into()
            } else {
                format!("jump to {r:02X}")
            }
        }
    }
}

fn speed_row(l: u8, r: u8) -> String {
    format!(
        "{l:02X} {r:02X} (vibrato speed/depth, porta speed ${:04X} or funktempo {l}/{r})",
        u16::from_be_bytes([l, r])
    )
}

fn pattern_row(row: &[u8]) -> String {
    let (note, ins, cmd, data) = (row[0], row[1], row[2], row[3]);
    let n = match note {
        0x60..=0xBC => note_name(note - 0x60),
        0xBD => "...".into(),
        0xBE => "---".into(),
        0xBF => "+++".into(),
        0xFF => "end".into(),
        v => format!("?{v:02X}"),
    };
    let i = if ins == 0 {
        "--".to_string()
    } else {
        format!("{ins:02X}")
    };
    format!("{n} {i}{}", command(cmd, data))
}

fn orderlist(entries: &[u8], restart: u8) -> String {
    let mut parts = Vec::new();
    for &e in entries {
        parts.push(match e {
            0x00..=0xCF => format!("{e:02X}"),
            // Shown as the editor does: $D0 is R1 (once), $DF is R0 (16 times).
            0xD0..=0xDF => format!("R{:X}", (e - 0xD0 + 1) & 0x0F),
            0xE0..=0xEF => format!("-{:X}", 0xF0 - u16::from(e)),
            0xF0..=0xFE => format!("+{:X}", e - 0xF0),
            0xFF => "RST?".into(),
        });
    }
    format!("{} RST {restart:02X}", parts.join(" "))
}

fn instrument(d: &mut Dump<'_>, label: String) -> Result<(), AnnotateError> {
    d.line(25, "instrument", |b| {
        let gate = b[7];
        format!(
            "{label} {} AD {:02X} SR {:02X} wave {:02X} pulse {:02X} filter {:02X} vib {:02X}/{:02X} gate {:02X}{}{} 1st {:02X}",
            text(&b[9..25]),
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            gate & 0x3F,
            if gate & 0x80 != 0 { " no-HR" } else { "" },
            if gate & 0x40 != 0 { " no-gateoff" } else { "" },
            b[8]
        )
    })?;
    Ok(())
}

fn tables(d: &mut Dump<'_>) -> Result<(), AnnotateError> {
    for (name, describe) in [
        ("wavetable", wave_row as fn(u8, u8) -> String),
        ("pulsetable", pulse_row),
        ("filtertable", filter_row),
        ("speedtable", speed_row),
    ] {
        let n = usize::from(d.byte("table length")?);
        d.pos -= 1;
        d.line(1, "table length", |_| format!("{name}: {n} rows"))?;
        let left_at = d.pos;
        let left = d.take(n, "table left side")?.to_vec();
        let right = d.take(n, "table right side")?.to_vec();
        d.heading(&format!(
            "{name} (left ${left_at:04X}, right ${:04X})",
            left_at + n
        ));
        for k in 0..n {
            let _ = writeln!(
                d.out,
                "      {:02X}: {:02X} {:02X}        {}",
                k + 1,
                left[k],
                right[k],
                describe(left[k], right[k])
            );
        }
    }
    Ok(())
}

/// Annotate a GoatTracker 2 song (`GTS5`).
pub fn sng(data: &[u8]) -> Result<String, AnnotateError> {
    let mut d = Dump::new(data);
    if d.take(4, "magic")? != b"GTS5" {
        return Err(AnnotateError::BadMagic {
            expected: "GoatTracker 2 song (GTS5)",
        });
    }
    d.pos = 0;
    d.line(4, "magic", |_| "GTS5".into())?;
    for what in ["song name", "author", "copyright"] {
        d.line(32, "header string", |b| format!("{what} {}", text(b)))?;
    }
    let subtunes = d.line(1, "subtune count", |b| format!("{} subtunes", b[0]))?[0];
    d.heading("orderlists");
    for s in 0..subtunes {
        for ch in 0..3 {
            let n = usize::from(d.byte("orderlist length")?);
            d.pos -= 1;
            d.line(n + 2, "orderlist", |b| {
                let body = &b[1..];
                let ok = n.checked_sub(1).and_then(|i| body.get(i)) == Some(&0xFF);
                let entries = &body[..n.saturating_sub(1)];
                format!(
                    "subtune {} voice {}: {}{}",
                    s + 1,
                    ch + 1,
                    orderlist(entries, body[n]),
                    if ok { "" } else { " (no endmark!)" }
                )
            })?;
        }
    }
    let instruments = d.line(1, "instrument count", |b| format!("{} instruments", b[0]))?[0];
    d.heading("instruments");
    for i in 0..instruments {
        instrument(&mut d, format!("{:02X}", i + 1))?;
    }
    tables(&mut d)?;
    let patterns = d.line(1, "pattern count", |b| format!("{} patterns", b[0]))?[0];
    for p in 0..patterns {
        let rows = usize::from(d.byte("pattern length")?);
        d.pos -= 1;
        d.line(1, "pattern length", |_| {
            format!("pattern {p:02X}: {rows} rows including the end mark")
        })?;
        for r in 0..rows {
            d.line(4, "pattern row", |b| {
                format!("  {p:02X}.{r:02X}  {}", pattern_row(b))
            })?;
        }
    }
    d.finish()
}

/// Annotate a GoatTracker 2 instrument (`GTI5`).
pub fn ins(data: &[u8]) -> Result<String, AnnotateError> {
    let mut d = Dump::new(data);
    if d.take(4, "magic")? != b"GTI5" {
        return Err(AnnotateError::BadMagic {
            expected: "GoatTracker 2 instrument (GTI5)",
        });
    }
    d.pos = 0;
    d.line(4, "magic", |_| "GTI5".into())?;
    instrument(&mut d, "instrument".into())?;
    tables(&mut d)?;
    d.finish()
}

/// Annotate GoatTracker sound-effect data (as `ins2snd2 -b` writes it).
pub fn sfx(data: &[u8]) -> Result<String, AnnotateError> {
    let mut d = Dump::new(data);
    d.line(1, "attack/decay", |b| format!("attack/decay {:02X}", b[0]))?;
    d.line(1, "sustain/release", |b| {
        format!("sustain/release {:02X}", b[0])
    })?;
    d.line(1, "pulse width", |b| {
        let w = (u16::from(b[0] & 0x0F) << 8) | u16::from(b[0] & 0xF0);
        format!("pulse width ${w:03X} (nybbles reversed)")
    })?;
    let mut frame = 0;
    loop {
        let b = d.byte("effect wavetable")?;
        d.pos -= 1;
        match b {
            0x00 => {
                d.line(1, "end", |_| format!("end after {frame} frames"))?;
                break;
            }
            0x82..=0xDF => {
                frame += 1;
                // A note, optionally followed by a waveform for the same frame.
                let wave = d
                    .data
                    .get(d.pos + 1)
                    .copied()
                    .filter(|w| (0x01..=0x81).contains(w));
                let n = if wave.is_some() { 2 } else { 1 };
                d.line(n, "effect frame", |bytes| {
                    let note = note_name(bytes[0] - 0x80);
                    match bytes.get(1) {
                        Some(&w) => format!(
                            "frame {frame:2}: note {note}, wave ${w:02X} {}",
                            waveform(w)
                        ),
                        None => format!("frame {frame:2}: note {note}"),
                    }
                })?;
            }
            0x01..=0x81 => {
                d.line(1, "effect waveform", |bytes| {
                    format!(
                        "wave ${:02X} {} without a note (invalid)",
                        bytes[0],
                        waveform(bytes[0])
                    )
                })?;
            }
            _ => {
                d.line(1, "effect byte", |bytes| {
                    format!("${:02X}: not valid effect data", bytes[0])
                })?;
            }
        }
    }
    if d.data.len() > 128 {
        let _ = writeln!(d.out, "!! {} bytes: over the 128-byte limit", d.data.len());
    }
    d.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orderlist_commands_are_shown_as_the_editor_shows_them() {
        assert_eq!(
            orderlist(&[0xD0, 0xDE, 0xDF, 0xF0, 0xFE, 0xE1, 0x07], 0),
            "R1 RF R0 +0 +E -F 07 RST 00"
        );
    }

    #[test]
    fn notes_and_waveforms() {
        assert_eq!(note_name(57), "A-4");
        assert_eq!(note_name(0), "C-0");
        assert_eq!(waveform(0x41), "pulse+gate");
        assert_eq!(waveform(0x00), "none");
    }

    #[test]
    fn sfx_frames() {
        // AD 09, SR 00, pulse $800, frame 1 note $DC + noise, frame 2 note $D2, end.
        let out = sfx(&[0x09, 0x00, 0x08, 0xDC, 0x81, 0xD2, 0x00]).unwrap();
        assert!(out.contains("pulse width $800"), "{out}");
        assert!(
            out.contains("frame  1: note G#7, wave $81 noise+gate"),
            "{out}"
        );
        assert!(out.contains("frame  2: note A#6"), "{out}");
        assert!(out.contains("end after 2 frames"), "{out}");
    }

    #[test]
    fn rejects_truncated_and_trailing() {
        assert!(matches!(
            sfx(&[0x09, 0x00, 0x08, 0xDC]),
            Err(AnnotateError::Truncated { .. })
        ));
        assert_eq!(
            sfx(&[0x09, 0x00, 0x08, 0x00, 0x55]),
            Err(AnnotateError::Trailing {
                offset: 4,
                count: 1
            })
        );
        assert!(matches!(sng(b"GTS4"), Err(AnnotateError::BadMagic { .. })));
    }
}
