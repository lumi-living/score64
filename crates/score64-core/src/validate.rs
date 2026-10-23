//! The validator for `docs/TAD.md` §1.3: checks a song against the SID, the player and its subtunes' voice profiles,
//! and reports every problem with its subtune, bar, voice and a hint for fixing it.
//!
//! [`validate`] covers rules 1–6 and 9 from the song plus the instrument facts it needs. Rule 8 (packed size and zero page)
//! is [`check_pack`], run after packing. Rule 7 (sound-effect data) waits for the sound-effect model (E13).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::adaptive::{AdaptiveSet, LayerProblem};
use crate::profile::{Profile, ProfileName, Profiles, VoiceUse};
use crate::score::{
    Attack, Note, Pitch, PresetId, Role, Song, Subtune, SubtuneName, Voice, VoiceLine,
};

/// Which TAD §1.3 rule a problem breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// 1: one note at a time per voice (one line per voice, no overlaps, legato only straight after a note).
    Monophony,
    /// 2: everything on the subtune's row grid (inside its length, in order).
    Grid,
    /// 3: music only on the profile's voices, in roles the profile allows there.
    Voices,
    /// 4: the filter only where the profile allows it, and explicit pulse widths on expendable voices (errors);
    /// short notes and no lead or bass on expendable voices (warnings).
    FilterAndExpendable,
    /// 5: declared adaptive sets: layers present, in order at consecutive positions, a timing mark at every switch
    /// point the set declares.
    TimingMarks,
    /// 6: each note inside its role's pitch range.
    PitchRange,
    /// 7: sound-effect data in the format's rules (checked once effects have a model, E13).
    Effects,
    /// 8: packed size and zero page within the budget.
    Pack,
    /// 9: hard restart: gate timers below every row's tempo, and a gate-off that leaves the previous note sounding.
    HardRestart,
}

impl Rule {
    /// The rule's number in TAD §1.3.
    pub const fn number(self) -> u8 {
        match self {
            Rule::Monophony => 1,
            Rule::Grid => 2,
            Rule::Voices => 3,
            Rule::FilterAndExpendable => 4,
            Rule::TimingMarks => 5,
            Rule::PitchRange => 6,
            Rule::Effects => 7,
            Rule::Pack => 8,
            Rule::HardRestart => 9,
        }
    }
}

/// How serious a problem is. Front-ends fail on errors only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Something the player or the format gets wrong: the song cannot ship like this.
    Error,
    /// A matter of taste or arrangement practice: worth a look, but the song works.
    Warning,
}

/// Whether any of `problems` is an error.
pub fn has_errors(problems: &[Problem]) -> bool {
    problems.iter().any(Problem::is_error)
}

/// One thing wrong with a song, located as precisely as the rule allows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Problem {
    /// The rule broken.
    pub rule: Rule,
    /// Error or warning.
    pub severity: Severity,
    /// The subtune, when the problem is in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtune: Option<SubtuneName>,
    /// The bar (0-based), when the problem is at a place in the subtune.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,
    /// The row, when the problem is at a place in the subtune.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u32>,
    /// The voice, when the problem is on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<Voice>,
    /// What is wrong.
    pub message: String,
    /// How to fix it.
    pub hint: String,
}

impl Problem {
    /// Whether it is an error (not a warning).
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{severity}, rule {}", self.rule.number())?;
        if let Some(s) = &self.subtune {
            write!(f, ", {}", s.as_str())?;
        }
        if let Some(b) = self.bar {
            write!(f, ", bar {}", b + 1)?;
        }
        if let Some(v) = self.voice {
            write!(f, ", V{}", v.get())?;
        }
        write!(f, ": {} ({})", self.message, self.hint)
    }
}

