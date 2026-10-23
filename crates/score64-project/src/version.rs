//! Version identity (#27, ADR-014 §6): a track's version is the blake3 of its resolved intent, its sources' hashes
//! and the tool version, in the cache's canonical JSON, so the same intent always has the same id and any change that
//! can change the output gives another.
//!
//! The intent is what the track says once the project's values are applied:
//! - `track.toml`, without its `[target]` and `[pack]` overrides, which count only through the values they resolve to,
//!   and without its sources' paths: a source is identified by its hash, so moving the file is no new version;
//! - `decisions.toml`, in subtune and id order, whatever order the file lists them in;
//! - the resolved video standard and `[pack]` (an explicit `hard_restart_adsr = 0x0F00`, gt2reloc's default, still
//!   differs from leaving it out: a needless new version, never a wrong match);
//! - the definition of the profile each subtune plays with (its own, else the track's default, else `full`), so a
//!   change to a profile no subtune uses leaves the track's versions alone;
//! - the project's `[validate]` and `[tools]`, which every track uses.
//!
//! Not in it, because they don't change the output: approvals, comments and `audio.lock` (ADR-004). Presets join it
//! when they land (#49), with a new [`SCHEME`]. The track's id isn't in it either: two tracks with the same intent
//! share a version, so whatever records a track's history keys on the track and the version together.
//!
//! The sources' hashes are those `track.toml` records; [`crate::Project::check_sources`] says whether the files still
//! have them. They are hashed twice, in the track and as the key's inputs, on purpose: the inputs are what the cache
//! keys on.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use score64_core::profile::builtin;
use score64_core::{ContentHash, Profile, ProfileName, VideoStandard};

use crate::cache::Cache;
use crate::project::{Pack, Project, Tools, Track, TrackFile, ValidateSettings};

/// The version of the identity scheme itself: a change to what goes into the hash changes it, so old and new ids
/// never collide.
pub const SCHEME: &str = "version@1";

/// A track's version: the hash of its intent, sources and tool version.
pub type VersionId = ContentHash;

/// What the version hashes as intent.
#[derive(Serialize)]
struct Intent<'a> {
    track: TrackFile,
    decisions: Vec<&'a score64_core::Decision>,
    video: VideoStandard,
    pack: Option<&'a Pack>,
    profiles: BTreeMap<&'a ProfileName, &'a Profile>,
    validate: &'a ValidateSettings,
    tools: &'a Tools,
}

/// Why a version can't be worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The track names a profile that is neither built in nor the project's (`Project::track` refuses it too).
    UnknownProfile(ProfileName),
    /// The project's profiles don't load.
    Profiles(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownProfile(p) => write!(f, "profile `{p}` isn't built in or the project's"),
            Error::Profiles(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Error {}

/// The version of `track` in `project`, made by `tool_version` (the Score64 version that runs it).
pub fn version_id(
    project: &Project,
    track: &Track,
    tool_version: &str,
) -> Result<VersionId, Error> {
    let (intent, sources) = intent(project, track)?;
    Ok(Cache::key(
        "version",
        SCHEME,
        &serde_json::json!({ "intent": intent, "tool_version": tool_version }),
        &sources,
    ))
}

/// The intent as JSON and the sources' hashes, in `track.toml`'s order.
fn intent(project: &Project, track: &Track) -> Result<(Value, Vec<ContentHash>), Error> {
    let profiles = project.file.profiles().map_err(Error::Profiles)?;
    let (_, validate, tools) = project.file.version_parts();
    let full = ProfileName::new(builtin::FULL).expect("a valid built-in name");
    let default = track.file.profile.as_ref().unwrap_or(&full);
    let mut used = BTreeMap::new();
    for name in track
        .file
        .subtunes
        .iter()
        .map(|s| s.profile.as_ref().unwrap_or(default))
    {
        let profile = profiles
            .get(name)
            .ok_or_else(|| Error::UnknownProfile(name.clone()))?;
        used.insert(profile.name(), profile);
    }
    // Owned for the borrow below; the overrides count only through what they resolve to.
    let mut file = track.file.clone();
    file.target = None;
    file.pack = None;
    let mut decisions: Vec<&score64_core::Decision> = track.decisions.decision.iter().collect();
    // Parsed files can't hold two decisions with one id in a subtune; a built one could, so the rest breaks ties.
    decisions.sort_by(|a, b| {
        (&a.subtune, &a.id)
            .cmp(&(&b.subtune, &b.id))
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });
    let intent = Intent {
        track: file,
        decisions,
        video: track.video,
        pack: track.pack.as_ref(),
        profiles: used,
        validate,
        tools,
    };
    let mut json = serde_json::to_value(&intent).expect("the intent serialises");
    // A source is its hash, not where it lies.
    if let Some(sources) = json
        .pointer_mut("/track/source")
        .and_then(Value::as_array_mut)
    {
        for s in sources.iter_mut().filter_map(Value::as_object_mut) {
            s.remove("path");
        }
    }
    let sources = track
        .file
        .sources
        .iter()
        .map(|s| ContentHash::from_hex(s.blake3.as_str()).expect("a checked blake3"))
        .collect();
    Ok((json, sources))
}

#[cfg(test)]
mod tests;
