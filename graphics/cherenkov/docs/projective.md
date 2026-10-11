# Projective layers (#84)

A layer can carry a perspective pose: a card flip, a view rotated in depth,
a perspective carousel. Recorded content stays affine. Projection happens
once, when the layer's completed image composes into its parent.

## API

```rust
pub struct Projective { /* rows: [[f64; 4]; 4], column vectors */ }
pub enum ProjectiveError { NonFinite, NonInvertible, InvalidPerspectiveDistance }

impl Projective {
    pub const IDENTITY: Self;
    pub fn from_rows(rows: [[f64; 4]; 4]) -> Result<Self, ProjectiveError>;
    pub fn perspective(distance: f64) -> Result<Self, ProjectiveError>;
    pub const fn as_rows(&self) -> &[[f64; 4]; 4];
    pub fn checked_mul(self, rhs: Self) -> Result<Self, ProjectiveError>; // self · rhs
    pub const fn plane_homography(&self) -> [[f64; 3]; 3];
}
impl TryFrom<Affine> for Projective { type Error = ProjectiveError; }

pub trait ProjectiveLayers: Backend {}          // Gpu, Raster

impl<B: ProjectiveLayers> LayerEdit<B> {
    pub fn projection(&mut self, value: impl Into<Live<Projective>>) -> &mut Self;
    pub fn tilt(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self;   // radians, about X then Y
    pub fn depth(&mut self, value: impl Into<Live<f64>>) -> &mut Self;   // along Z, layer units
    pub fn clear_projection(&mut self) -> &mut Self;
}
```

```rust
let camera = Projective::perspective(800.0)?;
surface.update(|tx| {
    tx[&card].projection(camera).pivot(Vec2::new(160.0, 100.0));
});
surface.update_animated(Spring::smooth(), |tx| {
    tx[&card].tilt(Vec2::new(0.0, std::f64::consts::PI));
});
```

- **Conventions.** `rows[row][column]` is row-major storage on column
  vectors: `(x, y, z, 1)` maps to `rows · (x, y, z, 1)ᵀ`. Positive Z points
  toward the viewer, and x and y follow the layer's downward-y space. A
  point is visible where the homogeneous `W > 0`. `M` and `−M` select
  opposite half-spaces, so they are different transforms. Positive scaling
  of the whole matrix changes nothing.
- **`perspective(d)`** puts the camera `d` units in front of the `z = 0`
  plane, so `W = 1 − z/d`. `d` must be finite and positive.
- **Construction checks** finite coefficients and invertibility. The
  sampled pose is checked again after composition. An invalid pose fails
  the render with `RenderError::ProjectivePose { layer, error }`.
- **`tilt` and `depth`** without `projection` make the layer projective
  with an identity base: an orthographic depth rotation. `clear_projection`
  removes projection, tilt and depth, together with their subscriptions and
  tracks. The layer becomes affine again, and its affine components are
  unchanged.

## Pose

With column vectors, and #77's components, the layer's local-to-parent map
is:

```text
M = embed(transform) · T(translation + pivot) · projection · T(0, 0, depth)
  · Rz(rotation) · Ry(tilt.y) · Rx(tilt.x) · embed(skew · scale) · T(−pivot)
```

`embed` places an affine map in the `z = 0` plane. With the identity
projection and zero tilt and depth, `M` is #77's affine matrix. Positive
`tilt.x` turns the top edge away from the viewer, and positive `tilt.y`
turns the right edge away. Angles are unwrapped: `0 → 2π` is a full flip.
Both faces render, and nothing is culled.

- **Animation.** Tilt and depth are component tracks, like rotation, with
  curves, springs, retargeting that keeps position and velocity, and
  unwrapped angles. The raw projection matrix is not animatable. A new
  value replaces the base, and `.animation(...)` on it is the
  non-animatable-property panic. Nothing decomposes a matrix into
  components.
- **Storage.** Projective state lives in a sparse side table of the
  surface tree, keyed by layer. An affine-only tree allocates nothing, and
  it keeps the existing affine traversal. The backend reads either the
  sampled affine transform or the sampled complete pose, never both
  (`SurfaceTree::projective_pose`).

## Composition

A projective layer is a **flattening boundary**, even when its matrix is the
identity:

- Its content, clip, scroll offset, filter and children render into a
  layer-local image. The existing affine rasterizers render it under a
  local-to-texel scale.
- The clip moves with the layer. Scroll offset moves content and children
  beneath it. The filter runs before projection, on the local raster, so
  its parameters are in local raster pixels.
- The completed image is projected into the parent raster, under the
  ancestor clips. The layer's opacity and blend apply there, once.
- A nested projective layer flattens into its parent's local image, and the
  parent's image is then projected. Depths never carry across a boundary:
  there is no preserve-3d, no depth test, and painting order is tree order.
- A non-`Normal` blend composites against the parent destination. A
  destructive Porter-Duff blend (`clear`, `src`, `src-in`, `src-out`,
  `dest-in`, `dest-atop`) keeps its operator domain. That domain is the
  projected layer clip, geometrically intersected with the ancestor clips,
  never the image's alpha. A transparent part of the image still replaces
  the destination there.
- The opacity passthrough optimisation never crosses a projective boundary.

**Clip required.** The clip bounds are the layer's finite local source
domain. A projective layer without a clip is
`RenderError::Unsupported("projective-unclipped")`.

**Backdrop.** A backdrop group inside a projective layer captures that
layer's local target. A group's members must share one composition space:
the surface, or one projective layer's image. A group spanning several
spaces is `Unsupported("projective-backdrop-cross-space")`. A projective
layer that is itself a backdrop member is
`Unsupported("projective-backdrop-member")`: its sample would have to be
projected from the parent's capture space, and this issue does not
implement that.

