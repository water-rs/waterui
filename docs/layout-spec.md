# WaterUI layout specification

This document is normative. It describes the layout protocol that every
container in `waterui-layout`, every leaf a backend hosts, and every backend
bridge implements. It was frozen at 0.5.0 (AGENTS.md, Principle 11): the
semantics below do not change without a major-version decision recorded by the
maintainer, and a pull request that changes them is rejected regardless of
what it fixes. A difference between this document and the code is a bug in
the code; a backend that disagrees with it is fixed on that backend.

The reference model is SwiftUI's layout protocol. Where the observable
behaviour of SwiftUI is known it is the tie-breaker for anything this
document leaves open; where WaterUI deliberately differs, the difference is
stated here and is part of the contract.

## 1. Units and coordinates

- Every layout value is a logical pixel (a point, a dp). Backends convert to
  device pixels with the display scale; layout never sees a device pixel, and
  a backend snaps only when it rasterizes (text line boxes snap to the display
  scale; a 1 pt divider is 1 pt).
- `Rect` origins are physical left-to-right coordinates. `LayoutDirection`
  (leading/trailing) is resolved by containers when they place children; leaf
  and renderer code receives physical coordinates.
- Extents are finite and non-negative except a maximum, where
  `f32::INFINITY` means "unbounded".

## 2. The proposal protocol

A parent negotiates with a child through `ProposalSize { width, height }`,
each axis independently:

| proposal | question |
| --- | --- |
| `None` | the ideal (intrinsic) extent |
| `Some(0.0)` | the minimum extent |
| `Some(INFINITY)` | the maximum extent |
| `Some(v)` | fit into `v` |

The child answers with `ViewDimensions`: a `Size` plus optional alignment
guides. The answer is the child's choice and the proposal is advice, but on
each axis the three probe answers satisfy `min <= ideal <= max`. Breaking
that is never a local error: a minimum above the ideal makes a stack compress
a child past a size it cannot take, and the misplacement surfaces somewhere
else entirely.

### 2.1 The `SubView` contract

Every child a container measures is a `SubView`:

- `measure(proposal)` answers as above. It is a pure function of the proposal
  within one layout pass and may be called many times with different
  proposals. Caching is the `SubView`'s job (text shaping must cache); the
  `Layout` trait has no cache and never gains one.
- `stretch_axis()` declares which axes the view fills when offered more than
  it measures: `None`, `Horizontal`, `Vertical`, `Both`, or the stack-relative
  `MainAxis` / `CrossAxis`. It is read before `body` runs and without an
  environment, so a view whose body contains a filling leaf declares that
  leaf's axis itself.
- `priority()` orders the compression bands (§4.3); ties share.

Measurement is single-threaded and runs on the thread that drives layout; a
`SubView` is neither `Send` nor `Sync`.

### 2.2 The `Layout` contract

A container is a `Layout`:

- `size_that_fits(proposal, children) -> Size` is the container's answer to
  a proposal. It may probe children freely.
- `place(bounds, proposal, children) -> Vec<SubviewPlacement>` places every
  child, in order, inside `bounds`. `proposal` is the proposal this container
  was measured with — never a probe it sent a child and never a proposal
  derived from `bounds`. Each placement carries the child's frame and the
  proposal the child is placed with, and the child's own layout pass receives
  that proposal verbatim. Equal frames may carry different proposals; the
  frame does not determine the offer that produced it.
- Explicit alignment guides (`explicit_horizontal` / `explicit_vertical`)
  come from the placed children; a container exposes the alignments it can
  answer.

## 3. Stretch

`stretch_axis` is the only way a view claims space beyond its measured size.

- A leaf that names a stretch axis is placed at `max(measured, offered)` on
  that axis.
- A container claims nothing of its own. Its stretch is the union of its
  children's, resolved against its axis: a `VStack` holding a `Spacer`
  (`MainAxis`) stretches vertically; one holding a `Color` (`Both`) stretches
  on both axes; a column of labels stretches on neither. The space a child
  asks for has to be asked for again by every container between it and
  whoever owns the space.
- Modifier containers are transparent: `Padding`, `Background`, `Overlay`,
  `AspectRatio` and every metadata wrapper report their content child's axis.