/// What the validator needs to know about an instrument preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InstrumentFacts {
    /// Its filter table routes the voice through the filter.
    pub filter: bool,
    /// It sets a pulse width when a note starts (its pulse table's first step), rather than inheriting one.
    pub pulse_at_start: bool,
    /// GoatTracker's HR/gate timer (0–63): how many frames before the next row the player fetches that row and,
    /// if it holds a new note, turns the gate off. It must be less than the frames per row on every row this
    /// instrument plays.
    pub gate_timer: u8,
    /// Whether that gate-off also hard-restarts (gate-timer bit `$80` clear): the player writes the hard-restart
    /// ADSR (gt2reloc `-A`). It does not change when the gate goes off.
    pub hard_restart: bool,
    /// Whether a new note with this instrument turns the gate off before it at all (gate-timer bit `$40` clear).
    /// Without it the instrument is a legato instrument: no gate-off and no hard restart before its notes.
    pub gate_off: bool,
}

/// An inclusive pitch range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PitchRange {
    /// Lowest pitch.
    pub low: Pitch,
    /// Highest pitch.
    pub high: Pitch,
}

impl PitchRange {
    const fn of(low: u8, high: u8) -> Self {
        match (Pitch::new(low), Pitch::new(high)) {
            (Some(low), Some(high)) => PitchRange { low, high },
            _ => panic!("pitch out of range"),
        }
    }

    /// Whether `p` is inside.
    pub fn contains(&self, p: Pitch) -> bool {
        (self.low..=self.high).contains(&p)
    }
}

/// The pitch range of each role, after octave folding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RoleRanges {
    /// Lead.
    pub lead: PitchRange,
    /// Bass.
    pub bass: PitchRange,
    /// Arpeggio (the chord's root note).
    pub arp: PitchRange,
    /// Harmony.
    pub harmony: PitchRange,
    /// Drums.
    pub drums: PitchRange,
    /// Drone.
    pub drone: PitchRange,
    /// Ostinato.
    pub ostinato: PitchRange,
    /// Bed.
    pub bed: PitchRange,
    /// Texture.
    pub texture: PitchRange,
}

impl RoleRanges {
    /// The range for `role`.
    pub fn of(&self, role: Role) -> PitchRange {
        match role {
            Role::Lead => self.lead,
            Role::Bass => self.bass,
            Role::Arp => self.arp,
            Role::Harmony => self.harmony,
            Role::Drums => self.drums,
            Role::Drone => self.drone,
            Role::Ostinato => self.ostinato,
            Role::Bed => self.bed,
            Role::Texture => self.texture,
        }
    }
}

impl Default for RoleRanges {
    /// Starting points for C64 arrangements (bass C-1 to C-3, lead C-3 to C-6, as SID practice goes), to tune
    /// against real material. Drums and texture use noise and pitch sweeps, so they may use the whole range.
    fn default() -> Self {
        RoleRanges {
            lead: PitchRange::of(36, 72),
            bass: PitchRange::of(12, 43),
            arp: PitchRange::of(36, 72),
            harmony: PitchRange::of(24, 60),
            drums: PitchRange::of(0, 92),
            drone: PitchRange::of(12, 48),
            ostinato: PitchRange::of(24, 60),
            bed: PitchRange::of(12, 72),
            texture: PitchRange::of(0, 92),
        }
    }
}

/// What the validator needs besides the song, gathered from the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// The profiles subtunes may name: built-in plus the project's.
    pub profiles: Profiles,
    /// The adaptive sets the project declares over this song.
    pub adaptive: Vec<AdaptiveSet>,
    /// Facts about every instrument preset the song uses.
    pub instruments: BTreeMap<PresetId, InstrumentFacts>,
    /// Role pitch ranges.
    pub ranges: RoleRanges,
    /// Notes on an expendable voice longer than this many rows get a warning: after an effect the voice stays
    /// silent until the next note, so a long note may not come back at all.
    pub expendable_long_rows: u32,
}

impl Default for Context {
    /// Built-in profiles, no adaptive sets, no instrument facts, default ranges, and a warning for expendable notes
    /// longer than a beat (4 rows).
    fn default() -> Self {
        Context {
            profiles: Profiles::default(),
            adaptive: vec![],
            instruments: BTreeMap::new(),
            ranges: RoleRanges::default(),
            expendable_long_rows: 4,
        }
    }
}

