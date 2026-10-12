# #2536 coverage specification: one rendering contract for both coverage routes

water-rs/waterui#2536 adds a GPU compute coverage route for large and
self-intersecting paths behind an unchanged API. This document (#2542) is
the single rendering specification both routes answer to, so the GPU route
is an implementation of the existing visual design rather than a second
one. It describes what is in the tree today, line by line, and states
what the GPU route must reproduce. Everything that is not yet decided is
an explicit open question with its alternatives; nothing is decided here
by omission.

Scope: only coverage computation moves to compute. Painting, blending,
clipping and composition stay in the raster pipeline, which reads the same
emission form from either route.

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

**Open question C1 — accumulation precision on the GPU route.** The CPU
route accumulates in `f32`. A compute route can accumulate in `f32`, in
`f16` (cheaper, and the hardware floor guarantees f16 arithmetic), or in
fixed-point integers with atomics.
Options: (a) require `f32` accumulation and a `f32` prefix sum, matching
the CPU route's error character exactly; (b) permit `f16`/fixed-point and
widen the tolerance in section 4 to cover it; (c) require the quantized
R8 byte to be *bit-identical* to the CPU route's for the same geometry,
which forces `f32` and a matching summation order. Not decided here.

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

The contract: the GPU route receives the already-expanded, already-flattened
device-space segment list — the output of step 3 in section 1.1 — and is
responsible only for steps 5 through 8.

**Open question G1 — where `resolve_winding` runs.** It is a CPU left-to-right
sweep with a sort, and its cost on large self-intersecting paths is part of
what #2536 wants to remove. Options: (a) keep it on the CPU and hand the
GPU route resolved boundary edges, so the GPU route is a pure
accumulate-and-fold kernel and the fill rule is always `NonZero` for
overlapping input (smallest change, but keeps the sweep's cost);
(b) implement the overlap-correct fold directly in the kernel (e.g. per-pixel
winding-interval accumulation) and drop `resolve_winding` for GPU-routed
paths, which must then reproduce the union's true coverage without the
sweep; (c) run the sweep on the GPU as a separate pass. (b) is the only
option that removes the sweep from the CPU, and the only one that requires
proving overlap correctness independently of the CPU implementation. Not
decided here.

## 3. The shared emission form

Both routes emit the same thing: full-coverage runs as solid instances,
partial coverage as R8 atlas cells. This is `path::emit` (`path.rs:228`)
and it is the interface the GPU route must satisfy.

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
  its device rect, its atlas origin (filled on the render thread), its
  shelf slot, and `interior` — the `start | end << 16` column interval,
  relative to `x0`, that is known fully covered inside the cell
  (`path.rs:324`). A cell emission greedily consumes
  `partial… full… partial…` in one pass (`path.rs:312`–`338`), so one
  cell may carry one retained full interval.
- **Interior promotion.** On replay, a cell with `interior != 0` under a
  solid paint becomes `KIND_REGION` (`lower.rs:4810`), whose opaque
  vertex stage narrows the quad to the interior interval
  (`shader.wgsl:715`) and whose partial fragment stage samples the atlas
  exactly like `KIND_GLYPH` (`shared.wgsl:188`, `shader.wgsl:819`).
- **Texel order.** A cell's texels are row-major, `w * h` bytes,
  appended to the lowering batch's arena as a byte range
  (`CellTexels = (u32, u32, Range<usize>)`, `glyph.rs:1954`); cell order
  in `PathEmit::cells` matches the raster order index for index
  (`path.rs:217`).

The GPU route must produce byte-identical texels and the same
span/cell decomposition for the same geometry, or an explicitly
specified different one — see open question E1.

**Open question E1 — who performs the emission split on the GPU route.**
The split needs the coverage grid's exact `0.0`/`1.0` classification per
column. Options: (a) the kernel writes the full `f32` (or R8) coverage
grid to a buffer and the CPU runs the existing `emit`, which keeps the
emission bit-identical but pays a readback and loses most of the win;
(b) a second compute pass classifies columns and compacts the strips on
the GPU, writing cell texels and a span/cell descriptor list the render
thread consumes without reading coverage back — no readback, but the
`FULL_RUN_MIN`/`PARTIAL_GAP_MAX`/`SMALL_BBOX` heuristics must be
reimplemented in WGSL and kept in lockstep with `path.rs`;
(c) the GPU route emits *only* cells (no span extraction), accepting more
sampled instances and more atlas bytes in exchange for a trivial kernel.
Not decided here. Note (c) changes performance, not pixels, only if
`KIND_REGION`'s interior promotion is preserved.

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

**Open question T1 — bit-identity versus tolerance.** If open question C1
resolves to (c) (`f32`, matching summation order) the GPU route can be
required to produce *bit-identical* R8 texels, and then the whole corpus
stays bit-identical and section 4.3's thresholds are never exercised.
Options: (a) require bit-identity and treat any difference as a bug,
which is the strongest contract and the simplest gate but constrains the
kernel's summation order and precision; (b) require only the thresholds
above, allowing a faster kernel at the cost of per-scene churn in the
committed metrics; (c) require bit-identity for scenes the CPU route
would have taken anyway and thresholds only for paths the CPU route
could not have handled in budget. Not decided here. Note (b) means the
#2536 PR legitimately changes committed metrics and PNG bytes, which
`AGENTS.md` permits only for a change that intends to change pixels —
that intent would have to be stated in the PR.

**Open question T2 — which scenes the gate runs on.** The GPU route only
triggers for large/self-intersecting paths (section 6), and the current
corpus's path scenes are small (most are 128×128). Options: (a) add
corpus scenes large enough to trigger the GPU route, so the gate above
measures the route it is meant to measure; (b) add a test-only override
that forces the GPU route for every path scene, accepting that the
override is test-only machinery and not a public hint; (c) both. Without
(a) or (b) the thresholds in 4.3 are unenforceable, because no existing
corpus scene would select the GPU route. Not decided here. `AGENTS.md`
("every new capability gets corpus scenes") points at (a), and (b) would
have to avoid becoming a hint that changes output, which section 6
forbids.

## 5. Atlas integration

GPU-routed cells enter the *existing* atlas lifecycle (`docs/design/119-pan-coverage.md`);
they do not get a parallel cache.

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
bytes as a CPU-routed cell's key, or be a different key.** Since both
routes implement one specification, a shared key is correct *only* if
the bytes are identical for the same geometry.

**Open question K1 — key namespace.** Options: (a) one namespace, which
requires bit-identical output (open question C1 option (c) / T1 option
(a)) and gives free cross-route reuse — a path rasterized on the CPU
this frame and GPU-routed the next hits the same entry; (b) a route tag
mixed into the content hash, which makes the two routes' entries
disjoint, guarantees no incorrect sharing under any precision choice,
and costs a re-rasterization whenever a path's route flips; (c) one
namespace plus a per-entry route marker, so a hit from the other route
is accepted only when a recorded equivalence flag is set. Not decided
here. (b) interacts badly with open question R1: a route that can flip
with zoom or segment count would thrash the atlas.

### 5.2 Residency, eviction and frame lifetime

Unchanged from #119, and the GPU route inherits all of it:

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

### 5.3 Compute-to-raster synchronization

The constraint the current structure imposes: **lowering has no device.**
`Renderer::lower_content` (`mod.rs:6259`) takes no `wgpu::Device` or
`Queue` and runs on scoped worker threads, one per surface
(`mod.rs:5829`–`5838`). Atlas placement and upload happen afterwards on
the render thread, in `apply_pending` (`mod.rs:6722`) →
`apply_raster` (`mod.rs:6817`) → `Atlas::place_path` (`glyph.rs:1321`),
then `Atlas::upload_committed` (`glyph.rs:662`), which issues one
`write_texture` per newly allocated contiguous shelf region. So a
compute dispatch cannot be issued during lowering; the GPU route must
emit a *pending* record that carries the geometry (not the texels) and be
dispatched on the render thread before the raster pass that samples the
atlas.

Two hard facts constrain the write path. The atlas texture is created
with `TEXTURE_BINDING | COPY_DST` only (`glyph.rs:536`) and its format
is `R8Unorm` (`glyph.rs:535`), which is not a storage-texture format a
compute shader can write directly in portable wgpu. And cell origins are
not known until `Atlas::alloc` runs on the render thread, after planning
and possible eviction.

**Open question S1 — how compute output reaches the atlas.** Options:
(a) the kernel writes packed R8 bytes to a storage buffer, the render
thread allocates cells, then `copy_buffer_to_texture` per shelf region
— reuses #169's one-copy-per-region shape and needs no texture usage
change, but needs the kernel to write into a layout the copy can address
after allocation; (b) add `STORAGE_BINDING` with an `R32Uint` view (or a
separate `R32Uint` scratch texture) and have the kernel write cells
directly, requiring allocation to be known before the dispatch — which
inverts the current ordering; (c) a two-phase frame: dispatch coverage
for *bounding boxes* in one submission, read back only the strip
classification, then allocate and copy. Not decided here.

**Open question S2 — where the dispatch sits in the submission.**
Options: (a) one compute pass at the head of the frame's command encoder,
before the raster pass, with the atlas copy in between — one submission,
and the raster pass's sampling is ordered by the encoder;
(b) a separate earlier submission so the copy can be issued after a
readback of classification data (needed by S1 option (c)), costing a
submission and possibly a stall; (c) dispatch for frame `n` and consume
in frame `n + 1`, which hides latency but makes a path's first frame fall
back to the CPU route and so makes the output depend on frame history —
which section 1 forbids unless the fallback is bit-identical. Not
decided here.

**Open question S3 — pending-record shape.** The pending path record
today is `PendingRaster::Path { key, emit, cells }` with CPU texels in
the batch arena (`lower.rs:4724`). A GPU-routed path has no texels at
lowering time. Options: (a) a new `PendingRaster::GpuPath` variant
carrying the device-space segment range and the coverage rect, resolved
at apply; (b) reuse `PendingRaster::Path` with an empty arena range and a
side table of dispatch descriptors; (c) make the texel source an enum
inside `CellWrite`. Not decided here.

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
- **Candidate inputs** for the decision: flattened segment count, the
  device-space coverage rect's area (and whether it was surface-clipped),
  and a crossing-density estimate — the signal `resolve_winding` already
  computes incidentally (it reports whether the input overlapped at all,
  `lowering.rs:2045`).
- **Hints.** Any hint that exists may only be semantic (large, static,
  dynamic) and the engine may ignore it. **Rendered output must be
  identical with and without any hint, and identical whichever route
  ran** (subject to open question T1's resolution). No `PathHint` or
  volatility type exists in the tree today; #2536 would introduce it.

**Open question R1 — the selection predicate and its stability.** A
threshold on segment count or area makes the route flip as a path is
zoomed or edited, which under open question K1 option (b) thrashes the
atlas, and under option (a) requires bit-identity to be invisible.
Options: (a) a fixed threshold pair (segments, area) with hysteresis per
cache key; (b) select per frame from a measured cost model and accept
flipping; (c) select once per content hash and keep the decision for as
long as the entry is resident. Not decided here.

**Open question R2 — what happens when the GPU route fails at runtime.**
A dispatch can fail to find atlas room exactly like a CPU raster can.
Options: (a) the same `AtlasExhausted` path as today, with no CPU retry;
(b) fall back to the CPU sweep within the frame, which is correct only if
the two routes are bit-identical, otherwise the output depends on atlas
pressure; (c) fail the frame. Not decided here; `AGENTS.md`'s fail-fast
rule argues against a silent (b).

## 7. Summary of open questions

| id | question |
| --- | --- |
| C1 | GPU accumulation precision: `f32`, `f16`/fixed-point, or bit-identical-to-CPU |
| G1 | Where overlap/winding resolution runs (CPU sweep, in-kernel, GPU sweep) |
| E1 | Who performs the span/cell emission split (CPU after readback, second kernel, cells-only) |
| T1 | Bit-identity versus tolerance as the acceptance contract |
| T2 | Which corpus scenes exercise the GPU route at all |
| K1 | Atlas key namespace: shared, route-tagged, or shared with an equivalence marker |
| S1 | How compute output reaches an `R8Unorm` atlas (buffer copy, storage view, two-phase) |
| S2 | Where the dispatch sits in the frame's submission |
| S3 | Pending-record shape for a GPU-routed path |
| R1 | The selection predicate and its stability across frames |
| R2 | Behaviour when the GPU route cannot complete |

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
