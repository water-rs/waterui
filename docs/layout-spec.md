# WaterUI layout specification

This document is normative. It describes the layout protocol that every
container in `waterui-layout`, every leaf a backend hosts, and every backend
bridge implements. It is frozen at 0.5.0: the semantics below do not change
without a major-version decision recorded by the maintainer, and a pull request
that changes them is rejected regardless of what it fixes. A difference between
this document and the code is a bug in the code.

The reference model is SwiftUI's layout protocol as observed on iOS 26 and
macOS 26. Where SwiftUI's observable behaviour is known it is the tie-breaker;
where WaterUI deliberately differs the difference is listed in §8.

## 1. Units and coordinates

- Every layout value is a logical pixel (point, dp). Backends convert to device
  pixels with the display scale; layout never sees a device pixel.
- `Rect` origins are physical left-to-right, top-to-bottom coordinates.
  `LayoutDirection` (leading/trailing) is resolved by containers when they
  place children; leaf and renderer code receives physical coordinates.
- Extents are finite and non-negative except a maximum, where
  `f32::INFINITY` means "unbounded". A container's answer may be `INFINITY`
  on an axis only when a child answered `INFINITY` on it.

## 2. The proposal protocol

A parent negotiates with a child through `ProposalSize { width, height }`, each
axis independently:

| proposal | question |
| --- | --- |
| `None` | ideal (intrinsic) extent |
| `Some(0.0)` | minimum extent |
| `Some(INFINITY)` | maximum extent |
| `Some(v)` | fit into `v` |

The child answers with `ViewDimensions`: a `Size` plus optional explicit
alignment guides. The answer is the child's choice; the proposal is advice. On
each axis the three probe answers satisfy `min <= ideal <= max`.

### 2.1 The SubView contract

Every child a container measures is a `SubView`:

- `measure(proposal)` answers as above. It is pure for a given proposal within
  one layout pass and may be called many times with different proposals.
  Caching is the `SubView`'s job (text shaping must cache); the `Layout` trait
  has no cache and never gains one.
- `stretch_axis()` declares which axes the view fills when offered more than it
  measures: `None`, `Horizontal`, `Vertical`, `Both`, or the stack-relative
  `MainAxis` / `CrossAxis`. It is read before `body` runs and without an
  environment, so a view with a filling leaf inside declares it here.
- `priority()` orders the compression bands (§4.3); ties share.

Measurement is single-threaded and runs on the thread that drives layout; a
`SubView` is neither `Send` nor `Sync`.

### 2.2 The Layout contract

A container is a `Layout`:

- `size_that_fits(proposal, children) -> Size` is the container's answer to a
  proposal. It may probe children freely.
- `place(bounds, proposal, children) -> Vec<SubviewPlacement>` places every
  child, in order, relative to `bounds`. `bounds` is the frame the parent
  resolved for this container; `proposal` is the proposal this container was
  measured with, available to a container whose rule needs it (§5, Grid).
  Each placement carries the child's frame and the proposal the child is
  placed with; the child's own layout pass receives that proposal verbatim.
- Explicit alignment guides (`explicit_horizontal` / `explicit_vertical`)
  come from the placed children; a container exposes the alignments it can
  answer. Resolving an explicit guide runs the container's `place`, so a
  container's guide is always consistent with its placement.

### 2.3 Placement is a negotiation against the resolved bounds

Placement is not a replay of measurement. When a container is placed it
negotiates with its children again, against the bounds it was actually given,
on **both** axes. The bounds equal the container's own answer whenever the
container is content-sized, so the two passes agree there; they differ exactly
when a parent stretched the container, overflowed it, or measured it with an
unspecified, zero or infinite proposal. In every such case the children see
the real geometry, never a stale probe.

Concretely: a stack allocates its main axis from `bounds` (§4.2 with
`M = bounds.main`) and proposes its resolved cross extent (`bounds.cross`) to
every child; a `ZStack`, `Overlay`, `Background`, `Absolute` and `Padding`
propose the bounds (less insets) they place into; a `Frame` proposes the
region it resolved; an `AspectRatio` proposes the ratio-correct box. A
container never places a child under a proposal it did not derive from
`bounds`, with the single exception in §5 (Grid column widths).

