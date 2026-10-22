# Bundled fonts

The app's text fonts, the same files as Convert64's (`crates/convert64-app/assets/fonts`), so the two apps look like one family. Both families are licensed under the SIL Open Font License 1.1; the licence texts ship next to the fonts.

| File | Family / weight | Release | Size (bytes) | SHA-256 |
|---|---|---|---|---|
| `Inter-Regular.ttf` | Inter Regular (`FontFamily::Proportional`) | [rsms/inter v4.1](https://github.com/rsms/inter/releases/tag/v4.1), `Inter-4.1.zip` → `extras/ttf/` | 411 640 | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| `Inter-Medium.ttf` | Inter Medium (named family `medium`, for headings and emphasis) | same archive | 417 300 | `97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872` |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono Regular (`FontFamily::Monospace`) | [JetBrains/JetBrainsMono v2.304](https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304), `JetBrainsMono-2.304.zip` → `fonts/ttf/` | 273 900 | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |
| `OFL-Inter.txt` | Inter's `LICENSE.txt` from the same archive | | | |
| `OFL-JetBrainsMono.txt` | JetBrains Mono's `OFL.txt` from the same archive | | | |

The icon font comes from the `egui_material_icons` crate (Material Symbols Outlined, Apache-2.0), not from here. Inter has glyphs of its own in the private-use range the icons use, so icons are drawn in the icon font explicitly (`Icon::rich_text`), never as plain text in Inter.

To update: copy the files from Convert64 (or from the upstream releases above), update this table, and keep both apps on the same files.
