# Cherenkov migration (issue #205)

The working state of the Vello → Cherenkov port: pins, counters, fixtures,
the boundary check, the consumer inventory, and the conversion checklist.
This file is the migration's own documentation — the design of record is the
pinned plan comment on water-rs/hydrolysis#205.

## Dependency tuple

| package | pinned rev / version | role in the migration |
| --- | --- | --- |
| `waterui` (dev) | `5c58500d75c5eee8e5a52fc69974ede2a723a175` | `Scene2D`/`SceneRecording` trait surface, `DrawContext` (in `waterui-backend-core`), `GpuSurface`/`GpuView`, `vello-scene` + `cpu-scene` features on `waterui-graphics` |
| `hydrolysis-m3` | `d8e6806ece90aa6cd2cdd578c4eb9550958b4d36` | M controls drawing; ported at M1 |
| `cherenkov` | lands at the coordinated cutover (C1 first) | the replacement engine; no dep added until then |
| `vello` stack | fork `lexoliu/vello` revs `76724fcf…` (vello/vello_encoding/vello_shaders 0.9.0) and `a01e5039…` (vello_common/vello_cpu/vello_hybrid/vello_sparse_shaders 0.2.0) | what is being removed |

## Frame counters (`FrameCounters::migration`)

`MigrationCounters` (`src/renderer/migration_counters.rs`) is reset once per
rendered pump in `reset_scene`, so each pump is self-counting. The
`frame_profile` example prints them next to the existing counters.

| counter | counts | expected direction after cutover |
| --- | --- | --- |
| `semantic_builds` | `RenderNode::build` dispatches | down (retained nodes stop rebuilding) |
| `structural_patches` | reconcile/materialize mutations | steady, scoped to real changes |
| `measure_calls` | `measure_body` entries | down — only layout-affecting changes |
| `layout_calls` | `RenderNode::layout` entries | down |
| `recorded_view_contents` | leaf draw-arm re-encodes | down — static structure records once |
| `live_operand_updates` | live-operand writes to recorded commands | up from 0 (engine-era field) |
| `layer_creations` / `layer_removals` | retained engine layer churn | up from 0 when engine layers exist |
| `font_registrations` | `draw_glyphs` calls | down — fonts stop re-registering per frame |
| `image_registrations` | `draw_image` calls + `Brush::Image` sightings | steady |
| `gpu_submissions` | `queue.submit` / `render_to_texture` calls | down — idle windows submit nothing |
| `host_wakeups` | drain-side wakeups: `take_*` true, GPU-surface dirty drains, `platform.request_redraw` | down |

## Acceptance fixtures

`src/renderer/tests/cherenkov_migration.rs` — counter-family assertions
(nonzero/zero direction; exact numbers live in the baseline, not in
assertions, so unrelated dev churn does not break the suite).

| plan fixture family | test |
| --- | --- |
| nested clip/blend/opacity | `nested_clip_blend_opacity_counts` |
| transformed image brushes | `transformed_image_brush_counts` |
| glyph-only scenes | `glyph_only_scene_counts` |
| variable / COLR / bitmap fonts | `variable_and_color_fonts_count` (+ `test-fonts/TestVariable-ABC.ttf`, `BungeeColor-Regular.ttf`) |
| all shadow silhouettes | `shadow_silhouettes_count` |
| context-menu holes | `context_menu_holes_render` |
| popup opening | `popup_opening_counts` |
| scrolling | `scrolling_counts` |
| GPU content under clips/effects | `gpu_content_under_clip_counts` |
| native-view interleaving | not headless-runnable: `record_native_view_layer` exists only under `hydrolysis_macos_system_webview` (winit + macOS + webview-system) and the WKWebView bridge requires a real window — the boundary check quarantines the call site instead |
| capture determinism | `repeated_fixed_clock_captures_are_identical` (byte-identical RGBA; per-pump counters legitimately differ once caches are warm) |

## Boundary check