This is the SwiftUI behaviour (`placeSubviews` receives resolved bounds and a
fresh proposal) and the one breaking change 0.5.0 makes to the 0.4 rule
("placement keeps the measurement probe"): a title in a column that an
unshrinkable row has made wider than the viewport lays out on one line across
the column, as the reference twin does, instead of staying wrapped at the
proposal the column was measured with.

## 3. Stretch

`stretch_axis` is the only way a view claims space beyond its measured size.

- A leaf that names a stretch axis is offered the container's extent on that
  axis and takes at least what it measured: `extent = max(measured, offered)`.
- A container claims nothing of its own. Its stretch is the union of its
  children's, resolved against its axis: a `VStack` holding a `Spacer`
  (`MainAxis`) stretches vertically; a `VStack` holding a `Color` (`Both`)
  stretches on both axes; a row of labels stretches on neither.
- Modifier containers are transparent: `Padding`, `Background`, `Overlay`,
  `AspectRatio` and metadata wrappers report their content child's axis.
- A `Frame` stretches on an axis only when its `max` on that axis is
  `INFINITY`; `.width(v)` (`min = max = v`) and `.max_width(100)` do not
  stretch.
- `Absolute` is `Both` regardless of its children.
- A lazy container (`List`, lazy stacks) cannot enumerate its children before
  they are realised, so it reports its layout's axis over an empty child set:
  `None` for a lazy stack. A lazy stack that must fill is placed by a parent
  that stretches it (`ScrollView`, `Absolute`, a `Frame` with `max = INFINITY`).
- `ScrollView` is `Both`; `Spacer` is `MainAxis`; `Divider` is `CrossAxis`;
  `Color` and shapes are `Both`; text, buttons, toggles and other content
  controls are `None`; a text field is `Horizontal`.

## 4. Stacks (`HStack`, `VStack`)

The description below is written for an `HStack` (main axis horizontal, cross
axis vertical); a `VStack` is the same with the axes swapped.

### 4.1 Cross axis: the alignment envelope

