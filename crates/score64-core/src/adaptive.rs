//! Adaptive sets (`docs/TAD.md` §3.1): a project declares which subtunes of a song are layers of one adaptive piece,
//! in the order the game steps through them, and where the game may switch between them. The validator checks the
//! song against it (rule 5): the layers sit at consecutive positions in that order, so the game can switch by subtune
//! number, and every layer has a timing mark at every declared switch point, so the game knows when it is there.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::score::{Song, Subtune, SubtuneName};

/// Where the game may switch between an adaptive set's layers. Each switch point needs a timing mark.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Switch {
    /// On the first row of every bar.
    #[default]
    EveryBar,
    /// On the first row of every N bars, counting from bar 1.
    EveryBars(NonZeroU32),
    /// On these rows only, in increasing order.
    Rows(Vec<u32>),
}

impl Switch {
    /// The switch rows in `subtune`, and any declared rows past its end.
    fn points(&self, subtune: &Subtune) -> (Vec<u32>, Vec<u32>) {
        let length = subtune.length.get();
        let bar = u32::from(subtune.rows_per_bar.get());
        match self {
            Switch::EveryBar => ((0..length).step_by(bar as usize).collect(), vec![]),
            Switch::EveryBars(n) => {
                let step = bar.saturating_mul(n.get());
                ((0..length).step_by(step as usize).collect(), vec![])
            }
            Switch::Rows(rows) => rows.iter().partition(|&&r| r < length),
        }
    }

    fn is_every_bar(&self) -> bool {
        *self == Switch::EveryBar
    }
}

/// An ordered list of layers, each a subtune by name; for example `calm`, `tension`, `pulse`, `peak`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "AdaptiveSetFields", into = "AdaptiveSetFields")]
pub struct AdaptiveSet {
    name: String,
    layers: Vec<SubtuneName>,
    switch: Switch,
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
    /// Where the game may switch layers: `"every_bar"` (the default), `{ every_bars = N }` or
    /// `{ rows = [...] }`.
    #[serde(default, skip_serializing_if = "Switch::is_every_bar")]
    switch: Switch,
}

impl AdaptiveSet {
    /// A set switching on every bar, if it has a name and at least one layer, with no layer twice.
    pub fn new(name: impl Into<String>, layers: Vec<SubtuneName>) -> Result<Self, String> {
        Self::with_switch(name, layers, Switch::EveryBar)
    }

    /// A set with its switch points; explicit rows must be non-empty and increasing.
    pub fn with_switch(
        name: impl Into<String>,
        layers: Vec<SubtuneName>,
        switch: Switch,
    ) -> Result<Self, String> {
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
        if let Switch::Rows(rows) = &switch
            && (rows.is_empty() || rows.windows(2).any(|w| w[0] >= w[1]))
        {
            return Err(format!(
                "adaptive set `{name}` must list its switch rows once each, in increasing order"
            ));
        }
        Ok(AdaptiveSet {
            name,
            layers,
            switch,
        })
    }

    /// Its label.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its layers, in order.
    pub fn layers(&self) -> &[SubtuneName] {
        &self.layers
    }

    /// Where the game may switch layers.
    pub fn switch(&self) -> &Switch {
        &self.switch
    }

    /// Every way `song` breaks this set: a layer missing or out of place, a switch point without a timing mark, or a
    /// declared switch row past a layer's end.
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
            let (points, outside) = self.switch.points(subtune);
            for row in points {
                if !subtune.marks.contains(&row) {
                    out.push(LayerProblem::Mark {
                        layer: layer.clone(),
                        row,
                    });
                }
            }
            for row in outside {
                out.push(LayerProblem::PastEnd {
                    layer: layer.clone(),
                    row,
                });
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
    /// The switch point at `row` of `layer` has no timing mark.
    Mark {
        /// The layer.
        layer: SubtuneName,
        /// The row.
        row: u32,
    },
    /// The declared switch row `row` is past the end of `layer`.
    PastEnd {
        /// The layer.
        layer: SubtuneName,
        /// The row.
        row: u32,
    },
}

impl TryFrom<AdaptiveSetFields> for AdaptiveSet {
    type Error = String;
    fn try_from(f: AdaptiveSetFields) -> Result<Self, String> {
        AdaptiveSet::with_switch(f.name, f.layers, f.switch)
    }
}

impl From<AdaptiveSet> for AdaptiveSetFields {
    fn from(s: AdaptiveSet) -> Self {
        AdaptiveSetFields {
            name: s.name,
            layers: s.layers,
            switch: s.switch,
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
                row: 16
            }]
        );
    }

    #[test]
    fn sets_declare_their_switch_points() {
        let layers = || vec![n("calm"), n("tension")];
        let mut s = song(&["calm", "tension"]);
        s.subtunes[0].length = NonZeroU32::new(64).unwrap(); // four bars
        s.subtunes[0].marks = vec![0, 32];
        s.subtunes[1].marks = vec![0];

        let every_two = AdaptiveSet::with_switch(
            "s",
            layers(),
            Switch::EveryBars(NonZeroU32::new(2).unwrap()),
        );
        assert_eq!(
            every_two.unwrap().check(&s),
            [],
            "bars 1 and 3 of calm, bar 1 of tension"
        );

        let rows = AdaptiveSet::with_switch("s", layers(), Switch::Rows(vec![0, 40])).unwrap();
        assert_eq!(
            rows.check(&s),
            [
                LayerProblem::Mark {
                    layer: n("calm"),
                    row: 40
                },
                LayerProblem::PastEnd {
                    layer: n("tension"),
                    row: 40
                },
            ]
        );
        assert!(AdaptiveSet::with_switch("s", layers(), Switch::Rows(vec![16, 16])).is_err());
        assert!(AdaptiveSet::with_switch("s", layers(), Switch::Rows(vec![])).is_err());

        // The default switches on every bar: bar 2 of calm and of tension are unmarked.
        assert_eq!(AdaptiveSet::new("s", layers()).unwrap().check(&s).len(), 3);

        for (text, want) in [
            ("", Switch::EveryBar),
            ("switch = \"every_bar\"\n", Switch::EveryBar),
            (
                "switch = { every_bars = 4 }\n",
                Switch::EveryBars(NonZeroU32::new(4).unwrap()),
            ),
            ("switch = { rows = [0, 64] }\n", Switch::Rows(vec![0, 64])),
        ] {
            let toml = format!("name = \"s\"\nlayers = [\"calm\"]\n{text}");
            let set: AdaptiveSet = ::toml::from_str(&toml).unwrap();
            assert_eq!(set.switch(), &want, "{text}");
            let back: AdaptiveSet = ::toml::from_str(&::toml::to_string(&set).unwrap()).unwrap();
            assert_eq!(back, set);
        }
        assert!(
            ::toml::from_str::<AdaptiveSet>(
                "name = \"s\"\nlayers = [\"a\"]\nswitch = { every_bars = 0 }\n"
            )
            .is_err()
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
