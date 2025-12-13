# waterui-layout

Layout building blocks for arranging views in WaterUI applications.

## Overview

`waterui-layout` provides the fundamental layout primitives used to compose user interfaces in WaterUI. Unlike traditional UI frameworks that manually calculate positions, this crate implements a declarative, constraint-based layout system inspired by SwiftUI's layout protocol. All components render to native platform widgets (SwiftUI on Apple, Jetpack Compose on Android) rather than drawing custom pixels.

The crate bridges the declarative `View` trait with the imperative, backend-driven layout pass through the `Layout` trait, enabling flexible composition of stacks, spacers, frames, and scrollable containers. All layout values use logical pixels (points/dp) matching design tool specifications exactly, with native backends handling density-aware conversion to physical pixels.

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
waterui-layout = "0.1.0"
```

Or use the main `waterui` crate which re-exports all layout components:

```toml
[dependencies]
waterui = "0.1.0"
```

## Quick Start

Here's a simple toolbar layout demonstrating horizontal stacking and spacers:

```rust
use waterui_layout::{stack, spacer};
use waterui_text::text;
use waterui_core::View;

pub fn toolbar() -> impl View {
    stack::hstack((
        text("WaterUI"),
        spacer(),
        text("v0.1"),
    ))
    .spacing(8.0)
}
```

This creates a horizontal layout with "WaterUI" on the left, "v0.1" on the right, and flexible space between them.

## Core Concepts

### Layout Trait

The `Layout` trait defines how containers arrange their children through a two-phase protocol:

1. **Sizing Phase**: `size_that_fits(proposal, children)` determines the container's size given a parent proposal
2. **Placement Phase**: `place(bounds, children)` positions children within the final bounds

Layouts can query children multiple times with different proposals to negotiate optimal sizing.

### Stretch Behavior

Views communicate their flexibility through `StretchAxis`:

- `None` - Content-sized, uses intrinsic dimensions
- `Horizontal` - Expands width only (e.g., TextField)
- `Vertical` - Expands height only
- `Both` - Greedy, fills all space (e.g., Spacer, Color)
- `MainAxis` - Stretches along parent's main axis (used by Spacer)
- `CrossAxis` - Stretches along parent's cross axis (used by Divider)

Stack containers distribute remaining space among stretching children proportionally.

### Logical Pixels

All layout values use **logical pixels** (points/dp) - the same unit as Figma, Sketch, and Adobe XD:

- `spacing(8.0)` = 8pt in design tools
- `width(100.0)` = 100pt/dp, same physical size across all devices
- iOS/macOS: Uses points natively
- Android: Converts dp → pixels via `displayMetrics.density`

This ensures pixel-perfect design implementation across platforms.

## Examples

### Building a Form Layout

```rust
use waterui_layout::{stack, padding::EdgeInsets};
use waterui_text::text;
use waterui_controls::TextField;
use waterui_reactive::binding;
use waterui_core::View;

pub fn login_form() -> impl View {
    let username = binding("");
    let password = binding("");

    stack::vstack((
        text("Login").size(24.0).bold(),
        TextField::new(&username)
            .label(text("Username"))
            .prompt("Enter username"),
        TextField::new(&password)
            .label(text("Password"))
            .prompt("Enter password")
            .secure(true),
    ))
    .alignment(stack::HorizontalAlignment::Leading)
    .spacing(16.0)
    .padding_with(EdgeInsets::all(20.0))
}
```

### Creating a Toolbar with Spacers

```rust
use waterui_layout::{stack, spacer};
use waterui_controls::button;
use waterui_text::text;
use waterui_core::View;

pub fn app_toolbar() -> impl View {
    stack::hstack((
        button("Menu", || { /* action */ }),
        spacer(),
        text("My App").bold(),
        spacer(),
        button("Settings", || { /* action */ }),
    ))
    .spacing(12.0)
}
```

### Overlaying Content

```rust
use waterui_layout::{stack, overlay, stack::Alignment};
use waterui_graphics::Color;
use waterui_text::text;
use waterui_core::View;

pub fn badge_overlay() -> impl View {
    overlay(
        Color::blue().frame(100.0, 100.0),
        text("5").foreground(Color::white()),
    )
    .alignment(Alignment::TopTrailing)
}
```

### Scrollable Content

```rust
use waterui_layout::{scroll, stack};
use waterui_text::text;
use waterui_core::View;

