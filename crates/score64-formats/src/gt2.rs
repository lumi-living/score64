//! GoatTracker 2 instrument files (`.ins`, `GTI5`), read and written byte for byte (#48,
//! `docs/reference/goattracker-files.md`).
//!
//! A file is the magic `GTI5`, the instrument's 25 bytes, then its four tables (wave, pulse, filter, speed), each as
//! a length and that many left and right bytes. Table pointers in the instrument count from 1 into its own tables;
//! 0 means none.

use std::fmt;

/// The magic at the start of an instrument file.
pub const INS_MAGIC: &[u8; 4] = b"GTI5";

/// Gate-timer bit: skip the hard-restart ADSR write before this instrument's notes; the gate still goes off
/// (`docs/reference/sid.md`).
pub const NO_HARD_RESTART_ADSR: u8 = 0x80;

/// Gate-timer bit: no gate-off before a new note played with this instrument, which makes it a legato instrument. The
/// new note's instrument decides this; the timing comes from the instrument before it (`docs/reference/sid.md`).
pub const NO_GATE_OFF: u8 = 0x40;

/// An instrument's 25 bytes, in file order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Instrument {
    /// Attack and decay (SID `$05`).
    pub ad: u8,
    /// Sustain and release (SID `$06`).
    pub sr: u8,
    /// Wavetable start, 1-based; 0 for none.
    pub wave: u8,
    /// Pulsetable start, 1-based; 0 for none.
    pub pulse: u8,
    /// Filtertable start, 1-based; 0 for none.
    pub filter: u8,
    /// Vibrato parameter: a speedtable entry, 1-based; 0 for none.
    pub vibrato: u8,
    /// Vibrato delay, in frames.
    pub vibrato_delay: u8,
    /// Gate timer: how many frames before the next row the player fetches it (gate-off for a new note), in the low
    /// 6 bits, with the [`NO_HARD_RESTART_ADSR`] and [`NO_GATE_OFF`] flags above.
    pub gate_timer: u8,
    /// The waveform register's value on a note's first frame.
    pub first_wave: u8,
    /// The name, as stored: 16 bytes, zero-padded.
    pub name: [u8; 16],
}

impl Instrument {
    /// The record as stored, in a `.ins` file after the magic and in a `.sng` file's instrument list.
    pub fn from_bytes(b: &[u8; 25]) -> Self {
        Instrument {
            ad: b[0],
            sr: b[1],
            wave: b[2],
            pulse: b[3],
            filter: b[4],
            vibrato: b[5],
            vibrato_delay: b[6],
            gate_timer: b[7],
            first_wave: b[8],
            name: b[9..25].try_into().expect("16 bytes"),
        }
    }

    /// The 25 bytes of the record.
    pub fn to_bytes(&self) -> [u8; 25] {
        let mut b = [0; 25];
        b[..9].copy_from_slice(&[
            self.ad,
            self.sr,
            self.wave,
            self.pulse,
            self.filter,
            self.vibrato,
            self.vibrato_delay,
            self.gate_timer,
            self.first_wave,
        ]);
        b[9..].copy_from_slice(&self.name);
        b
    }

    /// The name as text: up to the first zero byte, with anything but printable ASCII shown as `?`.
    pub fn name_text(&self) -> String {
        self.name
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

    /// Set the name from text: its first 15 characters, zero-padded so it always ends in a zero, with anything but
    /// printable ASCII stored as `?`.
    pub fn set_name(&mut self, name: &str) {
        self.name = [0; 16];
        for (slot, c) in self.name[..15].iter_mut().zip(name.chars()) {
            *slot = if (' '..='~').contains(&c) {
                c as u8
            } else {
                b'?'
            };
        }
    }
}

/// One table: rows of a left and a right byte (at most 255).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Table {
    /// The rows, in order.
    pub rows: Vec<(u8, u8)>,
}

impl Table {
    /// Read a table as stored (a length *n*, *n* left bytes, *n* right bytes) from `data` at `*at`, moving `*at` on.
    pub fn read(data: &[u8], at: &mut usize, name: &'static str) -> Result<Self, InsError> {
        let n = usize::from(*data.get(*at).ok_or(InsError::Truncated(name))?);
        let body = data
            .get(*at + 1..*at + 1 + 2 * n)
            .ok_or(InsError::Truncated(name))?;
        *at += 1 + 2 * n;
        let (left, right) = body.split_at(n);
        Ok(Table {
            rows: left.iter().copied().zip(right.iter().copied()).collect(),
        })
    }

