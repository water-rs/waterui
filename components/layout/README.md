# WaterUI Layout

`waterui-layout` is the composable layout engine that powers every WaterUI view tree.  
It translates the declarative `View` hierarchy into concrete sizes and positions, and it
ships the most commonly used layout primitives (stacks, grids, spacers, overlays, etc.)
out of the box. This document is a complete guide to the crate: how the protocol works,
how the supplied components behave, and how to extend the system with your own layouts.

---

## Quick Start

```rust,ignore
use waterui_core::ViewExt;
use waterui_layout::{overlay, stack, spacer};
use waterui_text::text;

pub fn notification() -> impl waterui_core::View {
    overlay(
        stack::hstack((
            text("Inbox"),
            spacer(),
            text("12").padding(),
        )),
        text("•").padding(), // red badge layered on top
    )
}
```

Every layout view returns something that implements `waterui_core::View`. The renderer
asks the layout objects to size and position children during the render pass using the
protocol described below.

---

## Architectural Overview

### The Two-Pass Layout Protocol

All layout containers implement the `Layout` trait defined in `core.rs`. The trait
encodes a deterministic three-step algorithm (proposal, sizing, placement) that runs
top-to-bottom, then bottom-to-top, then top-to-bottom again:

1. **`propose`** *(top → bottom)*  
   Parents send a `ProposalSize` to each child describing how much space is available.
   The child returns a new proposal for its own children.

2. **`size`** *(bottom → top)*  
   After children respond, the layout calculates its intrinsic `Size` using the reported
   `ChildMetadata` (stretch flags, measured width/height, etc.) and bubbles the value up.

3. **`place`** *(top → bottom)*  
   With the final bounds known, parents call `place` on each layout to receive an array
   of `Rect`s describing where every child should be rendered.

```rust,ignore
pub trait Layout: Debug {
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize>;
    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size;
    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect>;
}
```

### Core Data Structures

- `ProposalSize`: soft constraints (`Option<f32>`) for width/height (`None` = unconstrained).
- `Size`: a concrete width and height in pixels.
- `Point` / `Rect`: absolute positions relative to the parent layout.
- `ChildMetadata`: measurement info returned by renderers (proposal echo, `stretch`, priority).

### Wrapping Layouts in Views

Layouts are imperative objects; they become declarative views through two wrappers:

- `FixedContainer`: takes a layout and a tuple of child views. Suitable for static trees.
- `Container`: similar, but stores children in a reconstructable `AnyViews`, enabling lazy
  structures (`ForEach`, diffing lists, etc.).

You rarely construct these directly. High-level helpers (`stack::vstack`, `overlay`, `scroll`,
etc.) call into them for you.

---

## Built-In Layout Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `stack::vstack` | `stack/vstack.rs` | Vertical flow with alignment and stretch aware spacing. |
| `stack::hstack` | `stack/hstack.rs` | Horizontal flow with alignment and stretch semantics. |
| `stack::zstack` | `stack/zstack.rs` | Overlays multiple layers; expands to the largest child. |
| `overlay` | `overlay.rs` | Two-layer overlay that locks size to the base child. |
| `Spacer` / `spacer()` | `spacer.rs` | Flexible spacing element used inside stacks. |
| `padding::Padding` | `padding.rs` | Insets a child by configurable edge insets. |
| `frame::Frame` | `frame.rs` | Prototype for size-clamping single-child containers. |
| `grid` | `grid.rs` | Two-dimensional arrangements with automatic column sizing. |
| `scroll` | `scroll.rs` | Signals backends to wrap content in a scrollable surface. |

### Stacks

Stacks are the workhorses of WaterUI layouts.

- **`HStack`**: lays out children left-to-right, honours `VerticalAlignment`, distributes
  remaining width across `stretch` children (e.g. `Spacer`).
- **`VStack`**: lays out children top-to-bottom, honours `HorizontalAlignment`, distributes
  remaining height across stretch children.
- **`ZStack`**: overlays an arbitrary number of children. Every child receives the same
  proposal, and the final size is the maximum width/height reported by the children. Use
  when each layer should influence the container size.

### Overlay vs. ZStack

