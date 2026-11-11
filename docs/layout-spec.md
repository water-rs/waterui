# WaterUI layout specification

This document is normative. It describes the layout protocol that every
container in `waterui-layout`, every leaf a backend hosts, and every backend
bridge implements. It was frozen at 0.5.0 and amended once, at 0.6.0, by the
decision below: the semantics do not change
without a major-version decision recorded by the maintainer, and a pull request
that changes them is rejected regardless of what it fixes. A difference between
this document and the code is a bug in the code.

The reference model is SwiftUI's layout protocol as observed on iOS 26 and
macOS 26. Where SwiftUI's observable behaviour is known it is the tie-breaker;
where WaterUI deliberately differs the difference is listed in §8.

**0.6.0 amendment: response geometry.** The 0.6.0 major version adds one
quantity to the contract (water-rs/waterui#1323). Until 0.5 a view had an
*allocation* — the frame its parent assigned — and placed its children; it
had no positioned rectangle of its own answer. From 0.6 every placed view also
has a *response rectangle*: its answer to its placement proposal, positioned
inside its allocation by its layout's rule (§2.4, §5, §6). The two are
separate: children, drawing and hit testing keep using the allocation, and
accessibility uses the response (§7.2). The same amendment fixes the one
undecided case it touches, the start of an underfilled stack's main-axis
block (§4.2). This is a semantic change, not a fix of 0.5 behaviour.

## 1. Units and coordinates

- Every layout value is a logical pixel (point, dp). Backends convert to device
  pixels with the display scale; layout never sees a device pixel.
- `Rect` origins are physical left-to-right, top-to-bottom coordinates.
  `LayoutDirection` (leading/trailing) is resolved by containers when they
  place children; leaf and renderer code receives physical coordinates.
- Extents are finite and non-negative except a maximum, where
  `f32::INFINITY` means "unbounded". A container's answer may be `INFINITY`
  on an axis only when a child answered `INFINITY` on it — with one narrow
  exception: `Absolute` (§5) answers a maximum probe with the probe itself.
- Placement geometry is finite. An answer placed in finite bounds is
  *concretized*: a `+INFINITY` axis takes the assigned extent and every other
  axis keeps the answer. The raw answer is retained beside it. NaN, negative
  infinity and negative extents are contract violations (`LayoutError`), never
  synonyms for "unbounded". A zero extent is real geometry.
- Within one layout invocation the bounds, the response rectangle and the
  child frames share one coordinate system, and every geometry query is
  translation-covariant: translating the bounds by `d` translates the answer
  by `d`.

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

The *answer* is always the raw answer to a proposal. A container may account
for a child at a larger *reported extent* (a stack's `max(answer, offer)`,
§4.2); that promotion decides the child's allocation, and never overwrites
the answer the child gave.

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
- `resolve_extent(bounds, proposal)` answers where the view's answer to
  `proposal` sits when it is assigned `bounds`: a `ResolvedExtent` holding
  the raw answer and the positioned response rectangle, in the coordinates of
  `bounds`. It is a pure query with the same purity and caching rules as
  `measure`: it places nothing, installs no frame, emits no accessibility and
  commits nothing, and it may be asked with a proposal other than the one the
  view is placed with. A leaf answers by its leaf rule (§6), a view driven by
  a `Layout` through `resolve_layout_extent`, and a layout-transparent wrapper
  forwards the query with the same bounds and proposal.

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
- `resolve_extent(input, children) -> Rect` positions the container's own
  response. `input` is a `PlacementInput`: the assigned `bounds`, the
  incoming `proposal` (never reconstructed from the bounds) and the
  container's `answer` to it, supplied by the driver. The rule is the
  container's own (§5) and mandatory; there is no default. A container whose
  rule derives from a child asks that child's `resolve_extent`, never its
  placement; a container whose rule reads no child reads none, so a lazy
  container can ask it with no children.
- The driver owns the pairing. `place_layout(layout, bounds, proposal,
  children)` measures the answer once, resolves the response and places the
  children from the same snapshot, returning a `LayoutPlacement` (answer,
  response rectangle, child placements); `resolve_layout_extent` is the
  query alone and is what a container's `SubView::resolve_extent` returns.
  Both reject a response whose extent is not the concretized answer.
- Explicit alignment guides (`explicit_horizontal` / `explicit_vertical`)
  come from the placed children; a container exposes the alignments it can
  answer. Resolving an explicit guide runs the container's `place`, so a
  container's guide is always consistent with its placement. Guides stay in
  allocation-local coordinates: the response origin never rebases them.

### 2.3 Placement is a negotiation against the resolved bounds

Placement is not a replay of measurement. When a container is placed it
negotiates with its children again, against the bounds it was actually given,
on **both** axes. Equal negotiation inputs produce equal results. Placement
nevertheless negotiates against resolved bounds, which may differ from the
proposal used for measurement even when the container is content-sized.
Equality between a container's measured extent and its bounds does not
require a child's response to be idempotent under reproposal. In every
case the children see the real geometry, never a stale probe.

Concretely: a stack allocates its main axis from `bounds` (§4.2 with
`M = bounds.main`) and proposes its resolved cross extent (`bounds.cross`) to
every child; a `ZStack`, `Overlay`, `Background`, `Absolute` and `Padding`
propose the bounds (less insets) they place into; a `Frame` proposes the
region it resolved; an `AspectRatio` proposes the ratio-correct box. A
container never places a child under a proposal it did not derive from
`bounds`, with the single exception in §5 (the Grid cell proposals).

This is the SwiftUI behaviour (`placeSubviews` receives the same incoming
proposal plus the resolved bounds; which proposal each child then sees is
the layout's own decision) and the one breaking change 0.5.0 makes to the
0.4 rule ("placement keeps the measurement probe"): a title in a column
that an unshrinkable row has made wider than the viewport lays out on one
line across the column, as the reference twin does, instead of staying
wrapped at the proposal the column was measured with.

Resolving a response never replaces this negotiation. A wrapper's response
query follows the proposal that produced its answer (§2.4), while its child
is still placed under the bounds-derived proposal above; the two may differ,
and a geometry query is never a hidden child placement.

### 2.4 Response geometry

Every placed view has a response rectangle. It obeys:

- **Extent identity.** On every axis its extent is the view's concretized
  answer to its placement proposal (§1). No union, clip, stretch inference or
  accessibility-child count changes it.
- **Answer, not content.** It is the view's answer to its incoming proposal,
  positioned by an explicit rule. It does not promise to enclose the
  placement-time children or their drawing: under §2.3 a padded child can be
  placed under a larger proposal and answer more than the padding's own
  answer allowed for.
- **No containment.** It may extend outside the allocation, and children and
  drawing may extend outside it. Neither implies clipping.
- **Provenance.** It is resolved from the exact proposal and layout snapshot
  the view was placed with; another probe never overwrites the selected
  answer.
- **Owned by the layout.** The rule belongs to the view's layout or leaf
  contract. A backend never chooses a centre, reconstructs the rectangle from
  descendants, or substitutes another node's rectangle for a zero one.

## 3. Stretch

`stretch_axis` is the only way a view claims space beyond its measured size.

- A leaf that names a stretch axis takes at least what it measured where the
  container's allocation rule promotes it: `extent = max(measured, offered)`
  under the rules that promote (a stack's offers in §4.1 and §4.2). The
  promotion is per container, never universal — a `ZStack` proposes the
  bounds but keeps a stretcher's finite answer (§5). Promotion changes the
  child's allocation, not its answer: a `Spacer` allocated a 40-pt gap still
  answers its minimum length, and its response rectangle is that answer.
  Naming the allocated gap is an explicit choice of geometry source (§7.2).
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
- A lazy stack cannot enumerate its children before they are realised, so it
  reports its layout's axis over an empty child set: `None`. A lazy stack that
  must fill is placed by a parent that stretches it (`ScrollView`, `Absolute`,
  a `Frame` with `max = INFINITY`).
- A `List` is `Both`: like a `ScrollView`, it fills the space it is offered,
  and `List`s stacked in one container share the space left after their
  siblings by the stack's main-axis negotiation (§4.2).
- A `ScrollView` stretches on its scrolling axis, and on the non-scrolling
  axis only when its content stretches there (`Axis::All` is `Both`);
  `Spacer` is `MainAxis`; `Divider` is `CrossAxis`;
  `Color` and shapes are `Both`; text, buttons and other content controls
  are `None`; a text field is `Horizontal`; a toggle is `Horizontal` when it
  draws the switch style with a visible label, and `None` otherwise; a stepper
  is `Horizontal` when its label is visible, and `None` otherwise; a picker
  is `Horizontal` in the segmented style, and `None` otherwise.

## 4. Stacks (`HStack`, `VStack`)

The description below is written for an `HStack` (main axis horizontal, cross
axis vertical); a `VStack` is the same with the axes swapped.

### 4.1 Cross axis: the alignment envelope

Every child answers a cross extent `e` and a guide `g` for the stack's
alignment — its explicit guide when it answered one, otherwise the default
position of that alignment (`Top = 0`, `Center = e / 2`, `Bottom = e`;
`FirstTextBaseline` / `LastTextBaseline` are the leaf's baselines). Guides are
**not** clamped into `[0, e]` in a stack: a child may raise its guide above
its own top or below its own bottom, and the stack honours it. The
no-clamping rule is stack-only — Grid cells bound theirs (§5).

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
cross-axis stretcher is placed at `bounds.min` with extent
`max(its measured extent, bounds.cross)` — a stretcher that answered
`INFINITY` fills `bounds.cross`.
Nothing is clamped: a child wider than the bounds overflows and the alignment
decides which edges it crosses (`Center` crosses both equally). Nothing is
clipped by layout.

The stack's response rectangle has the stack's answer as its extent. On the
cross axis it is anchored by the same alignment — start for `Top` /
`Leading`, centre for `Center`, end for `Bottom` / `Trailing`, start for
baselines and custom guides — using the measured envelope as its extent, not
the union of the final child frames.

### 4.2 Main axis: one probe, then allocation

With an unspecified main proposal every child keeps its ideal extent (probe:
cross proposal, main `None`). `0` and `INFINITY` main proposals are forwarded
to the children unchanged. With a finite, non-zero main proposal the stack
negotiates:

1. `available` is the finite main extent after subtracting member spacing,
   clamped to zero.
2. Each child's `min` is probed at main proposal zero. Its `target` is
   `max(min, available)` when it stretches on the main axis; otherwise its
   maximum-probe answer limited to `available` and raised to its `min`.
3. If every target fits, each child is proposed its target. If the minima
   alone exceed `available`, each child is proposed its `min` and the stack
   overflows.
4. Otherwise the priority bands negotiate from highest to lowest. Within a
   band the children negotiate by increasing flexibility — the
   maximum-probe answer minus the `min`; a main-axis stretcher has an
   effective unbounded maximum — and equal flexibility is resolved by
   original logical member order, independently of layout direction. Before
   each offer the reported extents of processed children are deducted from
   `available`; the `min` of every unprocessed lower-priority child is
   reserved; the remaining band budget is divided by the number of
   unprocessed children in the band; and the offer is limited so the other
   unprocessed children in the band retain their `min`, clamped between the
   child's `min` and `target`.
5. The child is measured once, at the offer and the negotiation's unchanged
   cross proposal. Its reported main extent is its answer, or
   `max(answer, offer)` when it stretches on the main axis; that reported
   extent is deducted before the next child. A processed child is not
   offered space again during a negotiation. The complete selected
   dimensions and proposal are preserved for each child. Answers larger
   than offers may cause overflow; they are not clipped or replaced by the
   offers.

The stack's main answer is the sum of the reported main extents and member
spacing. Its cross answer is the alignment envelope of the selected
dimensions. A sum exceeds a budget only when it is larger by more than
the `f32` forward error of the operation, `ε · max(|sum|, |budget|, 1) · n`
for `n` terms; no visual "close enough" threshold enters child-response
accounting.

At placement the same negotiation runs afresh with `main = bounds.main` and
`cross = bounds.cross`. The negotiated block — the members' reported extents
plus member spacing — is centred in `bounds.main`, crossing both edges
equally when it overflows them; the members are then placed from its start
in logical member order, reversed physically for a right-to-left `HStack`,
at their reported extents. Centring moves the block as a whole: it does not
redistribute unused space or rerun the negotiation. Each placement carries
the proposal that negotiation selected, not a proposal reconstructed from its
frame.

On the main axis the stack's response rectangle is centred in the bounds.
The placement-time block and the response may differ in extent (§2.3); both
are centred on the same line.

### 4.3 Priority

`layout_priority(n)` sets a child's band. Priority determines the order in
which bands negotiate their sizes: higher bands receive their offers before
lower bands, while every unprocessed child retains its measured `min`
reservation. Within a band, §4.2's flexibility order applies. Priority
determines sizing opportunities; it does not force a child to accept an
offer or cause a processed child to be reconsidered. Declined space remains
available to subsequently processed children, including children in lower
bands. A response query never changes the negotiation order or causes a
processed child to be reconsidered.

An ordinary child's band defaults to `0`, a `Spacer`'s to `i32::MIN` —
below every ordinary child, so it absorbs only what remains. An explicit
`layout_priority(n)` replaces the default outright: a `Spacer` raised to
`0` negotiates as an ordinary child that stretches.

### 4.4 Spacing and membership

Spacing is reactive; a spacing or membership change invalidates the stack and
returns the original geometry when reverted. A child that renders nothing is
not a stack member: it takes no slot and no spacing, and contributes to
neither the alignment envelope, nor the guides the stack exports, nor the
container's stretch — which is a semantic
question, not a size answer, since a zero-size `Color` or `Spacer` is still a
member — and a conditional switching between rendering nothing and rendering
a view is a membership change, so it invalidates the stack the same way a
view swap does. An invalidation invalidates retained response geometry with
the rest of the layout. How many semantic children a view exposes never
changes its layout membership or its response.

### 4.5 Structural transition retention

A view with a removal transition leaves semantic membership immediately: it
is absent from accessibility, testing, hit testing, and focus traversal. Its
outgoing realization becomes a non-interactive ghost. A row leaving a
collection's membership (`ForEach`, `List`, a lazy stack) keeps a layout-only
slot in that collection. The slot is a fixed-size leaf: every proposal
returns the dimensions and alignment guides recorded at removal, its stretch
axis is `None`, and its layout priority is preserved. It remains a layout
member, including spacing, until the transition completes. At completion the
slot is removed and siblings move using the container's animation. Without a
removal transition, removal releases the slot immediately as in §4.4.

Insertion allocates the new slot immediately at its normal layout size and
animates only the content. Re-insertion during an exit creates a new semantic
instance and a new slot; the old ghost completes independently. An exiting
slot is still placed every frame, so its global frame follows scrolling and
ancestor movement. A pixel transition is drawn in a window-filling overlay
anchored to that frame; ancestor clipping does not clip the effect. Retention
does not preserve removed descendants as semantic members.

The semantic rules apply at every structural site: dynamic replacement,
collection membership, navigation, and overlays. Scalar property updates do
not create transition instances or affect membership.

A conditional (`when`, `watch`) owns the ghosts of the content it replaces,
whatever layout encloses it, and so does a scroll, page, or overlay root:
none of them holds a slot for a ghost. The enclosing layout sees one child,
sized by the live content alone. The ghost paints at its last placed frame,
overlapping that slot beneath the live content, and follows scrolling and
ancestor movement; it takes no part in layout and, as above, no part in
input or accessibility. A navigation page root's declared transition is that
page's insertion transition on push and its removal transition on pop, in
place of the stack's own transition for that page.

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
  measured with the bounds as its proposal and placed at its own answer (an
  unbounded answer fills the bounds) on the envelope line its own guide
  forms for the overlay's alignment inside the bounds — the anchor is the
  decoration's envelope, never the base's — unclamped. Decorations never
  influence the parent's sizing.
- **Background** (`content.background(view)`): sized by the content alone; at
  placement both the background and the content are placed over the whole
  bounds and proposed them. `Material` and `Glass` backgrounds are metadata
  the backend projects; they do not enter layout.
- **Padding**: measures the child with the proposal shrunk by the insets and
  adds them back to the answer; at placement the child is placed in the bounds
  inset by the edges and proposed that inset region. Negative insets are
  legal: the container extent is `max(0, child + insets)` on each axis and
  the child sits at the inset offset, which may lie outside the bounds.
  Transparent to stretch and guides.
- **Frame** (`width/height/min/max/ideal`): an inverted constraint — `min`
  above `max` on one axis — resolves in the `min`'s favour, the same
  precedence CSS gives a `min-width` over a conflicting `max-width`: the
  effective maximum is the minimum, and the axis resolves to `min`. On each
  axis the child hears the
  parent's proposal clamped into `[min, max]`, with `ideal` answering only an
  axis the parent left unspecified; the frame answers
  `min(max, max(child, proposal))` on each axis — the child's answer, raised
  to the offer when the offer is larger and `max` allows, and never below the
  child's answer even when the offer is narrower.
  With no `max` it is exactly as big as its child, clamped up by `min`;
  `max = INFINITY` is how a view opts into filling. The frame measures its
  child a second time under the exact proposal it will place with (the
  resolved frame region) whenever that differs from the first probe, so its
  answer and its placement agree. At placement the child is placed at its
  answer to that proposal, aligned by the frame's alignment on the envelope
  line, unclamped (a child larger than the frame overflows).
- **AspectRatio**: `Fit` answers the largest box of the ratio inside the
  offer, `Fill` the smallest box covering it. A non-finite proposal axis —
  `None` or `INFINITY` — counts as unspecified: with one axis unspecified
  the bound axis sets the box and the other follows the ratio; with both
  unspecified the box follows the child's intrinsic answer —
  `(w, w / ratio)` when it reports a nonzero width, `(h * ratio, h)` when
  it reports a zero width. At placement the box is resolved against the
  bounds, centred in them, and the child is placed in it and proposed it.
- **Absolute** (`absolute(...)`, `position_in`, `pin`): answers the proposal
  (`0` for an unspecified axis, the proposal itself for a maximum probe —
  the §1 exception) and hands every child the full bounds as both frame and
  proposal; children position themselves. A `pin` resolves its child
  against the bounds by edges: an explicit `width`/`height` beats the extent
  a paired edge implies (`leading` + `trailing`, `top` + `bottom`); without
  either, the extent follows the child's answer to the pinned proposal; on
  the origin, `leading` and `top` win when both edges are pinned.
  `leading`/`trailing` are logical edges — the resolved layout direction
  mirrors them. A `position_in(p)` / `position(x, y)` places the child's
  anchor point on the target; the anchor is explicit
  (`position_in_anchor`, `position_anchor`) and defaults to the child's
  centre when omitted. Leading and trailing in the anchor and the target
  resolve to physical coordinates before placement. A window-level overlay
  layer (snackbar, dialog host) is an `Absolute` with `StretchAxis::Both`,
  never a content-sized `ZStack`.
- **Grid**: fixed column count. Under a finite width proposal every column
  is `max(0, (width - spacing * (columns - 1)) / columns)` wide and the
  grid answers the proposal width; under an unspecified or infinite width
  each column is as wide as its widest cell's ideal answer. Each cell is
  measured with its column width and an unspecified height; each row is as
  tall as its tallest finite answer. Every child the grid is built with
  keeps a cell, a semantically empty child included — §4.4's membership
  rule is a stack rule, not a grid rule; removing a child from the
  collection is what removes its slot. A `GridRow` is construction-time
  grouping: the grid flattens each row's children into that row's cells and
  the row contributes no size, guide or stretch of its own. Cells align by
  the grid alignment inside their cell and an unbounded answer fills the
  cell; a cell's explicit guides are honoured inside the cell but clamped
  to `[0, cell extent]` — §4.1's open envelope is stack-only. Placement
  keeps the measurement pass's cell proposals: columns from the width
  proposal the grid was measured with and each cell proposed
  `(column width, None)`, so row heights remain the cells' intrinsic
  answers — the one documented exception to §2.3, kept so a content-sized
  grid keeps its content-sized columns and rows.
- **Spacer**: `MainAxis`; it stretches on the enclosing stack's main axis
  and answers its minimum length (`spacer_min(n)`, 0 by default) to every
  measurement on that axis. It answers zero on other axes and under every
  other container, and claims nothing in a `ZStack`. During finite stack
  allocation its reported main extent is `max(minimum length, offer)`. A
  `Spacer` participates in §4.2's ordinary priority and flexibility
  ordering; its default priority is below ordinary content, so it receives
  the space remaining after that content's reported extents, including space
  declined under compression. Multiple `Spacer`s follow the same
  minimum-reserving allocation rule; there is no separate redistribution
  pass.
- **Divider**: the enclosing container's axis decides the line's
  orientation — `HStack` (fixed or lazy) establishes a horizontal axis and
  the divider draws 1 pt wide filling the cross; `VStack` establishes
  vertical and it draws 1 pt high; a container that establishes no axis
  (`ZStack`, `Grid`, `Overlay`, `Absolute`, `Padding`, `Frame`,
  `AspectRatio`, `Background`) leaves it a 1-pt-high horizontal rule. Its
  stretch is `CrossAxis` either way; the line is drawn in the `BorderColor`
  theme slot.
- **Lazy containers** (`List`, lazy stacks): virtualised along one axis with
  the same per-child protocol over the realised children; membership diffs by
  identity. Stretch is §3.

### 5.1 Response rectangles

Each container positions its response (§2.4) of extent `S`, its concretized
answer to the incoming proposal `P`, inside its bounds `B`. `E(child, P, B)`
is the child's own response query. Leading and trailing resolve to physical
sides first; overflow is not clamped.

- **HStack / VStack**: §4.1 and §4.2 — centred on the main axis, anchored by
  the alignment on the cross axis.
- **ZStack**: anchored by its alignment on both axes, custom guides at the
  start. A layer overflowing the envelope does not move it.
- **Overlay**: the base decides it: the origin of `E(base, P, B)`, at
  `S`. The two coincide whenever the base answers finitely. Decorations
  contribute nothing. The base is queried with `P`, though it is placed
  under the bounds-derived proposal.
- **Background**: `E(content, P, B)`. The background layer, its drawing and
  `Material` / `Glass` metadata contribute nothing, so adding a background
  never moves the response.
- **Padding**: with physical insets `l, t, r, b`,
  `C = E(child, P shrunk by the insets, B inset by the insets)`, and the
  response is `S` at `C.origin - (l, t)` — the child's response out-set by
  the edges, the leading-only inset belonging wholly to the leading side.
  Negative insets keep this origin and clamp only the extent; a collapsed
  zero extent keeps its calculated origin.
- **Frame**: `S` centred in `B`. The frame's alignment aligns its child
  inside the frame; it never positions the frame's own box, and a child
  overflowing the frame does not move it.
- **AspectRatio**: `S` centred in `B`. When `P` and `B` differ, this is not
  the ratio box `place` resolves against the bounds.
- **Absolute and positioning wrappers** (`position_in`, `pin`): a canvas —
  `S` at `B`'s origin, never the union of positioned children. A label on a
  canvas names the canvas; a label on the positioned child names the child.
- **Grid**: the grid block at `B`'s top-leading corner, where its rows are
  placed. The alignment aligns cells inside their cells and never moves the
  block. A `GridRow` owns no response.
- **Lazy containers**: the corresponding stack rule, from the same logical
  extent state their measurement uses, virtualisation estimates included —
  never the union of realised children. Changing realisation or an estimate
  invalidates it.
- **List rows**: a row allocates its content the whole content rect and
  positions the content's response at that rect's leading edge, centred
  vertically. The content keeps the whole rect as its allocation, so the
  row's hit target spans it; only where the response sits moves.
- **Layout-transparent wrappers** (metadata, alignment guides, bridge
  hosts): forward the query to the content with the same `B` and `P`.

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
  text field is one 22 pt line with no border, fill or vertical floor. A
  switch-style toggle with a visible label answers the proposal width
  (`Horizontal`) and its intrinsic height, placing the label at the leading
  edge and the switch at the trailing edge with the free space between; a
  checkbox-style toggle (box, then label) and a toggle with a hidden label
  answer the control's intrinsic size. The automatic style resolves to the
  platform's default style — switch on iOS and Android, checkbox on macOS —
  before this rule applies. A stepper with a visible label answers the
  proposal width (`Horizontal`) and its intrinsic height, placing the label at
  the leading edge and the buttons at the trailing edge with the free space
  between; a stepper with a hidden label answers its intrinsic size. A picker
  in the segmented style answers the proposal width (`Horizontal`) and its
  intrinsic height, dividing the width into equal segments; an unbounded
  width proposal answers the intrinsic width. Every other picker style
  answers the control's intrinsic size.
- **ScrollView**: on the scrolling axis a scroll claims the whole offer — a
  finite proposal is answered with the proposal, `None` with the content's
  intrinsic extent, `0` with `0`. On the non-scrolling axis it answers the
  content's own answer to the content proposal — that axis gets the scroll's
  offer, the scrolling axis `None` — clamped to a finite offer, so a scroll
  hugs non-filling content instead of claiming the slot; `0` and `None` keep
  their existing behaviour (the content's minimum and intrinsic extent).
  Content that itself fills the non-scrolling axis still fills it.
  `Axis::All` fills both axes. The content is measured with the viewport
  extent on the non-scrolling axis and `None` on the scrolling axis; its
  frame is its own answer on the non-scrolling axis, and it is **centred**
  there whether it is narrower or wider than the viewport (a wider content
  crosses both edges by the same amount). A scroll surface that touches
  window chrome extends under it (safe-area extension rule).
- **Spacer** hosted natively answers its minimum length on the stack's main
  axis and zero on the cross axis, whatever the proposal (§5).
- **GPU surfaces, shapes, colours**: `Both`.
- **Images**: a non-resizable image answers its intrinsic (natural) size to an
  unspecified proposal. Under a smaller finite proposal it scales down,
  preserving its aspect ratio, and never scales up. It claims no leftover space
  (`None`). `.resizable()` fills the proposal per `content_mode`.
- **Video**: a `Fit` video answers a finite width proposal — `0` included —
  with that width and the height its source's aspect ratio gives at it,
  whatever height is offered; until the source reports its size the ratio is
  16:9. To an unspecified width it answers its natural size, as an image does:
  the source's pixel dimensions taken as points, 320 × 180 until the source
  reports them. A maximum probe (`INFINITY`) is answered with an unbounded
  width at the natural height. It stretches horizontally (`Horizontal`).
  `Fill` and `Stretch` answer the proposal on both axes (`Both`), 320 × 180 on
  an unspecified axis.

A leaf that answers a probe with the proposal instead of its content on an
axis it does not stretch is a backend bug: it makes every container above it
report the proposal, and the failure surfaces as a stretched card or a
leading-parked column far from the leaf.

**The leaf rule.** Every leaf answers the response query, native leaves
included. A leaf's response is its concretized answer centred in its
allocation (`ResolvedExtent::centred`): a text that is the whole content of a
navigation page or a window root sits in the middle of that slot, at its own
size. A leaf with an explicit content-placement policy states it through the
same query, never through backend gravity. A backend draws a leaf's
intrinsic content in its response rectangle — glyphs, a control's chrome, an
image's pixels — so drawing and accessibility agree; a metadata or clip
bridge never gains permission to re-centre its child. `Spacer` and `Divider`
keep their measurement rules, so a `Spacer`'s response is its minimum length
even where a stack allocated it more. A `ScrollView`'s response is its own
answer under the leaf rule; its viewport remains its allocation and its
content extent a third, separate quantity.

## 7. Placement on the backends

A backend applies the Rust placement verbatim: the child's frame is the
placement rect and its layout pass receives the placement proposal. A bridge
wrapper standing in a child's slot (a metadata wrapper, a clip host) lays its
content over the whole slot; it never re-centres, re-measures or gravity-packs
the content inside the slot.

The root proposal is the window's (or scene's) content size, both axes
finite; the root is placed at its answer to that proposal, stretched to the
window on the axes it declares. Safe-area insets are a backend concern applied
outside the protocol (§7.1); they never change a
proposal a Rust container sees. Layout is single-threaded; a backend that
measures on another thread is outside the contract.

A backend drives a container through `place_layout` and retains, with each
committed node, its `PlacedGeometry`: the placement proposal, the raw answer,
the allocation and the allocation-local response rectangle. A node's
geometry becomes visible atomically with its placement. A layout-transparent
bridge wrapper forwards the response query with the same bounds and proposal
however many host views the backend builds for it.

### 7.1 Safe area

The safe area has two regions on each edge: *container* (system bars,
display cutouts, the home indicator, window chrome) and *keyboard* (the
software keyboard and other input-method surfaces). On each edge the area a
view is laid out in ends at the deepest region it does not ignore; a view
*touches* an edge when its laid-out frame, before scroll offsets and visual
transforms, ends on that boundary.

- **Layout avoids.** A view is laid out clear of every region it does not
  ignore. `IgnoreSafeArea` names the regions and edges a view ignores; a
  stack lays its children out inside what remains.
- **Paint extends for fills.** A background whose content is a fill (a solid
  color, a gradient or a material) extends past its frame to the window edge
  on every edge it touches, through both regions, without moving the content
  it backs. An `IgnoreSafeArea` on the fill replaces that default with the
  regions and edges it names. A background whose content is any other view,
  and a fill that is not a background, extends only where it ignores the
  safe area.
- **Scroll surfaces.** A scroll surface touching an edge extends under that
  edge's regions and insets its content by them; its content is not laid
  out against the safe area, so nothing inside it touches an edge. When a
  text field inside a scroll surface gains focus, or the keyboard inset
  grows while one is focused, the surface scrolls the minimum distance that
  brings the field's frame clear of the keyboard region and inside the
  surface's own frame.
- **Chrome.** A chrome container (a navigation, tool or tab bar host)
  extends each bar it draws under the regions of the edge the bar touches.
  Its content touches no edge where a bar sits; on the edges where it draws
  no bar, the content is laid out against the safe area like any other
  view. A bar stays docked to its edge: it is laid out clear of that edge's
  container region only, so the keyboard region covers it instead of
  lifting it, while the content the chrome container hosts is laid out
  clear of both regions. A navigation page is not the chrome container
  when its bar is drawn by the window or by the platform's navigation
  controller: the page touches the edge the bar sits on, with the bar's
  area as container region, so a scroll surface in it extends under the
  bar and insets its content by it, while non-scrolling content is laid
  out clear of the bar.
- **Keyboard motion.** A change of the keyboard inset follows the platform's
  keyboard animation frame by frame. A backend never avoids the keyboard by
  resizing the window or its rendering surface.

### 7.2 Accessibility geometry

An accessibility element has a *geometry owner* — the logical view whose
rectangle it reports — and an *action owner* — the object that performs its
actions. They may differ: in `button("OK").padding(8).a11y_label("x")` the
padded view owns the geometry and the button the actions, while
`button("OK").a11y_label("x").padding(8)` makes the labelled button the
geometry owner. Modifier order decides.

- An element's bounds are its geometry owner's response rectangle, unless a
  naming modifier names another source: the owner's allocation, or an
  explicit owner-local region. Semantic cardinality never chooses the
  geometry: an explicit element with no semantic children, a scope that
  collapses onto its one child, and a group that stands over several all
  report the scope owner's geometry. A collapse may keep the child's
  identity, actions, state and role; it never replaces the geometry owner.
- The rectangle is projected through the geometry owner's presentation
  transform, scroll ancestry, viewport and explicit clips, exactly once, at
  the platform boundary — never through the action owner's after a collapse.
  A clip decides exposure and touch exploration; it does not change the
  logical rectangle, and a smaller group rectangle is not an implicit clip on
  its descendants.
- A backend never shrinks or moves a platform view to make its accessibility
  frame come out right: it reports the projected rectangle through the
  platform's accessibility-frame API and leaves the Rust-assigned allocation
  in place. Accessibility activation dispatches the retained action; it does
  not assume the centre of the rectangle hits the control.

Hit testing uses the interaction owner's region and transform, a focus ring
the focus owner's shape — its response by default — and drawing the existing
placements. A naming modifier changes none of them.

## 8. Divergence register

Behaviour that is deliberately not SwiftUI's. Each entry is a decision, not a
gap; changing one is a contract change.

The response rules of §5.1 and §6 and the accessibility rules of §7.2 are
WaterUI's own decisions. They agree with the SwiftUI cases observed — a text
that is a navigation destination's whole content is centred in the page and
framed at its own size — but no executable twin establishes every rule, so
this document does not claim SwiftUI accessibility equivalence beyond those
cases.

| WaterUI | SwiftUI | why |
| --- | --- | --- |
| Default stack spacing is a fixed 10 pt. | Platform-dependent, content-dependent default. | One value across backends keeps parity tests meaningful. |
| Controls are intrinsic-only (`None`) unless they declare an axis. | Some controls stretch by style. | The style attribute, not the widget type, decides; backends declare per style. |
| Non-resizable images are `None` and scale down only, preserving their aspect ratio. | Images are fixed-size unless `.resizable()`. | Capped media fits smaller finite proposals without scaling up; an unspecified proposal receives the intrinsic size. |
| Grid columns at placement use the measurement proposal width. | `Grid` re-resolves columns against bounds. | Keeps a content-sized grid's columns content-sized under the bounds rule. |
| A finite-width Grid divides the proposal into equal columns and answers the proposal width. | `Grid` columns are intrinsic-sized; only flexible columns share the remainder. | Equal columns keep every backend's grid identical and the answer predictable. |
| `AspectRatio` reports the resolved ratio box and hands it to the child as its frame. | `aspectRatio` transforms the proposal but reports the child's answer. | The ratio box is the contract; the child receives it resolved, not re-negotiated. |
| `Background` places the secondary view over the whole bounds and proposes them. | The secondary view may keep a smaller rigid answer inside the base's size. | Whole-bounds allocation keeps a background flush with what it backs. |
| A window or navigation page root is proposed the window's content size and fills it. | On macOS, a page whose ideal width is below the window's is placed in a centred column of that width. | One root rule on every backend: content-sized pages are already centred by the scroll's own centring rule, and filling pages keep the whole window. |
| `overlay(alignment:)` anchors a decoration on the envelope its own guide forms inside the bounds. | A decoration's guides offset it against the base's alignment lines. | Anchoring the decoration's own envelope keeps a badge inside the bounds that a base-anchored offset would cross. |

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
returning to the original geometry. Since 0.6 they also cover: the response
extent equalling the concretized answer; translation covariance; each
container's response rule under asymmetric and negative padding, an
overflowing fixed frame, every alignment, right-to-left layout, and an
incoming proposal that differs from the bounds; zero-size and lazy
responses; and an underfilled stack's centred main-axis block. Response
geometry never changes an allocation or a selected proposal the earlier
assertions fix. A pull request that has to weaken one of these assertions is
changing the contract and is rejected.
