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
waterui = "0.1.0"
waterui-gtk4 = "0.1.0"
```

## Building the Counter Step by Step

Let's build our counter app incrementally, learning WaterUI concepts along the way.

### Step 1: Basic Structure

Start with a simple view structure. Since our initial view doesn't need state, we can use a function:

**Filename**: `src/main.rs`
```rust,ignore
use waterui::{text, View, Environment};
use waterui_gtk4::{Gtk4App, init};

// Simple function returning impl View - no View trait needed!
fn counter_app() -> impl View {
    text("Counter App")
        .size(24.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    app.run(counter_app)
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
use waterui::{text, vstack, View, Environment};
use waterui_gtk4::{Gtk4App, init};

struct CounterApp;

impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        vstack((
            text("Counter App")
                .size(24.0)
                .weight(.bold),
                
            text("Count: 0")
                .size(20.0),
        ))
        .spacing(20.0)
        .padding(40.0)
        .alignment(.center)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    app.run(counter_app)
}
```

> **Note**: `vstack` creates a vertical stack of views. We'll learn about `hstack` (horizontal) and `zstack` (overlay) later.

### Step 3: Adding Reactive State

Now comes the exciting part - let's add reactive state! We'll use the `s!` macro from nami for reactive computations and the `text!` macro for reactive text:

```rust,ignore
use waterui::{
    text, text!, button, vstack, hstack, 
    binding, View, Environment
};
use nami::s;
use waterui_gtk4::{Gtk4App, init};

// Still using a function since state is local to the view
fn counter_app() -> impl View {
    // Create reactive state
    let count = binding(0);
    
    vstack((
        text("Counter App")
            .size(24.0)
            .weight(.bold),
            
        // Best practice: Use text! macro for reactive text
        text!("Count: {}", count)
            .size(20.0),
                
            hstack((
                button("- Decrement")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n - 1)
                    }),
                    
                button("+ Increment")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n + 1)
                    }),
            ))
            .spacing(10.0),
        ))
        .spacing(20.0)
        .padding(40.0)
        .alignment(.center)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    app.run(counter_app)
}
```

Run this and try clicking the buttons! The counter should update in real-time.

## Understanding the Code

Let's break down the key concepts introduced:

### Reactive State with `binding`

```rust,ignore
let count = binding(0);
```

This creates a reactive binding with an initial value of 0. When this value changes, any UI elements that depend on it will automatically update.

### Signal Mapping

```rust,ignore
text(count.signal().map(|&n| format!("Count: {}", n)))
```

- `count.signal()` creates a read-only signal from the binding
- `.map()` transforms the signal value (similar to `Iterator::map`)
- The text will automatically update whenever `count` changes

### Event Handling

```rust,ignore
button("+ Increment")
    .action({
        let count = count.clone();
        move |_| count.update(|n| n + 1)
    })
```

- `.action()` attaches an event handler to the button
- We clone `count` so it can be moved into the closure
- `count.update()` modifies the current value using a function

### Layout with Stacks

```rust,ignore
vstack((...))  // Vertical stack
hstack((...))  // Horizontal stack
```

Stacks are the primary layout tools in WaterUI, allowing you to arrange views vertically or horizontally.

## Step 4: Adding More Features

Let's enhance our counter with additional features:

```rust,ignore
use waterui::{
    text, button, vstack, hstack, spacer,
    binding, View, Environment, ViewExt, Color
};
use waterui_gtk4::{Gtk4App, init};

struct CounterApp;

impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        
        // Computed values based on count
        let is_positive = count.signal().map(|&n| n > 0);
        let is_even = count.signal().map(|&n| n % 2 == 0);
        let abs_value = count.signal().map(|&n| n.abs());
        
        vstack((
            text("Interactive Counter")
                .size(28.0)
                .weight(.bold)
                .color(Color::primary()),
                
            // Main counter display
            text(count.signal().map(|&n| format!("Count: {}", n)))
                .size(32.0)
                .weight(.bold)
                .color(is_positive.map(|&pos| {
                    if pos { Color::green() } else { Color::red() }
                })),
                
            // Status indicators
            hstack((
                text(is_even.map(|&even| {
                    if even { "Even" } else { "Odd" }
                }))
                .color(is_even.map(|&even| {
                    if even { Color::blue() } else { Color::orange() }
                })),
                
                spacer(),
                
                text(abs_value.map(|&abs| format!("Absolute: {}", abs)))
                    .size(14.0)
                    .color(Color::secondary()),
            ))
            .spacing(20.0),
            
            // Control buttons
            hstack((
                button("−")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n - 1)
                    })
                    .min_width(50.0),
                    
                button("Reset")
                    .action({
                        let count = count.clone();
                        move |_| count.set(0)
                    })
                    .style(ButtonStyle::Secondary),
                    
                button("+")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n + 1)
                    })
                    .min_width(50.0),
            ))
            .spacing(15.0),
            
            // Advanced controls
            hstack((
                button("×2")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n * 2)
                    }),
                    
                button("÷2")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n / 2)
                    })
                    .disabled(count.signal().map(|&n| n == 0)),
                    
                button("²")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n * n)
                    }),
            ))
            .spacing(10.0),
        ))
        .spacing(20.0)
        .padding(40.0)
        .max_width(400.0)
        .alignment(.center)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    app.run(counter_app)
}
```

## New Concepts Introduced

### Computed Values
```rust,ignore
let is_positive = count.signal().map(|&n| n > 0);
let is_even = count.signal().map(|&n| n % 2 == 0);
```

These create new reactive values derived from `count`. They automatically update when `count` changes.

### Dynamic Styling
```rust,ignore
.color(is_positive.map(|&pos| {
    if pos { Color::green() } else { Color::red() }
}))
```

UI properties can be reactive too! The text color changes based on whether the count is positive or negative.

### Conditional Disabling
```rust,ignore
.disabled(count.signal().map(|&n| n == 0))
```

The divide button is disabled when count is zero to prevent division by zero.

### Using `spacer()`
```rust,ignore
spacer()
```

Spacer components push other elements apart in a stack, useful for layout alignment.

## Step 5: Adding Input Controls

Let's add a text field to set the counter to a specific value:

```rust,ignore
use waterui::{
    text, text_field, button, vstack, hstack, spacer,
    binding, View, Environment, ViewExt, Color
};
use waterui_gtk4::{Gtk4App, init};

