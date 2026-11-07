# Vey UI & Enhancement Design Guidelines

Every new tab, view, dialog, and UI enhancement in **Vey** MUST follow the **Controls Tile Design System** (`page_controls` / `tile` in `src/ui.rs`). Keep the interface calm, tactile, borderless, and scannable at a glance.

## 1. Core Visual Principles

1. **Borderless Matte Cards (No Wireframe Outlines)**
   - Render cards and interactive rows as solid rounded fills on the black notch canvas (`BLACK = Color(0.0, 0.0, 0.0, 1.0)`):
     - Resting surface: `SURF` (`Color::white(0.09)`)
     - Hover surface: `HOVER` (`Color::white(0.17)`), smoothly interpolated with `hot_k` where animated.
   - **Never** draw `1px` grey `stroke_rrect` outlines around resting cards, list rows, or status pills. Reserve strokes strictly for active keyboard/search focus or floating popovers that overlap other surfaces.
   - Standard corner radii:
     - Standard cards/tiles (`50–54 px` tall): `18.0`
     - Compact list rows (`36–40 px` tall): `14.0`
     - Header chips / search pills (`22–24 px` tall): `11.0–12.0`

2. **Tinted Circular Icon Badges**
   - Anchor every card or list item on the left with a soft circular icon badge:
     - Standard tile (`50–54 px` tall): `34×34` rect at `r.x + 12.0`, vertically centered (`radius = 17.0`).
     - Compact row (`36–40 px` tall): `25×25` rect at `r.x + 8.5`, vertically centered (`radius = 12.5`).
   - Fill the circle with a low-opacity wash of the card's semantic color (`tint.fade(0.16 * ca)`) and draw a crisp Segoe Fluent / MDL2 glyph (`&f.icon14`) in full `tint.fade(ca)`.
   - Semantic accent palette:
     - Sky / Info / Links: `C_CPU` (`Color(0.30, 0.78, 0.98, 1.0)`)
     - Amber / Display / Config: `C_MEM` (`Color(0.98, 0.70, 0.20, 1.0)`)
     - Violet / Audio / Discussion: `C_UP` (`Color(0.72, 0.58, 1.0, 1.0)`)
     - Sage / Success / Completed: `C_BAT` (`Color(0.40, 0.85, 0.50, 1.0)`)
     - Coral / Rose / Alert: `Color(0.98, 0.45, 0.55, 1.0)`
     - Neutral: `T1` (`Color::white(0.96)`)

3. **Two-Line Stacked Typography**
   - Align text cleanly to the right of the circular badge (`x = r.x + 56.0` on standard tiles, `x = r.x + 42.0` on compact rows):
     - **Primary line (top)**: `&f.body` (or `&f.small` in dense 4-row lists) in `T1` (`Color::white(0.96)`) — shows the primary label or current value.
     - **Secondary line (bottom)**: `&f.tiny` in `T3` (`Color::white(0.38)`) — shows concise context, field role, or secondary metadata.
   - Never invert the hierarchy by placing tiny field labels above cramped controls when the standard two-line tile pattern fits.

4. **Strict Grid Alignment**
   - Content area horizontal margins: `x = 24.0`, total width `552.0` (`EXP_W - 48.0`, where `EXP_W = 600.0`).
   - Standard 2-column grid: left column `x = 24.0, w = 270.0`, `12.0` horizontal gap, right column `x = 306.0, w = 270.0`, `8.0` vertical row gap.
   - Full-width banner cards span `x = 24.0, w = 552.0` so their left and right edges align flush with the 2-column tiles below them.
   - Every secondary action (e.g., opening a link, viewing comments, configuring settings, retrying sync) belongs inside a grid-aligned tile rather than floating as an isolated pill or corner circle.

## 2. Verification Checklist for UI Changes

- Reuse `card_tile` / `tile` patterns in `src/ui.rs` whenever adding new cards, settings, or actions.
- Verify hit targets and render Direct2D visual snapshots via `cargo test` (`WORK_PREVIEW_DIR` / `PREVIEW_DIR`) to confirm alignment, contrast, and text truncation before completing any UI task.