pub fn scrollable_list() -> impl View {
    scroll(
        stack::vstack((
            text("Item 1"),
            text("Item 2"),
            text("Item 3"),
            // ... many more items
        ))
        .spacing(10.0)
    )
}
```

### Responsive Layout with Frames

```rust
use waterui_layout::frame::Frame;
use waterui_layout::stack::Alignment;
use waterui_text::text;
use waterui_core::View;

pub fn constrained_content() -> impl View {
    Frame::new(text("Limited Width"))
        .min_width(100.0)
        .max_width(300.0)
        .height(50.0)
        .alignment(Alignment::Center)
}
```

## API Overview

### Stack Containers

- **`stack::hstack(content)`** - Arranges children horizontally left-to-right
  - `.spacing(f32)` - Sets spacing between children
  - `.alignment(VerticalAlignment)` - Sets vertical alignment (Top, Center, Bottom)

- **`stack::vstack(content)`** - Arranges children vertically top-to-bottom
  - `.spacing(f32)` - Sets spacing between children
  - `.alignment(HorizontalAlignment)` - Sets horizontal alignment (Leading, Center, Trailing)

- **`stack::zstack(content)`** - Overlays children in the same space
  - `.alignment(Alignment)` - Sets 2D alignment for overlaid content

### Layout Primitives

- **`spacer()`** - Flexible space that expands to push views apart
- **`spacer_min(f32)`** - Spacer with minimum length
- **`ScrollView`** - Scrollable container for overflow content
  - `scroll(content)` - Vertical scrolling
  - `scroll_horizontal(content)` - Horizontal scrolling
  - `scroll_both(content)` - Bidirectional scrolling

- **`Frame`** - Constrains child size with min/max/ideal dimensions
  - `.width(f32)`, `.height(f32)` - Sets ideal dimensions
  - `.min_width(f32)`, `.max_width(f32)` - Sets size constraints
  - `.alignment(Alignment)` - Aligns child within frame

- **`Padding`** - Insets child with edge spacing
  - `EdgeInsets::all(f32)` - Equal padding on all edges
  - `EdgeInsets::symmetric(vertical, horizontal)` - Symmetric padding
  - `EdgeInsets::new(top, bottom, leading, trailing)` - Custom edges

### Advanced Layouts

- **`overlay(base, layer)`** - Layers content on top of base without affecting layout size
- **`OverlayLayout`** - Layout engine where base child dictates container size
- **`LazyContainer`** - Efficient container for dynamic collections with `ForEach`
- **`IgnoreSafeArea`** - Metadata to extend content into safe area regions
  - `EdgeSet` - Bitflags for specifying which edges ignore safe area

### Alignment Types

- **`Alignment`** - 2D alignment (TopLeading, Top, TopTrailing, Leading, Center, Trailing, BottomLeading, Bottom, BottomTrailing)
- **`HorizontalAlignment`** - Leading, Center, Trailing
- **`VerticalAlignment`** - Top, Center, Bottom
- **`Axis`** - Horizontal, Vertical (for stack direction)

## Features

This crate supports optional features:

- **`serde`** - Enables serialization/deserialization of layout types via serde

Enable features in your `Cargo.toml`:

```toml
[dependencies]
waterui-layout = { version = "0.1.0", features = ["serde"] }
```


## Architecture Notes

### Backend Integration

Layouts communicate with native backends through the `Layout` trait's protocol:

1. Backend calls `size_that_fits(proposal, children)` to measure
2. Backend calls `place(bounds, children)` to get child rectangles
3. Backend renders native widgets at the calculated positions

The FFI layer in `waterui-ffi` handles the Rust ↔ Native boundary.

### Performance Characteristics

- All layout calculations happen in Rust, then native backends cache results
- The `SubView` trait enables measurement caching at the platform level
- Lazy containers (`LazyContainer`) defer child instantiation for large collections
- Layout is pure (no side effects), enabling aggressive optimization by backends

### Layout Compression

When children exceed available space:

- **HStack**: Compresses largest non-stretching children first, preserving small labels
- **VStack**: Children maintain intrinsic heights, may overflow bounds (scrollable)
- Minimum size enforcement prevents unreadable content (20pt minimum for compressed children)

This behavior matches native platform conventions for graceful degradation.
