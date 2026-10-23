//! GoatTracker 2 song files (`.sng`, `GTS5`), read and written byte for byte (#47, `docs/reference/goattracker-files.md`).
//!
//! A file is the magic `GTS5`; the song name, author and copyright, 32 bytes each; the subtunes' orderlists, three
//! per subtune; the instruments; the wave, pulse, filter and speed tables; and the patterns. [`Sng::read`] checks the
//! structure (lengths, end marks, nothing left over) and [`Sng::check`] the limits and references GoatTracker states,
//! which [`Sng::write`] requires.

use std::fmt;

use super::{InsError, Instrument, TABLES, Table};

/// The magic at the start of a song file.
pub const SNG_MAGIC: &[u8; 4] = b"GTS5";

/// Most subtunes a song may have.
pub const MAX_SUBTUNES: usize = 32;
/// Most entries in one orderlist, before its end mark.
pub const MAX_ORDERLIST: usize = 254;
/// Most patterns a song may have (numbers `$00`–`$CF`).
pub const MAX_PATTERNS: usize = 208;
/// Most rows in one pattern, before its end mark.
pub const MAX_ROWS: usize = 128;
/// Most instruments a song may have (numbers `$01`–`$3F`; `$00` means no change).
pub const MAX_INSTRUMENTS: usize = 63;

/// The first note value, C-0.
pub const NOTE_FIRST: u8 = 0x60;
/// The last note value, G#7.
pub const NOTE_LAST: u8 = 0xBC;
/// No new note: the voice carries on.
pub const REST: u8 = 0xBD;
/// Key off: the gate goes off.
pub const KEY_OFF: u8 = 0xBE;
/// Key on: the gate goes on again, without a new note.
pub const KEY_ON: u8 = 0xBF;
/// The end mark of an orderlist (followed by the restart position) and of a pattern.
pub const END: u8 = 0xFF;

/// One voice's orderlist in one subtune.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Orderlist {
    /// The entries before the end mark: pattern numbers (`$00`–`$CF`), repeats (`$D0`–`$DF`) and transposes
    /// (`$E0`–`$FE`).
    pub entries: Vec<u8>,
    /// Where playback goes after the last entry, as an index into `entries`.
    pub restart: u8,
}

/// One pattern row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Row {
    /// A note (`$60`–`$BC`), [`REST`], [`KEY_OFF`] or [`KEY_ON`].
    pub note: u8,
    /// An instrument number, or 0 for no change.
    pub instrument: u8,
    /// A command, `$0`–`$F`.
    pub command: u8,
    /// The command's parameter.
    pub data: u8,
}

/// One pattern: its rows, without the end mark.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pattern {
    /// The rows, in order.
    pub rows: Vec<Row>,
}

/// A song file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sng {
    /// The song name, as stored: 32 bytes, zero-padded.
    pub name: [u8; 32],
    /// The author, likewise.
    pub author: [u8; 32],
    /// The copyright, likewise.
    pub copyright: [u8; 32],
    /// Each subtune's orderlists, for voices 1, 2 and 3.
    pub subtunes: Vec<[Orderlist; 3]>,
    /// Instruments 1 onwards (instrument 0, the empty one, is not stored).
    pub instruments: Vec<Instrument>,
    /// The wavetable.
    pub wave: Table,
    /// The pulsetable.
    pub pulse: Table,
    /// The filtertable.
    pub filter: Table,
    /// The speedtable.
    pub speed: Table,
    /// Patterns 0 onwards.
    pub patterns: Vec<Pattern>,
}

/// Why bytes are not a song file, or a song can't be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SngError {
    /// It does not start with `GTS5`.
    BadMagic,
    /// It ends inside the named part.
    Truncated(&'static str),
    /// An orderlist's end mark is not where its length puts it, at this offset.
    NoOrderlistEnd(usize),
    /// A pattern's last row is not the end mark, at this offset.
    NoPatternEnd(usize),
    /// Bytes are left over after the last pattern, at this offset.
    Trailing(usize),
    /// A count or length is outside what GoatTracker allows.
    TooMany(String),
    /// Something refers to what isn't there, or holds a value GoatTracker doesn't define.
    Invalid(String),
}

impl fmt::Display for SngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SngError::BadMagic => write!(f, "not a GoatTracker 2 song (no GTS5 magic)"),
            SngError::Truncated(what) => write!(f, "the file ends inside the {what}"),
            SngError::NoOrderlistEnd(at) => {
                write!(f, "an orderlist has no end mark at offset {at}")
            }
            SngError::NoPatternEnd(at) => write!(f, "a pattern has no end mark at offset {at}"),
            SngError::Trailing(at) => write!(
                f,
                "bytes are left over after the patterns, from offset {at}"
            ),
            SngError::TooMany(why) | SngError::Invalid(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for SngError {}

impl From<InsError> for SngError {
    fn from(e: InsError) -> Self {
        match e {
            InsError::Truncated(what) => SngError::Truncated(what),
            other => SngError::Invalid(other.to_string()),
        }
    }
}

/// Reads bytes in order, naming the part it was reading when they run out.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], SngError> {
        let bytes = self
            .data
            .get(self.at..self.at + n)
            .ok_or(SngError::Truncated(what))?;
        self.at += n;
        Ok(bytes)
    }

    fn byte(&mut self, what: &'static str) -> Result<u8, SngError> {
        Ok(self.take(1, what)?[0])
    }
}

