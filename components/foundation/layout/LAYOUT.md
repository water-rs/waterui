# Layout contract

This document defines WaterUI's shared layout protocol. Changes to the protocol and its conformance coverage are tracked in issue #959. Backend implementations own native measurement and placement; they do not define separate stack allocation rules.

## Units and ownership

All Rust geometry uses logical points. A backend converts between logical points and its native coordinate system at the boundary. Pixel rounding belongs to that conversion, not to the shared allocation algorithm. Font and native control metrics may differ across platforms; identical deterministic leaf measurements must produce identical shared geometry.

A container owns negotiation, ordering and alignment. A `SubView` supplies dimensions, explicit alignment guides, current stretch metadata and priority. A backend applies the resulting child frame and recursively places the child with its selected proposal.

## Proposals and responses

`ProposalSize` carries independent width and height offers:

| Axis input | Meaning |
| --- | --- |
| `None` | Query ideal extent for this axis. |
| `Some(0.0)` | Query minimum extent. |
| `Some(v)`, finite and positive | Offer `v` logical points. |
| `Some(f32::INFINITY)` | Query maximum extent; an unbounded response is permitted. |

A proposal is a suggestion. A rigid child may return more than a finite offer. Overflow does not imply clipping, and an assigned host region does not authorize compressing a child below its reported minimum. Clipping requires the relevant explicit clipping or viewport behavior.

For otherwise unchanged inputs, the minimum, ideal and maximum answers on an axis satisfy `minimum <= ideal <= maximum`. Finite responses are nonnegative. Only maximum queries can yield unbounded extents. Mixed-axis queries retain the other axis: measuring minimum width under a finite height differs from measuring minimum width under an ideal height. Negative or NaN values are not logical proposal extents.

The native ABI encodes `None` as NaN. Decoding must distinguish NaN from infinity; converting every nonfinite value to unspecified loses the maximum query. The bit-level ABI tests also cover signed zero and NaN payloads.

## Measurement and selected placement

The protocol is:

```rust
pub trait SubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;
    fn stretch_axis(&self) -> StretchAxis;
    fn priority(&self) -> i32;
}

pub struct SubviewPlacement {
    pub frame: Rect,
    pub proposal: ProposalSize,
}

pub trait Layout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size;
    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement>;
}
```

These excerpts omit the traits' additional bounds and guide/invalidation methods; the definitions in `waterui-core` are authoritative.

A parent may probe a child multiple times before selecting an offer. `place` receives that selected input, irrespective of the most recent probe. Each returned packet carries the offer selected for the corresponding child, not an offer reconstructed from its frame. Placements have the same count and order as `children`; their coordinates include the supplied bounds origin. Native parents establish the child's local coordinate system while retaining the packet's logical proposal.

Equal bounds do not imply equal placement. For two bounded flexible leaves with minima 20/60, ideals 40/120 and unbounded maxima, an ideal main-axis query produces 40/120, while a finite offer of 160 produces 80/80. Both containers measure 160. Rigid sections with minima 40/120 retain those minima under offers of 80 or 160. These independently captured examples are exercised on both axes in `src/tests/contract.rs`.

Transparent metadata and effects preserve the incoming proposal. A padding layout transforms it by its insets. A frame measures ideal queries without inventing a finite offer, but during placement explicitly offers its resolved region on each constrained axis; unconstrained axes preserve the incoming proposal. Thus a fixed 28-by-18 frame gives its content 28-by-18 even when its parent queried the frame’s ideal size. A background or overlay preserves its authoritative content's offer while offering the resolved content region to decoration. A scrolling container offers unspecified extent on each scrolling axis and forwards that same offer during recursive placement. Window roots and native widget-owned content regions explicitly create their bounded offers.

`PlacedSubview` resolves dimensions and alignment guides using the selected proposal. A guide belongs to the measured response; measuring it under a reconstructed frame can change wrapping and therefore the guide. Maximum responses with an infinite axis are not placed and do not resolve finite placement guides.

## Stack negotiation

