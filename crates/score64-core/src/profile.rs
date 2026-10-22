//! Voice profiles (`docs/TAD.md` §3.1): which SID voices carry music in a subtune, with which roles, and which voice
//! sound effects may take. A profile belongs to a subtune (the song's is the default), because one packed mission
//! set mixes stages with different voice rules.
//!
//! This module defines the rules as data. The validator (#13) checks songs against them and reports per bar and
//! voice.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::score::{Role, Song, Voice};

/// A subtune's voice profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Three voices, any role on any voice.
    Menu,
    /// V1 lead, V2 bass or arpeggio, V3 expendable for sound effects (drums, arpeggio or texture only).
    Combat,
    /// Two voices (V1 lead, V2 bass or arpeggio); V3 has no music, so sound effects have it to themselves.
    Combat2,
    /// Mission stage 0: one voice of bed on V1.
    Bed,
    /// Mission stage 1: the bed plus a drone on V3.
    Drone,
    /// Mission stage 2: bed, ostinato on V2 and drone; all three voices are music.
    Ostinato,
    /// Mission stage 3: the climax, under `combat` rules (V3 expendable).
    Climax,
    /// Mission stages 4 and 5: short end stings (victory, defeat), three voices, any role.
    Sting,
}

/// What a profile does with one voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceUse {
    /// Music with one of these roles (any role if empty).
    Music(&'static [Role]),
    /// Music with one of these roles that sound effects may interrupt. After an effect the voice stays silent until
    /// the music's next new note (#119), so it should carry short, frequent notes; notes start with an explicit
    /// pulse width and the voice is not routed through the filter (validator rule 4).
    Expendable(&'static [Role]),
    /// No music on this voice.
    Unused,
}

impl VoiceUse {
    /// Whether music may use this voice in `role`.
    pub fn allows(self, role: Role) -> bool {
        match self {
            VoiceUse::Music(roles) | VoiceUse::Expendable(roles) => {
                roles.is_empty() || roles.contains(&role)
            }
            VoiceUse::Unused => false,
        }
    }
}

const ANY: VoiceUse = VoiceUse::Music(&[]);
const LEAD: VoiceUse = VoiceUse::Music(&[Role::Lead]);
const BASS_OR_ARP: VoiceUse = VoiceUse::Music(&[Role::Bass, Role::Arp]);
const EXPENDABLE: VoiceUse = VoiceUse::Expendable(&[Role::Drums, Role::Arp, Role::Texture]);
const UNUSED: VoiceUse = VoiceUse::Unused;

impl Profile {
    /// Every profile.
    pub const ALL: [Profile; 8] = [
        Profile::Menu,
        Profile::Combat,
        Profile::Combat2,
        Profile::Bed,
        Profile::Drone,
        Profile::Ostinato,
        Profile::Climax,
        Profile::Sting,
    ];

    /// The name used in files.
    pub const fn name(self) -> &'static str {
        match self {
            Profile::Menu => "menu",
            Profile::Combat => "combat",
            Profile::Combat2 => "combat2",
            Profile::Bed => "bed",
            Profile::Drone => "drone",
            Profile::Ostinato => "ostinato",
            Profile::Climax => "climax",
            Profile::Sting => "sting",
        }
    }

    /// What the profile does with voices 1, 2 and 3.
    pub const fn voices(self) -> [VoiceUse; 3] {
        match self {
            Profile::Menu | Profile::Sting | Profile::Ostinato => [ANY, ANY, ANY],
            Profile::Combat | Profile::Climax => [LEAD, BASS_OR_ARP, EXPENDABLE],
            Profile::Combat2 => [LEAD, BASS_OR_ARP, UNUSED],
            Profile::Bed => [ANY, UNUSED, UNUSED],
            Profile::Drone => [ANY, UNUSED, ANY],
        }
    }

    /// What the profile does with `voice`.
    pub const fn voice(self, voice: Voice) -> VoiceUse {
        self.voices()[(voice.get() - 1) as usize]
    }

    /// The voices that carry music.
    pub fn music_voices(self) -> impl Iterator<Item = Voice> {
        (1..=3)
            .filter_map(Voice::new)
            .filter(move |&v| !matches!(self.voice(v), VoiceUse::Unused))
    }

    /// The voice sound effects interrupt music on, if any.
    pub fn expendable_voice(self) -> Option<Voice> {
        (1..=3)
            .filter_map(Voice::new)
            .find(|&v| matches!(self.voice(v), VoiceUse::Expendable(_)))
    }

    /// Whether music on `voice` may have `role`.
    pub fn allows(self, voice: Voice, role: Role) -> bool {
        self.voice(voice).allows(role)
    }

    /// Whether this is one of the mission stages.
    pub const fn is_mission_stage(self) -> bool {
        matches!(
            self,
            Profile::Bed | Profile::Drone | Profile::Ostinato | Profile::Climax | Profile::Sting
        )
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The subtunes of a mission set, in the order the game numbers them: bed, drone, ostinato, climax, then up to two
/// stings (victory, defeat).
pub const MISSION_LAYOUT: [Profile; 6] = [
    Profile::Bed,
    Profile::Drone,
    Profile::Ostinato,
    Profile::Climax,
    Profile::Sting,
    Profile::Sting,
];

/// The fewest subtunes a mission set has: the four stages, without stings.
pub const MISSION_MIN_SUBTUNES: usize = 4;

/// Why a song that uses mission stages is not a mission set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionLayoutError {
    /// It has this many subtunes, not 4–6.
    Count(usize),
    /// Subtune `index` (called `name`) plays under `found` where the layout has `expected`.
    Stage {
        /// Position (the game's subtune number).
        index: usize,
        /// Its name.
        name: String,
        /// What the layout puts there.
        expected: Profile,
        /// What it has.
        found: Profile,
    },
}

impl fmt::Display for MissionLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MissionLayoutError::Count(n) => write!(
                f,
                "a mission set has {MISSION_MIN_SUBTUNES} to {} subtunes (bed, drone, ostinato, climax, stings), not {n}",
                MISSION_LAYOUT.len()
            ),
            MissionLayoutError::Stage {
                index,
                name,
                expected,
                found,
            } => write!(
                f,
                "subtune {index} ({name}) is `{found}`, but a mission set has `{expected}` there"
            ),
        }
    }
}