/// Every problem in `song`, ordered by subtune, bar, voice and rule. Empty means the song passes rules 1–6 and 9.
pub fn validate(song: &Song, ctx: &Context) -> Vec<Problem> {
    let mut out = Vec::new();
    for set in &ctx.adaptive {
        adaptive(&mut out, song, set);
    }
    for subtune in &song.subtunes {
        let name = song.profile_of(subtune);
        let profile = ctx.profiles.get(name);
        let mut checker = Checker {
            out: &mut out,
            subtune,
            name,
            profile,
            ctx,
        };
        if profile.is_none() {
            checker.report(
                Rule::Voices,
                None,
                None,
                format!("no profile is called `{name}`"),
                "use a built-in profile (full, sfx-shares-v3, sfx-owns-v3, sfx-owns-v2-v3) or define it in the project",
            );
        }
        checker.run();
    }
    out.sort_by(|a, b| {
        let index = |p: &Problem| {
            p.subtune
                .as_ref()
                .and_then(|n| song.subtune(n))
                .map(|(i, _)| i)
        };
        (index(a), a.bar, a.voice, a.rule, a.row).cmp(&(index(b), b.bar, b.voice, b.rule, b.row))
    });
    out
}

struct Checker<'a> {
    out: &'a mut Vec<Problem>,
    subtune: &'a Subtune,
    /// The profile's name, for messages.
    name: &'a ProfileName,
    /// The profile; `None` when the name is unknown, which skips the profile checks.
    profile: Option<&'a Profile>,
    ctx: &'a Context,
}