Horizontal and vertical stacks use the same main-axis allocation implementation. Main/cross axes are projections of the same protocol:

- Unspecified main-axis offers retain ideal child measurements.
- Zero and infinite main-axis queries forward that query to the children.
- A finite offer accounts for spacing, measures the applicable child minimums and maximums and allocates through the shared priority pool. Finite maximums cap growth; ideal size does not cap a flexible child.
- Higher numeric priority resists compression longer; ordinary content defaults to zero and `Spacer` defaults to `Spacer::DEFAULT_LAYOUT_PRIORITY`, the lowest integer priority. An explicit priority overrides that default.
- Every finite allocation is remeasured, including when ideal extents fit, and retains the exact child proposal. Width-dependent height and explicit guides therefore correspond to the allocated width.
- If the minimum extents and spacing exceed the offer, the container reports the required extent. Fixed dimensions are not rewritten to fit the offer.
- Content-sized stacks derive cross-axis size from child responses and alignment. A rigid cross-axis response survives a smaller host region.

Stretch metadata expresses participation in filling, not permission to erase minimums. `MainAxis` and `CrossAxis` are resolved in the surrounding stack orientation. Containers query their current children's metadata; they must not freeze it when the tree is first built. Background/overlay decoration does not make otherwise content-sized authoritative content greedy. Frame constraints and explicit expanding content can change a container's stretch response.

## Invalidation and cache lifetime

Repeated measurement under unchanged inputs is deterministic. Reordering unrelated probes must not change selected placement. Rendering must not depend on the last measurement call.

Measurement caches belong to child proxies and are valid only while their input state remains valid. `MemoizedSubView` provides call-scoped memoization. A retained backend may retain a child cache only with an invalidation lifetime covering content, proposal, font/environment metrics, layout signals and child membership. `Layout` has no persistent negotiation cache.

`watch_invalidation` subscribes to the precise reactive fields that affect a layout, such as spacing and frame constraints. Backend containers retain those subscriptions for the layout lifetime and schedule a new measurement/placement when they fire. Changes to priority, stretch metadata, native metrics or child membership also invalidate affected ancestors. Relayout preserves the existing semantic tree and state; it does not require reconstructing a view body or introducing renderer-local state slots.

An A/B/A resize sequence must return to the original geometry when all other inputs return to their original values. The same applies after adding and removing a child, changing and restoring spacing, or replacing a dynamic child's metrics.

## Native hosting and safe areas

Native hosts own safe-area and keyboard insets. They calculate the content region, account for scoped `IgnoreSafeArea` metadata, and issue the resulting proposal at the host boundary. Shared layouts operate on the supplied logical coordinates. Insets are not silently applied again at every descendant.

Native measurement adapters preserve legal responses and proposal categories. A platform-specific measure mode cannot collapse ideal and maximum queries or force a rigid response below its minimum. Native allocation may use an exact frame for the platform object, but recursive WaterUI layout still consumes the original selected proposal.

## Conformance evidence

| Layer | Required observation | Owner |
| --- | --- | --- |
| Proposal ABI | Mixed-axis ideal/minimum/maximum/finite transport; dimensions, guides, priority, stretch and order | `ffi/src/components/layouting/layout.rs` tests |
| Shared negotiation | Independent expected sizes and positions; rigid overflow; both axes; priority; repeated/reordered probes | Layout unit tests and `src/tests/contract.rs` |
| Composition and time | Nested constraints, decoration, nonzero origins, reactive changes, membership changes and A/B/A resize | Layout composition tests |
| Backend transport | Same bounds with different selected offers; wrappers; scroll axes; native priority; dynamic relayout | Each backend repository |
| User-visible layout | Real native/rendering paths with semantic bounds and visual inspection | Each backend's platform suite |

Expected geometry is derived independently of the implementation. Updating an expected value requires a demonstrated contract error in that expectation. Pixel-exact comparison across native fonts or GPU adapters is not a cross-platform geometry oracle. A passing shared unit test does not certify a backend, and a backend screenshot does not establish the shared allocation formula.
