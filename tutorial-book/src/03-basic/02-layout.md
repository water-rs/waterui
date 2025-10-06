# Layout Components

Layouts determine how views measure themselves and where they end up on screen. WaterUI follows
a two-stage process similar to SwiftUI and Flutter: first the framework proposes sizes to each
child, then it places those children inside the final bounds returned by the renderer. This
chapter documents the high-level containers you will reach for most often and explains how they
map to the lower-level layout primitives exposed in `waterui_layout`.

## How the Layout Pass Works

1. **Proposal** – A parent view calls `Layout::propose` on its children with the size it is
   willing to offer. Children can accept the full proposal, clamp it, or ignore it entirely.
2. **Measurement** – Each child reports back an intrinsic `Size` based on the proposal. Stacks,
   grids, and other composite containers aggregate those answers to determine their own size.
3. **Placement** – The container receives a rectangle (`Rect`) that represents the concrete space
   granted by the renderer. It positions every child within that rectangle via `Layout::place`.

Understanding these stages helps you reason about why a view grows or shrinks, and which modifier
(`padding`, `alignment`, `Frame`) to reach for when the default behaviour does not match your
expectation.

## Stack Layouts

Stacks are the bread and butter of WaterUI. They arrange children linearly or on top of each other
and are zero-cost abstractions once the layout pass completes.

### Vertical Stacks (`vstack` / `VStack`)

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::stack::vstack;
use waterui::reactive::binding;

pub fn profile_card() -> impl View {
    let name = binding("Ada Lovelace");
    let followers = binding(128_000);

    vstack((
        text!("{name}"),
        text!("Followers: {followers}"),
    ))
    .spacing(12.0)               // Vertical gap between rows
    .alignment(HorizontalAlignment::Leading)
    .padding()
}
```

Key points:

- Children are measured with the parent’s width proposal and natural height.
- `.spacing(distance)` sets the inter-row gap. `.alignment(...)` controls horizontal alignment,
  using `Leading`, `Center`, or `Trailing`.
- To contribute flexible space within a stack, insert a `spacer()` (discussed later).

### Horizontal Stacks (`hstack` / `HStack`)

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::{spacer, stack::hstack};

pub fn toolbar() -> impl View {
    hstack((
        text("WaterUI"),
        spacer(),
        button("Docs"),
        button("Blog"),
    ))
    .spacing(16.0)
    .alignment(VerticalAlignment::Center)
    .padding_with(EdgeInsets::symmetric(8.0, 16.0))
}
```

Horizontal stacks mirror vertical stacks but swap the axes: alignment describes vertical
behaviour, spacing applies horizontally, and spacers expand along the x-axis.

### Overlay Stacks (`zstack` / `ZStack`)

`zstack` draws every child in the same rectangle. It is perfect for badges, overlays, and
background effects.

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::padding::EdgeInsets;
use waterui::component::layout::stack::zstack;
use waterui::components::media::Photo;

pub fn photo_with_badge() -> impl View {
    zstack((
        Photo::new("https://example.com/cover.jpg"),
        text("LIVE")
            .padding_with(EdgeInsets::symmetric(4.0, 8.0))
            .background(waterui::background::Background::color((0.9, 0.1, 0.1).into()))
            .alignment(Alignment::TopLeading)
            .padding_with(EdgeInsets::new(8.0, 0.0, 0.0, 0.0)),
    ))
    .alignment(Alignment::Center)
}
```

Overlay stacks honour their `Alignment` setting (`Center` by default) when positioning children.
Combined with padding you can fine-tune overlay offsets without writing custom layout code.

## Spacers and Flexible Space

`spacer()` expands to consume all remaining room along the stack’s main axis. It behaves like
SwiftUI’s spacer or Flutter’s `Expanded` with a default flex of 1.

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::{spacer, stack::hstack};

pub fn pagination_controls() -> impl View {
    hstack((
        button("Previous"),
        spacer(),
        text("Page 3 of 10"),
        spacer(),
        button("Next"),
    ))
}
```

Need a spacer that never shrinks below a certain size? Use `spacer_min(120.0)` to guarantee the
minimum gap.