- A `Frame` stretches on an axis only when its `max` on that axis is
  `INFINITY`. `.width(v)` (`min = max = v`) and `.max_width(100)` do not
  stretch.
- Leaves: `ScrollView`, `Color`, shapes, GPU surfaces, images, maps, web
  views are `Both`; `Spacer` is `MainAxis`; `Divider` is `CrossAxis`; a text
  field, secure field and slider are `Horizontal`; text, buttons, toggles,
  pickers, steppers, menus and every other content control are `None`.

## 4. Stacks (`HStack`, `VStack`)

### 4.1 Cross axis: content-sized

A stack's cross extent is the widest finite child answer, measured with the
stack's own cross proposal. A child answering `INFINITY` sets no floor (the
fill pass gives it the stack's extent) and makes the stack answer `INFINITY`
to a maximum query. A stack never widens to its proposal on its own: a
column of two labels is as wide as the wider label wherever it is placed, and
a card built from labels and buttons is as wide as its widest row, not as
wide as the slot it sits in.

A child wider than the stack's bounds overflows. It is placed at its measured
size and the stack's alignment decides which edges it crosses: `Center`
overflows both edges equally. Layout clips nothing.

### 4.2 Main axis: one probe, then allocation

With an unspecified main proposal every child keeps its ideal extent. With a
finite, non-zero main proposal `M`:

1. `available = M - spacing * (n - 1)`, floored at zero.
2. Each child's `min` is its answer to `0`. Its `ideal` is `available`
   (clamped up to `min`) when it stretches on the main axis, otherwise its
   answer to `INFINITY` clamped into `[min, available]`.
3. `compress_to_fit` allocates `available`. When every ideal fits, every
   child gets its ideal. Otherwise higher `priority` bands are allocated
   first with every lower band's `min` reserved, and inside a band a common
   cap is lowered (water-filling) so the widest children give up the most and
   equal children shrink equally. No child goes below its `min`; when the
   minima alone exceed `available` the stack overflows rather than collapsing
   a child to nothing.
4. Every child is measured once more at its allocation; a main-axis stretcher
   is reported at `max(answer, allocation)`.

`0` and `INFINITY` main proposals are forwarded to the children unchanged and
answer the stack's minimum and maximum.

### 4.3 Priority

`layout_priority(n)` sets a child's band; `Spacer` sits at `i32::MIN`.
Compression takes space from the lowest band first; within a band the
water-fill rule applies. Priority never grants a child more than its ideal.

### 4.4 Placement proposal

At placement each child receives the proposal it was measured with in §4.2:
its main-axis allocation, and on the cross axis the stack's own cross
proposal (the probe). The child is placed at the size that measurement
recorded, aligned by the stack's alignment inside the bounds; a cross-axis
stretcher is placed at `max(answer, bounds)`.

The placement proposal is therefore never derived from the bounds. Sizing and
placement stay consistent by construction: a text that wrapped at the probe
is placed with the probe and keeps its line breaks, so the height the stack
was sized with is the height it is placed with. This is a deliberate
difference from SwiftUI, which re-proposes the placed bounds; it is visible
only when a rigid child makes a column wider than the column's proposal, and
in that case the other children keep the layout they were measured with
instead of re-flowing into the overflow.

### 4.5 Alignment guides

A child may answer explicit guides for an alignment. When any child answers
one for the stack's alignment, the stack aligns every child on that guide
line (the widest intrinsic leading extent); otherwise it aligns on the bounds
edge or centre.

### 4.6 Spacing and membership

Spacing is reactive. A spacing or membership change invalidates the stack,
and reverting it returns the original geometry.

## 5. Other containers

- **ZStack**: every child is measured with the stack's proposal; the stack's
  size is the per-axis maximum of the children's answers; children are placed
  by the stack alignment inside the bounds and stretchers fill them.
- **Overlay** (`base.overlay(decoration)`): sized by the base child alone;
  the overlay child is measured with the base's size and aligned inside it.
  It never influences the parent's sizing.
- **Background** (`content.background(view)`): sized by the content alone;
  the background child fills the content's bounds and is offered exactly
  them. `Material` and `Glass` backgrounds are metadata a backend projects;
  they do not enter layout.
