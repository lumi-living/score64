//! The tracker-grid score (ADR-006): notes on integer rows, one line per SID voice, subtunes with a frames-per-row
//! tempo. Time in the score is always rows; seconds appear only in [`SourceRef`](crate::SourceRef), which points back
//! into source audio.

use std::fmt;
use std::num::{NonZeroU8, NonZeroU32};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::profile::Profile;
use crate::provenance::{Confidence, DecisionId, SourceRef, valid_id};

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
#[serde(try_from = "SongFields", into = "SongFields")]
pub struct Song {
    /// Layout version; always [`SCHEMA_VERSION`].
    pub schema: Schema,
    /// The voice profile of every subtune that does not name its own (`docs/TAD.md` §3.1).
    pub profile: Profile,
    /// Subtunes, with unique names. The position is the number the game passes to the player's init in A
    /// (0-based); everything else in the project refers to a subtune by name, so reordering cannot re-target it.
    pub subtunes: Vec<Subtune>,
}

impl Song {
    /// The profile subtune `subtune` plays under: its own, or the song's.
    pub fn profile_of(&self, subtune: &Subtune) -> Profile {
        subtune.profile.unwrap_or(self.profile)
    }

    /// The subtune called `name`, with its position.
    pub fn subtune(&self, name: &SubtuneName) -> Option<(usize, &Subtune)> {
        self.subtunes
            .iter()
            .enumerate()
            .find(|(_, s)| &s.name == name)
    }
}

/// [`Song`] as written in files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct SongFields {
    /// Layout version.
    schema: Schema,
    /// Default voice profile.
    profile: Profile,
    /// Subtunes, with unique names; the position is the player's subtune number.
    subtunes: Vec<Subtune>,
}

impl TryFrom<SongFields> for Song {
    type Error = String;
    fn try_from(f: SongFields) -> Result<Self, String> {
        for (i, a) in f.subtunes.iter().enumerate() {
            if f.subtunes[..i].iter().any(|b| b.name == a.name) {
                return Err(format!("two subtunes are called {:?}", a.name.as_str()));
            }
        }
        Ok(Song {
            schema: f.schema,
            profile: f.profile,
            subtunes: f.subtunes,
        })
    }
}

impl From<Song> for SongFields {
    fn from(s: Song) -> Self {
        SongFields {
            schema: s.schema,
            profile: s.profile,
            subtunes: s.subtunes,
        }
    }
}

/// A subtune's stable name, which decisions and project files use (for example `bed`, `climax`, `victory`): 1–64
/// characters of `a`–`z`, `0`–`9`, `-` and `_`, starting with a letter or digit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SubtuneName(String);

impl SubtuneName {
    /// `s`, if it is a valid name.
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        valid_id(&s).then_some(SubtuneName(s))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SubtuneName {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let shown = format!("{s:?}");
        SubtuneName::new(s).ok_or_else(|| {
            format!("{shown} is not a subtune name (1–64 of a-z, 0-9, - and _, starting with a letter or digit)")
        })
    }
}

impl From<SubtuneName> for String {
    fn from(n: SubtuneName) -> String {
        n.0
    }
}

/// One subtune: a grid of rows at a tempo, with one line per voice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Subtune {
    /// Stable name, unique in the song.
    pub name: SubtuneName,
    /// This subtune's voice profile, when it is not the song's. A mission set mixes stages with different voice rules
    /// in one packed file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    /// Frames (50 Hz player calls) per row at row 0.
    pub frames_per_row: FramesPerRow,
    /// Tempo changes after row 0, in row order.
    pub tempo: Vec<TempoChange>,
    /// Rows per bar (16 for 4/4 at four rows per beat). Bars place decisions and timing marks.
    pub rows_per_bar: NonZeroU8,
    /// Length in rows.
    pub length: NonZeroU32,
    /// Row the subtune loops back to after its last row.
    pub loop_row: u32,
    /// Rows that carry a timing mark. The player has one mark byte (start+$3F with `-H1`), which the game reads per
    /// subtune, so marks belong to the subtune, not to a voice (`docs/reference/player.md`).
    pub marks: Vec<u32>,
    /// Voice lines, at most one per SID voice.
    pub voices: Vec<VoiceLine>,
}

