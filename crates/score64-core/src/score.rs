//! The tracker-grid score (ADR-006): notes on integer rows, one line per SID voice, subtunes with a frames-per-row
//! tempo. Time in the score is always rows; seconds appear only in [`SourceRef`](crate::SourceRef), which points back
//! into source audio.

use std::fmt;
use std::num::{NonZeroU8, NonZeroU32};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, DecisionId, SourceRef};

/// The version of the score's file layout this code reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// The schema version field of a [`Song`]. Only [`SCHEMA_VERSION`] deserialises: a file from a newer Score64 is an
/// error, not a silent partial read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Schema(u32);

impl Default for Schema {
    fn default() -> Self {
        Schema(SCHEMA_VERSION)
    }
}

impl TryFrom<u32> for Schema {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        if v == SCHEMA_VERSION {
            Ok(Schema(v))
        } else {
            Err(format!(
                "score schema {v} is not supported; this build reads {SCHEMA_VERSION}"
            ))
        }
    }
}

impl From<Schema> for u32 {
    fn from(s: Schema) -> u32 {
        s.0
    }
}

/// A whole song: one or more subtunes under one voice profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Song {
    /// Layout version; always [`SCHEMA_VERSION`].
    pub schema: Schema,
    /// The voice profile every subtune follows (rules in `docs/TAD.md` §3.1).
    pub profile: Profile,
    /// Subtunes; the position is the subtune number the player's init takes in A (0-based).
    pub subtunes: Vec<Subtune>,
}

/// Voice profile names (`docs/TAD.md` §3.1). The rules themselves live with the validator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Three voices, any role on any voice.
    Menu,
    /// V1 lead, V2 bass, V3 expendable for sound effects.
    Combat,
    /// Two voices; V3 silent, reserved for sound effects.
    Combat2,
    /// Layered subtunes: bed, drone, ostinato, climax, stings.
    Mission,
}

/// One subtune: a grid of rows at a tempo, with one line per voice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Subtune {
    /// Frames (50 Hz player calls) per row at row 0.
    pub frames_per_row: NonZeroU8,
    /// Tempo changes after row 0, in row order.
    pub tempo: Vec<TempoChange>,
    /// Rows per bar (16 for 4/4 at four rows per beat). Bars place decisions and timing marks.
    pub rows_per_bar: NonZeroU8,
    /// Length in rows.
    pub length: NonZeroU32,
    /// Row the subtune loops back to after its last row.
    pub loop_row: u32,
    /// Voice lines, at most one per SID voice.
    pub voices: Vec<VoiceLine>,
}

/// A tempo change on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TempoChange {
    /// The first row at the new tempo.
    pub row: u32,
    /// Frames per row from here on.
    pub frames_per_row: NonZeroU8,
}

/// SID voice 1, 2 or 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Voice(u8);

impl Voice {
    /// Voice `n` (1–3).
    pub const fn new(n: u8) -> Option<Self> {
        if n >= 1 && n <= 3 {
            Some(Voice(n))
        } else {
            None
        }
    }

    /// The voice number, 1–3.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Voice {
    type Error = String;
    fn try_from(n: u8) -> Result<Self, String> {
        Voice::new(n).ok_or_else(|| format!("voice must be 1, 2 or 3, not {n}"))
    }
}

impl From<Voice> for u8 {
    fn from(v: Voice) -> u8 {
        v.0
    }
}

/// What a voice line does in the arrangement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The melody.
    Lead,
    /// The bass line.
    Bass,
    /// Chords as wavetable arpeggios.
    Arp,
    /// Drums.
    Drums,
    /// A held note.
    Drone,
    /// A repeated figure.
    Ostinato,
    /// A quiet background part.
    Bed,
    /// Noise and effects-like material.
    Texture,
}

/// One SID voice's part in a subtune.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VoiceLine {
    /// The SID voice.
    pub voice: Voice,
    /// Its role.
    pub role: Role,
    /// The instrument preset its notes use.
    pub instrument: PresetId,
    /// Notes in row order. The validator checks they do not overlap (`docs/TAD.md` §1.3 rule 1).
    pub notes: Vec<Note>,
    /// Wavetable arpeggios, in row order.
    pub arps: Vec<ArpEvent>,
    /// Rows that carry a timing mark (`docs/reference/player.md`).
    pub marks: Vec<u32>,
}

/// An instrument preset name (`presets/*.toml`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct PresetId(pub String);

/// One note on the grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// Start row.
    pub row: u32,
    /// Length in rows. The gate goes off at `row + length` unless the next note is [`Attack::Legato`].
    pub length: NonZeroU32,
    /// Pitch.
    pub pitch: Pitch,
    /// Whether the note gates a fresh attack or ties on from the note before.
    pub attack: Attack,
    /// Where in the source audio this note came from; absent for notes made by hand or by a rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    /// How sure the transcription or rule was.
    pub confidence: Confidence,
    /// The decision that put this note here, if one did. Notes point at decisions, never the other way round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<DecisionId>,
}

/// How a note starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Attack {
    /// A new gate: the player hard-restarts before the note (gate off a few frames early), then gates it on.
    Fresh,
    /// Tied to the note before, which must end on this note's row: the gate stays on and only the pitch changes.
    Legato,
}

/// A wavetable arpeggio over a span of rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ArpEvent {
    /// Start row.
    pub row: u32,
    /// Length in rows.
    pub length: NonZeroU32,
    /// Semitones above the note, one per step, starting with 0 (for example 0, 4, 7 for a major chord).
    pub intervals: Vec<u8>,
    /// The decision that chose this arpeggio, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<DecisionId>,
}

/// A pitch GoatTracker patterns can play: C-0 to G#7, semitones 0–92 (pattern note bytes `$60`–`$BC`).
/// Written in files as GoatTracker names it: `"C-0"`, `"C#0"`, …, `"G#7"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Pitch(u8);

const NAMES: [&str; 12] = [
    "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
];

impl Pitch {
    /// The lowest playable pitch, C-0.
    pub const MIN: Pitch = Pitch(0);
    /// The highest playable pitch, G#7.
    pub const MAX: Pitch = Pitch(92);

    /// The pitch `semitone` semitones above C-0, if GoatTracker can play it.
    pub const fn new(semitone: u8) -> Option<Self> {
        if semitone <= Self::MAX.0 {
            Some(Pitch(semitone))
        } else {
            None
        }
    }

    /// Semitones above C-0.
    pub const fn semitone(self) -> u8 {
        self.0
    }

    /// From a MIDI note number (A4 = 69 is GoatTracker's A-4), if it is in range.
    pub const fn from_midi(midi: u8) -> Option<Self> {
        match midi.checked_sub(12) {
            Some(s) => Self::new(s),
            None => None,
        }
    }

    /// The MIDI note number.
    pub const fn midi(self) -> u8 {
        self.0 + 12
    }
}

impl fmt::Display for Pitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", NAMES[usize::from(self.0 % 12)], self.0 / 12)
    }
}

impl FromStr for Pitch {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let bad =
            || format!("{s:?} is not a pitch from C-0 to G#7 (GoatTracker names: C-4, C#4, …)");
        if s.len() != 3 || !s.is_ascii() {
            return Err(bad());
        }
        let class = NAMES.iter().position(|n| *n == &s[..2]).ok_or_else(bad)?;
        let octave = s[2..].parse::<u8>().map_err(|_| bad())?;
        Pitch::new(octave * 12 + class as u8).ok_or_else(bad)
    }
}

impl TryFrom<String> for Pitch {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

impl From<Pitch> for String {
    fn from(p: Pitch) -> String {
        p.to_string()
    }
}
