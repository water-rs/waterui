# WaterUI layout specification

This document is normative. It describes the layout protocol that every
container in `waterui-layout`, every leaf a backend hosts, and every backend
bridge implements. It is frozen at 0.5.0: the semantics below do not change
without a major-version decision recorded by the maintainer, and a pull request
that changes them is rejected regardless of what it fixes. A difference between
this document and the code is a bug in the code.

The reference model is SwiftUI's layout protocol. Where SwiftUI's observable
behaviour is known it is the tie-breaker; where WaterUI deliberately differs the
difference is stated here.

## 1. Units and coordinates

- Every layout value is a logical pixel (point, dp). Backends convert to device
  pixels with the display scale; layout never sees a device pixel.
- `Rect` origins are physical left-to-right coordinates. `LayoutDirection`
  (leading/trailing) is resolved by containers when they place children; leaf
  and renderer code receives physical coordinates.
- Extents are finite and non-negative except a maximum, where
  `f32::INFINITY` means "unbounded".

## 2. The proposal protocol

A parent negotiates with a child through `ProposalSize { width, height }`, each
axis independently:

| proposal | question |
| --- | --- |
| `None` | ideal (intrinsic) extent |
| `Some(0.0)` | minimum extent |
| `Some(INFINITY)` | maximum extent |
| `Some(v)` | fit into `v` |

The child answers with `ViewDimensions`: a `Size` plus optional alignment
guides. The answer is the child's choice; the proposal is advice. On each axis
the three probe answers satisfy `min <= ideal <= max`.

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
  child, in order, inside `bounds`. `proposal` is the proposal this container
  was measured with — never a probe it sent a child, never a proposal derived
  from `bounds`. Each placement carries the frame and the proposal the child
  is placed with; the child's own layout pass receives that proposal verbatim.
  Which proposal a container hands a child at placement is that container's
  rule (§4.4, §5).
- Explicit alignment guides (`explicit_horizontal` / `explicit_vertical`)
  come from the placed children; a container exposes the alignments it can
  answer.

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
- `ScrollView` is `Both`; `Spacer` is `MainAxis`; `Divider` is `CrossAxis`;
  `Color` and shapes are `Both`; text, buttons, toggles and other content
  controls are `None`; a text field is `Horizontal`.

## 4. Stacks (`HStack`, `VStack`)

### 4.1 Cross axis: content-sized

A stack's cross extent is the widest finite child answer, measured with the
stack's own cross proposal; a child answering `INFINITY` sets no floor (the
fill pass gives it the stack's extent) and makes the stack answer `INFINITY`
on a maximum query. A stack never widens to its proposal on its own: a column
of two labels is as wide as the wider label, wherever it is placed. Default
spacing is 10.

A child wider than the stack's bounds overflows; it is placed at its measured
size and the stack's alignment decides which edges it crosses (`Center`
overflows both edges equally). Nothing is clipped by layout.

### 4.2 Main axis: one probe, then allocation

With an unspecified main proposal every child keeps its ideal extent. With a
finite main proposal `M`:

1. `available = M - spacing * (n - 1)`.
2. Each child's `min` is its answer to `0`; its `ideal` is `available`
   (clamped up to `min`) when it stretches on the main axis, otherwise its
   answer to `INFINITY` clamped into `[min, available]`.
3. `compress_to_fit` allocates `available`: everything fits → every child gets
   its ideal; otherwise higher `priority` bands are allocated first with every
   lower band's `min` reserved, and inside a band a common cap is lowered
   (water-filling) so the widest children give up the most and equal children
   shrink equally. No child goes below its `min`; when the minima alone exceed
   `available` the stack overflows rather than collapsing children.
4. Every child is measured once more at its allocation; a main-axis stretcher
   is reported at `max(answer, allocation)`.

`0` and `INFINITY` main proposals are forwarded to the children unchanged.

### 4.3 Priority

`layout_priority(n)` sets a child's band. Compression takes space from the
lowest band first; within a band the water-fill rule applies. Priority never
grants space beyond a child's ideal.

### 4.4 Placement proposal

At placement each child of an `HStack` or `VStack` receives, on the main axis,
the extent allocated in §4.2 under the proposal the stack was measured with,
and on the cross axis **the stack's resolved cross extent — the cross extent
of the bounds the stack was placed in**. The child is placed at the size it
answers to that proposal, aligned by the stack's alignment within the bounds;
a cross-axis stretcher is placed at `max(answer, bounds)`.