impl Checker<'_> {
    /// An error.
    fn report(
        &mut self,
        rule: Rule,
        row: Option<u32>,
        voice: Option<Voice>,
        message: String,
        hint: impl Into<String>,
    ) {
        self.push(Severity::Error, rule, row, voice, message, hint.into());
    }

    /// A warning.
    fn warn(
        &mut self,
        rule: Rule,
        row: Option<u32>,
        voice: Option<Voice>,
        message: String,
        hint: impl Into<String>,
    ) {
        self.push(Severity::Warning, rule, row, voice, message, hint.into());
    }

    fn push(
        &mut self,
        severity: Severity,
        rule: Rule,
        row: Option<u32>,
        voice: Option<Voice>,
        message: String,
        hint: String,
    ) {
        let rows_per_bar = u32::from(self.subtune.rows_per_bar.get());
        self.out.push(Problem {
            rule,
            severity,
            subtune: Some(self.subtune.name.clone()),
            bar: row.map(|r| r / rows_per_bar),
            row,
            voice,
            message,
            hint,
        });
    }

    fn run(mut self) {
        self.grid();
        let mut seen: Vec<Voice> = Vec::new();
        for line in &self.subtune.voices {
            if seen.contains(&line.voice) {
                self.report(
                    Rule::Monophony,
                    None,
                    Some(line.voice),
                    format!("voice {} has more than one line", line.voice.get()),
                    "merge the lines into one, or move one to a free voice",
                );
            }
            seen.push(line.voice);
            self.line(line);
            self.hard_restart(line);
        }
    }

    /// Rule 9: GoatTracker's gate timer against the tempo, and the gate-off before each new note.
    ///
    /// The player fetches the next row `gate_timer` frames before it starts, using the gate timer of the instrument
    /// playing now, and turns the gate off then if the fetched row holds a new note (not legato) whose instrument
    /// does gate-off. That takes the last `gate_timer` frames from the note before.
    fn hard_restart(&mut self, line: &VoiceLine) {
        let voice = Some(line.voice);
        let facts = |n: &Note| self.ctx.instruments.get(line.instrument_of(n)).copied();
        let ends: Vec<u32> = line
            .notes
            .iter()
            .skip(1)
            .map(|n| n.row)
            .chain([self.length()])
            .collect();
        for (i, note) in line.notes.iter().enumerate() {
            let Some(own) = facts(note) else { continue };
            let gate = u32::from(own.gate_timer);
            if own.gate_timer > 63 {
                self.report(
                    Rule::HardRestart,
                    Some(note.row),
                    voice,
                    format!(
                        "`{}` has gate timer {gate}; GoatTracker allows 0 to 63",
                        line.instrument_of(note).0
                    ),
                    "set a gate timer from 0 to 63 frames",
                );
                continue;
            }
            // Playback stops if the gate timer is not below the tempo on a row this instrument plays.
            let rows = note.row..ends[i].max(note.row + 1);
            if let Some(row) = rows
                .into_iter()
                .find(|&r| u32::from(self.subtune.frames_per_row_at(r).get()) <= gate)
            {
                let fpr = self.subtune.frames_per_row_at(row).get();
                self.report(
                    Rule::HardRestart,
                    Some(row),
                    voice,
                    format!(
                        "`{}` has gate timer {gate} at {fpr} frames per row; GoatTracker stops playback when the gate timer is not below the tempo",
                        line.instrument_of(note).0
                    ),
                    format!("lower the gate timer below {fpr}, or slow the tempo"),
                );
            }
        }

        for (i, pair) in line.notes.windows(2).enumerate() {
            let (prev, next) = (&pair[0], &pair[1]);
            // Only a fresh note straight after the previous one cuts it; after silence there is nothing to cut.
            if next.attack != Attack::Fresh || prev.row + prev.length.get() != next.row {
                continue;
            }
            let (Some(out), Some(incoming)) = (facts(prev), facts(next)) else {
                continue;
            };
            if !incoming.gate_off {
                // The score says new attack, but with the gate still on from the note before and no gate-off, the
                // instrument cannot retrigger it: the packed song plays it legato.
                self.warn(
                    Rule::HardRestart,
                    Some(next.row),
                    voice,
                    format!(
                        "`{}` has gate-off disabled (gate-timer bit $40), so this fresh note straight after another won't retrigger",
                        line.instrument_of(next).0
                    ),
                    "mark it legato, or use an instrument that turns the gate off",
                );
                continue;
            }
            // The gate has been on since the attack that started a run of legato notes.
            let attack = line.notes[..=i]
                .iter()
                .rev()
                .find(|n| n.attack == Attack::Fresh)
                .map_or(prev.row, |n| n.row);
            let frames = self.subtune.frames(attack, next.row);
            let gated = frames as i64 - i64::from(out.gate_timer);
            if gated <= 0 {
                self.report(
                    Rule::HardRestart,
                    Some(next.row),
                    voice,
                    format!(
                        "the gate goes off {} frames before row {}, at or before the note at row {attack} starts ({frames} frames), so it never sounds",
                        out.gate_timer, next.row
                    ),
                    "lengthen the earlier note, tie the new one (legato), or use a smaller gate timer",
                );
            } else if gated <= 2 {
                self.warn(
                    Rule::HardRestart,
                    Some(next.row),
                    voice,
                    format!(
                        "the gate goes off {} frames before row {}, leaving the note at row {attack} only {gated} gated frame{}",
                        out.gate_timer,
                        next.row,
                        if gated == 1 { "" } else { "s" }
                    ),
                    "it will probably sound clipped: lengthen it, tie the new note, or use a smaller gate timer",
                );
            }
        }
    }

    fn length(&self) -> u32 {
        self.subtune.length.get()
    }

    /// Rule 2 for the subtune itself: tempo changes, loop row and marks inside the grid and in order.
    fn grid(&mut self) {
        let length = self.length();
        let mut last = 0;
        for t in &self.subtune.tempo {
            if t.row == 0 || t.row <= last || t.row >= length {
                self.report(
                    Rule::Grid,
                    Some(t.row),
                    None,
                    format!("tempo change at row {} is not after the previous one inside the subtune", t.row),
                    "set the starting tempo in frames_per_row; keep changes in increasing rows before the end",
                );
            }
            last = t.row;
        }
        if self.subtune.loop_row >= length {
            self.report(
                Rule::Grid,
                None,
                None,
                format!(
                    "loop row {} is past the end (length {length})",
                    self.subtune.loop_row
                ),
                "loop to a row inside the subtune, usually a downbeat",
            );
        }
        let mut last = None;
        for &m in &self.subtune.marks {
            if m >= length || last.is_some_and(|l| m <= l) {
                self.report(
                    Rule::Grid,
                    Some(m),
                    None,
                    format!("timing mark at row {m} is out of order or past the end"),
                    "list marks once each, in increasing rows inside the subtune",
                );
            }
            last = Some(m);
        }
    }

    fn line(&mut self, line: &VoiceLine) {
        let voice = Some(line.voice);
        let rule = self.profile.map(|p| p.voice(line.voice).clone());
        let length = self.length();
        let name = self.name;

        // Rule 3: music only on the profile's voices, in allowed roles.
        if let Some(rule) = &rule
            && !rule.is_music()
            && !line.notes.is_empty()
        {
            self.report(
                Rule::Voices,
                line.notes.first().map(|n| n.row),
                voice,
                format!("`{name}` has no music on voice {}", line.voice.get()),
                "move these notes to a music voice, or choose a profile that uses this voice",
            );
        } else if let Some(rule) = &rule {
            for span in line.roles.spans() {
                if span.row < length && !rule.allows(span.role) {
                    self.report(
                        Rule::Voices,
                        Some(span.row),
                        voice,
                        format!(
                            "`{name}` does not allow {:?} on voice {}",
                            span.role,
                            line.voice.get()
                        ),
                        "change the role or move the part to a voice that allows it",
                    );
                }
            }
        }
        // Rule 4 warnings: lead or bass on an expendable voice is allowed, but effects will cut it.
        if let Some(rule) = &rule
            && rule.usage() == VoiceUse::Expendable
        {
            for span in line.roles.spans() {
                if span.row < length
                    && matches!(span.role, Role::Lead | Role::Bass)
                    && rule.allows(span.role)
                {
                    self.warn(
                        Rule::FilterAndExpendable,
                        Some(span.row),
                        voice,
                        format!(
                            "{:?} on expendable voice {}: sound effects will cut it, and it comes back only at its next note",
                            span.role,
                            line.voice.get()
                        ),
                        "move it to a music voice, or keep it if the gaps are intended",
                    );
                }
            }
        }
        for span in line.roles.spans() {
            if span.row >= length {
                self.report(
                    Rule::Grid,
                    Some(span.row),
                    voice,
                    format!("role change at row {} is past the end", span.row),
                    "remove it or lengthen the subtune",
                );
            }
        }

        let mut end = 0;
        for (i, note) in line.notes.iter().enumerate() {
            let at = Some(note.row);
            let note_end = note.row.saturating_add(note.length.get());

            // Rule 1: no overlaps, legato only straight after a note.
            if i > 0 && note.row < end {
                self.report(
                    Rule::Monophony,
                    at,
                    voice,
                    format!(
                        "note at row {} starts before the previous note ends (row {end})",
                        note.row
                    ),
                    "shorten the previous note, or move this one to another voice",
                );
            }
            if note.attack == Attack::Legato && (i == 0 || note.row != end) {
                self.report(
                    Rule::Monophony,
                    at,
                    voice,
                    format!(
                        "legato note at row {} does not follow straight on from a note",
                        note.row
                    ),
                    "make it a fresh attack, or end the previous note on its row",
                );
            }
            end = end.max(note_end);

            // Rule 2: inside the subtune.
            if note_end > length {
                self.report(
                    Rule::Grid,
                    at,
                    voice,
                    format!(
                        "note at row {} runs to row {note_end}, past the end (length {length})",
                        note.row
                    ),
                    "shorten the note or lengthen the subtune",
                );
            }

            // Rule 4: filter where allowed, explicit pulse width on the expendable voice.
            let instrument = line.instrument_of(note);
            match self.ctx.instruments.get(instrument) {
                None => self.report(
                    Rule::FilterAndExpendable,
                    at,
                    voice,
                    format!("no facts for instrument `{}`", instrument.0),
                    "add the preset, or state its filter, pulse width at note start, gate timer and hard restart",
                ),
                Some(facts) => {
                    if let Some(rule) = &rule {
                        if facts.filter && !rule.filter() {
                            self.report(
                                Rule::FilterAndExpendable,
                                at,
                                voice,
                                format!(
                                    "`{}` routes voice {} through the filter, which `{name}` does not allow there",
                                    instrument.0,
                                    line.voice.get(),
                                ),
                                "use an unfiltered instrument on this voice, or move the part to a voice that may filter",
                            );
                        }
                        if rule.usage() == VoiceUse::Expendable && !facts.pulse_at_start {
                            self.report(
                                Rule::FilterAndExpendable,
                                at,
                                voice,
                                format!(
                                    "`{}` does not set a pulse width at note start on the expendable voice",
                                    instrument.0
                                ),
                                "give the instrument a pulse table that sets the width, so notes after an effect sound right",
                            );
                        }
                    }
                }
            }

            if let Some(rule) = &rule
                && rule.usage() == VoiceUse::Expendable
                && note.length.get() > self.ctx.expendable_long_rows
            {
                self.warn(
                    Rule::FilterAndExpendable,
                    at,
                    voice,
                    format!(
                        "a {}-row note on expendable voice {}: an effect during it leaves the voice silent until the next note",
                        note.length.get(),
                        line.voice.get()
                    ),
                    format!(
                        "split it into notes of at most {} rows, or move it to a music voice",
                        self.ctx.expendable_long_rows
                    ),
                );
            }

            // Rule 6: the role's pitch range.
            let role = line.roles.at(note.row);
            let range = self.ctx.ranges.of(role);
            if !range.contains(note.pitch) {
                self.warn(
                    Rule::PitchRange,
                    at,
                    voice,
                    format!(
                        "{} is outside the {role:?} range {} to {}",
                        note.pitch, range.low, range.high
                    ),
                    fold_hint(note.pitch, range),
                );
            }
        }

        for arp in &line.arps {
            if arp.row.saturating_add(arp.length.get()) > length {
                self.report(
                    Rule::Grid,
                    Some(arp.row),
                    voice,
                    format!("arpeggio at row {} runs past the end", arp.row),
                    "shorten the arpeggio",
                );
            }
            // Rule 6, a hard limit: the player adds the interval to the note and its frequency table ends at B-7.
            let highest = arp.intervals.iter().copied().max().map_or(0, u16::from);
            let end = arp.row.saturating_add(arp.length.get());
            for note in line
                .notes
                .iter()
                .filter(|n| n.row < end && arp.row < n.row.saturating_add(n.length.get()))
            {
                if u16::from(note.pitch.semitone()) + highest > ARP_HIGHEST {
                    self.report(
                        Rule::PitchRange,
                        Some(note.row.max(arp.row)),
                        voice,
                        format!(
                            "{} plus the arpeggio's {highest} semitones passes B-7, the player's highest note",
                            note.pitch
                        ),
                        "lower the note or narrow the chord",
                    );
                }
            }
        }
    }
}