struct CounterApp;

impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        let input_text = binding(String::new());
        
        vstack((
            text("Interactive Counter")
                .size(28.0)
                .weight(.bold),
                
            text(count.signal().map(|&n| format!("Count: {}", n)))
                .size(32.0)
                .weight(.bold),
                
            // Input section
            hstack((
                text("Set to:"),
                text_field(input_text.clone())
                    .placeholder("Enter number...")
                    .width(100.0),
                button("Set")
                    .action({
                        let count = count.clone();
                        let input_text = input_text.clone();
                        move |_| {
                            if let Ok(value) = input_text.get().parse::<i32>() {
                                count.set(value);
                                input_text.set(String::new());
                            }
                        }
                    }),
            ))
            .spacing(10.0),
            
            // Original buttons...
            hstack((
                button("−")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n - 1)
                    }),
                button("Reset")
                    .action({
                        let count = count.clone();
                        move |_| count.set(0)
                    }),
                button("+")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n + 1)
                    }),
            ))
            .spacing(15.0),
        ))
        .spacing(20.0)
        .padding(40.0)
        .alignment(.center)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    app.run(counter_app)
}
```

### New Concepts: Text Fields

```rust,ignore
let input_text = binding(String::new());
text_field(input_text.clone())
    .placeholder("Enter number...")
```

Text fields in WaterUI are controlled by bindings. The field displays the binding's value and updates the binding when the user types.

### Error Handling in Actions

```rust,ignore
if let Ok(value) = input_text.get().parse::<i32>() {
    count.set(value);
    input_text.set(String::new());
}
```

Always handle potential errors when parsing user input. Invalid input is silently ignored here, but you might want to show error messages in a real app.

## Common Patterns and Best Practices

### 1. Clone Bindings for Closures
```rust,ignore
let count = count.clone();  // Clone before moving into closure
move |_| count.update(|n| n + 1)
```

Rust's ownership rules require cloning bindings before moving them into closures.

### 2. Use Descriptive Variable Names
```rust,ignore
let is_positive = count.signal().map(|&n| n > 0);
let formatted_count = count.signal().map(|&n| format!("Count: {}", n));
```

Clear names make your reactive logic easier to understand and maintain.

### 3. Separate Concerns
```rust,ignore
// Good: Separate data and presentation
let is_even = count.signal().map(|&n| n % 2 == 0);
text(is_even.map(|&even| if even { "Even" } else { "Odd" }))

// Avoid: Mixed logic
text(count.signal().map(|&n| {
    if n % 2 == 0 { "Even" } else { "Odd" }
}))
```

### 4. Handle Edge Cases
```rust,ignore
button("Divide by 2")
    .disabled(count.signal().map(|&n| n == 0))  // Prevent division by zero
```

Think about edge cases and handle them gracefully in your UI.

## Exercises

Try these exercises to reinforce your learning:

1. **History Feature**: Add a list that shows the last 10 counter values
2. **Step Size**: Add a stepper control to change how much the increment/decrement buttons change the value
3. **Bounds**: Add minimum and maximum bounds for the counter
4. **Themes**: Add a toggle to switch between light and dark themes
5. **Persistence**: Save the counter value to a file and restore it when the app starts

## Troubleshooting Common Issues

### "Value moved" Errors
**Problem**: Compiler error about moved values in closures.
**Solution**: Clone the binding before the `move` closure.

```rust,ignore
// Error
let count = binding(0);
button("Click").action(move |_| count.set(1));  // count moved here
button("Click2").action(move |_| count.set(2)); // error: count already moved

// Fixed
let count = binding(0);
button("Click").action({
    let count = count.clone();  // Clone first
    move |_| count.set(1)
});
button("Click2").action({
    let count = count.clone();  // Clone again
    move |_| count.set(2)
});
```

### UI Not Updating
**Problem**: UI doesn't update when data changes.
**Solution**: Make sure you're using `.signal().map()` for reactive updates.

```rust,ignore
// Wrong: Static text
text(format!("Count: {}", count.get()))

// Right: Reactive text
text(count.signal().map(|&n| format!("Count: {}", n)))
```

### Performance Issues
**Problem**: App feels slow with many updates.
**Solution**: Use `.computed()` for expensive operations.

```rust,ignore
// If expensive_calculation is slow, use computed()
let result = count.computed(|&n| expensive_calculation(n));
```

## Summary

In this chapter, you learned:

- ✅ How to create reactive state with `binding()`
- ✅ How to make UI update automatically with `.signal().map()`
- ✅ How to handle user events with `.action()`
- ✅ How to compose views with stacks
- ✅ How to create dynamic styling and conditional behavior
- ✅ How to work with text fields and user input

These concepts form the foundation of all WaterUI applications. In the next chapter, we'll dive deeper into the View system and learn how it enables powerful composition patterns.

Ready to learn more? Let's explore [Understanding Views](03-views.md)!