This is the SwiftUI behaviour and the one breaking change 0.5.0 makes to the
0.4 rule ("placement keeps the measurement probe on the cross axis"): a title
in a column that an unshrinkable row has made wider than the viewport lays out
on one line across the column, as the reference twin does, instead of staying
wrapped at the proposal the column was measured with. Because the stack's
bounds equal its own measured cross extent whenever the stack is
content-sized, the two rules agree everywhere except where a stack overflows
its proposal or was measured with an unspecified, zero or infinite cross
proposal.

### 4.5 Alignment guides

A child may answer explicit guides for an alignment. When any child answers
one for the stack's alignment, the stack aligns children on that guide line
(the widest intrinsic leading extent); otherwise it aligns on the bounds edge
or centre.

### 4.6 Spacing and membership

Spacing is reactive; a spacing or membership change invalidates the stack and
returns the original geometry when reverted.

## 5. Other containers

- **ZStack**: every child is measured with the stack's proposal; the stack's
  size is the per-axis maximum of the children's answers, capped by a finite
  proposal; children are re-measured with the same proposal at placement (a
  `ZStack` does not re-propose its bounds), placed by the stack alignment
  inside the bounds at `min(answer, bounds)`, and an unbounded answer fills the
  bounds.
- **Overlay** (`base.overlay(decoration)`): sized by the base child alone; the
  overlay child is measured with the base's size and aligned inside it. It
  never influences the parent's sizing.
- **Background** (`content.background(view)`): sized by the content alone;
  the background child fills the content's bounds and is proposed them; the
  content keeps the proposal it was measured with. `Material` and `Glass`
  backgrounds are metadata the backend projects; they do not enter layout.
- **Padding**: shrinks the child's proposal by the insets, adds them back to
  the answer, places the child inset; transparent to stretch and guides.
- **Frame** (`width/height/min/max/ideal`): the child hears the parent's
  proposal clamped into `[min, max]`, with `ideal` answering only an axis the
  parent left unspecified. The frame grows into what it was offered up to
  `max`; with no `max` it is exactly as big as its child, clamped up by `min`.
  `max = INFINITY` is how a view opts into filling.
- **AspectRatio**: fits the ratio inside what is offered; `Fit` shrinks to
  the offer, `Fill` covers it.
- **Absolute** (`absolute(...)`, `position_in`, `pin`): fills its parent and
  hands every child the full bounds; children position themselves. A
  window-level overlay layer (snackbar, dialog host) is an `Absolute` with
  `StretchAxis::Both`, never a content-sized `ZStack`.
- **Grid**: fixed column count; each column is as wide as its widest cell
  answer, each row as tall as its tallest; cells align by the grid alignment.
- **Spacer**: `MainAxis`, minimum length 0 (`spacer_min(n)`); it takes the
  allocation the stack gives it and nothing in a `ZStack`.
- **Divider**: 1 pt on the stack's main axis, fills the cross axis, drawn in
  the `BorderColor` theme slot.
- **Lazy containers** (`List`, lazy stacks): virtualised along one axis with
  the same per-child protocol; membership diffs by identity.

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
  the proposal width (`Horizontal`) and its intrinsic height.
- **ScrollView**: a scroll claims the whole offer — a finite proposal on
  either axis is answered with that proposal; only a `0` proposal measures the
  content, answering its intrinsic extent on the non-scrolling axis and `0` on
  the scrolling axis. The content is measured with the viewport extent on the
  non-scrolling axis and `None` on the scrolling axis; its frame is its own
  answer on the non-scrolling axis, and it is **centred** there whether it is
  narrower or wider than the viewport (a wider content crosses both edges by
  the same amount). A scroll surface that touches window chrome extends under
  it (safe-area extension rule).
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
the content inside the slot. Safe-area insets are a backend concern applied
outside the protocol (`IgnoresSafeArea` opts out); they never change a
proposal a Rust container sees.

## 8. Invariants a change must preserve

The contract tests in `components/foundation/layout/src/tests/contract.rs`
and the per-container tests encode this document. They cover: the reference
allocations of flexible and nested children on both axes; distinct selected
proposals surviving other probes; rigid cross-axis answers surviving any cross
proposal; a rigid child overflowing a smaller host; fill children keeping
their minimum in small bounds; unbounded answers surviving a maximum query;
the resolved cross extent proposed at placement; and spacing/membership
invalidation returning to the original geometry. A pull request that has to
weaken one of these assertions is changing the contract and is rejected.