/// Frames per row: a GoatTracker tempo from 3 to 127. Values below 3 select funktempo in GoatTracker, which the
/// score does not model yet, and a hard restart needs the frames anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct FramesPerRow(u8);

impl FramesPerRow {
    /// The fastest tempo.
    pub const MIN: FramesPerRow = FramesPerRow(3);
    /// The slowest tempo.
    pub const MAX: FramesPerRow = FramesPerRow(127);

    /// `n` frames per row, if GoatTracker plays it as a plain tempo.
    pub const fn new(n: u8) -> Option<Self> {
        if n >= Self::MIN.0 && n <= Self::MAX.0 {
            Some(FramesPerRow(n))
        } else {
            None
        }
    }

    /// Frames per row.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for FramesPerRow {
    type Error = String;
    fn try_from(n: u8) -> Result<Self, String> {
        FramesPerRow::new(n).ok_or_else(|| {
            format!("frames per row must be 3 to 127, not {n} (below 3 is GoatTracker's funktempo, not modelled)")
        })
    }
}

impl From<FramesPerRow> for u8 {
    fn from(f: FramesPerRow) -> u8 {
        f.0
    }
}

/// A tempo change on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TempoChange {
    /// The first row at the new tempo.
    pub row: u32,
    /// Frames per row from here on.
    pub frames_per_row: FramesPerRow,
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
    /// Its roles over time: a voice can change role at any row (usually a bar line), as `voice_role` decisions do.
    pub roles: Roles,
    /// The instrument its notes use unless a note names its own.
    pub instrument: PresetId,
    /// Notes in row order. The validator checks they do not overlap (`docs/TAD.md` §1.3 rule 1).
    pub notes: Vec<Note>,
    /// Wavetable arpeggios, in row order.
    pub arps: Vec<ArpEvent>,
}

impl VoiceLine {
    /// The instrument `note` plays with: its own, or the line's.
    pub fn instrument_of<'a>(&'a self, note: &'a Note) -> &'a PresetId {
        note.instrument.as_ref().unwrap_or(&self.instrument)
    }
}

/// A role from a row on, until the next span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RoleSpan {
    /// The first row in this role.
    pub row: u32,
    /// The role.
    pub role: Role,
}

/// A voice line's roles: spans in strictly increasing row order, the first at row 0, each lasting until the next.
/// So every row has exactly one role, with no gaps or overlaps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "Vec<RoleSpan>", into = "Vec<RoleSpan>")]
pub struct Roles(Vec<RoleSpan>);

impl Roles {
    /// One role for the whole line.
    pub fn constant(role: Role) -> Self {
        Roles(vec![RoleSpan { row: 0, role }])
    }

    /// `spans`, if the first is at row 0 and rows strictly increase.
    pub fn new(spans: Vec<RoleSpan>) -> Option<Self> {
        let ordered = spans.windows(2).all(|w| w[0].row < w[1].row);
        (spans.first().is_some_and(|s| s.row == 0) && ordered).then_some(Roles(spans))
    }

    /// The role at `row`.
    pub fn at(&self, row: u32) -> Role {
        let i = self.0.partition_point(|s| s.row <= row);
        self.0[i - 1].role
    }

    /// The spans.
    pub fn spans(&self) -> &[RoleSpan] {
        &self.0
    }
}

impl TryFrom<Vec<RoleSpan>> for Roles {
    type Error = String;
    fn try_from(spans: Vec<RoleSpan>) -> Result<Self, String> {
        Roles::new(spans)
            .ok_or_else(|| "roles must start at row 0, with rows in increasing order".into())
    }
}

impl From<Roles> for Vec<RoleSpan> {
    fn from(r: Roles) -> Self {
        r.0
    }
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
    /// This note's instrument, when it is not the line's (a drum line's kick, snare and hi-hat, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<PresetId>,
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
