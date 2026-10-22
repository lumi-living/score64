//! Voice profiles (`docs/TAD.md` §3.1): for each SID voice, whether it carries music, music that sound effects may
//! interrupt, or only sound effects; which roles its music may have; and whether it may use the SID's one filter.
//!
//! Profiles are validated data. Score64 ships four built-in profiles ([`Profiles::builtin`]); a project can define
//! more ([`Profiles::with`]). Subtunes refer to profiles by name. The validator checks songs against them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::provenance::valid_id;
use crate::score::{Role, Voice};

/// A profile's name: 1–64 of `a`–`z`, `0`–`9`, `-` and `_`, starting with a letter or digit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProfileName(String);

impl ProfileName {
    /// `s`, if it is a valid name.
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        valid_id(&s).then_some(ProfileName(s))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ProfileName {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let shown = format!("{s:?}");
        ProfileName::new(s).ok_or_else(|| {
            format!("{shown} is not a profile name (1–64 of a-z, 0-9, - and _, starting with a letter or digit)")
        })
    }
}

impl From<ProfileName> for String {
    fn from(n: ProfileName) -> String {
        n.0
    }
}

/// Whether a voice carries music.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VoiceUse {
    /// Music.
    Music,
    /// Music that sound effects may interrupt. After an effect the voice stays silent until the music's next new
    /// note (`docs/reference/player.md`), so it should carry short, frequent notes; its notes start with an explicit
    /// pulse width and it never uses the filter (rule 4).
    Expendable,
    /// Sound effects only; no music.
    Effects,
}

/// What a profile allows on one voice. Read from files through checks: an effects-only voice has no roles and no
/// filter; an expendable voice names its roles, which may not be lead or bass, and has no filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "VoiceRuleFields", into = "VoiceRuleFields")]
pub struct VoiceRule {
    usage: VoiceUse,
    roles: Vec<Role>,
    filter: bool,
}

/// [`VoiceRule`] as written in files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct VoiceRuleFields {
    /// `music`, `expendable` or `effects`.
    usage: VoiceUse,
    /// Roles its music may have; empty or absent means any (music voices only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    roles: Vec<Role>,
    /// Whether it may be routed through the filter.
    #[serde(default)]
    filter: bool,
}

impl VoiceRule {
    /// A rule, if it is consistent (see the type's docs).
    pub fn new(usage: VoiceUse, roles: Vec<Role>, filter: bool) -> Result<Self, String> {
        if roles
            .iter()
            .enumerate()
            .any(|(i, r)| roles[..i].contains(r))
        {
            return Err("a role is listed twice".into());
        }
        match usage {
            VoiceUse::Music => {}
            VoiceUse::Expendable => {
                if filter {
                    return Err("an expendable voice may not use the filter (rule 4)".into());
                }
                if roles.is_empty() {
                    return Err("an expendable voice must list its roles".into());
                }
                if roles.iter().any(|r| matches!(r, Role::Lead | Role::Bass)) {
                    return Err("an expendable voice may not carry lead or bass (rule 4)".into());
                }
            }
            VoiceUse::Effects => {
                if filter || !roles.is_empty() {
                    return Err("an effects-only voice has no roles and no filter".into());
                }
            }
        }
        Ok(VoiceRule {
            usage,
            roles,
            filter,
        })
    }

    /// Music with any role, filter allowed.
    pub fn music() -> Self {
        VoiceRule {
            usage: VoiceUse::Music,
            roles: vec![],
            filter: true,
        }
    }

    /// Sound effects only.
    pub fn effects() -> Self {
        VoiceRule {
            usage: VoiceUse::Effects,
            roles: vec![],
            filter: false,
        }
    }

    /// Music, music that effects may interrupt, or effects only.
    pub fn usage(&self) -> VoiceUse {
        self.usage
    }

    /// The roles its music may have; empty means any.
    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    /// Whether it may be routed through the filter.
    pub fn filter(&self) -> bool {
        self.filter
    }

    /// Whether it carries music (plain or expendable).
    pub fn is_music(&self) -> bool {
        self.usage != VoiceUse::Effects
    }

    /// Whether music on it may have `role`.
    pub fn allows(&self, role: Role) -> bool {
        self.is_music() && (self.roles.is_empty() || self.roles.contains(&role))
    }
}

impl TryFrom<VoiceRuleFields> for VoiceRule {
    type Error = String;
    fn try_from(f: VoiceRuleFields) -> Result<Self, String> {
        VoiceRule::new(f.usage, f.roles, f.filter)
    }
}

impl From<VoiceRule> for VoiceRuleFields {
    fn from(r: VoiceRule) -> Self {
        VoiceRuleFields {
            usage: r.usage,
            roles: r.roles,
            filter: r.filter,
        }
    }
}

