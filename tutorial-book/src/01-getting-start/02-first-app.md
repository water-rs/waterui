# Your First WaterUI App

Now that your development environment is set up, let's build your first interactive WaterUI application! We'll create a counter app that demonstrates the core concepts of views, state management, and user interaction.

## What We'll Build

Our counter app will feature:
- A display showing the current count
- Buttons to increment and decrement the counter
- A reset button
- Dynamic styling based on the counter value

By the end of this chapter, you'll understand:
- How to create interactive views
- How to manage reactive state
- How to handle user events
- How to compose views together

## Setting Up the Project

Create a new project for our counter app:

```bash
cargo new counter-app
cd counter-app
```

Update your `Cargo.toml`:

**Filename**: `Cargo.toml`
```toml
[package]
name = "counter-app"
version = "0.1.0"
edition = "2021"

[dependencies]
waterui = { path = ".." }
waterui_gtk4 = { path = "../backends/gtk4" }
```

## Building the Counter Step by Step

Let's build our counter app incrementally, learning WaterUI concepts along the way.

### Step 1: Basic Structure

Start with a simple view structure. Since our initial view doesn't need state, we can use a function:

**Filename**: `src/main.rs`
```rust,ignore
use waterui::View;
use waterui_gtk4::{Gtk4App, init};

pub fn counter() -> impl View {
    "Counter App"
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    Ok(app.run(counter).into())
}
```

Run this to make sure everything works:
```bash
cargo run
```

You should see a window with "Counter App" displayed.

### Step 2: Adding Layout

Now let's add some layout structure using stacks:

```rust,ignore
use waterui::{component::layout::stack::vstack, View};

pub fn counter() -> impl View {
    vstack((
        "Counter App",
        "Count: 0",
    ))
}
```

> **Note**: `vstack` creates a vertical stack of views. We'll learn about `hstack` (horizontal) and `zstack` (overlay) later.

### Step 3: Adding Reactive State

Now comes the exciting part - let's add reactive state! We'll use the `s!` macro from nami for reactive computations and the `text!` macro for reactive text:

```rust,ignore
use waterui::{
    component::{
        layout::stack::{vstack, hstack},
        button::button,
    },
    View,
};
use waterui_text::text;
use waterui::reactive::binding;

pub fn counter() -> impl View {
    let count = binding(0);
    vstack((
        "Counter App",
        // Use text! macro for reactive text
        text!("Count: {}", count),
        hstack((
            button("- Decrement").action_with(&count, |count| count.update(|n| n - 1)),
            button("+ Increment").action_with(&count, |count| count.update(|n| n + 1)),
        )),
    ))
}
```

Run this and try clicking the buttons! The counter should update in real-time.

## Understanding the Code

Let's break down the key concepts introduced:

### Reactive State with `binding`

```rust,ignore
let count = Binding::int(0);
```

This creates a reactive binding with an initial value of 0. When this value changes, any UI elements that depend on it will automatically update.

### Reactive Text Display

```rust,ignore
// ✅ Use the text! macro for reactive display
text!("Count: {}", count)
```

- The `text!` macro automatically handles reactivity
- The text will update whenever `count` changes

### Event Handling

```rust,ignore
button("- Decrement").action_with(&count, |count| count.update(|n| n - 1))
```

- `.action_with()` attaches an event handler with captured state
- `Binding<T>::update(|v| ...)` updates the value and notifies watchers

### Layout with Stacks

```rust,ignore
vstack((...))  // Vertical stack
hstack((...))  // Horizontal stack
```

Stacks are the primary layout tools in WaterUI, allowing you to arrange views vertically or horizontally.