Every child answers a cross extent `e` and a guide `g` for the stack's
alignment — its explicit guide when it answered one, otherwise the default
position of that alignment (`Top = 0`, `Center = e / 2`, `Bottom = e`;
`FirstTextBaseline` / `LastTextBaseline` are the leaf's baselines). Guides are
**not** clamped into `[0, e]`: a child may raise its guide above its own top
or below its own bottom, and the stack honours it.

The stack's cross envelope is

```
above = max over children of g
below = max over children of (e - g)
cross = above + below
```

Children answering an infinite extent are skipped; if any child answers
`INFINITY` the stack answers `INFINITY` on that axis (a maximum query or a
filling child), and the fill pass above resolves it. A stack never widens to
its proposal on its own: a column of two labels is as wide as the wider label,
wherever it is placed. Default spacing is 10.

At placement the envelope is anchored in `bounds` by the stack's alignment:

| alignment | line |
| --- | --- |
| `Top` / `Leading` | `bounds.min + above` |
| `Center` | `bounds.min + (bounds.cross - cross) / 2 + above` |
| `Bottom` / `Trailing` | `bounds.max - below` |
| explicit guide alignments, custom | as `Top` / `Leading` |

Every non-stretching child is placed at `line - g` with its own answer; a
cross-axis stretcher is placed at `bounds.min` with extent `bounds.cross`.
Nothing is clamped: a child wider than the bounds overflows and the alignment
decides which edges it crosses (`Center` crosses both equally). Nothing is
clipped by layout.

### 4.2 Main axis: one probe, then allocation

With an unspecified main proposal every child keeps its ideal extent (probe:
cross proposal, main `None`). `0` and `INFINITY` main proposals are forwarded
to the children unchanged. With a finite, non-zero main proposal `M`:

1. `available = max(0, M - spacing * (n - 1))`.
2. Each child's `min` is its answer to `(cross, 0)`; its `ideal` is
   `max(min, available)` when it stretches on the main axis, otherwise its
   answer to `(cross, INFINITY)` clamped into `[min, available]`.
3. `compress_to_fit` allocates `available`: everything fits → every child gets
   its ideal; otherwise higher `priority` bands are allocated first with every
   lower band's `min` reserved, and inside a band a common cap is lowered
   (water-filling) so the widest children give up the most and equal children
   shrink equally. No child goes below its `min`; when the minima alone exceed
   `available` the stack overflows rather than collapsing children.
4. Every child is measured once more at `(cross, allocation)`; a main-axis
   stretcher is reported at `max(answer, allocation)`.

The stack's main answer is the sum of the children's reported extents plus
spacing. At placement the same allocation runs with `M = bounds.main` and
`cross = bounds.cross`; children are laid out in order from `bounds.min`
(reversed under a right-to-left `LayoutDirection` in an `HStack`) at their
reported extents. A child is placed with the proposal it was last measured
with, `(bounds.cross, allocation)`.

### 4.3 Priority

`layout_priority(n)` sets a child's band. Compression takes space from the
lowest band first; within a band the water-fill rule applies. Priority never
grants space beyond a child's ideal.

### 4.4 Spacing and membership

Spacing is reactive; a spacing or membership change invalidates the stack and
returns the original geometry when reverted.

## 5. Other containers

- **ZStack**: every child is measured with the stack's proposal; the stack's
  answer on each axis is the alignment envelope of §4.1 over all children
  (`INFINITY` if any child answers it), **not** capped by the proposal. At
  placement children are measured again with `(bounds.width, bounds.height)`,
  placed by the stack alignment on the envelope line inside the bounds at their
  own answer (a child larger than the bounds overflows), and an unbounded
  answer fills the bounds. Every child is proposed the bounds.
- **Overlay** (`base.overlay(decoration)`): sized by the base child alone; the
  base is placed over the whole bounds and proposed them. Each decoration is
  measured with the bounds as its proposal, placed at its own answer (an
  unbounded answer fills the bounds) on the envelope line of its alignment
  inside the bounds, unclamped. Decorations never influence the parent's
  sizing.
- **Background** (`content.background(view)`): sized by the content alone; at
  placement both the background and the content are placed over the whole
  bounds and proposed them. `Material` and `Glass` backgrounds are metadata
  the backend projects; they do not enter layout.
- **Padding**: measures the child with the proposal shrunk by the insets and
  adds them back to the answer; at placement the child is placed in the bounds
  inset by the edges and proposed that inset region. Transparent to stretch
  and guides.
- **Frame** (`width/height/min/max/ideal`): on each axis the child hears the
  parent's proposal clamped into `[min, max]`, with `ideal` answering only an
  axis the parent left unspecified; the frame answers the child's answer
  clamped into `[min, max]`, growing into a finite offer only up to `max`.
  With no `max` it is exactly as big as its child, clamped up by `min`;
  `max = INFINITY` is how a view opts into filling. The frame measures its
  child a second time under the exact proposal it will place with (the
  resolved frame region) whenever that differs from the first probe, so its
  answer and its placement agree. At placement the child is placed at its
  answer to that proposal, aligned by the frame's alignment on the envelope
  line, unclamped (a child larger than the frame overflows).
- **AspectRatio**: `Fit` answers the largest box of the ratio inside the
  offer, `Fill` the smallest box covering it; an unspecified axis follows the
  child's answer on the other. At placement the box is resolved against the
  bounds, centred in them, and the child is placed in it and proposed it.
- **Absolute** (`absolute(...)`, `position_in`, `pin`): answers the proposal
  (`0` for an unspecified axis) and hands every child the full bounds as both
  frame and proposal; children position themselves. A window-level overlay
  layer (snackbar, dialog host) is an `Absolute` with `StretchAxis::Both`,
  never a content-sized `ZStack`.
- **Grid**: fixed column count. Under a finite width proposal every column is
  `(width - spacing * (columns - 1)) / columns` wide and the grid answers the
  proposal width; under an unspecified or infinite width each column is as
  wide as its widest cell's ideal answer. Each cell is measured with its
  column width and an unspecified height; each row is as tall as its tallest
  finite answer. Cells align by the grid alignment inside their cell; an
  unbounded answer fills the cell. Column widths at placement are computed
  from the width proposal the grid was measured with — the one documented
  exception to §2.3, kept so a content-sized grid keeps its content-sized
  columns.
- **Spacer**: `MainAxis`; on the stack's main axis it answers its minimum
  length (`spacer_min(n)`, 0 by default) to every proposal, so that length is
  the floor §4.2 keeps under compression — `spacer_min(40)` in a 50 pt column
  keeps 40 pt and the content around it compresses by priority — and it takes
  whatever surplus the stack allocates it. It answers zero on every other
  axis and under every other container, and claims nothing in a `ZStack`.
- **Divider**: 1 pt on the stack's main axis, fills the cross axis, drawn in
  the `BorderColor` theme slot.
- **Lazy containers** (`List`, lazy stacks): virtualised along one axis with
  the same per-child protocol over the realised children; membership diffs by
  identity. Stretch is §3.

## 6. Leaf contracts (backends)

A backend hosts native leaves inside Rust-driven containers. The leaf's
`measure` is the platform's answer, and it obeys the same protocol:

- **Text**: the answer's width is the laid-out width of the text — the widest
  line — never the proposal. A finite width proposal wraps; a `0` proposal
  answers the narrowest wrap; `None` answers the unwrapped line. Height is the
  line count times the platform line box, snapped to the display scale. A
  line limit caps the measured lines.
- **Controls** (button, toggle, picker, stepper, slider, progress): the
  platform control's intrinsic size, with the platform's own chrome padding
  for the selected style and none for a borderless one; a text field answers
  the proposal width (`Horizontal`) and its intrinsic height — a plain iOS
  text field is one 22 pt line with no border, fill or vertical floor.
- **ScrollView**: a scroll claims the whole offer — a finite proposal on
  either axis is answered with that proposal; only a `0` proposal measures the
  content, answering its intrinsic extent on the non-scrolling axis and `0` on
  the scrolling axis. The content is measured with the viewport extent on the
  non-scrolling axis and `None` on the scrolling axis; its frame is its own
  answer on the non-scrolling axis, and it is **centred** there whether it is
  narrower or wider than the viewport (a wider content crosses both edges by
  the same amount). A scroll surface that touches window chrome extends under
  it (safe-area extension rule).
- **Spacer** hosted natively answers its minimum length on the stack's main
  axis and zero on the cross axis, whatever the proposal (§5).
- **GPU surfaces, images, shapes, colours**: `Both`; an image with an
  intrinsic size answers it to `None` and fits the proposal otherwise.

A leaf that answers a probe with the proposal instead of its content on an
axis it does not stretch is a backend bug: it makes every container above it
report the proposal, and the failure surfaces as a stretched card or a
leading-parked column far from the leaf.

## 7. Placement on the backends

A backend applies the Rust placement verbatim: the child's frame is the
placement rect and its layout pass receives the placement proposal. A bridge
wrapper standing in a child's slot (a metadata wrapper, a clip host) lays its
content over the whole slot; it never re-centres, re-measures or gravity-packs
the content inside the slot.

The root proposal is the window's (or scene's) content size, both axes
finite; the root is placed at its answer to that proposal, stretched to the
window on the axes it declares. Safe-area insets are a backend concern applied
outside the protocol (`IgnoresSafeArea` opts out); they never change a
proposal a Rust container sees. Layout is single-threaded; a backend that
measures on another thread is outside the contract.

