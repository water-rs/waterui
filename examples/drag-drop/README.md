# Drag and Drop Example

This example demonstrates WaterUI's native drag and drop capabilities.

## Features

- **Draggable Views**: Make any view draggable with text or URL data
- **Drop Destinations**: Accept dropped content with handlers
- **Drop Events**: Handle on_enter, on_exit, and on_drop events
- **Data Types**: Supports Text and URL data

## Running

```bash
water run ios    # iOS Simulator
water run macos  # macOS
water run android # Android
```

## Usage

```rust
// Make a view draggable
text!("Drag me")
    .draggable(DragData::text("Hello!"));

// Create a drop destination
text!("Drop here")
    .drop_destination(|Use(data): Use<DragData>| {
        println!("Received: {}", data.as_str());
    });
```