    /// Append the table as stored.
    pub fn write(&self, out: &mut Vec<u8>, name: &'static str) -> Result<(), InsError> {
        let n = u8::try_from(self.rows.len()).map_err(|_| InsError::TableTooLong(name))?;
        out.push(n);
        out.extend(self.rows.iter().map(|&(l, _)| l));
        out.extend(self.rows.iter().map(|&(_, r)| r));
        Ok(())
    }
}

/// An instrument file: the instrument and its own four tables.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ins {
    /// The instrument.
    pub instrument: Instrument,
    /// The wavetable.
    pub wave: Table,
    /// The pulsetable.
    pub pulse: Table,
    /// The filtertable.
    pub filter: Table,
    /// The speedtable.
    pub speed: Table,
}

/// Why bytes are not an instrument file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsError {
    /// It does not start with `GTI5`.
    BadMagic,
    /// It ends inside the named part.
    Truncated(&'static str),
    /// Bytes are left over after the speedtable, at this offset.
    Trailing(usize),
    /// A table has more than 255 rows, so it can't be written.
    TableTooLong(&'static str),
    /// A table pointer past the end of its table, so the file can't be written.
    PointerOutOfRange(&'static str),
}

impl fmt::Display for InsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InsError::BadMagic => write!(f, "not a GoatTracker 2 instrument (no GTI5 magic)"),
            InsError::Truncated(what) => write!(f, "the file ends inside the {what}"),
            InsError::Trailing(at) => write!(
                f,
                "bytes are left over after the speedtable, from offset {at}"
            ),
            InsError::TableTooLong(name) => write!(f, "the {name} has more than 255 rows"),
            InsError::PointerOutOfRange(name) => {
                write!(f, "the instrument points past the end of its {name}")
            }
        }
    }
}

impl std::error::Error for InsError {}

const TABLES: [&str; 4] = ["wavetable", "pulsetable", "filtertable", "speedtable"];

impl Ins {
    /// Read an instrument file.
    pub fn read(data: &[u8]) -> Result<Self, InsError> {
        if data.get(..4) != Some(INS_MAGIC) {
            return Err(InsError::BadMagic);
        }
        let record: &[u8; 25] = data
            .get(4..29)
            .ok_or(InsError::Truncated("instrument"))?
            .try_into()
            .expect("25 bytes");
        let mut at = 29;
        let mut table = |name| Table::read(data, &mut at, name);
        let (wave, pulse, filter, speed) = (
            table(TABLES[0])?,
            table(TABLES[1])?,
            table(TABLES[2])?,
            table(TABLES[3])?,
        );
        if at != data.len() {
            return Err(InsError::Trailing(at));
        }
        Ok(Ins {
            instrument: Instrument::from_bytes(record),
            wave,
            pulse,
            filter,
            speed,
        })
    }

    /// Whether every table pointer is 0 or inside its own table (the vibrato parameter points into the speedtable).
    pub fn check(&self) -> Result<(), InsError> {
        let i = &self.instrument;
        for (pointer, table, name) in [
            (i.wave, &self.wave, TABLES[0]),
            (i.pulse, &self.pulse, TABLES[1]),
            (i.filter, &self.filter, TABLES[2]),
            (i.vibrato, &self.speed, TABLES[3]),
        ] {
            if usize::from(pointer) > table.rows.len() {
                return Err(InsError::PointerOutOfRange(name));
            }
        }
        Ok(())
    }

    /// The file's bytes, if every pointer is inside its table ([`Ins::check`]).
    pub fn write(&self) -> Result<Vec<u8>, InsError> {
        self.check()?;
        let mut out = Vec::with_capacity(4 + 25 + 4);
        out.extend_from_slice(INS_MAGIC);
        out.extend_from_slice(&self.instrument.to_bytes());
        for (table, name) in [&self.wave, &self.pulse, &self.filter, &self.speed]
            .into_iter()
            .zip(TABLES)
        {
            table.write(&mut out, name)?;
        }
        Ok(out)
    }
}

pub mod sng;
pub mod wave;

#[cfg(test)]
mod tests;
