# #2536 coverage specification: one rendering contract for both coverage routes

water-rs/waterui#2536 adds a GPU compute coverage route for large and
self-intersecting paths behind an unchanged API. This document (#2542) is
the single rendering specification both routes answer to, so the GPU route
is an implementation of the existing visual design rather than a second
one. It describes what is in the tree today, line by line, and states
what the GPU route must reproduce. Every design choice is decided here,
each with the alternatives it was chosen over; section 7 lists them.

Scope: only coverage computation moves to compute. Painting, blending,
clipping and composition stay in the raster pipeline, which reads the same
emission form from either route.

**Standing constraint: no CPU readback.** Hydrolysis's contract forbids
any runtime path that reads a GPU target back to the CPU. Nothing in the
GPU route may read coverage, a classification result, a cell count or a
buffer length back to the CPU inside a frame. This single constraint
decides most of what follows: the CPU cannot see the kernel's output, so
it cannot classify strips, cannot allocate atlas cells for a path whose
cell count it does not know, and cannot choose a route from a measured
result. Every decision below is the one that holds without a readback.

## 1. The coverage model

Both routes approximate the same mathematical object: for each device
pixel, the area of `pixel ∩ inside(flattened path, fill rule)`, in
`[0, 1]`. That is exactly what the oracle computes in `f64`
(`oracle/src/coverage.rs`: per-pixel geometric area, segments clipped to
the pixel, the pixel split into horizontal strips at endpoints and
crossings, crossings paired by the fill rule, winding seeded at the left
edge so overlap keeps its true coverage; `FLATTEN_TOLERANCE = 1e-4`,
predicate `EPS = 1e-9`). Anti-aliasing is therefore not a filter applied
after a binary decision: the coverage value *is* the analytic area, and
the raster pipeline multiplies it into the paint's alpha.

### 1.1 What the CPU route does today

The CPU sweep route is `gpu/src/render/path.rs` plus
`gpu/src/render/raster.rs` plus `src/lowering.rs::resolve_winding`, in
this order:

1. **Shape to local path.** `path::shape_path` (`path.rs:52`) takes the
   shared outline from `cherenkov::lowering::shape_outline`; a line
   becomes its open segment.
2. **Stroke expansion**, when the draw is a stroke
   (`gpu/src/render/lower.rs:4346`–`4360`): user-space tolerance
   `tol = path::FLATTEN / sigma_max(transform)` (clamped by `max(1e-12)`),
   then `kurbo::stroke(path, stroke, &StrokeOpts::default(), tol)`.
3. **Device flattening.** `path::flatten_segments(&device_path, FLATTEN)`
   (`path.rs:67`), `FLATTEN = 0.02` device pixels (`path.rs:30`), via
   `kurbo::flatten`. Every subpath is closed implicitly; zero-length
   segments are dropped; segment endpoints are narrowed to `f32`.
4. **Raster rect.** `path::rasterize` (`path.rs:122`) inflates the
   segment bbox by 1 px, intersects it with the surface rect, and takes
   `floor`/`ceil` to integers. `Coverage::clipped` records whether the
   surface clipped the rect; an empty intersection returns `None` and the
   draw emits nothing and stores nothing.
5. **Winding resolution.** `lowering::resolve_winding(segments, rule)`
   (`src/lowering.rs:2054`) sweeps the flattened device segments left to
   right inside crossing-free horizontal bands and re-emits each band's
   inside/outside boundary edges, so every covered region carries winding
   exactly `1`. Its purpose is stated in its own documentation
   (`lowering.rs:2033`–`2047`): the area accumulator reads net winding,
   so same-sign overlap would clamp (`0.6 + 0.6 → 1.0`, losing the
   union's true `0.84`) and opposite-sign overlap would cancel. When the
   input has no overlap it returns `None`, the original segments and rule
   are kept, and non-overlapping scenes stay bit-identical. When it
   returns `Some`, the rule used downstream becomes `NonZero` regardless
   of the source rule (`path.rs:140`–`146`).
6. **Signed-area accumulation.** `raster::Raster` (`raster.rs`) is the
   font-rs accumulation rasterizer: a `(w + 1) * h` `f32` buffer;
   `draw_line` (`raster.rs:37`) deposits, per scanline row, the signed
   area `dy * dir` split between the column the edge crosses and its
   right neighbour. Horizontal edges (`|y0 - y1| <= f32::EPSILON`)
   deposit nothing. `path::clip_x` (`path.rs:176`) clips each segment to
   `0 <= x <= w` first, drawing the `x < 0` part as a vertical stub at
   `x = 0` so its winding is not lost, and dropping the `x > w` part
   whose deposits land in the unread trailing column.
7. **Prefix sum and fill-rule fold.** `Raster::coverage_rule`
   (`raster.rs:129`) prefix-sums each row and folds:
   - `NonZero`: `acc.abs().min(1.0)`;
   - `EvenOdd`: `let a = acc.abs() % 2.0; 1.0 - (a - 1.0).abs()` — a
     triangle wave of period 2.
   The clamp is the only place coverage is bounded; the accumulator
   itself is unbounded `f32`.
8. **R8 quantization.** Coverage stays `f32` in `Coverage::data` and is
   quantized exactly once, when a cell's texels are appended to the
   lowering batch's arena: `(c.clamp(0.0, 1.0) * 255.0).round() as u8`
   (`path.rs:256`). `f32::round` is half-away-from-zero. Path-clip masks
   quantize identically at `lower.rs:4885`. The atlas texture is
   `R8Unorm` (`glyph.rs:454`), so the shader reads `n / 255`.

The specification for both routes is therefore: **per-pixel analytic
area of the fill-rule region of the device-space path flattened at
`FLATTEN = 0.02` device px, with overlap resolved so coverage is the
union's true area, quantized to 8 bits by `round(clamp(c, 0, 1) * 255)`.**

### 1.2 Error budget of the model

Three sources of deviation from the oracle are inherent to the
specification and permitted to both routes:

- **Flattening.** `FLATTEN = 0.02` device px against the oracle's
  `1e-4` user-space tolerance (`oracle/src/coverage.rs`), with the
  oracle's stroke path flattening at `SEGMENT_TOLERANCE =
  FLATTEN_TOLERANCE * 0.5` (`oracle/src/path.rs`). A chord 0.02 px from
  the true curve moves a boundary pixel's coverage by up to ~0.02.
- **`f32` accumulation.** The engine accumulates area and prefix-sums in
  `f32`; the oracle works in `f64`. The prefix sum's error grows with
  the row's column count.
- **R8 quantization.** `±1/510` (≈0.00196) of coverage at worst, per
  pixel, before paint.

The GPU route must not add a fourth source of a different *kind*. In
particular it must not substitute a sampled/stochastic coverage estimate,
a conservative or multi-sample approximation, or a different fill-rule
fold: the oracle-relative error must stay of the same character and the
same magnitude as the CPU route's (section 4).

**Decision C1 — the GPU route accumulates in `f32`.** Per-cell signed
area, per-cell cover and the strip's backdrop prefix are all `f32`, as is
the fill-rule fold, so the route's error is of the same character and
magnitude as the CPU route's. Quantization to R8 is the same expression
(section 1.1 step 8) applied to the same `[0, 1]` value.

Bit-identity with the CPU route is explicitly *not* the contract: the
summation order of a strip-parallel kernel differs from a row-serial
prefix sum, and requiring identical bits would dictate the kernel's
shape. Acceptance is the tolerance gate of section 4.3.

*Alternatives considered:* `f16` or fixed-point accumulation with atomics
(cheaper, and the hardware floor guarantees f16 arithmetic, but it adds
an error source of a different magnitude and would force the section 4
thresholds wider); bit-identical R8 output (strongest contract, but it
constrains summation order and precision for no visual gain).

## 2. Geometry semantics are route-independent

Fill rules, stroke joins, caps, miter limits, dashes and hairlines are
properties of the *geometry*, resolved before any coverage is computed.
They are therefore identical on both routes by construction, and the
GPU route must consume the same expanded outline rather than re-deriving
it:

- **Fill rules.** `FillRule::NonZero` and `FillRule::EvenOdd` only. The
  fold is section 1.1 step 7; after `resolve_winding` returns `Some`,
  the downstream rule is `NonZero`.
- **Stroke expansion** is `kurbo::stroke` with `StrokeOpts::default()`
  — joins (`Round`, `Miter` with its limit, `Bevel`), caps
  (start/end independently), dash pattern and dash offset are kurbo's
  semantics, not Cherenkov's. The oracle calls the same kurbo function
  (`oracle/src/path.rs`), which is why stroke scenes compare at all.
- **Tolerance scaling.** Stroke expansion happens in user space, so the
  device tolerance is divided by `sigma_max(transform)` — the largest
  singular value of the affine's linear part (`path.rs:40`, documented
  as mirroring `cherenkov_oracle::path::sigma_max`). Both engine and
  oracle use the same definition.
- **Hairlines and sub-pixel widths** are not special-cased anywhere in
  this path: a thin stroke is expanded by kurbo like any other and gets
  whatever analytic area its outline has. There is no minimum-width
  snapping and none may be introduced by the GPU route.
- **Analytic primitives bypass this route entirely.** Rounded rects,
  circles, ellipses, continuous-corner shapes and distance-field strokes
  are drawn by `KIND_FILL`/`KIND_STROKE_DIST` SDF instances, not by
  coverage cells. The GPU coverage route changes nothing there.

The contract: the GPU route receives the expanded outline in device space
— the output of step 2 in section 1.1 — and is responsible for steps 3
through 8. Flattening moves to the GPU with #2543; the tolerance, and the
`sigma_max` scaling that derives the user-space stroke tolerance from it,
are unchanged, so the flattened geometry the kernel integrates is the
same geometry the CPU route would have produced to within the flattener's
own tolerance.

**Decision G1 — winding resolves inside the kernels; no intersections
are solved anywhere.** The GPU route does not run `resolve_winding`, and
it does not solve segment intersections. Each cell accumulates the signed
area its segment pieces deposit *and* the signed cover (net winding
crossing the cell's right edge); a prefix along the strip turns per-cell
cover into each cell's backdrop winding; the fill-rule fold of section
1.1 step 7 then maps `backdrop + local area` to coverage. Overlap is
therefore correct because the accumulated quantity is a true winding
number per pixel, not a clamped sum of per-contour coverages — the
failure `resolve_winding` exists to prevent (`lowering.rs:2033`–2047)
cannot arise, because nothing clamps before the fold.

`resolve_winding` stays on the CPU route, unchanged, and keeps its
`None` fast path so non-overlapping CPU-routed scenes stay
bit-identical.

*Alternatives considered:* keep the CPU sweep and hand the kernel
resolved boundary edges (smallest change, but it leaves on the CPU
exactly the cost #2536 exists to remove, and the sweep's sort dominates
large self-intersecting paths); run the sweep itself as a GPU pass
(solves intersections on the GPU, far more complex than area-and-cover
accumulation and unnecessary once nothing clamps early).

## 3. The shared emission form

Both routes emit the same thing: full-coverage runs as solid instances,
partial coverage as R8 coverage cells. This is `path::emit`
(`path.rs:228`) and it is the semantic interface the GPU route must
satisfy. The *storage* of the partial cells differs by route — atlas for
the CPU route, a per-frame storage buffer for the GPU route (decision
S1) — and nothing else does.

- **Small bbox.** When `coverage.w <= SMALL_BBOX && coverage.h <= SMALL_BBOX`
  (`SMALL_BBOX = 32.0`, `path.rs:27`) the whole grid is one cell with no
  interior interval and no strip partitioning (`path.rs:277`).
- **Strips.** Otherwise the grid is cut into `STRIP_H = 4` row strips
  (`path.rs:15`, `path.rs:291`). The last strip is short when `h % 4 != 0`.
- **Column classes.** `strip_class` (`path.rs:355`) labels each column of
  a strip `0` empty (every row exactly `0.0`), `1` full (every row exactly
  `1.0`), `2` partial. The comparisons are exact `f32` equality, by
  design.
- **Full-run demotion.** A run of class-1 columns shorter than
  `FULL_RUN_MIN = 8` (`path.rs:19`) is relabelled `2`, so isolated full
  columns do not fragment a partial run (`path.rs:380`).
- **Gap absorption.** A run of class-0 columns of width `<=
  PARTIAL_GAP_MAX = 8` (`path.rs:23`) that is bounded on *both* sides by
  class-2 columns is relabelled `2` and carried as zero texels inside one
  cell (`path.rs:394`). A gap adjacent to a full run or to the grid edge
  is not absorbed.
- **Spans.** A maximal class-1 run becomes a device rect
  `[x0, strip_y0, x1, strip_y1]` pushed to `PathEmit::spans`
  (`path.rs:306`), replayed as `KIND_SPAN` instances — full coverage, no
  atlas sampling.
- **Cells.** Everything else becomes a `PathCell` (`glyph.rs:165`) with
  its device rect, the location of its texels (an atlas origin plus shelf
  slot on the CPU route, a storage-buffer offset on the GPU route), and
  `interior` — the `start | end << 16` column interval, relative to `x0`,
  that is known fully covered inside the cell (`path.rs:324`). A cell
  emission greedily consumes `partial… full… partial…` in one pass
  (`path.rs:312`–`338`), so one cell may carry one retained full
  interval.
- **Interior promotion.** On replay, a cell with `interior != 0` under a
  solid paint becomes `KIND_REGION` (`lower.rs:4810`), whose opaque
  vertex stage narrows the quad to the interior interval
  (`shader.wgsl:715`) and whose partial fragment stage fetches coverage
  exactly like `KIND_GLYPH` (`shared.wgsl:188`, `shader.wgsl:819`) —
  from the atlas on the CPU route, from the per-frame buffer on the GPU
  route (decision S1).
- **Texel order.** A cell's texels are row-major, `w * h` bytes,
  appended to the lowering batch's arena as a byte range
  (`CellTexels = (u32, u32, Range<usize>)`, `glyph.rs:1954`); cell order
  in `PathEmit::cells` matches the raster order index for index
  (`path.rs:217`).

**Decision E1 — a second compute kernel classifies and compacts.** The
coverage kernel writes coverage; a second kernel classifies each strip's
columns into empty, full-run and partial under *the same* semantics —
exact `0.0`/`1.0` per-row tests, `FULL_RUN_MIN = 8` demotion,
`PARTIAL_GAP_MAX = 8` two-sided gap absorption, `SMALL_BBOX = 32`
whole-grid cell, and the same greedy `partial… full… partial…` merge that
yields one retained `interior` interval per cell — and writes two compact
lists, spans and partial cells, plus the indirect draw arguments for
each. The raster passes draw them with indirect instanced draws, so the
CPU never learns how many of either there are.

The WGSL classifier and `path::strip_class`/`path::emit` are two
implementations of one specification; they are kept in lockstep by the
route-equivalence gate of section 4.3 threshold 4, which fails on any
divergence that changes pixels, and by the shared constants being stated
once here.

*Alternatives considered:* write the coverage grid to a buffer and run
the existing CPU `emit` on it (keeps the emission bit-identical, but
requires a readback — forbidden); emit cells only, with no span
extraction (trivial kernel, but it discards the full-run and
`KIND_REGION` interior optimizations that make large fills cheap, and
multiplies sampled instances and coverage bytes).

## 4. Oracle tolerance

### 4.1 How the measurement was taken

Measured on this document's base, `origin/dev` at `6091fc8f5`
("Merge pull request #2568 from water-rs/perf/2540-ellipse-tiers"), with
`cargo build --release -p cherenkov-bench --features cherenkov`, on the
MacBook Pro (Apple M2 Max, Metal, offscreen — no window, no display
output):

    cherenkov-bench render --engine cherenkov --scene scenes/corpus/<scene> --out <scene>.json

for every corpus scene whose content is path, stroke, path-clip or
path-silhouette coverage: 65 scenes. No `--present` flag, so the
comparison reads back the working-space target directly — premultiplied
**extended linear Display P3**, `Rgba16Float`, HDR channels unclamped —
and compares it to the oracle's `f64` working-space image. Nothing is
clipped to sRGB or to `[0, 1]` anywhere in the comparison: FLIP's XYZ
step uses the P3→XYZ matrix (`oracle/src/metrics.rs:11`–`15`) and
HDR-FLIP engages automatically on scenes with reference values above
`1.0`, over the fixed exposure ladder `0..4 EV`
(`metrics.rs:379`) so scenes stay comparable. `max_local_error` is
`max |box3x3(ref) − box3x3(test)|` per channel (`metrics.rs:423`).

Reported values are `metrics.flip_mean`, `metrics.flip_max`,
`metrics.max_local_error` from each scene's report. Five of the 65 scenes
are HDR (reference above SDR white) and were measured with HDR-FLIP.

### 4.2 Measured CPU route versus oracle, dev, 65 path scenes

| metric | min | median | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- |
| `flip_mean` | 0.000705 | 0.002847 | 0.004124 | 0.004638 | **0.005326** |
| `flip_max` | 0.000795 | 0.031712 | 0.060532 | 0.105260 | **0.135176** |
| `max_local_error` | 0.000131 | 0.006205 | 0.010507 | 0.053262 | **0.077068** |

Worst scene per metric: `flip_mean` `glyph-stroke-dashed` (0.005326);
`flip_max` `stroke-join-continuous-sharp-round` (0.135176);
`max_local_error` `stroke-dash-hdr` (0.077068, HDR). Full per-scene
table in appendix A.

Two observations that shape the thresholds. First, `flip_mean` is tightly
clustered: the whole corpus sits inside one order of magnitude, and the
existing suite bound for the destructive-scene gate is already
`flip_mean < 0.02` (`bench/tests/destructive.rs:117`), roughly 4× the
measured worst case. Second, `flip_max` and `max_local_error` have long
tails driven by single pixels on sharp stroke joins and by HDR scenes
where an absolute channel difference is scaled by a large value — those
are the two places a coverage change shows up first.

### 4.3 Proposed thresholds for the GPU route

The governing rule is `AGENTS.md`: a change that is not meant to change
pixels keeps every scene bit-identical, and a change that does must be
no worse than dev on every scene and better overall. The GPU route is
not meant to change pixels. The thresholds below are the *acceptance
gate for the GPU route when it is not bit-identical*, measured the same
way as section 4.1 (P3, unclipped, HDR-FLIP where the reference exceeds
SDR white):

1. **Per-scene primary gate — no regression.** For every scene, the
   GPU route's `flip_mean` must be `<= max(dev_flip_mean * 1.05,
   dev_flip_mean + 0.0005)`, and the corpus-wide mean of `flip_mean`
   must not increase. Basis: the measured spread between adjacent
   scenes of the same family (e.g. `stroke-join-rect-miter-hi` 0.002445
   versus `stroke-join-rect-miter-lo` 0.003032) is larger than 5%, so a
   5% band admits no visible change while the additive floor keeps the
   gate meaningful on the `0.0007`-class scenes.
2. **Absolute ceilings.** `flip_mean <= 0.006`, `flip_max <= 0.15`,
   `max_local_error <= 0.09` on every path scene. Basis: the measured
   maxima are 0.005326, 0.135176 and 0.077068; each ceiling is the
   measured maximum rounded up by roughly 10–15%, so a route that is
   genuinely equivalent passes and one that degrades a single sharp
   join or an HDR dash edge fails.
3. **No new worst scene.** The scene holding the maximum of each metric
   must not change identity, and its value must not rise. Basis: the
   maxima are concentrated in exactly the geometry a coverage change
   would hit first (`stroke-join-continuous-sharp-round` for `flip_max`,
   `stroke-dash-hdr` for `max_local_error`).
4. **Route-equivalence gate, stronger than the oracle gate.** For any
   scene a GPU route accepts, the same scene forced through the CPU
   sweep must agree with it: `flip_mean <= 0.0005` and
   `max_local_error <= 0.004` *between the two engine images*. Basis:
   these are below the smallest oracle-relative values measured
   (`flip_mean` 0.000705, `max_local_error` 0.000131 are the per-scene
   floors, and R8 quantization alone is ±0.00196 of coverage), so the
   bound says the two routes differ by at most quantization-scale noise
   rather than by a visible amount.
5. **Eye review.** Every GPU-routed scene is reviewed by looking at it
   and at its heatmap, per `AGENTS.md`; the numbers above are a gate,
   not the acceptance criterion.

**Decision T1 — acceptance is tolerance-based, with thresholds 1–5 as
written.** #2536 already states a tolerance contract, and decision C1
declines bit-identity, so the gate above *is* the acceptance criterion.
Thresholds 1–3 apply to the corpus scenes that route through the GPU;
threshold 4 applies between the CPU and GPU images of the same scene;
threshold 5 applies to all of them. Because the GPU route is only
selected for paths the CPU route handles slowly and the new scenes of
decision T2 arrive with it, the #2536 PR is a change that intends to
change pixels for those scenes and must say so, per `AGENTS.md`; every
existing scene that keeps the CPU route stays bit-identical.

*Alternatives considered:* require bit-identical R8 texels (simplest
gate and no corpus churn, but it forces C1 to a fixed summation order
and precision); require bit-identity for scenes the CPU route would have
taken and tolerance only beyond that (no churn either, but it makes the
contract depend on the selection predicate, so a predicate tweak could
turn a passing scene into a failing one).

**Decision T2 — new corpus scenes, plus a `#[cfg(test)]`-only route
hook.** #2546 adds path scenes large enough that the predicate of
section 6 selects the GPU route naturally, which is what thresholds 1–3
measure. Route-equivalence testing (threshold 4) additionally needs the
*same* scene rendered both ways, so it forces a route through an
internal hook compiled only under `#[cfg(test)]`. That hook is never
public API, is not reachable from any released build, and is therefore
not the implementation switch #2536 forbids — the shipped selection
stays internal and automatic, and section 6's hint-invariance still
holds.

*Alternatives considered:* new scenes only (no test-only machinery at
all, but then threshold 4 cannot be measured, since no scene can be
rendered both ways); the forcing hook only (no new scenes, but then the
route is never exercised at the sizes it exists for, and `AGENTS.md`
requires corpus scenes for every new capability).

## 5. Coverage storage, keys and lifetime

CPU-routed cells live in the existing R8 atlas, under the existing
lifecycle (`docs/design/119-pan-coverage.md`). GPU-routed cells live in a
per-frame storage buffer and are not cached at all (decision S1, 5.3).
Both carry the same emission form from section 3.

### 5.1 Keys

`path::placement(content_hash, transform, surface)` (`path.rs:596`)
builds the key from: the content hash; the four linear coefficients
`a, b, c, d` and the fractional translation `qx = e - floor(e)`,
`qy = f - floor(f)`, each narrowed to `f32` bits; and the surface
dimensions. The integer translation is kept out of the key as
`Placement::offset`, so identical content moved by whole pixels reuses
one entry; `key_exact()` adds the integer offset and is used when
`Coverage::clipped` is set, because surface-clipped coverage is only
valid at the offset it was rasterized under (`lower.rs:4718`,
`path.rs:109`). Content hashes are tagged by draw mode so a non-zero
fill, an even-odd fill and a stroke of the same outline never collide:
`fill_tag` gives `0`/`1` (`prepared.rs:123`) and `hash_stroke` prefixes
`2` plus every stroke parameter and the local tolerance
(`path.rs:479`, `path/hash_identity.rs:76`). Path-clip masks use
`hash_elements(elements, 2)` into a separate `masks` map
(`lower.rs:4843`).

Rule for #2536: **a GPU-routed cell's key must denote the same coverage
bytes as a CPU-routed cell's key, or be a different key.** Since
decision C1 declines bit-identity, the bytes are *not* identical for the
same geometry, so the keys must differ.

**Decision K1 — keys are route-tagged.** The route is mixed into the
content hash, so a GPU-routed entry and a CPU-routed entry of the same
geometry are disjoint keys and can never satisfy each other's lookups.
This is required by C1 rather than chosen for convenience: a shared key
would hand a lookup bytes produced by a different accumulation order
than the key promises.

In practice the tag costs nothing, because decision S1 gives GPU-routed
coverage no cache entry at all: the route tag keeps a GPU-routed
placement key from colliding with the atlas's CPU-routed entries, and the
route flip that would otherwise thrash the atlas (a path crossing the
selection predicate's boundary as it is zoomed) evicts nothing, since
only the CPU side of it was ever resident.

*Alternatives considered:* one namespace (free cross-route reuse, but
only sound under bit-identity, which C1 declines); one namespace plus a
per-entry equivalence marker (keeps reuse where the bytes happen to
match, but it has to prove byte equality to set the marker, which needs
a readback).

### 5.2 The CPU route's residency, eviction and frame lifetime

Unchanged from #119:

- Cells are allocated on shelf bands; eviction is bounded and at shelf
  granularity, strict allocation first then simulated eviction
  (`AtlasPlan::Fits` / `FitsEviction` / `Resize`, `glyph.rs:2040`).
  A shelf touched or written this commit cannot be evicted.
- Eviction does not bump the atlas generation; surviving emissions stay
  live. Emissions store `(slot, epoch)` references and are restamped in
  `apply_pending` only when every reference survived the commit
  (`mod.rs:6795`–`6800`).
- `PathEmit::slots` is a range into the atlas's `emit_slots` arena so a
  replay pin stays meaningful after an eviction (`glyph.rs:187`).
- Exhaustion surfaces as `RenderError::AtlasExhausted`
  (`mod.rs:6713`); there is no silent clear-and-rasterize fallback.
- Emissions from an abandoned or failed lowering are discarded, and a
  path that misses the surface stores nothing at all (`lower.rs:4706`).
- Pending writes are frame-scoped: `CellWrites` (`glyph.rs:1986`) holds
  one texel arena per lowering batch, consumed by
  `Atlas::upload_committed` and dropped with the commit, so nothing is
  retained across commits and no CPU buffer aliases the upload ring's
  in-flight staging.

### 5.3 The GPU route's storage: a per-frame buffer, not the atlas

**Decision S1 — GPU-routed partial cells do not enter the R8 atlas.**
This refines #2536, which assumed they would. Two facts make the atlas
unreachable for them:

- Without a readback the CPU cannot know how many cells a path produces
  before the dispatch, since the count is a result of the
  classification. Atlas allocation (`Atlas::alloc` via `place_path`,
  `glyph.rs:1321`) needs the cell list up front, and shelf planning
  (`AtlasPlan`, `glyph.rs:2040`) needs each cell's size.
- The atlas texture is `R8Unorm` with `TEXTURE_BINDING | COPY_DST`
  (`glyph.rs:535`–`536`), and `R8Unorm` is not a storage-texture format
  a compute shader can write in portable wgpu, so the kernel cannot
  write into it even if the origins were known.

So GPU-routed cells live in a per-frame storage buffer: R8 texel values,
four per `u32`, cells laid out back to back in the compacted order the
classifier emits, each cell's descriptor carrying its byte offset,
width, height and `interior` interval. The paint shader reads them
through a storage-buffer variant of the cell fetch it already does
against the atlas (`shared.wgsl:188`, `shader.wgsl:819`): same `[0, 1]`
value, same bilinear-free per-texel addressing, same
`KIND_SPAN`/`KIND_REGION` treatment of full coverage.

What does *not* change: the emission form (spans plus partial cells with
the semantics of section 3), the R8 quantization expression, interior
promotion, and the fact that painting, blending and composition read
coverage and do nothing else differently. Only where the texels sit
differs.

*Alternatives considered:* kernel writes to a buffer and the render
thread copies into atlas shelves with `copy_buffer_to_texture` (reuses
#169's one-copy-per-region shape, but the copy's destination rects are
exactly the cell origins the CPU cannot know without a readback); add
`STORAGE_BINDING` with an `R32Uint` view of the atlas and write cells
directly (removes the copy, but requires allocation before dispatch,
which inverts the current ordering for the same reason); a two-phase
frame that reads back only the classification (defeated by the
no-readback constraint).

**Decision R2 — buffers are sized from an exact CPU-side upper bound.**
Before the dispatch the CPU computes, in `O(n)` over the path's segments
and without touching coverage, an upper bound on the cells a path can
touch: each segment's device-space bounding box converted to strip and
cell units, summed, which bounds the touched cells of every strip it
crosses. Buffers — coverage scratch, cell texels, cell and span
descriptors — are allocated from that bound, so the kernels cannot
overflow and there is no overflow-detection path to read back.

A bound that exceeds a hard per-frame memory cap is an `Unsupported`
error, reported as such. It is never a silent fall-through to the CPU
route: `AGENTS.md` requires an explicit `Unsupported` for a case the
engine cannot serve, and a silent fallback here would hide an
unbounded-memory bug behind a frame that merely got slow.

*Alternatives considered:* grow-and-retry across frames (needs a
readback of the overflow flag, and makes a frame's output depend on
history); a fixed cap with clipped output (silently drops geometry,
which `AGENTS.md` forbids); conservative over-allocation by a constant
factor (either wastes memory or still overflows, and the memory rule
requires a bound that is justified, not guessed).

**Decision — GPU-routed coverage is not retained.** It is recomputed on
every frame the path is drawn, and the buffer is frame-scoped like
`CellWrites` (`glyph.rs:1986`) is today. Caching it would be a
memory-for-time trade that the memory rule requires be paid for in
measured time or energy, and that measurement does not exist yet; it is
out of scope for #2536 and belongs to its own issue if the per-frame
recompute proves to be the cost.

*Alternatives considered:* a GPU-side coverage cache keyed like the
atlas (saves the recompute for static paths, but adds a second cache
lifecycle, a second eviction policy and unjustified steady-state memory).

### 5.4 Compute-to-raster synchronization

The constraint the current structure imposes: **lowering has no device.**
`Renderer::lower_content` (`mod.rs:6259`) takes no `wgpu::Device` or
`Queue` and runs on scoped worker threads, one per surface
(`mod.rs:5829`–`5838`). Atlas placement and upload happen afterwards on
the render thread, in `apply_pending` (`mod.rs:6722`) →
`apply_raster` (`mod.rs:6817`) → `Atlas::place_path` (`glyph.rs:1321`),
then `Atlas::upload_committed` (`glyph.rs:662`), which issues one
`write_texture` per newly allocated contiguous shelf region. So a
compute dispatch cannot be issued during lowering; the GPU route emits a
*pending* record carrying geometry rather than texels, and is dispatched
on the render thread.

**Decision S2 — the dispatches are in the frame's own submission, ahead
of its raster passes.** One command encoder: the coverage kernel and the
classifier kernel are encoded first, then a barrier from compute to the
vertex and fragment stages that read the coverage buffer and the indirect
draw arguments, then the frame's raster passes. One submission, ordered
by the encoder, so no fence, no stall, and no cross-frame state.

There is no deferred or frame-latency variant. Dispatching for frame `n`
and consuming in frame `n + 1` would make a path's first frame render by
a different route than its second, so the output would depend on frame
history — which section 1 forbids, and which decision C1 makes
observable since the two routes are not bit-identical.

*Alternatives considered:* a separate earlier submission (only needed to
sequence a readback, which is forbidden); dispatch for frame `n`,
consume in `n + 1` (hides dispatch latency, but makes output
history-dependent as above).

**Decision S3 — a pending GPU-routed path records geometry, fill rule,
transform and placement key.** The pending path record today is
`PendingRaster::Path { key, emit, cells }` with CPU texels in the batch
arena (`lower.rs:4724`). A GPU-routed path has no texels at lowering
time, so its record instead holds: the range of its geometry in the
frame's compute input buffer, its fill rule, its transform, and its
placement key (route-tagged per K1, `key_exact` when the coverage rect
is surface-clipped). Because flattening is on the GPU (#2543), the
uploaded geometry is *curve elements*, not flattened segments — the
record describes what the kernel will flatten, so the CPU never
materializes the segment list.

*Alternatives considered:* reuse `PendingRaster::Path` with an empty
arena range plus a side table of dispatch descriptors (no new variant,
but it makes an empty arena range mean two different things); put the
texel source in an enum inside `CellWrite` (pushes the route distinction
into the atlas upload path, which decision S1 says GPU-routed cells never
enter).

## 6. Route selection

Selection is internal to the backend. The public API gains no route
selector, and no caller — Hydrolysis, WaterUI components or a host —
can ask for CPU or GPU coverage.

- **Capability.** `SharedDevice::has_compute` (`interop.rs:210`) reports
  compute support: `DownlevelFlags::COMPUTE_SHADERS` plus
  `max_compute_workgroups_per_dimension > 0` (`interop.rs:220`).
  Compute is not part of the hardware floor (`AGENTS.md`, Platform).
- **Devices without compute** route every path through the CPU sweep
  under this same specification. There is no reduced-quality fallback:
  the CPU route is the full specification, and the GPU route is an
  optimization of it.
- **Hints.** Any hint that exists may only be semantic (large, static,
  dynamic) and the engine may ignore it. **Rendered output must be
  identical with and without any hint**, and the two routes' outputs must
  agree to within section 4.3 threshold 4. No `PathHint` or volatility
  type exists in the tree today; #2536 would introduce it.

**Decision R1 — the predicate depends only on content and scale, and is
deterministic without hysteresis.** Its inputs are the path's own
content (its curve element count, and the crossing-density estimate that
falls out of the same `O(n)` segment-bound pass as decision R2's buffer
bound) and the scale of its transform, from which the device-space
coverage rect follows. Nothing else: not frame history, not the previous
frame's choice, not a measured timing, not atlas pressure, not whether an
entry is currently resident. The same path at the same scale therefore
takes the same route in every frame and in every process, which is what
makes the route-equivalence gate reproducible.

There is no hysteresis. A hysteretic predicate is history by
definition — the same path at the same scale would route differently
depending on how it got there, and under decision C1 that difference is
observable in the image. Route flipping across a zoom is acceptable
precisely because decision S1 gives the GPU route no cache to thrash: the
flip costs a recompute that the GPU route pays every frame anyway.

*Alternatives considered:* a fixed threshold with per-key hysteresis
(stabilizes the route across a zoom, but makes output depend on history);
per-frame selection from a measured cost model (adapts to the device, but
a measurement is history and makes output non-reproducible); decide once
per content hash and keep it while the entry is resident (stable, but
ties the route to cache residency, which is also history).

**Runtime failure.** The GPU route has no atlas allocation to fail
(decision S1) and no buffer overflow to detect (decision R2): the only
failure is a buffer bound above the hard memory cap, which is an
`Unsupported` error raised before the dispatch. There is no in-frame
fallback to the CPU sweep — under decision C1 that would make the image
depend on memory pressure.

## 7. Summary of decisions

| id | decision |
| --- | --- |
| — | No CPU readback anywhere in the route; this decides most of the rest |
| C1 | `f32` accumulation on the GPU; bit-identity with the CPU route is not the contract |
| G1 | Winding resolves in-kernel: per-cell area and cover, strip backdrop prefix, fill-rule fold, no intersections solved |
| E1 | A second kernel classifies strips and writes compact span/cell lists plus indirect draw arguments |
| T1 | Tolerance-based acceptance, thresholds 1–5 of section 4.3 |
| T2 | #2546 adds large corpus scenes; route equivalence uses a `#[cfg(test)]`-only forcing hook |
| K1 | Route-tagged keys; a GPU entry never shares a key with a CPU entry |
| S1 | GPU-routed cells live in a per-frame storage buffer, not the R8 atlas (refines #2536) |
| R2 | Buffers sized from an exact `O(n)` CPU-side cell bound; above the cap it is `Unsupported`, never a silent fallback |
| — | GPU-routed coverage is recomputed every frame; caching it is out of scope |
| S2 | Dispatches in the frame's own submission, ahead of its raster passes, with a compute→vertex/fragment barrier |
| S3 | Pending record holds curve elements (flattening is on the GPU, #2543), fill rule, transform and placement key |
| R1 | Predicate depends only on content and transform scale; deterministic, no hysteresis |

## Appendix A — per-scene measurements

`origin/dev` `6091fc8f5`, Apple M2 Max / Metal, offscreen,
`Rgba16Float` premultiplied extended linear Display P3, unclipped;
HDR-FLIP on the scenes marked HDR.

| scene | flip_mean | flip_max | max_local_error | HDR |
| --- | --- | --- | --- | --- |
| glyph-stroke-dashed | 0.005326 | 0.044147 | 0.006207 | no |
| clip-path-transform | 0.004251 | 0.052703 | 0.006593 | no |
| stroke-join-rounded-bevel | 0.004163 | 0.037400 | 0.007243 | no |
| stroke-join-rounded-miter-lo | 0.004163 | 0.037400 | 0.007243 | no |
| stroke-join-continuous-sharp-round | 0.003968 | 0.135176 | 0.024158 | no |
| path-overlap-winding-p3 | 0.003685 | 0.035101 | 0.004984 | no |
| glyph-stroke-transformed | 0.003534 | 0.046043 | 0.006479 | no |
| stroke-join-ellipse-bevel | 0.003249 | 0.041030 | 0.007804 | no |
| stroke-join-ellipse-miter-hi | 0.003249 | 0.041030 | 0.007804 | no |
| stroke-join-ellipse-miter-lo | 0.003249 | 0.041030 | 0.007804 | no |
| stroke-join-ellipse-miter-rt2 | 0.003249 | 0.041030 | 0.007804 | no |
| stroke-join-ellipse-round | 0.003249 | 0.041030 | 0.007804 | no |
| stroke-join-continuous-bevel | 0.003198 | 0.060528 | 0.010506 | no |
| stroke-join-continuous-miter-lo | 0.003198 | 0.060528 | 0.010506 | no |
| stroke-join-continuous-round | 0.003198 | 0.060528 | 0.010506 | no |
| stroke-join-continuous-miter-hi | 0.003150 | 0.060533 | 0.010507 | no |
| stroke-join-continuous-miter-rt2 | 0.003150 | 0.060533 | 0.010507 | no |
| shadow-silhouette-affine-path | 0.003136 | 0.038440 | 0.005851 | no |
| stroke-join-continuous-sharp-miter-hi | 0.003120 | 0.031712 | 0.006205 | no |
| stroke-join-continuous-sharp-miter-rt2 | 0.003120 | 0.031712 | 0.006205 | no |
| path-curved | 0.003115 | 0.033792 | 0.005422 | no |
| path-overlap-winding | 0.003110 | 0.029463 | 0.004581 | no |
| stroke-join-continuous-s00-bevel | 0.003032 | 0.031712 | 0.006205 | no |
| stroke-join-continuous-s00-miter-lo | 0.003032 | 0.031712 | 0.006205 | no |
| stroke-join-continuous-sharp-bevel | 0.003032 | 0.031712 | 0.006205 | no |
| stroke-join-continuous-sharp-miter-lo | 0.003032 | 0.031712 | 0.006205 | no |
| stroke-join-rect-bevel | 0.003032 | 0.031712 | 0.006205 | no |
| stroke-join-rect-miter-lo | 0.003032 | 0.031712 | 0.006205 | no |
| clip-path-nested | 0.002967 | 0.030345 | 0.005134 | no |
| fill-sharp-subpixel-p3 | 0.002935 | 0.006381 | 0.000613 | no |
| path-overlap-winding-hdr | 0.002898 | 0.088432 | 0.039872 | yes |
| clip-path-full-surface | 0.002895 | 0.028288 | 0.004744 | no |
| stroke-join-rounded-round | 0.002847 | 0.037400 | 0.007243 | no |
| stroke-join-rounded-miter-hi | 0.002847 | 0.037400 | 0.007243 | no |
| stroke-join-rounded-miter-rt2 | 0.002847 | 0.037400 | 0.007243 | no |
| path-curved-transform | 0.002806 | 0.032120 | 0.005187 | no |
| path-self-intersect | 0.002785 | 0.031956 | 0.004903 | no |
| glyph-transform-stroke | 0.002554 | 0.038402 | 0.004709 | no |
| stroke-join-continuous-s00-round | 0.002540 | 0.031712 | 0.006205 | no |
| stroke-join-rect-round | 0.002540 | 0.031712 | 0.006205 | no |
| stroke-dash-p3 | 0.002458 | 0.033036 | 0.004647 | no |
| stroke-join-continuous-s00-miter-hi | 0.002445 | 0.031712 | 0.006205 | no |
| stroke-join-continuous-s00-miter-rt2 | 0.002445 | 0.031712 | 0.006205 | no |
| stroke-join-rect-miter-hi | 0.002445 | 0.031712 | 0.006205 | no |
| stroke-join-rect-miter-rt2 | 0.002445 | 0.031712 | 0.006205 | no |
| shadow-silhouette-path | 0.002376 | 0.009972 | 0.000760 | no |
| fill-sharp-subpixel | 0.002367 | 0.005204 | 0.000620 | no |
| glyph-stroke-thin | 0.002271 | 0.015821 | 0.006281 | no |
| stroke-dash | 0.002111 | 0.026900 | 0.004569 | no |
| clip-path-oversized | 0.002048 | 0.006819 | 0.000758 | no |
| stroke-dash-hdr | 0.002007 | 0.039756 | 0.077068 | yes |
| path-nonzero | 0.001889 | 0.005687 | 0.000582 | no |
| path-evenodd | 0.001801 | 0.005905 | 0.000703 | no |
| stroke-caps-mixed | 0.001718 | 0.025595 | 0.003754 | no |
| fill-p3 | 0.001715 | 0.004121 | 0.000293 | no |
| paint-transform-radial-stroke | 0.001697 | 0.004267 | 0.000394 | no |
| stroke-curve | 0.001650 | 0.004116 | 0.000183 | no |
| stroke-joins | 0.001643 | 0.023563 | 0.003006 | no |
| stroke-caps | 0.001530 | 0.004962 | 0.000508 | no |
| stroke-continuous-s50 | 0.001446 | 0.002629 | 0.000163 | no |
| stroke-p3 | 0.001344 | 0.002027 | 0.000131 | no |
| stroke-widths | 0.001336 | 0.002014 | 0.000131 | no |
| fill-hdr | 0.001080 | 0.002873 | 0.006063 | yes |
| stroke-hdr | 0.000707 | 0.000795 | 0.004958 | yes |
| fill-sharp-subpixel-hdr | 0.000705 | 0.016116 | 0.009810 | yes |
