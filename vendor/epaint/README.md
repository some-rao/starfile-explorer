# epaint (vendored, patched)

Upstream: <https://github.com/emilk/egui> — crate `epaint` 0.31.1, MIT OR Apache-2.0.

This copy is wired in through `[patch.crates-io]` in the workspace `Cargo.toml`.
It must stay on the same `0.31.x` version that `eframe`/`egui` resolve to; when
bumping egui, copy the matching `epaint` release over this directory again and
re-apply the two changes below.

## Changes

### 1. Hinted glyph rasterization (`src/text/font.rs::allocate_glyph`)

ab_glyph rasterizes outlines **without TrueType hinting**, so vertical stems
straddle two pixel columns and every word has a different weight mix. Glyphs
are now rendered with `swash`'s hinted scaler (`Source::Outline`,
`.hint(true)`) and blitted into the atlas; **metrics still come from ab_glyph**
(advance, ascent, kerning), so layout is untouched. The em size handed to swash
is derived from the font's own metrics
(`scale_in_pixels × units_per_em / height_unscaled`), which also keeps variable
fonts (e.g. `SegUIVar.ttf`) at the same em size egui thinks it is using.

Hinting is enabled up to `MAX_HINT_EM` (32 px em) so large display text is not
shape-distorted. Disable with `SWIFT_EXPLORER_NO_HINT=1`.

### 2. Coverage→alpha curve in *final alpha* space (`coverage_to_alpha`)

egui uploads the font atlas after applying `coverage.powf(0.55)`
(`FontImage::srgba_pixels`), which silently re-inflates any curve designed in
raw coverage space (the earlier version of this patch had exactly that bug).
The curve is therefore specified as the **on-screen** alpha
`alpha = coverage^edge_gamma` and stored as `coverage^(edge_gamma / 0.55)`.
Defaults: `edge_gamma = 1.15`; tune with `SWIFT_EXPLORER_EDGE_GAMMA`
(1.0 = plain coverage, higher = lighter fringes). Skipped below 12 px em.

### Measurement (Segoe UI, 19 px em, GDI as ground truth)

"Downloads" line metrics — dark = >50 % ink pixels, mid = 2–50 % (the AA
fringe), ink = Σ darkness over the whole canvas:

| render                          | dark | mid | dark/mid | ink |
|---------------------------------|-----:|----:|---------:|----:|
| GDI **ClearTypeGridFit**        |  2958*| 2525*|     1.17*|  3000*|
| GDI **AntiAliasGridFit** (hint) |  299 | 239 |     1.25 |  320 |
| ab_glyph + old patch (floor .22)|  364 | 109 |     3.34 |  338 |
| swash hinted + `edge_gamma 1.15`|  316 | 281 |     1.12 |  334 |
| swash hinted + `edge_gamma 1.2` |  310 | 286 |     1.08 |  329 |

\* whole-canvas numbers, others are line-1 only; ratios are the comparable
quantity. The old patch collapsed the AA fringe (dark/mid 3.3× Windows');
the new pipeline lands within ~0.1 of Windows' grayscale-hinted profile and
~4 % of its ink. True ClearType (subpixel RGB) would additionally require a
per-channel atlas and shader changes in egui_glow and is out of scope.