## Padding and Insets

Any view gains padding via `ViewExt::padding()` or `padding_with(EdgeInsets)`.

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::padding::EdgeInsets;

fn message_bubble(text: impl Into<Str>) -> impl View {
    text(text)
        .padding_with(EdgeInsets::symmetric(8.0, 12.0))
        .background(waterui::background::Background::color((0.18, 0.2, 0.25).into()))
        .alignment(Alignment::Leading)
}
```

EdgeInsets helpers:
- `EdgeInsets::all(value)` – identical padding on every edge.
- `EdgeInsets::symmetric(vertical, horizontal)` – separate vertical and horizontal padding.
- `EdgeInsets::new(top, bottom, leading, trailing)` – full control per edge.

## Scroll Views

WaterUI exposes scroll containers that delegate behaviour to the active renderer. Use them when
content might overflow the viewport:

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::scroll::{scroll, scroll_horizontal, scroll_both};

pub fn article(body: impl View) -> impl View {
    scroll(body.padding())
}
```

- `scroll(content)` – vertical scrolling (typical for lists, articles).
- `scroll_horizontal(content)` – horizontal carousels.
- `scroll_both(content)` – panning in both axes for large canvases or diagrams.

Remember that actual scroll physics depend on the backend (SwiftUI, GTK4, Web, …). Keep your
content pure; avoid embedding interactive gestures that require platform-specific hooks until the
widget surfaces them.

## Grid Layouts

The `grid` API arranges rows and columns with consistent spacing. Every row is a `GridRow`, and the
container needs the number of columns up front.

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::grid::{grid, row};

pub fn emoji_palette() -> impl View {
    grid(4, [
        row(("😀", "😁", "😂", "🤣")),
        row(("😇", "🥰", "😍", "🤩")),
        row(("🤔", "🤨", "🧐", "😎")),
    ])
    .spacing(12.0)                             // Uniform horizontal + vertical spacing
    .alignment(Alignment::Center)              // Align cells inside their slots
    .padding()
}
```

Notes:
- Grids require a concrete width proposal. On desktop, wrap them in a parent that constrains width
  (e.g. `.frame().max_width(...)`) when needed.
- Each row may contain fewer elements than the declared column count; the layout simply leaves the
  trailing cells empty.
- Use `Alignment::Leading` / `Trailing` / `Top` / `Bottom` to align items inside each grid cell.

## Frames and Explicit Sizing

WaterUI’s `Frame` view pins a child to explicit size preferences. `view.frame(width, height)` is a
common SwiftUI pattern; in WaterUI you construct an explicit frame via `ViewExt::alignment` and the
methods on `Frame`:

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::frame::Frame;
use waterui::component::layout::stack::vstack;

fn gallery_thumbnail(content: impl View) -> impl View {
    Frame::new(content)
        .width(160.0)
        .height(120.0)
        .alignment(Alignment::Center)
}
```

Frames are most helpful when mixing flexible and fixed-size widgets (for example, pinning an avatar
while the surrounding text wraps naturally). Combine frames with stacks, grids, and padding to
create predictable compositions.

## Layout Troubleshooting Checklist

- **Unexpected stretching** – Make sure there isn’t an extra `spacer()` or a child returning an
  infinite proposal. Wrapping the content in `.padding_with(EdgeInsets::all(0.0))` can help
  visualise what area the view thinks it owns.
- **Grid clipping** – Provide a finite width (wrap in a parent frame) and watch for rows with taller
  content than their neighbours.
- **Overlapping overlays** – `zstack` honours alignment. Apply additional `.padding_with` or wrap
  the child in a `Frame` to fine-tune positions.
- **Platform differences** – Remember that scroll behaviour is delegated to backends. Test on each
  target platform when tweaking scrollable layouts.

## Where to Go Next

Explore the advanced layout chapter for details on implementing custom `Layout` types, or scan the
`waterui_layout` crate for lower-level primitives like `Container` and `ProposalSize`. Armed with
stacks, spacers, padding, grids, and frames you can replicate the majority of everyday UI
structures in a clear, declarative style.
