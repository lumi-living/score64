//! Built-in profiles, the checks on read, and project-defined profiles.

use std::num::{NonZeroU8, NonZeroU32};

use super::*;
use crate::score::{FramesPerRow, Schema, Song, Subtune, SubtuneName};

fn v(n: u8) -> Voice {
    Voice::new(n).unwrap()
}

fn name(s: &str) -> ProfileName {
    ProfileName::new(s).unwrap()
}

fn get<'a>(profiles: &'a Profiles, s: &str) -> &'a Profile {
    profiles.get(&name(s)).unwrap()
}

fn music(p: &Profile) -> Vec<u8> {
    p.music_voices().map(Voice::get).collect()
}

#[test]
fn builtin_profiles() {
    let all = Profiles::builtin();
    let names: Vec<&str> = all.iter().map(|p| p.name().as_str()).collect();
    assert_eq!(
        names,
        ["full", "sfx-shares-v3", "sfx-owns-v3", "sfx-owns-v2-v3"]
    );

    let full = get(&all, builtin::FULL);
    assert_eq!(music(full), [1, 2, 3]);
    assert!((1..=3).all(|n| full.may_filter(v(n)) && full.allows(v(n), Role::Lead)));

    let shares = get(&all, builtin::SFX_SHARES_V3);
    assert_eq!(music(shares), [1, 2, 3]);
    assert_eq!(shares.expendable_voices().collect::<Vec<_>>(), [v(3)]);
    // Any role on the expendable voice, lead and bass included (the validator warns about those); never the filter.
    for r in [Role::Drums, Role::Arp, Role::Lead, Role::Bass] {
        assert!(shares.allows(v(3), r), "{r:?}");
    }
    assert!(!shares.may_filter(v(3)));

    let owns3 = get(&all, builtin::SFX_OWNS_V3);
    assert_eq!(music(owns3), [1, 2]);
    assert_eq!(owns3.voice(v(3)).usage(), VoiceUse::Effects);

    let owns23 = get(&all, builtin::SFX_OWNS_V2_V3);
    assert_eq!(music(owns23), [1]);
    assert!(!owns23.allows(v(2), Role::Bass));
}

#[test]
fn voice_rules_are_checked() {
    use VoiceUse::*;
    let err = |u, roles: Vec<Role>, filter| VoiceRule::new(u, roles, filter).unwrap_err();
    assert!(err(Expendable, vec![Role::Drums], true).contains("filter"));
    assert!(err(Expendable, vec![], true).contains("filter"));
    // Lead or bass on an expendable voice, and an empty list (any role), are allowed.
    assert!(VoiceRule::new(Expendable, vec![Role::Lead, Role::Bass], false).is_ok());
    assert!(
        VoiceRule::new(Expendable, vec![], false)
            .unwrap()
            .allows(Role::Lead)
    );
    assert!(err(Effects, vec![Role::Drums], false).contains("no roles"));
    assert!(err(Effects, vec![], true).contains("no filter"));
    assert!(err(Music, vec![Role::Bed, Role::Bed], true).contains("twice"));
    let e = Profile::new(
        name("silent"),
        [
            VoiceRule::effects(),
            VoiceRule::effects(),
            VoiceRule::effects(),
        ],
    );
    assert!(e.unwrap_err().contains("no music voice"));
}

/// A project's own profiles, as it would write them in TOML.
const PROJECT: &str = r#"
[[profile]]
name = "calm"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "effects" }
v3 = { usage = "effects" }

[[profile]]
name = "tension"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "music", roles = ["drone", "ostinato"] }
v3 = { usage = "effects" }
"#;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    profile: Vec<Profile>,
}

#[test]
fn projects_define_their_own_profiles() {
    let file: ProjectFile = toml::from_str(PROJECT).unwrap();
    let all = Profiles::with(file.profile.clone()).unwrap();
    let tension = get(&all, "tension");
    assert_eq!(music(tension), [1, 2]);
    assert!(tension.allows(v(1), Role::Bed) && !tension.allows(v(1), Role::Lead));
    assert!(tension.allows(v(2), Role::Ostinato) && !tension.may_filter(v(2)));
    assert_eq!(
        toml::from_str::<ProjectFile>(&toml::to_string(&file).unwrap())
            .unwrap()
            .profile,
        file.profile,
        "round trip"
    );

    let e = Profiles::with(file.profile.iter().chain(&file.profile).cloned()).unwrap_err();
    assert!(e.contains("`calm` is defined twice"), "{e}");
    let full = Profiles::builtin()
        .get(&name(builtin::FULL))
        .unwrap()
        .clone();
    assert!(Profiles::with([full]).unwrap_err().contains("built-in"));

    for bad in [
        PROJECT.replace("usage = \"effects\" }", "usage = \"effects\", colour = 1 }"),
        PROJECT.replace(
            "v3 = { usage = \"effects\" }\n\n[[profile]]",
            "\n[[profile]]",
        ),
        PROJECT.replace(
            "roles = [\"drone\", \"ostinato\"]",
            "roles = [\"drone\"], usage2 = \"x\"",
        ),
        format!(
            "{PROJECT}\n[[profile]]\nname = \"x\"\nv1 = {{ usage = \"music\" }}\nv2 = {{ usage = \"music\" }}\n\
             v3 = {{ usage = \"expendable\", filter = true }}\n"
        ),
    ] {
        assert!(toml::from_str::<ProjectFile>(&bad).is_err(), "{bad}");
    }
}

#[test]
fn subtunes_override_the_songs_profile_by_name() {
    let subtune = |n: &str, profile| Subtune {
        name: SubtuneName::new(n).unwrap(),
        profile,
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(16).unwrap(),
        loop_row: 0,
        marks: vec![0],
        voices: vec![],
    };
    let song = Song {
        schema: Schema::default(),
        profile: name(builtin::SFX_SHARES_V3),
        subtunes: vec![subtune("a", None), subtune("b", Some(name("calm")))],
    };
    assert_eq!(song.profile_of(&song.subtunes[0]).as_str(), "sfx-shares-v3");
    assert_eq!(song.profile_of(&song.subtunes[1]).as_str(), "calm");
    let json = serde_json::to_value(&song).unwrap();
    assert!(
        json["subtunes"][0].get("profile").is_none(),
        "the default is not repeated"
    );
    assert_eq!(json["subtunes"][1]["profile"], "calm");
}