/// The highest note an arpeggio may reach, B-7: the player's frequency table ends there, and it adds intervals modulo
/// 128 (`docs/reference/goattracker-files.md`).
const ARP_HIGHEST: u16 = 95;

/// Rule 5: one declared adaptive set.
fn adaptive(out: &mut Vec<Problem>, song: &Song, set: &AdaptiveSet) {
    for problem in set.check(song) {
        let (subtune, row, message, hint) = match problem {
            LayerProblem::Missing { layer } => (
                None,
                None,
                format!(
                    "adaptive set `{}` names `{}`, which the song does not have",
                    set.name(),
                    layer.as_str()
                ),
                "add the subtune or remove the layer from the set",
            ),
            LayerProblem::Position {
                layer,
                expected,
                found,
            } => (
                Some(layer.clone()),
                None,
                format!(
                    "`{}` is subtune {found}, but adaptive set `{}` puts it at {expected}",
                    layer.as_str(),
                    set.name()
                ),
                "order the layers at consecutive positions, as the set lists them",
            ),
            LayerProblem::Mark { layer, row } => (
                Some(layer.clone()),
                Some(row),
                format!(
                    "the switch point at row {row} of layer `{}` has no timing mark",
                    layer.as_str()
                ),
                "add a mark there, so the game knows it may switch layers",
            ),
            LayerProblem::PastEnd { layer, row } => (
                Some(layer.clone()),
                None,
                format!(
                    "adaptive set `{}` switches at row {row}, past the end of `{}`",
                    set.name(),
                    layer.as_str()
                ),
                "remove the row or lengthen the layer",
            ),
        };
        let bar = match (&subtune, row) {
            (Some(name), Some(row)) => song
                .subtune(name)
                .map(|(_, s)| row / u32::from(s.rows_per_bar.get())),
            _ => None,
        };
        out.push(Problem {
            rule: Rule::TimingMarks,
            severity: Severity::Error,
            subtune,
            bar,
            row,
            voice: None,
            message,
            hint: hint.into(),
        });
    }
}