impl std::error::Error for MissionLayoutError {}

/// Whether `song` is laid out as a mission set. A song is one when any of its subtunes plays under a mission stage;
/// other songs pass unchecked.
pub fn check_mission_layout(song: &Song) -> Result<(), MissionLayoutError> {
    let profiles: Vec<Profile> = song.subtunes.iter().map(|s| song.profile_of(s)).collect();
    if !profiles.iter().any(|p| p.is_mission_stage()) {
        return Ok(());
    }
    if !(MISSION_MIN_SUBTUNES..=MISSION_LAYOUT.len()).contains(&profiles.len()) {
        return Err(MissionLayoutError::Count(profiles.len()));
    }
    for (index, (&found, &expected)) in profiles.iter().zip(&MISSION_LAYOUT).enumerate() {
        if found != expected {
            return Err(MissionLayoutError::Stage {
                index,
                name: song.subtunes[index].name.as_str().to_owned(),
                expected,
                found,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU8, NonZeroU32};

    use super::*;
    use crate::score::{FramesPerRow, Schema, Subtune, SubtuneName};

    fn v(n: u8) -> Voice {
        Voice::new(n).unwrap()
    }

    fn voices(p: Profile) -> Vec<u8> {
        p.music_voices().map(Voice::get).collect()
    }

    #[test]
    fn combat_keeps_lead_and_bass_off_the_expendable_voice() {
        for p in [Profile::Combat, Profile::Climax] {
            assert!(p.allows(v(1), Role::Lead));
            assert!(!p.allows(v(1), Role::Bass));
            assert!(p.allows(v(2), Role::Bass) && p.allows(v(2), Role::Arp));
            assert!(!p.allows(v(2), Role::Lead));
            for r in [Role::Drums, Role::Arp, Role::Texture] {
                assert!(p.allows(v(3), r), "{p} {r:?}");
            }
            for r in [Role::Lead, Role::Bass, Role::Drone, Role::Bed] {
                assert!(!p.allows(v(3), r), "{p} {r:?}");
            }
            assert_eq!(p.expendable_voice(), Some(v(3)));
            assert_eq!(voices(p), [1, 2, 3]);
        }
        assert_eq!(Profile::Climax.voices(), Profile::Combat.voices());
    }

    #[test]
    fn voice_counts_and_free_profiles() {
        assert_eq!(voices(Profile::Combat2), [1, 2]);
        assert_eq!(Profile::Combat2.voice(v(3)), VoiceUse::Unused);
        assert_eq!(voices(Profile::Bed), [1]);
        assert_eq!(voices(Profile::Drone), [1, 3]);
        assert_eq!(voices(Profile::Ostinato), [1, 2, 3]);
        for p in [Profile::Menu, Profile::Sting, Profile::Ostinato] {
            for n in 1..=3 {
                for r in [Role::Lead, Role::Bass, Role::Drone, Role::Texture] {
                    assert!(p.allows(v(n), r), "{p} V{n} {r:?}");
                }
            }
        }
        for p in Profile::ALL {
            let expendable = matches!(p, Profile::Combat | Profile::Climax);
            assert_eq!(p.expendable_voice().is_some(), expendable, "{p}");
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                p.name(),
                "file name of {p}"
            );
        }
    }

    fn song(default: Profile, stages: &[Option<Profile>]) -> Song {
        let subtune = |i: usize, profile| Subtune {
            name: SubtuneName::new(format!("s{i}")).unwrap(),
            profile,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(16).unwrap(),
            loop_row: 0,
            marks: vec![0],
            voices: vec![],
        };
        Song {
            schema: Schema::default(),
            profile: default,
            subtunes: stages
                .iter()
                .enumerate()
                .map(|(i, &p)| subtune(i, p))
                .collect(),
        }
    }

    #[test]
    fn subtunes_override_the_songs_profile() {
        let s = song(Profile::Combat, &[None, Some(Profile::Menu)]);
        assert_eq!(s.profile_of(&s.subtunes[0]), Profile::Combat);
        assert_eq!(s.profile_of(&s.subtunes[1]), Profile::Menu);
        let json = serde_json::to_value(&s).unwrap();
        assert!(
            json["subtunes"][0].get("profile").is_none(),
            "the default is not repeated"
        );
        assert_eq!(json["subtunes"][1]["profile"], "menu");
    }

    #[test]
    fn mission_sets_follow_the_layout() {
        use Profile::*;
        let stages = |ps: &[Profile]| ps.iter().map(|&p| Some(p)).collect::<Vec<_>>();
        let full = song(Menu, &stages(&MISSION_LAYOUT));
        assert_eq!(check_mission_layout(&full), Ok(()));
        let no_stings = song(Menu, &stages(&[Bed, Drone, Ostinato, Climax]));
        assert_eq!(check_mission_layout(&no_stings), Ok(()));
        assert_eq!(
            check_mission_layout(&song(Menu, &stages(&[Bed, Drone, Climax]))),
            Err(MissionLayoutError::Count(3))
        );
        let swapped = song(Menu, &stages(&[Bed, Ostinato, Drone, Climax]));
        let e = check_mission_layout(&swapped).unwrap_err();
        assert_eq!(
            e,
            MissionLayoutError::Stage {
                index: 1,
                name: "s1".into(),
                expected: Drone,
                found: Ostinato
            }
        );
        assert_eq!(
            e.to_string(),
            "subtune 1 (s1) is `ostinato`, but a mission set has `drone` there"
        );
        // The song's default counts: four subtunes all defaulting to `bed` are a broken mission set.
        let defaulted = song(Bed, &[None, None, None, None]);
        assert!(matches!(
            check_mission_layout(&defaulted),
            Err(MissionLayoutError::Stage { index: 1, .. })
        ));
        // Songs without mission stages are not mission sets.
        assert_eq!(
            check_mission_layout(&song(Combat, &[None, Some(Menu)])),
            Ok(())
        );
    }
}
