//! Where score data came from: source audio references and decisions (ADR-004, ADR-008).
//!
//! A [`Decision`] holds a proposed change and who proposed it. It never changes the song by itself: an engine command
//! validates a decision and applies it, and the notes it produces record its [`DecisionId`]. Nothing in a decision
//! points at notes.

use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::score::{FramesPerRow, Pitch, Role, SubtuneName, Voice};

/// A blake3 hash: the identity of a file in the content-addressed cache (ADR-004). Written as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// From 64 hex digits (either case).
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, &[hi, lo]) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let digit = |c: u8| (c as char).to_digit(16);
            out[i] = (digit(hi)? * 16 + digit(lo)?) as u8;
        }
        Some(ContentHash(out))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl TryFrom<String> for ContentHash {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        ContentHash::from_hex(&s)
            .ok_or_else(|| format!("{s:?} is not a blake3 hash (64 hex digits)"))
    }
}

impl From<ContentHash> for String {
    fn from(h: ContentHash) -> String {
        h.to_string()
    }
}

/// A span of source audio: the source file or stem by content hash, and a time range in it. Survives renames and
/// matches the stage cache's identities (ADR-004). The only place in the score that holds wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "SourceRefFields", into = "SourceRefFields")]
pub struct SourceRef {
    content: ContentHash,
    start_us: u64,
    end_us: u64,
}

impl SourceRef {
    /// `start_us..end_us` (microseconds from the start of the file) of the file with hash `content`; the range must
    /// not be empty.
    pub fn new(content: ContentHash, start_us: u64, end_us: u64) -> Option<Self> {
        (start_us < end_us).then_some(SourceRef {
            content,
            start_us,
            end_us,
        })
    }

    /// The source file or stem.
    pub fn content(&self) -> ContentHash {
        self.content
    }

    /// Start, in microseconds from the start of the file.
    pub fn start_us(&self) -> u64 {
        self.start_us
    }

    /// End (exclusive), in microseconds from the start of the file.
    pub fn end_us(&self) -> u64 {
        self.end_us
    }
}

/// [`SourceRef`] as written in files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct SourceRefFields {
    /// blake3 of the source file or stem.
    content: ContentHash,
    /// Start, in microseconds from the start of the file.
    start_us: u64,
    /// End (exclusive), in microseconds; after `start_us`.
    end_us: u64,
}

impl TryFrom<SourceRefFields> for SourceRef {
    type Error = String;
    fn try_from(f: SourceRefFields) -> Result<Self, String> {
        SourceRef::new(f.content, f.start_us, f.end_us)
            .ok_or_else(|| format!("source range {}..{} µs is empty", f.start_us, f.end_us))
    }
}

impl From<SourceRef> for SourceRefFields {
    fn from(r: SourceRef) -> Self {
        SourceRefFields {
            content: r.content,
            start_us: r.start_us,
            end_us: r.end_us,
        }
    }
}

/// How sure a transcription, rule or model was: a finite number from 0 to 1. Written with the fewest digits that
/// read back to the same value (`0.1`, not `0.10000000149011612`).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Confidence(f32);

impl Confidence {
    /// Certain.
    pub const CERTAIN: Confidence = Confidence(1.0);

    /// `v`, if it is in 0..=1.
    pub fn new(v: f32) -> Option<Self> {
        (0.0..=1.0).contains(&v).then_some(Confidence(v))
    }

    /// The value, 0–1.
    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f64> for Confidence {
    type Error = String;
    fn try_from(v: f64) -> Result<Self, String> {
        Confidence::new(v as f32).ok_or_else(|| format!("confidence must be from 0 to 1, not {v}"))
    }
}

impl From<Confidence> for f64 {
    fn from(c: Confidence) -> f64 {
        // f32's shortest round-trip decimal, read as f64: what a person would write.
        c.0.to_string().parse().expect("a formatted f32 parses")
    }
}

/// A decision's stable name, as notes and `decisions.toml` refer to it: 1–64 characters of `a`–`z`, `0`–`9`, `-`
/// and `_`, starting with a letter or digit. Unique within its subtune: a note refers to a decision of its own
/// subtune.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DecisionId(String);

impl DecisionId {
    /// `s`, if it is a valid id.
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        valid_id(&s).then_some(DecisionId(s))
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DecisionId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let shown = format!("{s:?}");
        DecisionId::new(s).ok_or_else(|| {
            format!("{shown} is not a decision id (1–64 of a-z, 0-9, - and _, starting with a letter or digit)")
        })
    }
}

