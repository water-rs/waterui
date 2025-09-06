# Layout Components

WaterUI provides powerful layout components for arranging UI elements. This chapter covers the essential layout tools.

## Stack Layouts

### VStack - Vertical Arrangement

```rust,ignore
use waterui::{View, ViewExt};
use waterui::component::layout::stack::vstack;
use waterui::component::layout::{Edge, Frame};

fn vertical_layout() -> impl View {
    vstack((
        "First item",
        "Second item",
        "Third item",
    ))
    .frame(Frame::new().margin(Edge::round(20.0)))
}
```

### HStack - Horizontal Arrangement

```rust,ignore
use waterui::{View, ViewExt};
use waterui::component::layout::stack::hstack;
use waterui::component::layout::spacer::spacer;
use waterui::component::button::button;
use waterui::component::layout::{Edge, Frame};

fn navigation_bar() -> impl View {
    hstack((
        button("← Back"),
        spacer(),  // Pushes items apart
        "Title",
        spacer(),
        button("Menu"),
    ))
    .frame(Frame::new().margin(Edge::round(15.0)))
}
```

### ZStack - Overlay Arrangement

```rust,ignore
// Overlay examples depend on your backend renderer; use zstack to layer views.
```

## Grid Layout

```rust,ignore
use waterui::View;
use waterui_layout::grid::Grid;
use waterui_layout::{row, Alignment};

fn photo_grid() -> impl View {
    Grid::new(
        Alignment::Center,
        [
            row((photo("1.jpg"), photo("2.jpg"), photo("3.jpg"))),
            row((photo("4.jpg"), photo("5.jpg"), photo("6.jpg"))),
            row((photo("7.jpg"), photo("8.jpg"), photo("9.jpg"))),
        ],
    )
}
```

## Scrolling

```rust,ignore
// Scrolling helpers exist; see waterui_layout::scroll for details.
```

## Sizing and Constraints

```rust,ignore
// Use Frame to control size constraints:
// view.frame(Frame::new().width(250.0).max_width(300.0))
```

Next: [Text and Typography](03-text.md)
