//! A score as a GoatTracker 2 song (#47): [`export`] turns a [`Song`] into an [`Sng`].
//!
//! - **Patterns.** Each voice's rows are cut at every bar start and at the loop row, into pieces of at most 128 rows,
//!   so all three voices' orderlists stay in step and the loop row starts a pattern. Identical patterns are shared.
//! - **Notes.** A fresh note plays its note with its instrument. A legato note ties with command `3` `00` and no
//!   instrument, so it isn't retriggered (#137). A note followed by silence ends with a key-off; one followed directly
//!   by another note lets the next note's hard restart close the gate.
//! - **Tempo and timing marks** are song-wide commands: `Fxx` (frames per row, all voices) on row 0 and at each tempo
//!   change, and `DXY` at each timing mark of an adaptive layer, with the values [`AdaptiveSet::timing_marks`] gives.
//!   Each goes on the first voice whose command slot is free on that row.
//! - **Arpeggios.** `8XY` points the voice at a wavetable program ([`wave::arpeggio`]) for the event, and back at
//!   the instrument's own program where the event ends while the note sounds on.
//! - **Instruments** come from a caller's library by preset id, numbered in order of first use, with their tables
//!   moved into the song's.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::num::NonZeroU8;

use score64_core::{AdaptiveSet, Attack, PresetId, Song, Subtune, SubtuneName, Voice, VoiceLine};

use super::sng::{self, KEY_OFF, NOTE_FIRST, Orderlist, Pattern, REST, Row, Sng};
use super::wave::{self, SET_WAVE_POINTER, WAVE_JUMP};
use super::{Ins, Table};

/// Command `3XY` with `$00`: tie the note, moving the pitch at once without retriggering it.
const TIE: u8 = 0x3;
/// Command `DXY` with X not 0: write XY to the timing-mark byte.
const TIMING_MARK: u8 = 0xD;
/// Command `FXY`, `$03`–`$7F`: the tempo on all voices.
const TEMPO: u8 = 0xF;
/// The highest note the player's frequency table holds, B-7: a note plus an arpeggio interval must stay at or below
/// it, because the player adds them modulo 128.
const HIGHEST_NOTE: u16 = 95;
/// Instruments 1–62: 63 is left free, because it can carry a tempo setting.
const MAX_INSTRUMENTS: usize = sng::MAX_INSTRUMENTS - 1;

/// Instruments by preset id: each a GoatTracker instrument with its own tables, as in a `.ins` file.
pub type Library = BTreeMap<PresetId, Ins>;

/// The song file's header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    /// The song name.
    pub name: String,
    /// The author.
    pub author: String,
    /// The copyright.
    pub copyright: String,
}

/// The song, and how the score's instruments were numbered in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
    /// The song.
    pub sng: Sng,
    /// Each preset's instrument number (1-based), in number order.
    pub instruments: Vec<(PresetId, u8)>,
    /// Whether the song writes timing marks, so it must be packed with author info and timing marks on (`-H1`);
    /// without it gt2reloc drops them.
    pub timing_marks: bool,
}

