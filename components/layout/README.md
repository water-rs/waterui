# WaterUI Layout Components

Layout components and utilities for arranging and positioning views in the WaterUI framework.

## Overview

`waterui-layout` provides comprehensive layout capabilities including stacks, grids, overlays, scrolling, spacing, and flexible frame-based sizing. These components handle the arrangement and positioning of UI elements.

## Components

### Stack Layouts

Linear arrangement of views:

```rust
use waterui_layout::stack::{VStack, HStack, ZStack};

// Vertical stack
let vstack = VStack::new([
    text("First item"),
    text("Second item"),
    text("Third item"),
])
.spacing(10.0)
.alignment(Alignment::Leading);

// Horizontal stack  
let hstack = HStack::new([
    button("Cancel"),
    spacer(),
    button("OK"),
])
.spacing(8.0);

// Depth stack (layered)
let zstack = ZStack::new([
    background_image(),
    overlay_content(),
])
.alignment(Alignment::Center);
```

### Grid Layout

Two-dimensional arrangements:

```rust
use waterui_layout::grid::{Grid, GridItem, row};

let grid = Grid::new([
    row![
        GridItem::new(text("A")),
        GridItem::new(text("B")),
        GridItem::new(text("C")),
    ],
    row![
        GridItem::new(text("D")).span(2),  // Spans 2 columns
        GridItem::new(text("E")),
    ],
])
.spacing(5.0)
.alignment(Alignment::Center);
```

### Overlay

Layered content positioning:

```rust
use waterui_layout::{overlay, Alignment};

let overlaid = overlay(
    base_content(),
    popup_content(),
    Alignment::TopTrailing
);
```

### Scroll View

Scrollable content containers:

```rust
use waterui_layout::{scroll, ScrollDirection};

let scrollable = scroll(
    very_long_content(),
    ScrollDirection::Vertical
)
.show_indicators(true)
.bounce(true);
```

### Spacer

Flexible spacing between elements:

```rust
use waterui_layout::{spacer, Spacer};

// Flexible spacer that takes available space
let flex_spacer = spacer();

// Fixed size spacer
let fixed_spacer = Spacer::fixed(20.0);

// Minimum size spacer
let min_spacer = Spacer::min(10.0);
```

## Layout Properties

### Frame

Control view sizing and positioning:

```rust
use waterui_layout::Frame;

let framed_view = some_view()
    .frame(Frame {
        width: 200.0,
        height: 100.0,
        min_width: 100.0,
        max_width: 300.0,
        alignment: Alignment::Center,
    });

// Convenience methods
let sized = some_view()
    .width(200.0)
    .height(100.0)
    .min_width(100.0)
    .max_height(150.0);
```

### Alignment

Position content within containers:

```rust
use waterui_layout::Alignment;

// Basic alignments
let leading = content().alignment(Alignment::Leading);      // Left/Right
let center = content().alignment(Alignment::Center);        // Center  
let trailing = content().alignment(Alignment::Trailing);    // Right/Left

// Two-dimensional alignments
let top_leading = content().alignment(Alignment::TopLeading);
let bottom_center = content().alignment(Alignment::BottomCenter);
```

## Advanced Layouts

### Aspect Ratio

Maintain proportional sizing:

```rust
let square = image()
    .aspect_ratio(1.0)          // 1:1 square
    .content_mode(ContentMode::Fit);

let widescreen = video()
    .aspect_ratio(16.0 / 9.0)   // 16:9 aspect ratio
    .content_mode(ContentMode::Fill);
```

### Padding and Margins

Control spacing around views:

```rust
// Uniform padding
let padded = content().padding(16.0);

// Directional padding
let custom_padding = content()
    .padding_top(20.0)
    .padding_horizontal(16.0)
    .padding_bottom(10.0);

// Edge insets
let inset = content().padding(EdgeInsets {
    top: 20.0,
    leading: 16.0,
    bottom: 10.0,
    trailing: 16.0,
});
```

### Clipping and Masking

Control view boundaries:

```rust
// Clip to bounds
let clipped = content()
    .clipped()
    .corner_radius(10.0);

// Custom clipping shape
let masked = content()
    .clip_shape(Circle())
    .shadow(radius: 5.0);
```

## Responsive Design

### Adaptive Layouts

Layouts that respond to size changes:

```rust
let adaptive = AdaptiveLayout::new()
    .compact(vstack![/* compact layout */])
    .regular(hstack![/* regular layout */])
    .large(grid![/* large layout */]);
```

### Size Classes

Different layouts for different screen sizes:

```rust
let responsive = ResponsiveView::new()
    .phone(mobile_layout())
    .tablet(tablet_layout())  
    .desktop(desktop_layout());
```

## Performance Optimizations

### Lazy Loading

Load content only when needed:

```rust
let lazy_grid = LazyGrid::new(data_source)
    .cell(|item| ItemView::new(item))
    .spacing(8.0)
    .cache_size(20);  // Keep 20 items in memory
```

### Virtual Scrolling

Efficient scrolling for large datasets:

```rust
let virtual_list = VirtualList::new(large_dataset)
    .item_height(60.0)
    .cell(|item, index| ListCell::new(item, index))
    .buffer_size(5);  // Render 5 extra items
```

## Dependencies

- `waterui-core`: Core framework functionality
- `waterui-reactive`: Reactive data binding

## Example

```rust
use waterui_layout::*;

struct MainView {
    items: Vec<String>,
}

impl View for MainView {
    fn body(self, env: &Environment) -> impl View {
        VStack::new([
            // Header
            HStack::new([
                text("My App").typography(Typography::HEADING_1),
                spacer(),
                button("Settings"),
            ])
            .padding(16.0),
            
            // Content
            scroll(
                VStack::new(
                    self.items.into_iter().map(|item| {
                        text(item).padding(8.0)
                    })
                )
                .spacing(4.0)
            )
            .direction(ScrollDirection::Vertical),
            
            // Footer
            HStack::new([
                button("Cancel"),
                spacer(),
                button("Save").style(ButtonStyle::Primary),
            ])
            .padding(16.0),
        ])
        .spacing(0.0)
        .frame_max_width()
        .frame_max_height()
    }
}
```