impl From<DecisionId> for String {
    fn from(d: DecisionId) -> String {
        d.0
    }
}

/// 1–64 of `a`–`z`, `0`–`9`, `-` and `_`, starting with a letter or digit: decision ids and subtune names.
pub(crate) fn valid_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && s.as_bytes()[0].is_ascii_alphanumeric()
}

/// One judgement call (`docs/TAD.md` §3.3): a proposed change at a place in the song, who proposed it, how sure they
/// were, and whether a person has reviewed it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// Stable name.
    pub id: DecisionId,
    /// The subtune, by name: stable when subtunes are added or reordered.
    pub subtune: SubtuneName,
    /// Bar (0-based) the decision is about.
    pub bar: u32,
    /// The change proposed. Applying it is an engine command's job, after validation.
    pub proposal: Proposal,
    /// Who or what proposed it.
    pub provenance: Provenance,
    /// How sure the proposer was.
    pub confidence: Confidence,
    /// Whether a person has reviewed it. Model decisions start unreviewed (ADR-008).
    pub reviewed: bool,
}

impl Decision {
    /// Whether this decision still needs a person: a model proposed it and nobody has reviewed it.
    pub fn needs_review(&self) -> bool {
        !self.reviewed && matches!(self.provenance, Provenance::Llm { .. })
    }
}

/// Who proposed a decision. Manual decisions override model ones, which override rules (`docs/TAD.md` §3.3).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum Provenance {
    /// A reduction rule.
    Rules {
        /// The rule's name and version, for example `"skyline@1"`.
        rule: String,
    },
    /// A language model, through the LLM decider (ADR-008).
    Llm {
        /// The model id.
        model: String,
        /// blake3 of the exact prompt, which is also the decision cache key.
        prompt: ContentHash,
    },
    /// A person, in the app or in `decisions.toml`.
    Manual,
}

/// A change a decision proposes. Each variant is one kind of decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum Proposal {
    /// Give a voice a role.
    VoiceRole {
        /// The voice.
        voice: Voice,
        /// Its role.
        role: Role,
    },
    /// Take the lead from this source part (lead selection).
    LeadSource {
        /// The stem or transcribed part, by content hash.
        source: ContentHash,
    },
    /// Shift a voice's notes in this bar by whole octaves (octave folding).
    Octave {
        /// The voice.
        voice: Voice,
        /// Octaves, negative for down.
        shift: i8,
    },
    /// The chord in this bar, for the arpeggio.
    Chord {
        /// Root as a pitch class, 0 (C) to 11 (B).
        root: u8,
        /// Semitones above the root, starting with 0.
        intervals: Vec<u8>,
    },
    /// Loop the subtune back to this row.
    LoopRow {
        /// The loop row.
        row: u32,
    },
    /// Snap the tempo to this many frames per row.
    FramesPerRow {
        /// Frames per row.
        frames_per_row: FramesPerRow,
    },
    /// Shorten a note so it ends where the next one on its voice starts (monophonise).
    TrimNote {
        /// The voice.
        voice: Voice,
        /// The note's start row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
        /// Its new length, in rows.
        length: NonZeroU32,
        /// Where in the source it came from, if it did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<SourceRef>,
    },
    /// Leave a note out of a voice (monophonise).
    DropNote {
        /// The voice.
        voice: Voice,
        /// The note's start row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
        /// Its length, in rows, which tells it from a repeat on the same row that was kept.
        length: NonZeroU32,
        /// Where in the source it came from, if it did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<SourceRef>,
        /// Why.
        reason: DropReason,
    },
    /// Start a note with a fresh attack: the note it was tied to no longer ends on its row (monophonise).
    Retrigger {
        /// The voice.
        voice: Voice,
        /// The note's start row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
        /// Where in the source it came from, if it did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<SourceRef>,
    },
}

/// Why a [`Proposal::DropNote`] leaves a note out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// Its confidence is below the ghost-note threshold.
    Ghost,
    /// It is shorter than the minimum note length.
    TooShort,
    /// It repeats a note of the same pitch on the same row, which keeps the longer of the two.
    Repeat,
    /// Another note starts on the same row, and the skyline keeps that one (the highest, or the lowest for bass).
    Skyline,
    /// It overlaps its neighbour too closely to trim to the minimum length, and has the lower confidence.
    Overlap,
    /// Another note on the same row wins: a higher-priority drum hit, or the line drums are interleaved with.
    Clash,
    /// A drum hit whose kind has no sound in the project's drum kit.
    Unmapped,
    /// A hi-hat left out when drums share a voice with another line.
    Interleave,
}