/// Why a score can't be written as a GoatTracker song.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// A voice or note uses a preset the library doesn't have.
    UnknownPreset(PresetId),
    /// Two commands want one voice's command slot on one row, or a song-wide command finds every voice's slot taken.
    CommandClash {
        /// The subtune.
        subtune: SubtuneName,
        /// The voice, if one voice's two commands clash.
        voice: Option<Voice>,
        /// The row.
        row: u32,
        /// What clashed.
        what: String,
    },
    /// A note plus an arpeggio interval passes B-7.
    TooHigh {
        /// The subtune.
        subtune: SubtuneName,
        /// The voice.
        voice: Voice,
        /// The row.
        row: u32,
    },
    /// An arpeggio's instrument has no gated waveform in its wavetable's first row to arpeggiate with.
    NoArpWaveform(PresetId),
    /// The song needs more than GoatTracker allows (patterns, instruments, table rows), or another wavetable problem.
    TooMuch(String),
    /// A table merge or the result failed GoatTracker's checks.
    Invalid(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::UnknownPreset(p) => write!(f, "no instrument for preset `{}`", p.0),
            ExportError::CommandClash {
                subtune,
                voice,
                row,
                what,
            } => match voice {
                Some(v) => write!(
                    f,
                    "`{}` voice {} row {row}: {what} need the same command slot",
                    subtune.as_str(),
                    v.get()
                ),
                None => write!(
                    f,
                    "`{}` row {row}: no voice has a free command slot for {what}",
                    subtune.as_str()
                ),
            },
            ExportError::TooHigh {
                subtune,
                voice,
                row,
            } => write!(
                f,
                "`{}` voice {} row {row}: an arpeggio note passes B-7",
                subtune.as_str(),
                voice.get()
            ),
            ExportError::NoArpWaveform(p) => write!(
                f,
                "preset `{}` has no gated waveform in its first wavetable row to arpeggiate with",
                p.0
            ),
            ExportError::TooMuch(why) | ExportError::Invalid(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// One voice's rows in one subtune, before cutting into patterns.
type Rows = Vec<Row>;

/// The song being built: instruments placed so far, the song's tables, and the patterns shared so far.
struct Builder<'a> {
    library: &'a Library,
    numbers: BTreeMap<PresetId, u8>,
    order: Vec<PresetId>,
    sng: Sng,
    patterns: HashMap<Pattern, u8>,
    timing_marks: bool,
}

impl Builder<'_> {
    /// The instrument number for `preset`, placing it and its tables on first use.
    fn instrument(&mut self, preset: &PresetId) -> Result<u8, ExportError> {
        if let Some(&n) = self.numbers.get(preset) {
            return Ok(n);
        }
        let ins = self
            .library
            .get(preset)
            .ok_or_else(|| ExportError::UnknownPreset(preset.clone()))?;
        // Instrument 63 can carry a tempo setting (`docs/reference/goattracker-files.md`), so it stays unused.
        if self.sng.instruments.len() >= MAX_INSTRUMENTS {
            return Err(ExportError::TooMuch(format!(
                "more than {MAX_INSTRUMENTS} instruments"
            )));
        }
        let offsets = Offsets {
            pulse: self.sng.pulse.rows.len(),
            filter: self.sng.filter.rows.len(),
            speed: self.sng.speed.rows.len(),
        };
        let mut i = ins.instrument.clone();
        i.wave = merge(&mut self.sng.wave, &ins.wave, i.wave, Some(&offsets))?;
        i.pulse = merge(&mut self.sng.pulse, &ins.pulse, i.pulse, None)?;
        i.filter = merge(&mut self.sng.filter, &ins.filter, i.filter, None)?;
        i.vibrato = merge_speed(&mut self.sng.speed, &ins.speed, i.vibrato)?;
        self.sng.instruments.push(i);
        let n = self.sng.instruments.len() as u8;
        self.numbers.insert(preset.clone(), n);
        self.order.push(preset.clone());
        Ok(n)
    }

    /// The pattern number for `pattern`, sharing an identical one.
    fn pattern(&mut self, pattern: Pattern) -> Result<u8, ExportError> {
        if let Some(&n) = self.patterns.get(&pattern) {
            return Ok(n);
        }
        if self.sng.patterns.len() >= sng::MAX_PATTERNS {
            return Err(ExportError::TooMuch(format!(
                "more than {} distinct patterns; shorten the song or repeat more of it",
                sng::MAX_PATTERNS
            )));
        }
        let n = self.sng.patterns.len() as u8;
        self.sng.patterns.push(pattern.clone());
        self.patterns.insert(pattern, n);
        Ok(n)
    }
}

/// Where an instrument's pulse, filter and speed tables start in the song's, for the wavetable's command rows.
struct Offsets {
    pulse: usize,
    filter: usize,
    speed: usize,
}

/// Append `from` (a wave, pulse or filter table) to `into` and return where `pointer` (1-based into `from`, 0 for
/// none) now points. `$FF` rows' jump targets move with it. For the wavetable, `wave` also moves the table pointers of
/// its command rows (`$F0`–`$FE`): commands 1–4 and E index the speedtable, 9 the pulsetable and A the filtertable, as
/// gt2reloc relocates them.
fn merge(
    into: &mut Table,
    from: &Table,
    pointer: u8,
    wave: Option<&Offsets>,
) -> Result<u8, ExportError> {
    let offset = into.rows.len();
    if offset + from.rows.len() > 255 {
        return Err(ExportError::TooMuch(
            "the instruments' tables pass 255 rows".into(),
        ));
    }
    let shift = |r: u8, by: usize| if r == 0 { 0 } else { r + by as u8 };
    into.rows
        .extend(from.rows.iter().map(|&(l, r)| match (l, wave) {
            (WAVE_JUMP, _) => (l, shift(r, offset)),
            (0xF1..=0xF4 | 0xFE, Some(o)) => (l, shift(r, o.speed)),
            (0xF9, Some(o)) => (l, shift(r, o.pulse)),
            (0xFA, Some(o)) => (l, shift(r, o.filter)),
            _ => (l, r),
        }));
    Ok(shift(pointer, offset))
}

