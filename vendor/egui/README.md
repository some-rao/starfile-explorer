# egui (vendored, patched)

Upstream: <https://github.com/emilk/egui> — crate `egui` 0.31.1, MIT OR Apache-2.0.

Wired in through `[patch.crates-io]` in the workspace `Cargo.toml` (together with
the sibling `vendor/epaint`). It must stay on the same `0.31.x` version that
`eframe`/`egui_extras` resolve to; when bumping egui, copy the matching release
over this directory again and re-apply the two changes below.

## Changes

### 1. Snappier wheel-drain smoothing (`src/input_state/mod.rs`, `begin_pass`)

egui treats ordinary mouse-wheel deltas (`MouseWheelUnit::Line`) as "not
smooth" and drains them into `smooth_scroll_delta` with an exponential filter,
originally "reach 90 % in 0.1 s". At 60 fps that smears every wheel notch over
~15 frames, so the list trails the hand for ~0.25 s and keeps drifting after
the last notch (the "laggy, rubber-band" feel this app is shipped to fix).

Changed to `exponential_smooth_factor(0.90, 0.05, dt)` — 90 % of the backlog
applies within ~50 ms (2–3 frames): the notch still glides a little, but lands
crisply. Closed as of this patch; whether upstream parameterizes it later
(TODO comment remains) may make this vendoring obsolete.

### 2. Device-pixel snapping of scroll offsets (`src/containers/scroll_area.rs`)

Scroll offsets are kept in points and were never rounded to the physical-pixel
grid. At fractional `pixels_per_point` rows then move in sub-pixel steps while
scrolling, and because the font and icon textures are LINEAR-sampled, text
shimmers/blurrs between sub-pixel positions. Windows' own list views always
stand on the pixel grid.

Added `snap_to_device_pixel()` and applied it where interactive scrolling sets
the offset: the wheel path (`state.offset[d] -= scroll_delta`), the
`scroll_to_*` target creation (covers animated targets and the instant case),
and scrollbar-thumb dragging.

### Related, in the app (not here): wheel *distance*

The per-notch distance lives in `egui::Options::line_scroll_speed` (egui
default: 40 px per reported wheel line ≈ one row). Swift Explorer sets it from
Windows' "lines per wheel notch" setting (`SPI_GETWHEELSCROLLLINES`) × the
list row height in `src/app.rs::apply_theme`, so a notch moves like Explorer's.

### 3. Touchpad pan: 1:1 tracking with momentum coasting

Measured on a real precision touchpad (`swift-wheel-log.txt`, see the app's
temporary `SWIFT_EXPLORER_WHEELLOG=1` logger): touchpad pan arrives as
**continuously fractional** `MouseWheelUnit::Line` deltas at ~30 Hz — while a
real mouse wheel notch is always a whole number of lines.

Patching `src/input_state/mod.rs`:

- **Classification:** `Line` deltas with a fractional axis are a touchpad pan.
  Whole-line deltas (and `Page` units) keep the classic wheel path.
- **Scale:** pan is converted with the new `Options::touchpad_line_scroll_speed`
  (app sets it to one list row) instead of `line_scroll_speed`, which the app
  raises to 3 rows per *mouse-wheel* notch — mixing the two was pan racing ~3×
  ahead of the finger.
- **Application:** pan deltas apply directly (1:1 tracking, no extra drain).
- **Momentum:** `InputState` keeps a low-passed pan velocity
  (`touch_pan_velocity`); once pan events fall silent > 100 ms (finger lifted —
  Windows even sends a final flick burst) the motion coasts with that
  velocity, decaying exponentially (τ = 0.3 s, capped at 5000 px/s, stops
  below 30 px/s). A slow crawl coasts a few px, a hard flick keeps flying.
  `wants_repaint_after` keeps frames flowing while coasting.

Real wheel notches and Ctrl+wheel zoom are untouched.

