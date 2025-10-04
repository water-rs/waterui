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
}
```

### ZStack - Overlay Arrangement

```rust,ignore
[WIP] - introduce the difference of zstack and overlay
```

## Grid Layout

```rust,ignore
use waterui::View;
use waterui_layout::grid::Grid;
use waterui_layout::{row, Alignment};

fn photo_grid() -> impl View {
    Grid::center([
        row((photo("1.jpg"), photo("2.jpg"), photo("3.jpg"))),
        row((photo("4.jpg"), photo("5.jpg"), photo("6.jpg"))),
        row((photo("7.jpg"), photo("8.jpg"), photo("9.jpg"))),
    ])
}
```

## Scrolling

```rust,ignore
[WIP]
```

## Sizing and Constraints

```rust,ignore
[WIP]
```


# Advanced Layout: Layout trait

```rust
pub trait Layout {
	fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize>;
	fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size;
	fn place(&mut self, bound: Rect, proposal: ProposalSize, children: &[ChildMetadata]) -> Vec<Rect>;
}
```


[WIP]