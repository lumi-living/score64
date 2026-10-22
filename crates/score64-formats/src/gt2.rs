//! GoatTracker 2 instrument files (`.ins`, `GTI5`), read and written byte for byte (#48,
//! `docs/reference/goattracker-files.md`).
//!
//! A file is the magic `GTI5`, the instrument's 25 bytes, then its four tables (wave, pulse, filter, speed), each as
//! a length and that many left and right bytes. Table pointers in the instrument count from 1 into its own tables;
//! 0 means none.

use std::fmt;

/// The magic at the start of an instrument file.
pub const INS_MAGIC: &[u8; 4] = b"GTI5";

/// Gate-timer bit: no hard restart before this instrument's notes.
pub const NO_HARD_RESTART: u8 = 0x80;

/// Gate-timer bit: no gate-off at the end of this instrument's notes (legato instruments).
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
    /// Gate timer: frames of gate-off (hard restart) before a note, in the low 6 bits, with the
    /// [`NO_HARD_RESTART`] and [`NO_GATE_OFF`] flags above.
    pub gate_timer: u8,
    /// The waveform register's value on a note's first frame.
    pub first_wave: u8,
    /// The name, as stored: 16 bytes, zero-padded.
    pub name: [u8; 16],
}

impl Instrument {
    /// The name as text: up to the first zero byte, with any non-ASCII byte shown as `?`.
    pub fn name_text(&self) -> String {
        self.name
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| if b.is_ascii() { char::from(b) } else { '?' })
            .collect()
    }

    /// Set the name from text: its first 16 bytes, zero-padded.
    pub fn set_name(&mut self, name: &str) {
        self.name = [0; 16];
        for (slot, b) in self.name.iter_mut().zip(name.bytes()) {
            *slot = b;
        }
    }
}

/// One table: rows of a left and a right byte (at most 255).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Table {
    /// The rows, in order.
    pub rows: Vec<(u8, u8)>,
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
        }
    }
}

impl std::error::Error for InsError {}

const TABLES: [&str; 4] = ["wavetable", "pulsetable", "filtertable", "speedtable"];

impl Ins {
    /// Read an instrument file.
    pub fn read(data: &[u8]) -> Result<Self, InsError> {
        let mut at = 0usize;
        let mut take = |n: usize, what: &'static str| -> Result<&[u8], InsError> {
            let bytes = data.get(at..at + n).ok_or(InsError::Truncated(what))?;
            at += n;
            Ok(bytes)
        };
        if take(4, "magic").map_err(|_| InsError::BadMagic)? != INS_MAGIC {
            return Err(InsError::BadMagic);
        }
        let b = take(25, "instrument")?;
        let instrument = Instrument {
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
        };
        let mut tables: [Table; 4] = Default::default();
        for (table, name) in tables.iter_mut().zip(TABLES) {
            let n = usize::from(take(1, name)?[0]);
            let left = take(n, name)?.to_vec();
            let right = take(n, name)?;
            table.rows = left.into_iter().zip(right.iter().copied()).collect();
        }
        if at != data.len() {
            return Err(InsError::Trailing(at));
        }
        let [wave, pulse, filter, speed] = tables;
        Ok(Ins {
            instrument,
            wave,
            pulse,
            filter,
            speed,
        })
    }

    /// The file's bytes.
    pub fn write(&self) -> Result<Vec<u8>, InsError> {
        let i = &self.instrument;
        let mut out = Vec::with_capacity(4 + 25 + 4);
        out.extend_from_slice(INS_MAGIC);
        out.extend_from_slice(&[
            i.ad,
            i.sr,
            i.wave,
            i.pulse,
            i.filter,
            i.vibrato,
            i.vibrato_delay,
            i.gate_timer,
            i.first_wave,
        ]);
        out.extend_from_slice(&i.name);
        for (table, name) in [&self.wave, &self.pulse, &self.filter, &self.speed]
            .into_iter()
            .zip(TABLES)
        {
            let n = u8::try_from(table.rows.len()).map_err(|_| InsError::TableTooLong(name))?;
            out.push(n);
            out.extend(table.rows.iter().map(|&(l, _)| l));
            out.extend(table.rows.iter().map(|&(_, r)| r));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