`overlay(base, layer)` is a convenience view for the most common overlay scenario:
decorating a single base view with a badge, border highlight, or similar adornment. The
`OverlayLayout` always reports the base child's measured size and simply positions the
layer according to the chosen `Alignment`. This prevents the overlay from inflating
nearby layouts (unlike `ZStack`, where a larger layer would expand the stack).

```rust,ignore
use waterui_layout::{overlay, stack};
use waterui_text::text;

let avatar = overlay(
    stack::zstack((text("👤"),)),        // base
    text("●").alignment(Alignment::BottomTrailing),
);
```

### Spacer

`Spacer` (and the helper `spacer()`) is a zero-sized view that declares `stretch = true`.
Stacks detect this flag and share the remaining space among all stretchable children.
`spacer_min` allows you to enforce a minimum length.

### Padding

`Padding` wraps a view and applies symmetric or per-edge insets. The incoming proposal is
shrunken before passing to the child; the reported size is inflated afterwards so parent
layouts continue to see the padded dimensions.

### Grid

`grid(columns, rows)` arranges content in a table-like structure. It requires a finite
width proposal to compute column widths. Row heights are determined by the tallest child
in the row. `grid` pairs with the `row` helper to keep call sites readable.

### ScrollView

`ScrollView` is a signalling view; it does not perform layout itself. Backends detect it
and mount the appropriate platform scroll container. Use `scroll`, `scroll_horizontal`, or
`scroll_both` constructors depending on the desired axis.

---

## Alignment and Stretch Semantics

- **`Alignment`** represents combined horizontal and vertical preferences (`TopTrailing`,
  `Center`, etc.) used by overlay-type containers.
- **`HorizontalAlignment` / `VerticalAlignment`** drive how stacks place children inside
  their cross axis.
- **Stretching**: backends mark views as `stretch` when they are happy to consume extra
  space. `Spacer` does so explicitly; other views (like text) typically do not. Stack
  layouts read the flag and distribute leftover space proportionally.

---

## Building Custom Layouts

1. Implement `Layout` using the three methods; store any state you need for subsequent
   passes on `self`.
2. Wrap the layout in `FixedContainer::new(layout, tuple_of_children)` or `Container::new`
   to expose it as a `View`.
3. Prefer returning `impl View` constructors so callers stay in the declarative DSL.

Guidelines:

- Use `ProposalSize::new` helpers to pass down constraints; never mutate the provided
  `ChildMetadata`.
- Keep floating-point math saturating at `0.0` to avoid negative sizes.
- Respect parent hints: clamp your chosen width/height to the parent's `Some(value)`.
- Provide documentation describing measurement semantics (does the layout grow to the
  largest child? does it force all children to match the base?).

The earlier `SquareLayout` example shows the pattern end-to-end.

---

## Module Index

- `core`: definitions of `Layout`, `Size`, `Point`, `Rect`, `ProposalSize`, `ChildMetadata`.
- `container`: `Container` and `FixedContainer` view wrappers that host layout objects.
- `stack`: `HStack`, `VStack`, `ZStack`, alignment enums.
- `overlay`: `OverlayLayout`, `Overlay` view, and the `overlay` constructor.
- `spacer`: `Spacer` view and helpers.
- `padding`: `Padding` view plus edge inset utilities.
- `grid`: grid layout primitives (`grid`, `row`, internal measurement helpers).
- `frame`: planned single-child clamp layout (currently a documented placeholder).
- `scroll`: signalling helpers for scrollable regions.

---

## Integration Tips

- **Prefer composition**: combine `padding`, `overlay`, and stacks instead of building new
  layouts whenever possible.
- **Observe stretch flags**: when authoring complex containers, consider exposing knobs
  (e.g. toggles) that allow callers to mark children as stretchable.
- **Interop with renderers**: custom layouts should avoid allocations in hot paths. Reuse
  scratch buffers on the layout struct when possible.
- **Testing**: unit tests can instantiate layouts directly and exercise `propose/size/place`
  with synthetic `ChildMetadata` to validate geometry logic.

---

## Future Work

- Promote `Frame` from a placeholder to a fully supported view with width/height clamps.
- Layer additional overlay utilities (tooltips, popovers, sheets) on top of `OverlayLayout`.
- Investigate constraint-solving helpers for advanced responsive layouts.
- Improve ergonomics around debug visualisation of layout bounds during development.

Contributions are welcome—see `ROADMAP.md` for larger project goals and open areas.
