# WaterUI Layout Components

Primitives for arranging views: stacks, grid, scroll, spacer, and simple frame metadata.

## Stack Layouts

```rust,ignore
use waterui_layout::stack::{vstack, hstack, zstack, VStack, HStack, ZStack};
use waterui_text::text;

// Vertical stack
let v: VStack = vstack((
    text("First"),
    text("Second"),
    text("Third"),
));

// Horizontal stack
let h: HStack = hstack((
    text("Left"),
    text("Right"),
));

// Layered stack
let z: ZStack = zstack((
    text("Back"),
    text("Front"),
));
```

## Grid (rows × columns)

```rust,ignore
use waterui_layout::{Alignment};
use waterui_layout::grid::{Grid, row};
use waterui_text::text;

let grid = Grid::new(
    Alignment::Center,
    [
        row((text("A"), text("B"), text("C"))),
        row((text("D"), text("E"))),
    ],
);
```

## Scroll View

```rust,ignore
use waterui_layout::scroll;
use waterui_text::text;

let scrollable = scroll((
    text("Line 1"),
    text("Line 2"),
    text("Line 3"),
));
```

## Spacer

```rust,ignore
use waterui_layout::spacer::spacer;

let gap = spacer();
```

## Frame and Padding

```rust,ignore
use waterui::ViewExt;           // for .frame() and .padding()
use waterui_layout::{Frame, Alignment};
use waterui_text::text;

let framed = text("Sized").frame(
    Frame::new().width(200.0).height(100.0).alignment(Alignment::Center)
);

// Padding attaches default margins around a view (backend-defined defaults)
let padded = text("With padding").padding();
```

Notes:
- `Alignment` variants: `Default`, `Leading`, `Center`, `Trailing`.
- `padding()` currently applies default inset values; custom padding values are not exposed yet.