impl Sng {
    /// An empty song: no name, one subtune whose voices each play pattern 0, one empty instrument-free pattern of one
    /// rest row, and no instruments or tables. A starting point to fill in.
    pub fn empty() -> Self {
        let voice = || Orderlist {
            entries: vec![0],
            restart: 0,
        };
        Sng {
            name: [0; 32],
            author: [0; 32],
            copyright: [0; 32],
            subtunes: vec![[voice(), voice(), voice()]],
            instruments: vec![],
            wave: Table::default(),
            pulse: Table::default(),
            filter: Table::default(),
            speed: Table::default(),
            patterns: vec![Pattern {
                rows: vec![Row {
                    note: REST,
                    ..Row::default()
                }],
            }],
        }
    }

    /// Read a song file. The structure must be whole; [`Sng::check`] says whether GoatTracker accepts its contents.
    pub fn read(data: &[u8]) -> Result<Self, SngError> {
        if data.get(..4) != Some(SNG_MAGIC) {
            return Err(SngError::BadMagic);
        }
        let mut r = Reader { data, at: 4 };
        let mut text = |what| -> Result<[u8; 32], SngError> {
            Ok(r.take(32, what)?.try_into().expect("32 bytes"))
        };
        let (name, author, copyright) = (text("song name")?, text("author")?, text("copyright")?);
        let subtunes = (0..r.byte("subtune count")?)
            .map(|_| -> Result<[Orderlist; 3], SngError> {
                let mut voice = || -> Result<Orderlist, SngError> {
                    // The length counts the entries and the end mark; the restart position follows.
                    let n = usize::from(r.byte("orderlist length")?);
                    let body = r.take(n + 1, "orderlist")?;
                    if n == 0 || body[n - 1] != END {
                        return Err(SngError::NoOrderlistEnd(r.at - 2));
                    }
                    Ok(Orderlist {
                        entries: body[..n - 1].to_vec(),
                        restart: body[n],
                    })
                };
                Ok([voice()?, voice()?, voice()?])
            })
            .collect::<Result<Vec<_>, _>>()?;
        let instruments = (0..r.byte("instrument count")?)
            .map(|_| {
                let record: &[u8; 25] = r.take(25, "instrument")?.try_into().expect("25 bytes");
                Ok(Instrument::from_bytes(record))
            })
            .collect::<Result<Vec<_>, SngError>>()?;
        let mut table = |name| Table::read(data, &mut r.at, name).map_err(SngError::from);
        let (wave, pulse, filter, speed) = (
            table(TABLES[0])?,
            table(TABLES[1])?,
            table(TABLES[2])?,
            table(TABLES[3])?,
        );
        let patterns = (0..r.byte("pattern count")?)
            .map(|_| {
                // The row count includes the end mark's row.
                let m = usize::from(r.byte("pattern length")?);
                let bytes = r.take(4 * m, "pattern")?;
                let mut rows: Vec<Row> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| Row {
                        note: b[0],
                        instrument: b[1],
                        command: b[2],
                        data: b[3],
                    })
                    .collect();
                if rows.pop().map(|end| end.note) != Some(END) {
                    return Err(SngError::NoPatternEnd(r.at.saturating_sub(4)));
                }
                Ok(Pattern { rows })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if r.at != data.len() {
            return Err(SngError::Trailing(r.at));
        }
        Ok(Sng {
            name,
            author,
            copyright,
            subtunes,
            instruments,
            wave,
            pulse,
            filter,
            speed,
            patterns,
        })
    }

    /// Whether GoatTracker accepts the song: its counts and lengths within the readme's limits, every orderlist
    /// ending on a pattern with a restart inside it, every pattern number, note, instrument and command defined, and
    /// every instrument's table pointers inside the song's tables.
    pub fn check(&self) -> Result<(), SngError> {
        let too_many = |what: &str, n: usize, max: usize| {
            Err(SngError::TooMany(format!(
                "{n} {what}; GoatTracker allows at most {max}"
            )))
        };
        if self.subtunes.is_empty() {
            return Err(SngError::Invalid(
                "a song needs at least one subtune".into(),
            ));
        }
        if self.subtunes.len() > MAX_SUBTUNES {
            return too_many("subtunes", self.subtunes.len(), MAX_SUBTUNES);
        }
        if self.instruments.len() > MAX_INSTRUMENTS {
            return too_many("instruments", self.instruments.len(), MAX_INSTRUMENTS);
        }
        if self.patterns.is_empty() {
            return Err(SngError::Invalid(
                "a song needs at least one pattern".into(),
            ));
        }
        if self.patterns.len() > MAX_PATTERNS {
            return too_many("patterns", self.patterns.len(), MAX_PATTERNS);
        }
        for (s, voices) in self.subtunes.iter().enumerate() {
            for (v, list) in voices.iter().enumerate() {
                self.check_orderlist(list).map_err(|why| {
                    SngError::Invalid(format!("subtune {} voice {}: {why}", s + 1, v + 1))
                })?;
            }
        }
        for (p, pattern) in self.patterns.iter().enumerate() {
            if pattern.rows.is_empty() || pattern.rows.len() > MAX_ROWS {
                return Err(SngError::TooMany(format!(
                    "pattern {p:02X} has {} rows; GoatTracker allows 1 to {MAX_ROWS}",
                    pattern.rows.len()
                )));
            }
            for (i, row) in pattern.rows.iter().enumerate() {
                self.check_row(row).map_err(|why| {
                    SngError::Invalid(format!("pattern {p:02X} row {i:02X}: {why}"))
                })?;
            }
        }
        for (k, i) in self.instruments.iter().enumerate() {
            for (pointer, table, name) in [
                (i.wave, &self.wave, TABLES[0]),
                (i.pulse, &self.pulse, TABLES[1]),
                (i.filter, &self.filter, TABLES[2]),
                (i.vibrato, &self.speed, TABLES[3]),
            ] {
                if usize::from(pointer) > table.rows.len() {
                    return Err(SngError::Invalid(format!(
                        "instrument {:02X} points past the end of the {name}",
                        k + 1
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_orderlist(&self, list: &Orderlist) -> Result<(), String> {
        let n = list.entries.len();
        if n > MAX_ORDERLIST {
            return Err(format!(
                "{n} orderlist entries; GoatTracker allows at most {MAX_ORDERLIST}"
            ));
        }
        match list.entries.last() {
            None => return Err("the orderlist is empty".into()),
            Some(&last) if last >= 0xD0 => {
                return Err(format!(
                    "the last entry is ${last:02X}, not a pattern number"
                ));
            }
            _ => {}
        }
        for &e in &list.entries {
            if e == END {
                return Err("an entry is $FF, the end mark".into());
            }
            if e < 0xD0 && usize::from(e) >= self.patterns.len() {
                return Err(format!("pattern {e:02X} does not exist"));
            }
        }
        if usize::from(list.restart) >= n {
            return Err(format!("restart position {} is past the end", list.restart));
        }
        Ok(())
    }

    fn check_row(&self, row: &Row) -> Result<(), String> {
        if !(NOTE_FIRST..=KEY_ON).contains(&row.note) {
            return Err(format!(
                "note value ${:02X} is not a note, rest, key-off or key-on",
                row.note
            ));
        }
        if usize::from(row.instrument) > self.instruments.len() {
            return Err(format!("instrument {:02X} does not exist", row.instrument));
        }
        if row.command > 0x0F {
            return Err(format!("command ${:02X} is not $0-$F", row.command));
        }
        Ok(())
    }

    /// The file's bytes, if the song passes [`Sng::check`].
    pub fn write(&self) -> Result<Vec<u8>, SngError> {
        self.check()?;
        let mut out = Vec::new();
        out.extend_from_slice(SNG_MAGIC);
        for text in [&self.name, &self.author, &self.copyright] {
            out.extend_from_slice(text);
        }
        out.push(self.subtunes.len() as u8);
        for list in self.subtunes.iter().flatten() {
            out.push(list.entries.len() as u8 + 1);
            out.extend_from_slice(&list.entries);
            out.extend([END, list.restart]);
        }
        out.push(self.instruments.len() as u8);
        for i in &self.instruments {
            out.extend_from_slice(&i.to_bytes());
        }
        for (table, name) in [&self.wave, &self.pulse, &self.filter, &self.speed]
            .into_iter()
            .zip(TABLES)
        {
            table.write(&mut out, name)?;
        }
        out.push(self.patterns.len() as u8);
        for p in &self.patterns {
            out.push(p.rows.len() as u8 + 1);
            for r in &p.rows {
                out.extend([r.note, r.instrument, r.command, r.data]);
            }
            out.extend([END, 0, 0, 0]);
        }
        Ok(out)
    }
}

/// Text as stored in a 32-byte header field: up to the first zero byte, with anything but printable ASCII shown as
/// `?`.
pub fn header_text(field: &[u8; 32]) -> String {
    field
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                char::from(b)
            } else {
                '?'
            }
        })
        .collect()
}

/// A 32-byte header field from text: its first 31 characters, zero-padded so it always ends in a zero, with anything
/// but printable ASCII stored as `?`.
pub fn header_field(text: &str) -> [u8; 32] {
    let mut field = [0; 32];
    for (slot, c) in field[..31].iter_mut().zip(text.chars()) {
        *slot = if (' '..='~').contains(&c) {
            c as u8
        } else {
            b'?'
        };
    }
    field
}

#[cfg(test)]
mod tests;