/// "Fold up 2 octaves to A-3", or why folding cannot help.
fn fold_hint(p: Pitch, range: PitchRange) -> String {
    let s = i16::from(p.semitone());
    let (low, high) = (
        i16::from(range.low.semitone()),
        i16::from(range.high.semitone()),
    );
    let octaves = if s < low {
        (low - s + 11) / 12
    } else {
        -((s - high + 11) / 12)
    };
    let folded = s + 12 * octaves;
    match u8::try_from(folded).ok().and_then(Pitch::new) {
        Some(f) if range.contains(f) => format!(
            "fold {} {} octave{} to {f}",
            if octaves > 0 { "up" } else { "down" },
            octaves.abs(),
            if octaves.abs() == 1 { "" } else { "s" }
        ),
        _ => "the range is narrower than an octave here; change the note or widen the range".into(),
    }
}

/// What a packed song may take (rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PackBudget {
    /// Most bytes the packed player and data may take.
    pub max_bytes: u32,
    /// The zero-page pair the player may use (gt2reloc `-Z`).
    pub zero_page: u8,
}

/// What packing produced (rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PackFacts {
    /// Size of the packed player and data.
    pub bytes: u32,
    /// The zero-page pair it uses.
    pub zero_page: u8,
}

/// Rule 8: the packed result within its budget.
pub fn check_pack(facts: PackFacts, budget: PackBudget) -> Vec<Problem> {
    let mut out = Vec::new();
    let problem = |message: String, hint: &str| Problem {
        rule: Rule::Pack,
        severity: Severity::Error,
        subtune: None,
        bar: None,
        row: None,
        voice: None,
        message,
        hint: hint.into(),
    };
    if facts.bytes > budget.max_bytes {
        out.push(problem(
            format!(
                "packed size {} bytes is over the budget of {}",
                facts.bytes, budget.max_bytes
            ),
            "shorten patterns, share instruments or raise the budget",
        ));
    }
    if facts.zero_page != budget.zero_page {
        out.push(problem(
            format!(
                "the player uses zero page ${:02X}–${:02X}, not the configured ${:02X}–${:02X}",
                facts.zero_page,
                facts.zero_page.wrapping_add(1),
                budget.zero_page,
                budget.zero_page.wrapping_add(1)
            ),
            "pack with gt2reloc -Z set to the configured pair",
        ));
    }
    out
}

#[cfg(test)]
mod tests;
