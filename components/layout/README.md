# WaterUI Layout Components

`waterui-layout` contains the building blocks that arrange WaterUI views. The
crate embraces a simple two-pass layout protocol shared by all backends: first
children are asked how large they would like to be (the *proposal* pass), then
the parent chooses a final rectangle for each child (the *placement* pass).

- [`Layout`](src/core.rs) is the trait every layout implements. The helpers
  [`ProposalSize`], [`Rect`], [`Size`], and [`ChildMetadata`] live alongside it
  in `core`.
- High-level views such as stacks, scroll containers, padding, and spacers are
  built on top of this trait and exported through `waterui_layout`.
- Backends are responsible for interpreting the layout instructions (for
  instance scroll views bridge into native widgets).

The sections below describe the provided components and how to extend them.

## Getting Started

The crate follows the same ergonomics as other WaterUI components. Import the
helpers you need and compose them inside your view:

```rust,ignore
use waterui_layout::stack::{self, hstack, vstack, Alignment, HorizontalAlignment};
use waterui_layout::spacer::spacer;
use waterui_text::text;

pub fn toolbar() -> impl waterui::View {
    hstack((
        text("WaterUI"),
        spacer(),
        stack::HStack::new(
            HorizontalAlignment::Trailing,
            8.0,
            (
                text("Docs"),
                text("Blog"),
                text("GitHub"),
            ),
        ),
    ))
}
```

## Layout Primitives

- **`Layout` trait** – receives [`ProposalSize`] hints from the parent, produces
  proposed sizes for each child, and finally returns a placement [`Rect`] for
  them. Implement this when building bespoke containers.
- **`ProposalSize`** – optional width/height hints (`None` means unconstrained,
  `Some(f64::INFINITY)` signals an infinite axis).
- **`ChildMetadata`** – backend-supplied data describing how each child would
  like to be sized. The `stretch` flag indicates whether a child is willing to
  expand beyond its intrinsic size.

Whenever you need to surface a custom layout, wrap the implementation in a
[`Container`](src/container.rs) so it becomes a `View`.

## Container

`Container::new(layout, contents)` stores a boxed layout object alongside a set
of child views. Renderer backends downcast to the layout implementation and call
its `Layout` methods during layout. The [`container!`](src/container.rs) macro
creates a concrete view type with an inlined layout struct.

```rust,ignore
use waterui_layout::{container::Container, Layout};

struct ConstrainedSquare;

impl Layout for ConstrainedSquare {
    fn propose(&mut self, parent: ProposalSize, _children: &[ChildMetadata]) -> Vec<ProposalSize> {
        vec![ProposalSize::new(parent.width, parent.width)]
    }

    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        let edge = children
            .first()
            .and_then(|child| child.proposal_width())
            .unwrap_or(0.0);
        let max = parent.width.unwrap_or(edge);
        Size::new(max, max)
    }

    fn place(&mut self, bound: Rect, _proposal: ProposalSize, _children: &[ChildMetadata]) -> Vec<Rect> {
        let edge = bound.width().min(bound.height());
        vec![Rect::new(bound.origin(), Size::new(edge, edge))]
    }
}

pub fn square(content: impl waterui::View) -> Container {
    Container::new(ConstrainedSquare, (content,))
}
```

## Stacks

Stacks are the workhorse layout primitives:

- **`VStack`** – arranges children vertically and respects the `stretch` flag to
  distribute leftover height. Construct with
  `VStack::new(HorizontalAlignment::Center, spacing, contents)` or the
  convenience `vstack((..))` which defaults to centered alignment and 10.0px
  spacing.
- **`HStack`** – identical to `VStack` but laid out horizontally, choosing a
  vertical alignment for children.
- **`ZStack`** – overlays children on top of each other according to an
  `Alignment`. Each child receives the parent’s proposal and can draw anywhere
  inside the final bounds (similar to SwiftUI’s ZStack).

Stretching is backend-driven. Any child marked as `stretch` receives an equal
share of the remaining space after intrinsic sizes and spacing are deducted.

## Spacer

`spacer()` produces a zero-minimum [`Spacer`] view that greedily consumes any
available space inside stacks. Use `spacer_min(length)` to enforce a minimum.
Spacers have no children and simply return their desired size to the renderer.

```rust,ignore
hstack((
    text("Leading"),
    spacer(),
    text("Trailing"),
));
```

## Padding

`Padding` adds insets around a child view. The layout shrinks the child’s
proposal by the requested edge insets, then re-expands the reported size when
placing the child.

- `Padding::new(edges, view)` constructs the view manually.
- The `ViewExt::padding()` extension (defined in `waterui-core`) applies the
  default inset (`EdgeInsets::all(10.0)`).
- Use the helpers on [`EdgeInsets`] (`new`, `all`, `symmetric`) to customise the
  values.

```rust,ignore
use waterui_core::ViewExt;
use waterui_layout::padding::EdgeInsets;

let compact = text("Tight").padding_with(EdgeInsets::symmetric(4.0, 8.0));
```

## Scroll Views

`ScrollView` exposes platform scrolling containers. They wrap a single child and
delegate scrolling behaviour to the active backend:

- `scroll(view)` – vertical scrolling (default).
- `scroll_horizontal(view)` – horizontal scrolling.
- `scroll_both(view)` – two-dimensional scrolling.

Backends interpret the `Axis` enum to configure the underlying widget.

## Advanced Topics

- **Custom layouts** – implement `Layout`, then wrap it in a `Container` or use
  the `container!` macro to declare a concrete view type.
- **Metadata** – `ChildMetadata::priority()` is reserved for future layout
  scheduling; current components rely mainly on the `stretch` flag and intrinsic
  proposals.
- **Bridging to FFI** – some features (such as scrolling) require renderer
  support. Pure Rust tests can still exercise layout math by instantiating the
  layouts directly.

## Roadmap & Status

Several modules exist as placeholders for upcoming work:

- `frame` – targeted for explicit width/height modifiers.
- `grid`, `overlay` – currently empty shells to be fleshed out with dedicated
  layouts.

Contributions are welcome. See [`ROADMAP.md`](../../ROADMAP.md) for the broader
WaterUI plan.

[`ProposalSize`]: src/core.rs
[`Rect`]: src/core.rs
[`Size`]: src/core.rs
[`ChildMetadata`]: src/core.rs
[`Container`]: src/container.rs
[`container!`]: src/container.rs
[`Spacer`]: src/spacer.rs
[`EdgeInsets`]: src/padding.rs
[`ScrollView`]: src/scroll.rs