## 8. Divergence register

Behaviour that is deliberately not SwiftUI's. Each entry is a decision, not a
gap; changing one is a contract change.

| WaterUI | SwiftUI | why |
| --- | --- | --- |
| Compression water-fills inside a priority band (the widest give up the most). | Offers space least-flexible child first, in order of flexibility. | Equal children shrink equally and a single wide child absorbs the deficit; independent of child order. |
| Default stack spacing is a fixed 10 pt. | Platform-dependent, content-dependent default. | One value across backends keeps parity tests meaningful. |
| Controls are intrinsic-only (`None`) unless they declare an axis. | Some controls stretch by style. | The style attribute, not the widget type, decides; backends declare per style. |
| Images are `Both` and fit the proposal. | Images are fixed-size unless `.resizable()`. | Fitting is the common case for cross-platform content; an intrinsic size is answered to `None`. |
| Grid columns at placement use the measurement proposal width. | `Grid` re-resolves columns against bounds. | Keeps a content-sized grid's columns content-sized under the bounds rule. |

## 9. Invariants a change must preserve

The contract tests in `components/foundation/layout/src/tests/contract.rs`
and the per-container tests encode this document. They cover: the reference
allocations of flexible and nested children on both axes; distinct selected
proposals surviving other probes; rigid cross-axis answers surviving any cross
proposal; a rigid child overflowing a smaller host; fill children keeping
their minimum in small bounds; unbounded answers surviving a maximum query;
the resolved extent proposed at placement on both axes, under stretch and
under underfill; the main axis allocated from the bounds; explicit guides
shaping the envelope on edge alignments; and spacing/membership invalidation
returning to the original geometry. A pull request that has to weaken one of
these assertions is changing the contract and is rejected.
