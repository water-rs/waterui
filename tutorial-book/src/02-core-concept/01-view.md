# Understanding Views

The View system is the heart of WaterUI. Everything you see on screen is a View, and understanding how Views work is crucial for building efficient and maintainable applications. In this chapter, we'll explore the View trait in depth and learn how to create custom components.

## What is a View?

A View in WaterUI represents a piece of user interface. It could be as simple as a text label or as complex as an entire application screen. The beauty of the View system is that simple and complex views work exactly the same way.

### The View Trait

Every View implements a single trait:

```rust,ignore
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

This simple signature enables powerful composition patterns. Let's understand each part:

- **`'static` lifetime**: Views can't contain non-static references, ensuring they can be stored and moved safely
- **`self` parameter**: Views consume themselves when building their body, enabling zero-cost moves
- **`env: &Environment`**: Provides access to shared configuration and dependencies
- **`-> impl View`**: Returns any type that implements View, enabling flexible composition

## Built-in Views

WaterUI provides many built-in Views for common UI elements:

### Text Views
```rust,ignore
// Static text
"Hello, World!"

// Equal to text(name.map(|n| format!("Hello, {n}!")))
text!("Hello, {}!", name)

// Styled text
"Important!"
    .size(24)
    .weight(.bold)
    .color(Color::red())
```

### Control Views
```rust,ignore
// Button
button("Click me")
    .action(|| println!("Clicked!"))

// Text field
let input = binding("");
text_field(&input)
    .placeholder("Enter text...")

// Toggle switch
let enabled = binding(false);
toggle(&enabled)
```

### Layout Views
```rust,ignore
// Vertical stack
vstack((
    "First",
    "Second",
    "Third",
))

// Horizontal stack
hstack((
    button("Cancel"),
    button("OK"),
))

// Overlay stack
zstack((
    background_view(),
    content_view(),
    overlay_view(),
))
```

## Creating Custom Views

The real power of WaterUI comes from creating your own custom Views. Let's explore different patterns:

### Function Views (Recommended)


```rust,ignore
// Simpler and cleaner - no View trait needed!
fn welcome_message(name: &str) -> impl View {
    vstack((
        "Welcome!"
            .size(24.0)
            .weight(.bold),
        text(format!("Hello, {}!", name))
            .color(Color::blue()),
    ))
    .spacing(10.0)
    .padding(20.0)
}

// Usage - functions are automatically views!
welcome_message("Alice")

// Can also use closures for lazy initialization
let lazy_view = || welcome_message("Bob");
```

### Struct Views (For Components with State)

Only use the View trait when your component needs to store state, or you prefer access environment directly (but why not use `use_env` function in your View?):

```rust,ignore
// Only needed when the struct holds state
struct CounterWidget {
    initial_value: i32,
    step: i32,
}

impl View for CounterWidget {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(self.initial_value);
        
        vstack((
            text!("Count: {}", count),
            button("+")
                .action_with(&count, |count| count.increment(step) }),
        ))
    }
}

// Usage
CounterWidget { 
    initial_value: 0,
    step: 5,
}
```