/// Append a speedtable, which has no jumps, and return where the vibrato pointer now points.
fn merge_speed(into: &mut Table, from: &Table, pointer: u8) -> Result<u8, ExportError> {
    let offset = into.rows.len();
    if offset + from.rows.len() > 255 {
        return Err(ExportError::TooMuch(
            "the instruments' tables pass 255 rows".into(),
        ));
    }
    into.rows.extend_from_slice(&from.rows);
    Ok(if pointer == 0 {
        0
    } else {
        pointer + offset as u8
    })
}

/// Write `song` as a GoatTracker 2 song, with instruments from `library` and timing marks for `adaptive`'s layers.
/// The song should pass the validator first; this checks only what the format itself needs.
pub fn export(
    song: &Song,
    library: &Library,
    adaptive: &[AdaptiveSet],
    header: &Header,
) -> Result<Exported, ExportError> {
    let mut sng = Sng::empty();
    sng.name = sng::header_field(&header.name);
    sng.author = sng::header_field(&header.author);
    sng.copyright = sng::header_field(&header.copyright);
    sng.subtunes.clear();
    sng.patterns.clear();
    let mut b = Builder {
        library,
        numbers: BTreeMap::new(),
        order: Vec::new(),
        sng,
        patterns: HashMap::new(),
        timing_marks: false,
    };
    for subtune in &song.subtunes {
        shape(subtune)?;
        let lists = subtune_rows(&mut b, subtune, adaptive)?;
        let cuts = cuts(subtune);
        let mut voices: [Orderlist; 3] = Default::default();
        for (rows, list) in lists.into_iter().zip(&mut voices) {
            for w in cuts.windows(2) {
                let n = b.pattern(Pattern::new(rows[w[0] as usize..w[1] as usize].to_vec()))?;
                list.entries.push(n);
            }
            list.restart = cuts
                .iter()
                .position(|&c| c == subtune.loop_row)
                .expect("the loop row starts a pattern") as u8;
        }
        b.sng.subtunes.push(voices);
    }
    if b.sng.subtunes.len() > sng::MAX_SUBTUNES {
        return Err(ExportError::TooMuch(format!(
            "{} subtunes; GoatTracker allows {}",
            b.sng.subtunes.len(),
            sng::MAX_SUBTUNES
        )));
    }
    b.sng
        .check()
        .map_err(|e| ExportError::Invalid(e.to_string()))?;
    let instruments = b.order.iter().map(|p| (p.clone(), b.numbers[p])).collect();
    Ok(Exported {
        sng: b.sng,
        instruments,
        timing_marks: b.timing_marks,
    })
}

/// What the export relies on that the validator checks too (rules 1 and 2), refused here rather than written wrong:
/// notes, tempo changes and the loop row inside the subtune, notes on one voice not overlapping, a legato note
/// straight after the note it ties to, and no note sounding across the loop row (after the jump back it couldn't).
fn shape(subtune: &Subtune) -> Result<(), ExportError> {
    let length = subtune.length.get();
    let bad = |why: String| {
        Err(ExportError::Invalid(format!(
            "`{}`: {why}",
            subtune.name.as_str()
        )))
    };
    if subtune.loop_row >= length {
        return bad(format!("the loop row {} is past the end", subtune.loop_row));
    }
    if let Some(t) = subtune.tempo.iter().find(|t| t.row >= length) {
        return bad(format!("a tempo change at row {} is past the end", t.row));
    }
    for line in &subtune.voices {
        let v = line.voice.get();
        let mut notes: Vec<_> = line.notes.iter().collect();
        notes.sort_by_key(|n| n.row);
        let mut sounding_until = None;
        for n in &notes {
            let end = u64::from(n.row) + u64::from(n.length.get());
            if end > u64::from(length) {
                return bad(format!(
                    "voice {v}: a note at row {} runs past the end",
                    n.row
                ));
            }
            if sounding_until.is_some_and(|e| u64::from(n.row) < e) {
                return bad(format!("voice {v}: notes overlap at row {}", n.row));
            }
            if n.attack == Attack::Legato && sounding_until != Some(u64::from(n.row)) {
                return bad(format!(
                    "voice {v}: the legato note at row {} has no note to tie to",
                    n.row
                ));
            }
            if n.row < subtune.loop_row && end > u64::from(subtune.loop_row) {
                return bad(format!(
                    "voice {v}: a note sounds across the loop row {}; start one on it or end it before",
                    subtune.loop_row
                ));
            }
            sounding_until = Some(end);
        }
    }
    Ok(())
}