- **Padding**: shrinks the child's proposal by the insets, adds them back to
  the answer, places the child inset; transparent to stretch and guides.
- **Frame** (`width`/`height`/`min`/`max`/`ideal`): the child hears the
  parent's proposal clamped into `[min, max]`, with `ideal` answering only an
  axis the parent left unspecified. The frame grows into what it was offered
  up to `max`; with no `max` it is exactly as big as its child, clamped up by
  `min`. `max = INFINITY` is how a view opts into filling.
- **AspectRatio**: fits the ratio inside what is offered; `Fit` shrinks to
  the offer, `Fill` covers it.
- **Absolute** (`absolute(...)`, `position_in`, `pin`): fills its parent and
  hands every child the full bounds; children position themselves. A
  window-level overlay layer (snackbar, dialog host) is an `Absolute` with
  `StretchAxis::Both`, never a content-sized `ZStack`.
- **Grid**: fixed column count; each column is as wide as its widest cell
  answer, each row as tall as its tallest; cells align by the grid alignment.
- **Spacer**: `MainAxis`, minimum length 0 (`spacer_min(n)`). It measures as
  its minimum length on both axes and takes the allocation the stack gives it
  on the main axis; in a `ZStack` it takes nothing.
- **Divider**: a `Frame` over the `BorderColor` theme colour, 1 pt on the
  stack's main axis, `CrossAxis` on the other.
- **Lazy containers** (`List`, lazy stacks): virtualised along one axis with
  the same per-child protocol; membership diffs by identity.

## 6. Leaf contracts (backends)

A backend hosts native leaves inside Rust-driven containers. The leaf's
`measure` is the platform's answer and obeys the same protocol. A leaf that
answers a probe with the proposal on an axis it does not stretch is a backend
bug: every container above it then reports the proposal, and the failure
surfaces far away as a stretched card, a leading-parked column or a
one-column grid.

- **Text**: the width is the laid-out width — the widest line — never the
  proposal. A finite width proposal wraps; a `0` proposal answers the
  narrowest wrap; `None` answers the unwrapped line. Height is the line
  count times the platform line box, snapped to the display scale. A line
  limit caps the measured lines. Hyphenation and line-break rules are the
  platform's.
- **Spacer**: its minimum length on both axes (§5); never the proposal.
- **Controls** (button, toggle, picker, stepper, progress): the platform
  control's intrinsic size, with the label measured under the control's
  proposal minus its chrome. A text field, secure field and slider answer
  the proposal width and their intrinsic height.
- **ScrollView**: a scroll claims the whole offer — a finite proposal on
  either axis is answered with that proposal, and only a `0` proposal
  measures the content, answering the content's intrinsic extent on the
  non-scrolling axis and `0` on the scrolling axis. The content is measured
  with the viewport extent proposed on the non-scrolling axis and `None` on
  the scrolling axis. On the scrolling axis the content is framed at its
  answer (short content sits at the leading end); on the non-scrolling axis
  it is framed at its answer and centred in the viewport, whether narrower
  or wider than it (wider content crosses both edges by the same amount). A
  scroll surface that touches window chrome extends under it (the safe-area
  extension rule).
- **GPU surfaces, images, shapes, colours**: `Both`; an image with an
  intrinsic size answers it to `None` and fits the proposal otherwise.

## 7. Placement on the backends

A backend applies the Rust placement verbatim: the child's frame is the
placement rect and its layout pass receives the placement proposal. A bridge
wrapper standing in a child's slot (a metadata wrapper, a clip host) lays its
content over the whole slot; it never re-centres, re-measures or
gravity-packs the content inside the slot. Safe-area insets are a backend
concern applied outside the protocol (`IgnoresSafeArea` opts out); they never
change a proposal a Rust container sees.

## 8. Invariants a change must preserve

The contract tests in `components/foundation/layout/src/tests/contract.rs`
and the per-container tests encode this document: the reference allocations
of flexible and nested children on both axes; distinct selected proposals
surviving other probes; rigid cross-axis answers surviving any cross
proposal; a rigid child overflowing a smaller host; fill children keeping
their minimum in small bounds; unbounded answers surviving a maximum query;
spacing and membership invalidation returning to the original geometry. A
pull request that has to weaken one of these assertions is changing the
contract and is rejected.
