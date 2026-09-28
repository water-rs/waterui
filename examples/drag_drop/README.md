# Drag and Drop Example

This example demonstrates WaterUI's native drag and drop capabilities.

## Features

- **Draggable Views**: Make any view draggable with a typed value
- **Typed Drop Destinations**: A destination accepts exactly the type its handler takes
- **Drop Events**: Handle on_enter, on_exit, and on_drop events
- **Value Types**: Text (`Str`), URLs (`Url`) and files (`Files`) cross applications; app types stay in the process

## Running

```bash
water run ios    # iOS Simulator
water run macos  # macOS
water run android # Android
```

## Usage

```rust
// Plain text travels to other applications too
text!("Drag me")
    .draggable(Str::from("Hello!"));

// The handler's first argument type is the accepted type
text!("Drop here")
    .drop_destination(|text: Str| {
        tracing::info!("received {text}");
    });

// An app type travels within the process only
#[derive(Debug, Clone, PartialEq)]
struct Fruit { emoji: &'static str, label: &'static str }
impl Transferable for Fruit {}
impl_constant!(Fruit);

fruit_card.draggable(Fruit { emoji: "🍎", label: "Apple" });
basket.drop_destination(|fruit: Fruit| { /* only fruit drags land here */ });
```