/// Where patterns start and end: every bar start and the loop row, with pieces longer than 128 rows split, and the
/// subtune's length last.
fn cuts(subtune: &Subtune) -> Vec<u32> {
    let length = subtune.length.get();
    let bar = u32::from(subtune.rows_per_bar.get());
    let mut starts: Vec<u32> = (0..length).step_by(bar as usize).collect();
    if subtune.loop_row < length {
        starts.push(subtune.loop_row);
    }
    starts.sort_unstable();
    starts.dedup();
    let mut cuts = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(length);
        cuts.extend((s..end).step_by(sng::MAX_ROWS));
    }
    cuts.push(length);
    cuts
}

/// The three voices' rows for one subtune, with every note, arpeggio, tempo and timing mark in place.
fn subtune_rows(
    b: &mut Builder<'_>,
    subtune: &Subtune,
    adaptive: &[AdaptiveSet],
) -> Result<[Rows; 3], ExportError> {
    let length = subtune.length.get() as usize;
    let rest = Row {
        note: REST,
        ..Row::default()
    };
    let mut voices: [Rows; 3] = std::array::from_fn(|_| vec![rest; length]);
    for line in &subtune.voices {
        voice_rows(
            b,
            subtune,
            line,
            &mut voices[usize::from(line.voice.get()) - 1],
        )?;
    }
    // Song-wide commands, each on the first voice with a free slot on its row.
    let mut global: Vec<(u32, u8, u8, String)> = vec![(
        0,
        TEMPO,
        subtune.frames_per_row.get(),
        "the tempo".to_string(),
    )];
    global.extend(subtune.tempo.iter().map(|t| {
        (
            t.row,
            TEMPO,
            t.frames_per_row.get(),
            "a tempo change".to_string(),
        )
    }));
    // After the jump back the tempo is whatever played last, so the loop row sets its own if that differs.
    let at_loop = subtune.frames_per_row_at(subtune.loop_row);
    let at_end = subtune.frames_per_row_at(subtune.length.get() - 1);
    if subtune.loop_row > 0
        && at_loop != at_end
        && !subtune.tempo.iter().any(|t| t.row == subtune.loop_row)
    {
        global.push((
            subtune.loop_row,
            TEMPO,
            at_loop.get(),
            "the tempo after the loop".to_string(),
        ));
    }
    if let Some(set) = adaptive.iter().find(|s| s.layers().contains(&subtune.name)) {
        let marks = set.timing_marks(subtune).marks;
        b.timing_marks |= !marks.is_empty();
        global.extend(
            marks
                .into_iter()
                .map(|m| (m.row, TIMING_MARK, m.value, "a timing mark".to_string())),
        );
    }
    for (row, command, data, what) in global {
        let r = row as usize;
        let free = voices.iter_mut().find(|v| v[r].command == 0);
        match free {
            Some(v) => {
                v[r].command = command;
                v[r].data = data;
            }
            None => {
                return Err(ExportError::CommandClash {
                    subtune: subtune.name.clone(),
                    voice: None,
                    row,
                    what,
                });
            }
        }
    }
    Ok(voices)
}

