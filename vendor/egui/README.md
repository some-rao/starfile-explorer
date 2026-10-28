# egui (vendored, patched)

Upstream: <https://github.com/emilk/egui> — crate `egui` 0.31.1, MIT OR Apache-2.0.

Wired in through `[patch.crates-io]` in the workspace `Cargo.toml` (together with
the sibling `vendor/epaint`). It must stay on the same `0.31.x` version that
`eframe`/`egui_extras` resolve to; when bumping egui, copy the matching release
over this directory again and re-apply the patches below.

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

Measured on a real precision touchpad (`swift-wheel-<pid>.log`, see the app's
temporary `SWIFT_EXPLORER_WHEELLOG=1` logger): touchpad pan arrives as
**continuously fractional** `MouseWheelUnit::Line` deltas at ~30 Hz — while a
real mouse wheel notch is always a whole number of lines.

Patching `src/input_state/mod.rs`:

- **Classification:** `Point` deltas and `Line` deltas with a fractional axis
  are a touchpad pan; whole-line `Line` and `Page` units keep the classic wheel
  path. (Previously only fractional `Line` was recognized, so a driver that
  reports pixel deltas — some precision touchpads and smooth-scroll mice do —
  fell through to the wheel path and stepped notch by notch.)
- **Scale:** pan is converted with `Options::touchpad_line_scroll_speed` (app
  sets it to one list row) instead of `line_scroll_speed`, which the app raises
  to 3 rows per *mouse-wheel* notch — mixing the two made pan race ~3× ahead of
  the finger.
- **Application (default, `Options::pan_pacing = false`):** the delta that
  arrived since the last frame is applied in that frame — 1:1 tracking, no
  backlog, so the content stays under the finger. `InputState::pan_momentum()`
  therefore reports zero slip in this mode.
- **Paced mode (`pan_pacing = true`, diagnostic A/B):** deltas fill
  `pan_pending` and each frame hands out `velocity * dt` — an even stream that
  trails the finger by the backlog. Selected with `SWIFT_PAN_PACING=1`.
- **Lift detection:** the pad sends no "contacts up" message, so a gap in the
  event stream is the only lift signal. The gate follows the observed cadence
  (`pan_gap`: 1.6 × the smoothed inter-event gap, clamped to 35–100 ms) instead
  of a fixed 100 ms, so a 30 Hz pad does not stall for three frames before the
  coast starts.
- **Momentum:** on the lift, the speed is measured over the last 80 ms of pad
  reports (`release_velocity`), not taken from the per-frame low-pass: a finger
  slows down before it leaves the glass, and the low-pass had already been
  dragged down by those small reports, so a flick coasted far less than the
  flick itself. It then coasts, decaying exponentially (τ = 0.3 s, capped at
  5000 pt/s, stops below 30 pt/s). A slow crawl coasts a few pt, a hard flick
  keeps flying. `wants_repaint_after` keeps frames flowing while coasting.
- **Sign bug fixed:** the settle test was `velocity.max_elem().abs()`, and
  `max_elem()` of a purely vertical pan is the *x* element, i.e. `0`. Any
  downward-only pan therefore looked settled and had its velocity zeroed every
  frame: momentum worked scrolling one way and vanished scrolling the other.
  `velocity.abs().max_elem()` (also in `wants_repaint_after`) is the correct
  test; `coast_works_in_both_directions` in `src/app.rs` guards it.

Real wheel notches and Ctrl+wheel zoom are untouched.

### 4. Momentum read-out for the app's diagnostics (`src/input_state/mod.rs`)

`InputState::pan_momentum()` (velocity, not-yet-applied backlog), `pan_delta()`
(this frame's pad delta) and `pan_events()` (how many pad reports were folded
into it — more than one means the frame was late and the pad's reports piled up)
expose the pan state to the app. They are read only by the
`SWIFT_EXPLORER_WHEELLOG` frame logger (`src/app.rs::perf_log`); no behaviour
depends on them.