## The local image

**Plane homography.** `H` is `M`'s rows and columns 0, 1 and 3, applied to
`(x, y, 1)`. The parent raster's affine placement is composed on the left.

**Visible domain.** The clip-bounds rectangle is clipped in homogeneous
coordinates, before any division, against `W > 0` and the parent viewport
widened by the two-pixel sampling footprint. The four viewport planes are
`X − x0·W ≥ 0`, and so on. There is no epsilon near plane and no
`max(W, ε)`. A horizon crossing is bounded by the finite viewport, never
by the infinite plane. Four cases contribute nothing:

- an empty visible polygon;
- an edge-on pose, where `det H = 0`;
- a layer entirely behind the viewer;
- a layer entirely off the raster.

**Density.** `ρ = 2^k` texels per layer unit, for the least `k ≥ 0` with
`2^k ≥ S`. `S` is a conservative upper bound of `σmax` of the
local-to-destination Jacobian over the visible domain:

- interval arithmetic on the Jacobian's entries (each numerator is affine,
  and `W` is affine and positive), then the midpoint's singular value plus
  the Frobenius norm of the radius;
- the worst leaf's longer side is halved until the bound and the sampled
  centroid value choose the same `k`, or until 64 leaves have been
  evaluated. At that limit the upper bound decides;
- a constant denominator takes the exact singular value.

The density never depends on whether an image was cached or prebuilt.

**Grid.** The texel grid starts at `floor(bounds.x0 · ρ)` and ends at
`ceil(bounds.x1 · ρ)` (y likewise). It covers the whole clip bounds, never a
per-frame crop of the visible polygon, so the image survives animation.

**Limits.** A backend has a texture dimension limit and a byte budget: the
engine budget minus its other resident bytes. An image beyond either is
`RenderError::ProjectiveUnsupported { layer, reason }`, and the reason
names the required size. The density is never capped to make an image fit.

**Mips.** Level sizes follow the hardware chain,
`max(1, floor(previous / 2))`, down to `1 × 1`. Every level spans the same
source extent. Texels are area-overlap averages of the previous level in
premultiplied extended linear Display P3. They are never unpremultiplied
or clamped, and an odd size never drops or duplicates an edge. An image's
bytes are exactly `8 · Σ w·h`: RGBA16F storage.

## Reconstruction

At a destination pixel centre `d`, the inverse homography gives the source
point, and the sample is transparent unless the preimage has `W > 0`. The
inverse map's Jacobian, in base texels per destination pixel, has singular
values `a ≥ b` and major-axis direction `e`:

```text
b_eff = max(1, b, a / 16)
lod   = log2(b_eff)            (linear between the two nearest levels)
N     = clamp(ceil(a / b_eff − 1/256), 1, 16)
tap t = centre + (t + ½ − N/2) · (a / N) · e      (equal weights)
```

The `1/256` slack is the reconstruction's precision. Without it, rounding
in the homography makes an isotropic footprint's ratio `1 + ε`, and `ceil`
turns that noise into a second tap along an arbitrary direction. Each tap
is a bilinear sample at two levels. Texels outside a level are
transparent at every level. The local image already holds primitive
coverage, so projection never applies coverage a second time.

- **GPU.** One composite quad covers the layer's conservative destination
  bounds. It uses `textureSampleLevel` with hardware linear filtering, at
  two explicit integer levels, with hardware anisotropy left at one. The
  out-of-domain weight comes from the per-axis in-domain fraction. The
  inverse homography rides in three rows of the gradient-stop buffer, so
  the ordinary instance layout is unchanged.
- **CPU.** Each scanline starts the inverse homography's two numerators
  and its denominator at the first pixel. It steps them across x and
  divides per sample. It never interpolates divided coordinates linearly.
  The band workers share the immutable image and its mips.

Hardware filtering is not bit-identical to CPU filtering. Within one
backend, a cached realization and a fresh one of the same state are
identical.

## Retention

A completed local image and its mips are keyed without the outer pose:

- the layer's content stamp (content, children, clip, filter, scroll,
  backdrop dependence);
- the density bucket;
- the local-to-texel grid and size;
- the renderer's image-replacement count. A replacement is the one change
  to what an image read that arrives without a tree edit, on either
  upload path, in place or reallocated.

Changing the matrix, tilt, depth, opacity or blend moves only the sample.
A warm matrix-only frame realizes nothing (`FrameStats::projective_realized
== 0`) and composes each visible projective layer once
(`projective_composed`). Images are held per density bucket, so a flip that
visits a few buckets keeps each one. A byte-bounded LRU evicts images the
current frame did not compose. Under memory pressure, the GPU backend
drops the images no surface composed last frame, and under `Critical` it
drops all of them. The CPU backend drops them all under `Critical`, like
its other caches. All image bytes count in `Engine::memory()`.

Retention respects deferred release (#199). A local image holds its own
pixels, never a handle, so it keeps no resource alive, and it does not
count as installed content when a release asks which surfaces draw a
resource. Adding or removing a resource does not affect a current image:
a removal only runs once no installed content draws the resource, so no
current image read it. Every image that did read it is one no frame can
compose again, because its layer's content stamp moved on. Before a
surface's frame plans its projective layers, it drops every such image,
and every image whose layer is gone or affine, so none outlives the
frame that carries out the release. Releasing a shader also drops, at
once, the GPU images that sampled it, so no retained image names a
released resource.

Glyph, path and clip coverage caches key on the raster transform and the
raster size they were produced for. A local image's raster is therefore a
distinct identity from the surface, and completed images never depend on
atlas residency.