/// Put one voice line's notes and arpeggios into its rows.
fn voice_rows(
    b: &mut Builder<'_>,
    subtune: &Subtune,
    line: &VoiceLine,
    rows: &mut Rows,
) -> Result<(), ExportError> {
    let length = rows.len();
    let clash = |row: usize, what: &str| ExportError::CommandClash {
        subtune: subtune.name.clone(),
        voice: Some(line.voice),
        row: row as u32,
        what: what.to_string(),
    };
    let mut notes: Vec<_> = line.notes.iter().collect();
    notes.sort_by_key(|n| n.row);
    // The instrument each note plays with: its own, or its line's; a tie keeps the one it ties to.
    let mut playing = Vec::with_capacity(notes.len());
    for (k, note) in notes.iter().enumerate() {
        let r = note.row as usize;
        rows[r].note = NOTE_FIRST + note.pitch.semitone();
        match note.attack {
            Attack::Fresh => {
                let preset = note.instrument.as_ref().unwrap_or(&line.instrument);
                let instrument = b.instrument(preset)?;
                rows[r].instrument = instrument;
                playing.push((preset.clone(), instrument));
            }
            Attack::Legato => {
                rows[r].command = TIE;
                rows[r].data = 0;
                let before = playing
                    .last()
                    .cloned()
                    .expect("shape: a tie follows a note");
                playing.push(before);
            }
        }
        let end = r + note.length.get() as usize;
        let next = notes.get(k + 1).map(|n| n.row as usize);
        if end < length && next != Some(end) {
            rows[end].note = KEY_OFF;
        }
    }
    // A note sounding at the end would sound on after the jump back; a key-off on the loop row stops it, unless a
    // note starts there (shape: none sounds across it).
    let loop_row = subtune.loop_row as usize;
    let at_end = notes
        .last()
        .is_some_and(|n| (n.row + n.length.get()) as usize == length);
    if at_end && rows[loop_row].note == REST {
        rows[loop_row].note = KEY_OFF;
    }
    let mut arps: Vec<_> = line.arps.iter().collect();
    arps.sort_by_key(|a| a.row);
    for (k, arp) in arps.iter().enumerate() {
        let (from, to) = (arp.row as usize, (arp.row + arp.length.get()) as usize);
        let highest = arp.intervals.iter().copied().max().map_or(0, u16::from);
        // Every note that sounds during the event: the one under its start, and each fresh note inside it, whose
        // start resets the wave pointer. A tie keeps the program running.
        for (n, (preset, instrument)) in notes.iter().zip(&playing) {
            let (start, end) = (n.row as usize, (n.row + n.length.get()) as usize);
            if end <= from || start >= to {
                continue;
            }
            if u16::from(n.pitch.semitone()) + highest > HIGHEST_NOTE {
                return Err(ExportError::TooHigh {
                    subtune: subtune.name.clone(),
                    voice: line.voice,
                    row: n.row.max(arp.row),
                });
            }
            let r = start.max(from);
            if r > from && n.attack == Attack::Legato {
                continue;
            }
            let own = b.sng.instruments[usize::from(*instrument) - 1].wave;
            let waveform = (own > 0)
                .then(|| b.sng.wave.rows[usize::from(own) - 1].0)
                .filter(|&w| {
                    wave::WAVEFORMS.contains(&w) && w & wave::GATE != 0 && w & wave::TEST == 0
                })
                .ok_or_else(|| ExportError::NoArpWaveform(preset.clone()))?;
            let program = wave::arpeggio(
                &mut b.sng.wave,
                waveform,
                &arp.intervals,
                NonZeroU8::new(1).expect("one"),
            )
            .map_err(|e| ExportError::TooMuch(e.to_string()))?;
            if rows[r].command != 0 {
                return Err(clash(
                    r,
                    "an arpeggio and another command (a tie, or another arpeggio)",
                ));
            }
            rows[r].command = SET_WAVE_POINTER;
            rows[r].data = program;
        }
        // Back to the instrument's own program where the event ends, if a note sounds on there and nothing else sets
        // the pointer: no note starts there, and no other arpeggio does.
        let sounding = notes
            .iter()
            .zip(&playing)
            .find(|(n, _)| (n.row as usize) < to && to < (n.row + n.length.get()) as usize);
        let next_arp = arps.get(k + 1).is_some_and(|a| a.row as usize == to);
        if let Some((_, (_, instrument))) = sounding
            && to < length
            && !next_arp
        {
            if rows[to].command != 0 {
                return Err(clash(to, "an arpeggio's end and another command"));
            }
            rows[to].command = SET_WAVE_POINTER;
            rows[to].data = b.sng.instruments[usize::from(*instrument) - 1].wave;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