/// A named profile: one rule per SID voice. At least one voice carries music.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "ProfileFields", into = "ProfileFields")]
pub struct Profile {
    name: ProfileName,
    voices: [VoiceRule; 3],
}

/// [`Profile`] as written in files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ProfileFields {
    /// Name, unique among the built-in and project profiles.
    name: ProfileName,
    /// Voice 1.
    v1: VoiceRule,
    /// Voice 2.
    v2: VoiceRule,
    /// Voice 3.
    v3: VoiceRule,
}

impl Profile {
    /// A profile, if at least one voice carries music.
    pub fn new(name: ProfileName, voices: [VoiceRule; 3]) -> Result<Self, String> {
        if !voices.iter().any(VoiceRule::is_music) {
            return Err(format!("profile `{name}` has no music voice"));
        }
        Ok(Profile { name, voices })
    }

    /// Its name.
    pub fn name(&self) -> &ProfileName {
        &self.name
    }

    /// The rule for `voice`.
    pub fn voice(&self, voice: Voice) -> &VoiceRule {
        &self.voices[usize::from(voice.get() - 1)]
    }

    /// The voices that carry music.
    pub fn music_voices(&self) -> impl Iterator<Item = Voice> + '_ {
        (1..=3)
            .filter_map(Voice::new)
            .filter(|&v| self.voice(v).is_music())
    }

    /// The voices sound effects may interrupt music on.
    pub fn expendable_voices(&self) -> impl Iterator<Item = Voice> + '_ {
        (1..=3)
            .filter_map(Voice::new)
            .filter(|&v| self.voice(v).usage() == VoiceUse::Expendable)
    }

    /// Whether music on `voice` may have `role`.
    pub fn allows(&self, voice: Voice, role: Role) -> bool {
        self.voice(voice).allows(role)
    }

    /// Whether `voice` may be routed through the filter.
    pub fn may_filter(&self, voice: Voice) -> bool {
        self.voice(voice).filter()
    }
}

impl TryFrom<ProfileFields> for Profile {
    type Error = String;
    fn try_from(f: ProfileFields) -> Result<Self, String> {
        Profile::new(f.name, [f.v1, f.v2, f.v3])
    }
}

impl From<Profile> for ProfileFields {
    fn from(p: Profile) -> Self {
        let [v1, v2, v3] = p.voices;
        ProfileFields {
            name: p.name,
            v1,
            v2,
            v3,
        }
    }
}

/// Names of the built-in profiles.
pub mod builtin {
    /// All three voices are music, any role, filter allowed.
    pub const FULL: &str = "full";
    /// Voices 1–2 are music; voice 3 is music that sound effects may interrupt (drums, arpeggio or texture only).
    pub const SFX_SHARES_V3: &str = "sfx-shares-v3";
    /// Voices 1–2 are music; voice 3 is for sound effects only.
    pub const SFX_OWNS_V3: &str = "sfx-owns-v3";
    /// Voice 1 is music; voices 2–3 are for sound effects only.
    pub const SFX_OWNS_V2_V3: &str = "sfx-owns-v2-v3";
}

/// The profiles a song may name: the built-in ones plus any a project defines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profiles(Vec<Profile>);

impl Profiles {
    /// The built-in profiles: `full`, `sfx-shares-v3`, `sfx-owns-v3` and `sfx-owns-v2-v3`.
    pub fn builtin() -> Self {
        let name = |s: &str| ProfileName::new(s).expect("valid built-in name");
        let shared = VoiceRule::new(
            VoiceUse::Expendable,
            vec![Role::Drums, Role::Arp, Role::Texture],
            false,
        )
        .expect("valid built-in rule");
        let p = |n: &str, voices| Profile::new(name(n), voices).expect("valid built-in profile");
        let (m, e) = (VoiceRule::music, VoiceRule::effects);
        Profiles(vec![
            p(builtin::FULL, [m(), m(), m()]),
            p(builtin::SFX_SHARES_V3, [m(), m(), shared]),
            p(builtin::SFX_OWNS_V3, [m(), m(), e()]),
            p(builtin::SFX_OWNS_V2_V3, [m(), e(), e()]),
        ])
    }

    /// The built-in profiles plus `project`'s, whose names must be new.
    pub fn with(project: impl IntoIterator<Item = Profile>) -> Result<Self, String> {
        let mut all = Self::builtin();
        for p in project {
            if all.get(p.name()).is_some() {
                return Err(format!(
                    "profile `{}` is defined twice, or redefines a built-in profile",
                    p.name()
                ));
            }
            all.0.push(p);
        }
        Ok(all)
    }

    /// The profile called `name`.
    pub fn get(&self, name: &ProfileName) -> Option<&Profile> {
        self.0.iter().find(|p| p.name() == name)
    }

    /// Every profile, built-in ones first.
    pub fn iter(&self) -> impl Iterator<Item = &Profile> {
        self.0.iter()
    }
}

impl Default for Profiles {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests;
