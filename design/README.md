# Design tokens

Score64 Studio uses the same family design tokens as Convert64 (egui visuals, spacing, text, semantic colours for light and dark, C64 palette), plus Score64 semantics such as voice colours and stage states. The tokens file and its generator arrive with epic E10; the plan is to share one source with Convert64 rather than fork it.

Until then, `crates/score64-app/src/theme/family.rs` is a verbatim copy of the family part of Convert64's generated theme (egui visuals, spacing, text styles and fonts, from commit 6c2d90b), and the app's status and accent colours match Convert64's. When the family tokens change, copy again; E10 replaces the copy with generation from one shared source.

