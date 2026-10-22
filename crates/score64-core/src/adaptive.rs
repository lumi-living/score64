//! Adaptive sets (`docs/TAD.md` §3.1): a project declares which subtunes of a song are layers of one adaptive piece,
//! in the order the game steps through them. The validator checks the song against it (rule 5): the layers sit at
//! consecutive positions in that order, so the game can switch by subtune number, and every layer has a timing mark
//! on every bar, so the switch can wait for a bar line.

use serde::{Deserialize, Serialize};

use crate::score::{Song, SubtuneName};

/// An ordered list of layers, each a subtune by name; for example `calm`, `tension`, `pulse`, `peak`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "AdaptiveSetFields", into = "AdaptiveSetFields")]
pub struct AdaptiveSet {
    name: String,
    layers: Vec<SubtuneName>,
}

/// [`AdaptiveSet`] as written in files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct AdaptiveSetFields {
    /// A label for messages.
    name: String,
    /// Subtune names, in the order the game steps through them; at least one, each once.
    layers: Vec<SubtuneName>,
}

impl AdaptiveSet {
    /// A set, if it has a name and at least one layer, with no layer twice.
    pub fn new(name: impl Into<String>, layers: Vec<SubtuneName>) -> Result<Self, String> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("an adaptive set needs a name".into());
        }
        if layers.is_empty() {
            return Err(format!("adaptive set `{name}` has no layers"));
        }
        if let Some(dup) = layers
            .iter()
            .enumerate()
            .find(|(i, l)| layers[..*i].contains(l))
        {
            return Err(format!(
                "adaptive set `{name}` lists `{}` twice",
                dup.1.as_str()
            ));
        }
        Ok(AdaptiveSet { name, layers })
    }

    /// Its label.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its layers, in order.
    pub fn layers(&self) -> &[SubtuneName] {
        &self.layers
    }

    /// Every way `song` breaks this set: a layer missing, a layer out of place, or a bar without a timing mark.
    pub fn check(&self, song: &Song) -> Vec<LayerProblem> {
        let mut out = Vec::new();
        let first = song.subtune(&self.layers[0]).map(|(i, _)| i);
        for (k, layer) in self.layers.iter().enumerate() {
            let Some((index, subtune)) = song.subtune(layer) else {
                out.push(LayerProblem::Missing {
                    layer: layer.clone(),
                });
                continue;
            };
            if let Some(expected) = first.map(|f| f + k)
                && index != expected
            {
                out.push(LayerProblem::Position {
                    layer: layer.clone(),
                    expected,
                    found: index,
                });
            }
            let rows_per_bar = u32::from(subtune.rows_per_bar.get());
            for row in (0..subtune.length.get()).step_by(rows_per_bar as usize) {
                if !subtune.marks.contains(&row) {
                    out.push(LayerProblem::Mark {
                        layer: layer.clone(),
                        bar: row / rows_per_bar,
                    });
                }
            }
        }
        out
    }
}

/// How a song breaks an [`AdaptiveSet`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerProblem {
    /// The song has no subtune called `layer`.
    Missing {
        /// The layer.
        layer: SubtuneName,
    },
    /// `layer` is subtune `found`, but the set's order puts it at `expected` (layers sit at consecutive positions,
    /// in order, from the first layer's).
    Position {
        /// The layer.
        layer: SubtuneName,
        /// Where the order puts it.
        expected: usize,
        /// Where it is.
        found: usize,
    },
    /// Bar `bar` (0-based) of `layer` has no timing mark on its first row.
    Mark {
        /// The layer.
        layer: SubtuneName,
        /// The bar.
        bar: u32,
    },
}

impl TryFrom<AdaptiveSetFields> for AdaptiveSet {
    type Error = String;
    fn try_from(f: AdaptiveSetFields) -> Result<Self, String> {
        AdaptiveSet::new(f.name, f.layers)
    }
}

impl From<AdaptiveSet> for AdaptiveSetFields {
    fn from(s: AdaptiveSet) -> Self {
        AdaptiveSetFields {
            name: s.name,
            layers: s.layers,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU8, NonZeroU32};

    use super::*;
    use crate::profile::ProfileName;
    use crate::score::{FramesPerRow, Schema, Subtune};

    fn n(s: &str) -> SubtuneName {
        SubtuneName::new(s).unwrap()
    }

    fn song(names: &[&str]) -> Song {
        Song {
            schema: Schema::default(),
            profile: ProfileName::new("full").unwrap(),
            subtunes: names
                .iter()
                .map(|s| Subtune {
                    name: n(s),
                    profile: None,
                    frames_per_row: FramesPerRow::new(6).unwrap(),
                    tempo: vec![],
                    rows_per_bar: NonZeroU8::new(16).unwrap(),
                    length: NonZeroU32::new(32).unwrap(),
                    loop_row: 0,
                    marks: vec![0, 16],
                    voices: vec![],
                })
                .collect(),
        }
    }

    fn set() -> AdaptiveSet {
        AdaptiveSet::new(
            "score",
            vec![n("calm"), n("tension"), n("pulse"), n("peak")],
        )
        .unwrap()
    }

    #[test]
    fn layers_sit_in_order_with_marks_on_every_bar() {
        // A title tune first, then the layers at 1–4: fine.
        assert_eq!(
            set().check(&song(&["title", "calm", "tension", "pulse", "peak", "win"])),
            []
        );

        assert_eq!(
            set().check(&song(&["calm", "pulse", "tension", "peak"])),
            [
                LayerProblem::Position {
                    layer: n("tension"),
                    expected: 1,
                    found: 2
                },
                LayerProblem::Position {
                    layer: n("pulse"),
                    expected: 2,
                    found: 1
                },
            ]
        );
        assert_eq!(
            set().check(&song(&["calm", "tension", "peak"])),
            [
                LayerProblem::Missing { layer: n("pulse") },
                LayerProblem::Position {
                    layer: n("peak"),
                    expected: 3,
                    found: 2
                },
            ]
        );
        let mut s = song(&["calm", "tension", "pulse", "peak"]);
        s.subtunes[2].marks = vec![0];
        assert_eq!(
            set().check(&s),
            [LayerProblem::Mark {
                layer: n("pulse"),
                bar: 1
            }]
        );
    }

    #[test]
    fn sets_are_checked_on_read() {
        assert!(AdaptiveSet::new("", vec![n("a")]).is_err());
        assert!(AdaptiveSet::new("s", vec![]).is_err());
        assert!(
            AdaptiveSet::new("s", vec![n("a"), n("a")])
                .unwrap_err()
                .contains("twice")
        );
        let toml = "name = \"score\"\nlayers = [\"calm\", \"tension\"]\n";
        let s: AdaptiveSet = ::toml::from_str(toml).unwrap();
        assert_eq!(s.layers(), [n("calm"), n("tension")]);
        assert!(
            ::toml::from_str::<AdaptiveSet>("name = \"s\"\nlayers = [\"a\"]\nloop = true\n")
                .is_err()
        );
    }
}
