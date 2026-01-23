---
name: waterui
description: Build cross-platform apps with WaterUI. Use when writing views, handling state, styling UI, or debugging WaterUI Rust code. Covers reactive bindings, layout, components, and the water CLI.
---

# WaterUI App Development

Build views with reactive state. When unsure, use Explore agent to search `examples/*/src/lib.rs`.

## Quick Start

```rust
use waterui::prelude::*;

#[hot_reload]
fn main() -> impl View {
    let count = Binding::new(0);

    vstack((
        text!("Count: {count}").headline(),
        button("+1")
            .with_state(&count)
            .action(|c| c.set(c.get() + 1)),
    ))
}
```

## Views

Functions and closures are views:
```rust
fn card(title: &str) -> impl View {
    vstack((text(title).title(), Divider))
}

// Use directly - no wrapper needed
vstack((card("Hello"), card("World")))
```

Conditional: `condition.then(|| view)` or `Option<impl View>`

## State

```rust
let toggle = Binding::bool(false);      // bool
let count = Binding::new(0);            // i32
let name = Binding::container(String::new()); // heap types

// Pass by reference to child views
fn section(count: &Binding<i32>) -> impl View { ... }
```

## Reactive Transforms

Methods on signals (no `.clone()` needed):
```rust
count.not()                    // bool negation
count.select(a, b)             // if-else
count.equal_to(5)              // equality check
count.gt(0)                    // comparisons: lt, le, ge
count.is_empty()               // for strings/collections
count.map(|v| v * 2)           // custom transform
count.zip(&other).map(|(a,b)| a + b)  // combine signals
```

Convert to Computed: `signal.computed()`

## Event Handlers

```rust
// Single state
button("Click")
    .with_state(&count)
    .action(|c| c.set(c.get() + 1))

// Multiple states → tuple
button("Reset")
    .with_state(&x)
    .with_state(&y)
    .action(|(x, y)| { x.set(0); y.set(0); })

// Async
button("Load").action_async(|_| async { fetch().await })

// Lifecycle
view.on_appear(|| setup())
view.on_change(&signal, |new_val| handle(new_val))
```

## Text

```rust
// Static
text("Hello").title()       // semantic sizes: title, headline, body, caption, footnote

// Reactive interpolation
text!("Count: {count}")     // auto-updates
text!("{a} + {b} = {sum}")  // multiple signals
```

## Layout

```rust
hstack((a, b, c)).spacing(8.0)
vstack((a, b)).padding()
zstack((background, content))
scroll(content)
spacer()                    // flexible space
spacer().height(16.0)       // fixed space
```

## Colors

```rust
// Built-in (zero-sized, efficient)
Blue, Green, Red, Orange, Purple, Cyan, Yellow, Pink

// Custom
const BRAND: Srgb = Srgb::from_hex("#3B82F6");

// Usage - colors are Views
view.background(Blue)
view.foreground(BRAND)
Blue.size(80.0, 80.0)       // colored rectangle
BRAND.with_opacity(0.5)
```

Theme colors: `Foreground`, `MutedForeground`, `Accent`, `Background`, `Surface`, `Border`

## Modifiers

```rust
.padding() / .padding_with(EdgeInsets::all(16.0))
.background(color) / .foreground(color)
.size(w, h) / .width(w) / .height(h)
.scale(x, y) / .rotation(degrees) / .offset(x, y)
.border(color, width) / .shadow() / .clip(shape)
.disabled(bool_signal) / .visible(bool_signal)
```

## Components

| Category | Components |
|----------|------------|
| Layout | `hstack`, `vstack`, `zstack`, `scroll`, `spacer`, `grid` |
| Controls | `button`, `toggle`, `Slider`, `Stepper`, `field`, `Menu` |
| Navigation | `NavigationStack`, `NavigationLink`, `TabView` |
| Media | `Photo`, `VideoPlayer`, `MediaPicker` |
| Graphics | `Canvas`, `Chart`, `Map`, `Barcode::qr()` |

## CLI Commands

```bash
water create my-app              # new project
water run --platform ios         # run on simulator
water run --platform android
water run --platform macos
water preview my_view            # preview #[preview] function
water run --logs debug           # with debug output
```

## Common Patterns

```rust
// Animated toggle
let scale = active.select(1.2_f32, 1.0).with(Animation::spring(300.0, 15.0));

// Conditional visibility
.visible(items.map(|i| !i.is_empty()).computed())

// List rendering
List::for_each(&items, |item| item_view(item))

// Form from struct
#[derive(FormBuilder)]
struct Settings { name: String, volume: f64 }
form(&settings_binding)
```

## Gotchas

**No `_f32` suffix** - use `as f32` cast:
```rust
// WRONG
.select(1.0_f32, 0.3)

// CORRECT
.select(1.0 as f32, 0.3)
```

**No `.get()` in view bodies** - breaks reactivity:
```rust
// WRONG
text(format!("Count: {}", count.get()))

// CORRECT
text!("Count: {count}")
```

**Two-param transforms:**
```rust
.scale(x, y)    // not .scale(uniform)
.offset(x, y)
.size(w, h)
```