`scripts/check_renderer_boundary.py` runs in CI on every PR (the only
enforcement — there is no merge freeze). It compares tokenized Vello
references against `scripts/renderer_boundary_baseline.json`, keyed by
file + enclosing item + token: a reference is rejected when it lands in an
untracked file, in an item with no baseline refs, or adds a token the item
did not have (deleting refs is always fine; adding or moving is not).
`use vello::… as Alias` makes `Alias::` uses count, and
`foo = { package = "vello" }` makes `foo::` a Vello reference in source and
`foo` a manifest reference — renamed imports and renamed Cargo deps do not
slip through.

- Regenerate the baseline **only** when a migration step removed references:
  `python3 scripts/check_renderer_boundary.py --write-baseline`. Do not
  regenerate to silence a failure on unrelated work — the failure text is
  the routing instruction.
- `strict: true` in the baseline (set at the final cutover) flips the check
  to zero-tolerance: any Vello reference in source *or* resolved Cargo
  metadata fails, including test and dev-dependency references, and the
  quarantine list is ignored.

Current baseline: **74 files, 1144 token references** (generated from dev at
the P0 commit). New drawing code routes through
`crate::renderer::recording::Recording` — the checker failure message prints
the API and this rule.

## Consumer inventory (resolved Cargo graph)

From `cargo metadata` on the pinned rev — every external package still
touching `Scene2D`, `DrawContext`, `GpuSurface`, or a Vello recording type:

| package | vello deps | where the types land |
| --- | --- | --- |
| `waterui-graphics` @5c58500 | `vello`, `vello_hybrid`, `vello_common`, `vello_cpu` (all optional behind `vello-scene`/`cpu-scene`) | `scene/scene2d_vello.rs`, `scene2d_hybrid.rs`, `scene2d_cpu.rs`, `scene_surface.rs`, `gpu/shared_context.rs`, `lib.rs` feature gates — the `Scene2D`/`SceneRecording` trait itself and the `VelloScene2D`/`HybridScene2D` impls are owned here (W1–W3) |
| `waterui-backend-core` | — | `DrawContext` trait + `Brush` re-export (waterui#1246 retires it) |
| `waterui-testing` | `vello` | `snapshot.rs` (kurbo Rect/Affine, peniko Color), `protocol.rs` (`vello_scene_layers` stat field) — test-harness geometry types only |
| `hydrolysis-m3` @d8e6806 | `vello` | 27 files: `DrawContext`-based control drawing (`theme/state_layer.rs`, `controls/*`, `layout/*`, `material_shapes.rs`, `icons.rs`), `GpuSurface`/`GpuView` imports — ported at M1 |
| `glifo` | `vello_common` | transitive via `vello_hybrid` — leaves with the stack |

## Conversion checklist

Trunk-first preparation (each step lands on dev, green on its own):

- [x] **P0** — counters + fixtures + boundary check + this doc
- [ ] **P1** — `vello::kurbo::*` → `kurbo`, `vello::peniko::*` → `peniko`
  imports (pin the same resolved versions); separate small m3 PR
- [ ] **P2** — private recording boundary
  `src/renderer/recording/{mod,vello}.rs` with the fixed `Recording` API
  (`fill`/`stroke`/`image`/`glyphs`/`blurred_rounded_rect`/`push_clip`/
  `push_group`/`pop_scope`/`append`); `Scene2D` impl + `DrawContext`
  adapter live in the legacy files; Vello only reachable through
  `recording/vello.rs` and `engine/vello_backend.rs`
- [ ] **P3a** — `metadata::{apply_shadow, apply_border}` + frame scopes
- [ ] **P3b** — `context_menu`/`popup_menu`/anchored-overlay/
  `interaction_layers`; destination-out hole inside the isolated
  scrim/shadow group
- [ ] **P3c** — controls + window chrome
- [ ] **P4** — `emit_glyph_run_to_scene`, `CheckedScene2D`,
  `text_service`, `subtree_capture`, test fixtures behind the boundary;
  image validation stays
- [ ] **P5** — stable render identity (`RenderId`/`PresentationId`/
  `RenderKey` in `renderer/retained/identity.rs`)

Then the coordinated W/H/M cutover (W1–W3, H1–H2 generated against dev
head, M1) after C1, and the retained-update activation on dev.
