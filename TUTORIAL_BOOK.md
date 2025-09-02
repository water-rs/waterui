# WaterUI Tutorial Book
## The Complete Guide to Cross-Platform UI Development with Rust

---

## Table of Contents

### Part I: Getting Started
1. [Introduction to WaterUI](#chapter-1-introduction-to-waterui)
2. [Setting Up Your Development Environment](#chapter-2-setting-up-your-development-environment)
3. [Your First WaterUI Application](#chapter-3-your-first-waterui-application)
4. [Understanding the View System](#chapter-4-understanding-the-view-system)

### Part II: Core Concepts
5. [The Environment System](#chapter-5-the-environment-system)
6. [Reactive State Management](#chapter-6-reactive-state-management)
7. [Component Composition](#chapter-7-component-composition)
8. [Event Handling](#chapter-8-event-handling)

### Part III: Building User Interfaces
9. [Layout Components](#chapter-9-layout-components)
10. [Text and Typography](#chapter-10-text-and-typography)
11. [Form Controls and Input](#chapter-11-form-controls-and-input)
12. [Media Components](#chapter-12-media-components)
13. [Navigation and Routing](#chapter-13-navigation-and-routing)

### Part IV: Advanced Topics
14. [Custom Components](#chapter-14-custom-components)
15. [Animation and Transitions](#chapter-15-animation-and-transitions)
16. [Plugin Development](#chapter-16-plugin-development)
17. [Memory Management and Performance](#chapter-17-memory-management-and-performance)

### Part V: Platform-Specific Development
18. [Desktop Applications with GTK4](#chapter-18-desktop-applications-with-gtk4)
19. [Web Applications with WASM](#chapter-19-web-applications-with-wasm)
20. [Embedded Development](#chapter-20-embedded-development)

### Part VI: Practical Projects
21. [Building a Todo Application](#chapter-21-building-a-todo-application)
22. [Creating a Media Player](#chapter-22-creating-a-media-player)
23. [Developing a Dashboard App](#chapter-23-developing-a-dashboard-app)

### Part VII: Production and Deployment
24. [Testing Strategies](#chapter-24-testing-strategies)
25. [Performance Optimization](#chapter-25-performance-optimization)
26. [Deployment and Distribution](#chapter-26-deployment-and-distribution)
27. [Troubleshooting and Debugging](#chapter-27-troubleshooting-and-debugging)

---

# Chapter 1: Introduction to WaterUI

Welcome to WaterUI, a modern UI framework for Rust that enables you to build applications for any platform using a single codebase. Whether you're targeting desktop, mobile, web, or even embedded devices, WaterUI provides the tools and abstractions you need to create beautiful, performant user interfaces.

## What is WaterUI?

WaterUI is a declarative, reactive UI framework built from the ground up in Rust. It combines the performance and safety of Rust with a developer-friendly API inspired by modern UI frameworks like SwiftUI and React.

### Key Features

- **Cross-Platform**: Write once, run everywhere - desktop, web, mobile, and embedded
- **Type-Safe**: Leverage Rust's powerful type system for compile-time correctness
- **Reactive**: Automatic UI updates when data changes
- **Declarative**: Describe what your UI should look like, not how to build it
- **Performance**: Zero-cost abstractions and memory-efficient design
- **No-std Support**: Run on embedded devices with minimal resources

### Why Choose WaterUI?

1. **Single Codebase**: No need to maintain separate codebases for different platforms
2. **Rust's Safety**: Memory safety and thread safety built into the language
3. **Modern API**: Intuitive, declarative syntax that's easy to learn and use
4. **Performance**: Native performance on all platforms
5. **Ecosystem**: Leverage the growing Rust ecosystem

## Architecture Overview

WaterUI is built around three core concepts:

### 1. Views
Views are the building blocks of your UI. They describe what should be displayed and how it should look.

```rust
struct HelloWorld;

impl View for HelloWorld {
    fn body(self, _env: &Environment) -> impl View {
        text("Hello, World!")
    }
}
```

### 2. Reactive State
State that automatically updates the UI when it changes.

```rust
let counter = binding(0);
let display = text(counter.display());
```

### 3. Environment
A type-safe way to pass configuration and dependencies through your view hierarchy.

```rust
let env = Environment::new()
    .with(Theme::Dark)
    .with(Language::English);
```

## What You'll Learn

By the end of this book, you'll be able to:

- Build complete applications using WaterUI
- Understand reactive programming patterns
- Create custom components and layouts
- Optimize performance for different platforms
- Deploy applications to multiple targets
- Integrate with platform-specific features

## Prerequisites

This book assumes:

- Basic knowledge of Rust programming language
- Familiarity with ownership, borrowing, and lifetimes
- Understanding of traits and generic programming
- Basic command-line experience

If you're new to Rust, we recommend reading "The Rust Programming Language" first.

## Book Structure

This book is structured in seven parts:

- **Part I** covers the basics and gets you started
- **Part II** dives deep into core concepts
- **Part III** teaches you how to build user interfaces
- **Part IV** covers advanced topics and customization
- **Part V** focuses on platform-specific development
- **Part VI** provides practical, real-world projects
- **Part VII** covers production concerns and deployment

Each chapter builds on previous knowledge, so we recommend reading them in order for the best experience.

---

# Chapter 2: Setting Up Your Development Environment

Before we start building with WaterUI, let's set up a proper development environment. This chapter will guide you through installing Rust, setting up your editor, and creating your first WaterUI project.

## Installing Rust

WaterUI requires Rust 1.85 or later. The easiest way to install Rust is through rustup:

### On macOS, Linux, or WSL

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### On Windows

Download and run the installer from [rustup.rs](https://rustup.rs/).

### Verify Installation

```bash
rustc --version
cargo --version
```

You should see version 1.85 or later.

## Editor Setup

### VS Code (Recommended)

1. Install [Visual Studio Code](https://code.visualstudio.com/)
2. Install the "rust-analyzer" extension
3. Install the "CodeLLDB" extension for debugging

### Other Editors

- **IntelliJ IDEA**: Install the Rust plugin
- **Vim/Neovim**: Use rust.vim and coc-rust-analyzer
- **Emacs**: Use rust-mode and lsp-mode

## Creating Your First Project

Let's create a new WaterUI project:

```bash
cargo new my-waterui-app
cd my-waterui-app
```

### Adding WaterUI Dependency

Edit your `Cargo.toml`:

```toml
[package]
name = "my-waterui-app"
version = "0.1.0"
edition = "2021"

[dependencies]
waterui = "0.1.0"
# Add backend dependencies based on your target
waterui-gtk4 = "0.1.0"  # For desktop
# waterui-web = "0.1.0"    # For web
```

## Project Structure

A typical WaterUI project structure:

```
my-waterui-app/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── views/
│   │   ├── mod.rs
│   │   ├── home.rs
│   │   └── settings.rs
│   ├── components/
│   │   ├── mod.rs
│   │   └── custom_button.rs
│   └── state/
│       ├── mod.rs
│       └── app_state.rs
├── assets/
│   ├── images/
│   └── fonts/
└── tests/
    └── integration_tests.rs
```

## Development Tools

### Essential Cargo Tools

```bash
# Code formatting
cargo install rustfmt

# Linting
cargo install clippy

# Documentation generation
cargo doc --open
```

### Useful Development Commands

```bash
# Build your project
cargo build

# Run your application
cargo run

# Run tests
cargo test

# Check for errors without building
cargo check

# Format code
cargo fmt

# Lint code
cargo clippy
```

## Platform-Specific Setup

### For Desktop (GTK4)

#### Ubuntu/Debian
```bash
sudo apt update
sudo apt install libgtk-4-dev build-essential
```

#### macOS
```bash
brew install gtk4
```

#### Windows
Install GTK4 development libraries through vcpkg or MSYS2.

### For Web (WebAssembly)

```bash
# Install wasm-pack
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# Add WebAssembly target
rustup target add wasm32-unknown-unknown
```

## Hello World Example

Let's verify everything works with a simple "Hello World" application:

```rust
// src/main.rs
use waterui::{text, View, Environment};
use waterui_gtk4::{Gtk4App, init};

struct HelloWorld;

impl View for HelloWorld {
    fn body(self, _env: &Environment) -> impl View {
        text("Hello, WaterUI!")
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the GTK4 backend
    init()?;
    
    // Create and run the application
    let app = Gtk4App::new("com.example.hello-waterui");
    app.run(HelloWorld)
}
```

### Building and Running

```bash
cargo run
```

You should see a window with "Hello, WaterUI!" displayed.

## Troubleshooting Common Issues

### GTK4 Not Found
- **Linux**: Install development packages for GTK4
- **macOS**: Install GTK4 through Homebrew
- **Windows**: Set up MSYS2 or vcpkg

### Rust Version Too Old
```bash
rustup update
```

### Permission Issues
```bash
# On Unix systems
chmod +x target/debug/my-waterui-app
```

## Next Steps

Now that you have a working development environment, you're ready to build your first real WaterUI application. In the next chapter, we'll explore the fundamental concepts of the View system and build a simple interactive application.

---

# Chapter 3: Your First WaterUI Application

Now that your development environment is set up, let's build your first interactive WaterUI application. We'll create a simple counter app that demonstrates the core concepts of Views, state management, and event handling.

## The Counter App

Our counter app will have:
- A display showing the current count
- A button to increment the counter
- A button to decrement the counter
- A reset button

## Creating the Basic Structure

First, let's set up the main application structure:

```rust
// src/main.rs
use waterui::{
    text, button, vstack, hstack,
    binding, View, Environment, ViewExt
};
use waterui_gtk4::{Gtk4App, init};

struct CounterApp;

impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        // We'll implement this step by step
        text("Counter App")
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init()?;
    let app = Gtk4App::new("com.example.counter-app");
    app.run(CounterApp)
}
```

## Adding State Management

WaterUI uses reactive state management. Let's add a counter state:

```rust
impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        // Create reactive state
        let count = binding(0);
        
        vstack([
            // Display the current count
            text(format!("Count: {}", count.get()))
                .size(24.0),
                
            // We'll add buttons next
        ])
        .spacing(20.0)
        .padding(40.0)
    }
}
```

## Making It Reactive

The above code won't update when the count changes. We need to make it reactive:

```rust
impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        
        vstack([
            // Reactive text that updates when count changes
            text(count.map(|c| format!("Count: {}", c)))
                .size(24.0),
                
            hstack([
                // Increment button
                button("+ Increment")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c + 1)
                    }),
                    
                // Decrement button  
                button("- Decrement")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c - 1)
                    }),
            ])
            .spacing(10.0),
            
            // Reset button
            button("Reset")
                .action({
                    let count = count.clone();
                    move |_| count.set(0)
                })
                .style(ButtonStyle::Secondary),
        ])
        .spacing(20.0)
        .padding(40.0)
        .alignment(.center)
    }
}
```

## Understanding Reactive State

Let's break down the reactive state concepts:

### Creating State
```rust
let count = binding(0);  // Creates a reactive binding with initial value 0
```

### Reading State
```rust
count.get()              // Gets current value (not reactive)
count.map(|c| ...)       // Maps value reactively - UI updates automatically
```

### Updating State
```rust
count.set(42)            // Sets new value
count.update(|c| c + 1)  // Updates based on current value
```

## Adding Computed Values

Let's add some computed values that depend on our counter:

```rust
impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        
        // Computed values
        let is_even = count.map(|c| c % 2 == 0);
        let is_positive = count.map(|c| c > 0);
        
        vstack([
            text(count.map(|c| format!("Count: {}", c)))
                .size(24.0)
                .weight(.bold),
                
            // Display computed values
            text(is_even.map(|even| {
                if even { "Even number" } else { "Odd number" }
            }))
            .color(is_even.map(|even| {
                if even { Color::blue() } else { Color::red() }
            })),
            
            text("Positive!")
                .opacity(is_positive.map(|pos| if pos { 1.0 } else { 0.3 })),
            
            hstack([
                button("+ Increment")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c + 1)
                    }),
                    
                button("- Decrement")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c - 1)
                    }),
            ])
            .spacing(10.0),
            
            button("Reset")
                .action({
                    let count = count.clone();
                    move |_| count.set(0)
                })
                .style(ButtonStyle::Secondary),
        ])
        .spacing(15.0)
        .padding(40.0)
        .alignment(.center)
    }
}
```

## Building and Running

Save your code and run:

```bash
cargo run
```

You should see a counter application with:
- A display showing the current count
- Text indicating whether the number is even or odd
- Conditional text for positive numbers
- Buttons to increment, decrement, and reset

## Understanding the Code Structure

### Views are Composable
```rust
vstack([         // Vertical stack
    text(...),   // Text component
    hstack([     // Horizontal stack
        button(...), // Button components
        button(...),
    ]),
    button(...),
])
```

### State is Reactive
- Changes to `count` automatically update all UI elements that depend on it
- No manual DOM manipulation required
- Type-safe state management

### Events are Handled Functionally
```rust
button("Click me")
    .action(move |_event| {
        // Handle the click event
        println!("Button clicked!");
    })
```

## Adding More Features

Let's enhance our app with additional features:

```rust
struct CounterApp;

impl View for CounterApp {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        let step = binding(1);
        
        vstack([
            text("Counter Application")
                .size(28.0)
                .weight(.bold),
                
            text(count.map(|c| format!("Count: {}", c)))
                .size(24.0),
                
            // Step size control
            hstack([
                text("Step size:"),
                stepper(step.clone())
                    .range(1..=10),
            ])
            .spacing(10.0),
            
            // Main counter controls
            hstack([
                button("- Decrease")
                    .action({
                        let count = count.clone();
                        let step = step.clone();
                        move |_| count.update(|c| c - step.get())
                    }),
                    
                button("+ Increase")
                    .action({
                        let count = count.clone();
                        let step = step.clone();
                        move |_| count.update(|c| c + step.get())
                    }),
            ])
            .spacing(10.0),
            
            // Additional controls
            hstack([
                button("Double")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c * 2)
                    }),
                    
                button("Half")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c / 2)
                    })
                    .disabled(count.map(|c| c == 0)),
                    
                button("Reset")
                    .action({
                        let count = count.clone();
                        move |_| count.set(0)
                    })
                    .style(ButtonStyle::Secondary),
            ])
            .spacing(10.0),
            
            // Statistics
            text(count.map(|c| {
                format!("Absolute value: {} | Square: {}", 
                       c.abs(), c * c)
            }))
            .size(14.0)
            .color(Color::secondary_text()),
        ])
        .spacing(15.0)
        .padding(40.0)
        .max_width(400.0)
        .alignment(.center)
    }
}
```

## Key Concepts Learned

1. **Reactive State**: Using `binding()` to create reactive state
2. **State Updates**: Using `set()`, `update()`, and `map()` methods
3. **Event Handling**: Using `.action()` to handle button clicks
4. **View Composition**: Combining multiple views with stacks
5. **Conditional UI**: Using reactive state to conditionally show/hide elements
6. **Computed Values**: Deriving new reactive values from existing state

## Common Patterns

### State Cloning for Closures
```rust
let count = count.clone();  // Clone for move into closure
move |_| count.update(|c| c + 1)
```

### Conditional Styling
```rust
.color(state.map(|s| if s { Color::green() } else { Color::red() }))
```

### Disabled States
```rust
.disabled(count.map(|c| c <= 0))
```

## Exercises

Try these exercises to reinforce your understanding:

1. Add a "Double Step" button that increases by `step * 2`
2. Add bounds checking to prevent the counter from going below 0 or above 100
3. Add a text field where users can input a specific value to set the counter
4. Create a history display showing the last 5 counter values

## Next Steps

In the next chapter, we'll dive deeper into the View system and learn about:
- The View trait in detail
- Custom view creation
- View modifiers and styling
- Performance considerations

---

# Chapter 4: Understanding the View System

The View system is the heart of WaterUI. Understanding how Views work is crucial for building efficient and maintainable applications. In this chapter, we'll explore the View trait in depth and learn how to create custom components.

## The View Trait

Every UI element in WaterUI implements the View trait:

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

This simple trait enables powerful composition patterns. Let's understand each part:

### `'static` Lifetime
All Views must have a `'static` lifetime, meaning they can't contain non-static references. This ensures Views can be stored and moved safely.

### `self` Parameter
Views consume themselves when building their body. This allows for zero-cost moves and ensures Views are used exactly once.

### Environment Parameter
The `&Environment` provides context and configuration to Views.

### Return Type
`impl View` allows returning any type that implements View, enabling flexible composition.

## Built-in Views

WaterUI provides many built-in Views:

### Text Views
```rust
text("Hello")                    // Static text
text(binding.map(|s| s))        // Reactive text
```

### Layout Views
```rust
vstack([view1, view2])          // Vertical stack
hstack([view1, view2])          // Horizontal stack
zstack([view1, view2])          // Overlay stack
```

### Control Views
```rust
button("Click me")              // Button
text_field(binding)             // Text input
toggle(binding)                 // Toggle switch
slider(binding)                 // Range slider
```

## Creating Custom Views

Let's create custom Views to encapsulate reusable UI patterns:

### Simple Custom View

```rust
struct WelcomeMessage {
    name: String,
}

impl View for WelcomeMessage {
    fn body(self, _env: &Environment) -> impl View {
        vstack([
            text("Welcome!")
                .size(24.0)
                .weight(.bold),
            text(format!("Hello, {}!", self.name))
                .color(Color::blue()),
        ])
        .spacing(10.0)
        .padding(20.0)
    }
}

// Usage
WelcomeMessage { name: "Alice".to_string() }
```

### Stateful Custom View

```rust
struct Counter {
    initial_value: i32,
}

impl View for Counter {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(self.initial_value);
        
        vstack([
            text(count.map(|c| format!("Count: {}", c)))
                .size(20.0),
                
            hstack([
                button("-")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c - 1)
                    }),
                    
                button("+")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|c| c + 1)
                    }),
            ])
            .spacing(10.0),
        ])
        .spacing(15.0)
    }
}

// Usage
Counter { initial_value: 0 }
```

### Parameterized Custom View

```rust
struct IconButton {
    icon: String,
    text: String,
    action: Box<dyn Fn() + 'static>,
}

impl View for IconButton {
    fn body(self, _env: &Environment) -> impl View {
        button(format!("{} {}", self.icon, self.text))
            .action(move |_| (self.action)())
            .style(ButtonStyle::Primary)
            .min_width(120.0)
    }
}

// Usage
IconButton {
    icon: "📁".to_string(),
    text: "Open File".to_string(),
    action: Box::new(|| println!("Opening file...")),
}
```

## View Modifiers

View modifiers allow you to customize the appearance and behavior of Views:

### Styling Modifiers
```rust
text("Hello")
    .size(18.0)                 // Font size
    .weight(.bold)              // Font weight
    .color(Color::blue())       // Text color
    .background(Color::yellow()) // Background color
```

### Layout Modifiers
```rust
view
    .padding(20.0)              // Add padding
    .margin(10.0)               // Add margin
    .width(200.0)               // Set width
    .height(100.0)              // Set height
    .alignment(.center)         // Set alignment
```

### Interaction Modifiers
```rust
view
    .on_tap(|| println!("Tapped!"))     // Handle tap
    .disabled(binding.map(|b| !b))      // Conditional disable
    .opacity(0.5)                       // Set transparency
```

## Conditional Views

Sometimes you need to show different Views based on state:

### Option Views
```rust
let user: Binding<Option<User>> = binding(None);

// Option<View> automatically implements View
user.map(|user_opt| {
    user_opt.map(|user| {
        text(format!("Welcome, {}", user.name))
    })
    // None case shows nothing
})
```

### Result Views
```rust
let result: Binding<Result<String, Error>> = binding(Ok("Success".to_string()));

result.map(|res| {
    match res {
        Ok(message) => text(message).color(Color::green()),
        Err(error) => text(format!("Error: {}", error)).color(Color::red()),
    }
})
```

### Boolean Conditions
```rust
let show_details = binding(false);

vstack([
    button("Toggle Details")
        .action({
            let show_details = show_details.clone();
            move |_| show_details.update(|b| !b)
        }),
        
    // Conditional view
    show_details.map(|show| {
        if show {
            Some(details_view())
        } else {
            None
        }
    }),
])
```

## Dynamic Views

For collections of data, use dynamic Views:

### List Views
```rust
let items: Binding<Vec<String>> = binding(vec![
    "Item 1".to_string(),
    "Item 2".to_string(),
    "Item 3".to_string(),
]);

vstack([
    text("My List"),
    
    // Dynamic list
    items.map(|items| {
        items.into_iter()
            .map(|item| {
                text(item)
                    .padding(5.0)
                    .background(Color::light_gray())
            })
            .collect::<Vec<_>>()
    }),
])
```

### For Each Pattern
```rust
struct TodoList {
    todos: Binding<Vec<Todo>>,
}

impl View for TodoList {
    fn body(self, _env: &Environment) -> impl View {
        let todos = self.todos;
        
        vstack([
            text("Todo List")
                .size(24.0)
                .weight(.bold),
                
            scroll(
                todos.map(|todos| {
                    todos.into_iter()
                        .enumerate()
                        .map(|(index, todo)| {
                            TodoItem {
                                todo,
                                on_toggle: {
                                    let todos = todos.clone();
                                    move || {
                                        todos.update(|todos| {
                                            todos[index].completed = !todos[index].completed;
                                        });
                                    }
                                },
                            }
                        })
                        .collect::<Vec<_>>()
                })
            ),
        ])
    }
}
```

## View Performance

Understanding View performance is important for responsive applications:

### View Creation Cost
Views are created fresh on each update. Keep View creation lightweight:

```rust
// Good: Lightweight View creation
impl View for MyView {
    fn body(self, _env: &Environment) -> impl View {
        text(self.message)  // Simple, fast creation
    }
}

// Avoid: Heavy computation in body()
impl View for MyView {
    fn body(self, _env: &Environment) -> impl View {
        // Don't do expensive work here!
        let result = expensive_computation();
        text(result)
    }
}
```

### Reactive Optimization
Use reactive computations for expensive operations:

```rust
// Good: Compute once, use reactively
let expensive_result = binding(initial_data)
    .map(|data| expensive_computation(data));

text(expensive_result.map(|result| format!("Result: {}", result)))
```

### Conditional Rendering
Only create Views that are actually shown:

```rust
// Good: Conditional creation
if show_expensive_view {
    Some(ExpensiveView { /* ... */ })
} else {
    None
}

// Avoid: Always creating then hiding
ExpensiveView { /* ... */ }
    .opacity(if show_expensive_view { 1.0 } else { 0.0 })
```

## Advanced View Patterns

### View Factories
```rust
fn create_card(title: &str, content: &str) -> impl View {
    vstack([
        text(title)
            .size(18.0)
            .weight(.bold),
        text(content)
            .color(Color::secondary_text()),
    ])
    .padding(15.0)
    .background(Color::card_background())
    .corner_radius(8.0)
}

// Usage
vstack([
    create_card("Title 1", "Content 1"),
    create_card("Title 2", "Content 2"),
])
```

### Generic Views
```rust
struct DataView<T: Display> {
    data: T,
    formatter: Box<dyn Fn(&T) -> String>,
}

impl<T: Display + 'static> View for DataView<T> {
    fn body(self, _env: &Environment) -> impl View {
        text((self.formatter)(&self.data))
    }
}
```

## Best Practices

1. **Keep Views Simple**: Each View should have a single responsibility
2. **Use Composition**: Build complex UIs from simple, reusable components
3. **Minimize State**: Only make state reactive when necessary
4. **Name Things Well**: Use descriptive names for custom Views
5. **Document Complex Views**: Add comments explaining complex logic

## Common Mistakes

1. **Heavy Computation in `body()`**: Move expensive operations to reactive computations
2. **Over-nesting**: Too many nested stacks can hurt performance
3. **Unnecessary State**: Not every variable needs to be reactive
4. **Missing Clone**: Forgetting to clone bindings before moving into closures

## Exercises

1. Create a `ProfileCard` View that takes a user object and displays name, avatar, and bio
2. Build a `ProgressBar` View with animated progress updates
3. Create a `TabView` component that manages multiple content views
4. Implement a `SearchableList` that filters items based on a search query

## Summary

The View system provides:
- Simple, composable building blocks
- Type-safe UI construction
- Reactive updates
- Reusable components
- Performance optimization opportunities

In the next chapter, we'll explore the Environment system and learn how to pass configuration and dependencies through your view hierarchy.

---

# Chapter 5: The Environment System

The Environment system is WaterUI's solution for dependency injection and configuration management. It allows you to pass data, themes, localization settings, and other dependencies through your view hierarchy in a type-safe manner.

## Understanding Environment

The Environment is a type-based storage system where each type serves as a unique key:

```rust
#[derive(Debug, Clone, Default)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
}
```

### Key Concepts

1. **Type-based Keys**: Each type can have at most one value in the environment
2. **Immutable Updates**: Environment changes create new instances
3. **Thread-Safe**: Uses `Rc<dyn Any>` for safe sharing
4. **Hierarchical**: Child views inherit parent environments

## Basic Usage

### Storing Values

```rust
// Create an environment with values
let env = Environment::new()
    .with(Theme::Dark)
    .with(Language::English)
    .with(DatabaseConfig {
        url: "localhost:5432".to_string(),
    });
```

### Retrieving Values

```rust
// In a View implementation
impl View for MyView {
    fn body(self, env: &Environment) -> impl View {
        // Get theme from environment
        let theme = env.get::<Theme>().unwrap_or(&Theme::Light);
        
        text("Hello")
            .color(theme.primary_color())
    }
}
```

### Updating Environment

```rust
// Add or replace a value
let updated_env = env.with(Theme::Light);

// Remove a value
let mut env_mut = env.clone();
env_mut.remove::<Theme>();
```

## Common Use Cases

### Theming

```rust
#[derive(Debug, Clone)]
pub struct Theme {
    pub primary_color: Color,
    pub secondary_color: Color,
    pub background_color: Color,
    pub text_color: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            primary_color: Color::rgb(0.0, 0.5, 1.0),
            secondary_color: Color::rgb(0.3, 0.3, 0.3),
            background_color: Color::rgb(0.1, 0.1, 0.1),
            text_color: Color::white(),
        }
    }
    
    pub fn light() -> Self {
        Self {
            primary_color: Color::rgb(0.0, 0.3, 0.8),
            secondary_color: Color::rgb(0.8, 0.8, 0.8),
            background_color: Color::white(),
            text_color: Color::black(),
        }
    }
}

// Using themes
struct ThemedButton {
    text: String,
}

impl View for ThemedButton {
    fn body(self, env: &Environment) -> impl View {
        let theme = env.get::<Theme>().unwrap_or(&Theme::light());
        
        button(self.text)
            .background(theme.primary_color)
            .color(theme.text_color)
    }
}
```

### Localization

```rust
#[derive(Debug, Clone)]
pub struct Localizer {
    current_language: Language,
    translations: HashMap<String, HashMap<Language, String>>,
}

impl Localizer {
    pub fn get(&self, key: &str) -> String {
        self.translations
            .get(key)
            .and_then(|translations| translations.get(&self.current_language))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
}

// Usage in views
struct LocalizedText {
    key: String,
}

impl View for LocalizedText {
    fn body(self, env: &Environment) -> impl View {
        let localizer = env.get::<Localizer>().unwrap();
        text(localizer.get(&self.key))
    }
}
```

### Application State

```rust
#[derive(Debug, Clone)]
pub struct AppState {
    pub user: Option<User>,
    pub preferences: UserPreferences,
    pub session_id: String,
}

impl View for UserProfile {
    fn body(self, env: &Environment) -> impl View {
        let app_state = env.get::<AppState>().unwrap();
        
        match &app_state.user {
            Some(user) => vstack([
                text(format!("Welcome, {}", user.name)),
                text(format!("Email: {}", user.email)),
            ]),
            None => text("Please log in"),
        }
    }
}
```

## Advanced Patterns

### Environment Modifiers

You can create Views that modify the environment for their children:

```rust
struct WithTheme<V> {
    theme: Theme,
    content: V,
}

impl<V: View> View for WithTheme<V> {
    fn body(self, env: &Environment) -> impl View {
        let new_env = env.clone().with(self.theme);
        
        // Use the Metadata component to provide new environment
        use_env(move |_| self.content).environment(new_env)
    }
}

// Usage
WithTheme {
    theme: Theme::dark(),
    content: my_content_view(),
}
```

### Environment Observers

Create Views that react to environment changes:

```rust
struct EnvironmentObserver<T, V> {
    content: V,
    _marker: PhantomData<T>,
}

impl<T: 'static + Clone, V: View> View for EnvironmentObserver<T, V> {
    fn body(self, env: &Environment) -> impl View {
        use_env(move |env| {
            if let Some(value) = env.get::<T>() {
                println!("Environment value changed: {:?}", value);
            }
            self.content
        })
    }
}
```

### Dependency Injection

```rust
// Define service traits
trait UserService: 'static {
    fn get_user(&self, id: u64) -> Result<User, Error>;
    fn update_user(&self, user: &User) -> Result<(), Error>;
}

// Concrete implementation
struct HttpUserService {
    base_url: String,
}

impl UserService for HttpUserService {
    fn get_user(&self, id: u64) -> Result<User, Error> {
        // HTTP request implementation
        todo!()
    }
    
    fn update_user(&self, user: &User) -> Result<(), Error> {
        // HTTP request implementation  
        todo!()
    }
}

// Using services in views
struct UserEditor {
    user_id: u64,
}

impl View for UserEditor {
    fn body(self, env: &Environment) -> impl View {
        let user_service = env.get::<Box<dyn UserService>>().unwrap();
        
        // Use service to load and update user
        // Implementation would involve reactive state management
        text("User Editor")
    }
}
```

## Environment Best Practices

### 1. Use Specific Types
Don't store generic types like `String` or `i32` directly. Create specific wrapper types:

```rust
// Good
#[derive(Debug, Clone)]
pub struct ApiEndpoint(pub String);

#[derive(Debug, Clone)]  
pub struct MaxRetries(pub u32);

// Avoid
env.with("https://api.example.com".to_string());  // Which string is this?
env.with(5u32);  // What does 5 represent?
```

### 2. Provide Defaults
Always provide sensible defaults for optional environment values:

```rust
impl View for MyView {
    fn body(self, env: &Environment) -> impl View {
        let theme = env.get::<Theme>()
            .cloned()
            .unwrap_or(Theme::default());
            
        // Use theme...
    }
}
```

### 3. Document Environment Dependencies
Document what environment values your Views expect:

```rust
/// A button that adapts to the current theme.
/// 
/// Environment dependencies:
/// - `Theme`: Used for colors and styling (optional, defaults to light theme)
/// - `Localizer`: Used for button text localization (optional)
struct AdaptiveButton {
    key: String,
}
```

### 4. Keep Environment Minimal
Only store values that multiple views need. Don't use Environment for local component state:

```rust
// Good: Shared configuration
env.with(Theme::dark())
   .with(ApiConfig { base_url: "..." })

// Avoid: Local component state
env.with(ButtonClickCount(0))  // This should be local state
```

## Plugin System Integration

The Environment system integrates with WaterUI's plugin system:

```rust
trait Plugin: Sized + 'static {
    fn install(self, env: &mut Environment);
    fn uninstall(self, env: &mut Environment);
}

// Example plugin
struct ThemePlugin {
    theme: Theme,
}

impl Plugin for ThemePlugin {
    fn install(self, env: &mut Environment) {
        env.insert(self.theme);
    }
    
    fn uninstall(self, env: &mut Environment) {
        env.remove::<Theme>();
    }
}

// Usage
let env = Environment::new()
    .install(ThemePlugin { theme: Theme::dark() })
    .install(LocalizationPlugin::new("en"));
```

## Testing with Environment

Environment makes testing easier by allowing dependency injection:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    struct MockUserService;
    
    impl UserService for MockUserService {
        fn get_user(&self, _id: u64) -> Result<User, Error> {
            Ok(User {
                id: 1,
                name: "Test User".to_string(),
                email: "test@example.com".to_string(),
            })
        }
        
        fn update_user(&self, _user: &User) -> Result<(), Error> {
            Ok(())
        }
    }
    
    #[test]
    fn test_user_profile_view() {
        let env = Environment::new()
            .with(Box::new(MockUserService) as Box<dyn UserService>);
            
        let view = UserProfile { user_id: 1 };
        
        // Test view with mock environment
        // (Testing framework would render and verify output)
    }
}
```

## Common Patterns

### Configuration Cascade
```rust
// Global config
let global_env = Environment::new()
    .with(GlobalConfig::default());

// Feature-specific config
let feature_env = global_env
    .with(FeatureFlags::enabled())
    .with(FeatureConfig::production());

// Component-specific config  
let component_env = feature_env
    .with(ComponentTheme::custom());
```

### Conditional Environment
```rust
let env = Environment::new();

let env = if is_debug_mode {
    env.with(DebugConfig::verbose())
} else {
    env.with(ProductionConfig::optimized())
};
```

### Environment Validation
```rust
impl View for MyView {
    fn body(self, env: &Environment) -> impl View {
        // Validate required environment values
        let api_config = env.get::<ApiConfig>()
            .expect("ApiConfig must be provided in environment");
            
        let theme = env.get::<Theme>()
            .unwrap_or(&Theme::default());
            
        // Use validated values...
    }
}
```

## Performance Considerations

1. **Cloning Cost**: Environment cloning is relatively cheap (uses `Rc`)
2. **Lookup Cost**: Type-based lookups are O(log n) but fast in practice
3. **Memory Usage**: Environment holds references, not copies of data

## Summary

The Environment system provides:
- Type-safe dependency injection
- Hierarchical configuration management  
- Plugin system integration
- Testability through mock dependencies
- Performance-efficient value sharing

Key takeaways:
- Use specific types as environment keys
- Provide sensible defaults
- Document environment dependencies
- Keep the environment focused on shared concerns
- Leverage environment for testing

In the next chapter, we'll explore reactive state management with the Nami library and learn how to build complex, interactive applications.

---

# Chapter 6: Reactive State Management

Reactive state management is what makes WaterUI applications feel alive. When data changes, the UI automatically updates to reflect those changes. This chapter explores WaterUI's reactive system powered by the Nami library.

## Understanding Reactivity

Reactivity in WaterUI follows these principles:

1. **Automatic Updates**: When state changes, dependent UI elements update automatically
2. **Minimal Updates**: Only affected parts of the UI re-render
3. **Type Safety**: All state operations are checked at compile time
4. **Performance**: Zero-cost abstractions with no runtime overhead

## Core Reactive Types

### Binding

A `Binding` is a mutable reactive value:

```rust
use waterui::{binding, Binding};

// Create a binding with initial value
let counter: Binding<i32> = binding(0);

// Get current value (non-reactive)
let current = counter.get();

// Set new value
counter.set(42);

// Update based on current value
counter.update(|current| current + 1);
```

### Signal

A `Signal` is a read-only reactive value derived from other reactive values:

```rust
// Create a signal that doubles the counter
let doubled = counter.signal().map(|n| n * 2);

// Signals can be chained
let formatted = doubled.map(|n| format!("Value: {}", n));
```

### Computed

A `Computed` is like a Signal but caches its value:

```rust
let expensive_computation = counter.computed(|&n| {
    // This only runs when counter changes
    expensive_function(n)
});
```

## Basic Reactive Patterns

### Simple State

```rust
struct SimpleCounter;

impl View for SimpleCounter {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        
        vstack([
            // Reactive text that updates when count changes
            text(count.signal().map(|n| format!("Count: {}", n))),
            
            button("Increment")
                .action({
                    let count = count.clone();
                    move |_| count.update(|n| n + 1)
                }),
        ])
    }
}
```

### Derived State

```rust
struct CalculatorDisplay;

impl View for CalculatorDisplay {
    fn body(self, _env: &Environment) -> impl View {
        let value = binding(0.0);
        
        // Derived states
        let is_positive = value.signal().map(|&v| v > 0.0);
        let formatted = value.signal().map(|&v| format!("{:.2}", v));
        let color = is_positive.map(|&pos| {
            if pos { Color::green() } else { Color::red() }
        });
        
        vstack([
            text(formatted)
                .size(24.0)
                .color(color),
                
            text(is_positive.map(|&pos| {
                if pos { "Positive" } else { "Negative or Zero" }
            })),
        ])
    }
}
```

## Complex State Management

### Structured State

```rust
#[derive(Debug, Clone)]
struct User {
    id: u64,
    name: String,
    email: String,
    age: u32,
}

#[derive(Debug, Clone)]
struct AppState {
    current_user: Option<User>,
    is_loading: bool,
    error_message: Option<String>,
}

struct UserProfile;

impl View for UserProfile {
    fn body(self, _env: &Environment) -> impl View {
        let state = binding(AppState {
            current_user: None,
            is_loading: false,
            error_message: None,
        });
        
        vstack([
            // Loading indicator
            state.signal()
                .map(|state| state.is_loading)
                .map(|is_loading| {
                    if is_loading {
                        Some(text("Loading..."))
                    } else {
                        None
                    }
                }),
                
            // User info or login prompt
            state.signal()
                .map(|state| state.current_user.clone())
                .map(|user_opt| {
                    match user_opt {
                        Some(user) => vstack([
                            text(format!("Name: {}", user.name)),
                            text(format!("Email: {}", user.email)),
                            text(format!("Age: {}", user.age)),
                        ]).into(),
                        None => text("Please log in").into(),
                    }
                }),
                
            // Error message
            state.signal()
                .map(|state| state.error_message.clone())
                .map(|error_opt| {
                    error_opt.map(|error| {
                        text(error)
                            .color(Color::red())
                    })
                }),
        ])
    }
}
```

### Collections and Lists

```rust
struct TodoApp;

impl View for TodoApp {
    fn body(self, _env: &Environment) -> impl View {
        let todos = binding(Vec::<Todo>::new());
        let new_todo_text = binding(String::new());
        
        vstack([
            text("Todo App")
                .size(24.0)
                .weight(.bold),
                
            // Add new todo
            hstack([
                text_field(new_todo_text.clone())
                    .placeholder("Enter new todo..."),
                    
                button("Add")
                    .action({
                        let todos = todos.clone();
                        let new_todo_text = new_todo_text.clone();
                        move |_| {
                            let text = new_todo_text.get();
                            if !text.is_empty() {
                                todos.update(|todos| {
                                    todos.push(Todo {
                                        id: todos.len() as u64,
                                        text: text.clone(),
                                        completed: false,
                                    });
                                });
                                new_todo_text.set(String::new());
                            }
                        }
                    }),
            ])
            .spacing(10.0),
            
            // Todo list
            scroll(
                todos.signal().map(|todos| {
                    todos.iter().enumerate().map(|(index, todo)| {
                        TodoItem {
                            todo: todo.clone(),
                            on_toggle: {
                                let todos = todos.clone();
                                move || {
                                    todos.update(|todos| {
                                        todos[index].completed = !todos[index].completed;
                                    });
                                }
                            },
                            on_delete: {
                                let todos = todos.clone();
                                move || {
                                    todos.update(|todos| {
                                        todos.remove(index);
                                    });
                                }
                            },
                        }
                    })
                    .collect::<Vec<_>>()
                })
            ),
        ])
        .spacing(15.0)
        .padding(20.0)
    }
}

#[derive(Debug, Clone)]
struct Todo {
    id: u64,
    text: String,
    completed: bool,
}

struct TodoItem {
    todo: Todo,
    on_toggle: Box<dyn Fn() + 'static>,
    on_delete: Box<dyn Fn() + 'static>,
}

impl View for TodoItem {
    fn body(self, _env: &Environment) -> impl View {
        hstack([
            toggle(binding(self.todo.completed))
                .on_change({
                    let on_toggle = self.on_toggle;
                    move |_| on_toggle()
                }),
                
            text(self.todo.text)
                .strikethrough(self.todo.completed)
                .color(if self.todo.completed { 
                    Color::secondary_text() 
                } else { 
                    Color::primary_text() 
                }),
                
            spacer(),
            
            button("Delete")
                .style(ButtonStyle::Destructive)
                .action({
                    let on_delete = self.on_delete;
                    move |_| on_delete()
                }),
        ])
        .padding(10.0)
        .background(Color::item_background())
    }
}
```

## Advanced Reactive Patterns

### Signal Combination

```rust
struct WeatherWidget;

impl View for WeatherWidget {
    fn body(self, _env: &Environment) -> impl View {
        let temperature = binding(22.0);
        let humidity = binding(65.0);
        let is_sunny = binding(true);
        
        // Combine multiple signals
        let comfort_level = temperature.signal()
            .zip(humidity.signal())
            .zip(is_sunny.signal())
            .map(|((temp, humidity), sunny)| {
                match (temp, humidity, sunny) {
                    (t, h, true) if t > 20.0 && t < 26.0 && h < 70.0 => "Perfect!",
                    (t, _, _) if t > 30.0 => "Too hot",
                    (t, _, _) if t < 15.0 => "Too cold",
                    (_, h, _) if h > 80.0 => "Too humid",
                    (_, _, false) => "Cloudy",
                    _ => "Okay",
                }
            });
        
        vstack([
            text(temperature.signal().map(|t| format!("Temperature: {}°C", t))),
            text(humidity.signal().map(|h| format!("Humidity: {}%", h))),
            text(is_sunny.signal().map(|s| if s { "☀️ Sunny" } else { "☁️ Cloudy" })),
            text(comfort_level.map(|level| format!("Comfort: {}", level)))
                .weight(.bold),
        ])
        .spacing(10.0)
    }
}
```

### Async State

```rust
struct AsyncDataView;

impl View for AsyncDataView {
    fn body(self, _env: &Environment) -> impl View {
        let data_state = binding(DataState::Loading);
        
        // Simulate async data loading
        let data_state_for_task = data_state.clone();
        task::spawn(async move {
            // Simulate network request
            task::sleep(Duration::from_secs(2)).await;
            
            match fetch_data().await {
                Ok(data) => data_state_for_task.set(DataState::Loaded(data)),
                Err(err) => data_state_for_task.set(DataState::Error(err.to_string())),
            }
        });
        
        data_state.signal().map(|state| {
            match state {
                DataState::Loading => vstack([
                    progress_indicator(),
                    text("Loading data..."),
                ]).into(),
                
                DataState::Loaded(data) => vstack([
                    text("Data loaded successfully!"),
                    text(format!("Items: {}", data.len())),
                ]).into(),
                
                DataState::Error(error) => vstack([
                    text("Failed to load data")
                        .color(Color::red()),
                    text(error)
                        .size(12.0)
                        .color(Color::secondary_text()),
                    button("Retry")
                        .action({
                            let data_state = data_state.clone();
                            move |_| data_state.set(DataState::Loading)
                        }),
                ]).into(),
            }
        })
    }
}

#[derive(Debug, Clone)]
enum DataState {
    Loading,
    Loaded(Vec<String>),
    Error(String),
}

async fn fetch_data() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Simulate API call
    Ok(vec!["Item 1".to_string(), "Item 2".to_string()])
}
```

### State Persistence

```rust
struct PersistentCounter;

impl View for PersistentCounter {
    fn body(self, _env: &Environment) -> impl View {
        // Load initial value from storage
        let count = binding(load_counter_from_storage().unwrap_or(0));
        
        // Save to storage whenever count changes
        let count_for_storage = count.clone();
        count.signal().observe(move |&new_count| {
            if let Err(e) = save_counter_to_storage(new_count) {
                eprintln!("Failed to save counter: {}", e);
            }
        });
        
        vstack([
            text("Persistent Counter"),
            text(count.signal().map(|n| format!("Count: {}", n))),
            
            hstack([
                button("-")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n - 1)
                    }),
                button("+")
                    .action({
                        let count = count.clone();
                        move |_| count.update(|n| n + 1)
                    }),
            ])
            .spacing(10.0),
        ])
        .spacing(15.0)
    }
}

fn load_counter_from_storage() -> Result<i32, Box<dyn std::error::Error>> {
    // Implementation would read from file, database, etc.
    Ok(42)
}

fn save_counter_to_storage(count: i32) -> Result<(), Box<dyn std::error::Error>> {
    // Implementation would write to file, database, etc.
    println!("Saving count: {}", count);
    Ok(())
}
```

## Performance Optimization

### Avoiding Unnecessary Updates

```rust
// Good: Only update when necessary
let expensive_computed = value.signal()
    .distinct()  // Only emit when value actually changes
    .map(|&val| expensive_computation(val))
    .computed();  // Cache the result

// Good: Batch updates
batch_update(|| {
    counter1.set(10);
    counter2.set(20);
    counter3.set(30);
    // All updates happen together
});
```

### Memory Management

```rust
// Store signals in structs to avoid recreating them
struct OptimizedView {
    counter: Binding<i32>,
    doubled: Signal<i32>,
    formatted: Signal<String>,
}

impl OptimizedView {
    fn new() -> Self {
        let counter = binding(0);
        let doubled = counter.signal().map(|&n| n * 2);
        let formatted = doubled.map(|&n| format!("Count: {}", n));
        
        Self { counter, doubled, formatted }
    }
}

impl View for OptimizedView {
    fn body(self, _env: &Environment) -> impl View {
        // Reuse pre-created signals
        text(self.formatted)
    }
}
```

## Testing Reactive Code

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_counter_increment() {
        let counter = binding(0);
        
        // Test initial value
        assert_eq!(counter.get(), 0);
        
        // Test update
        counter.update(|n| n + 1);
        assert_eq!(counter.get(), 1);
        
        // Test set
        counter.set(42);
        assert_eq!(counter.get(), 42);
    }
    
    #[test]
    fn test_signal_mapping() {
        let counter = binding(5);
        let doubled = counter.signal().map(|&n| n * 2);
        
        // Test signal computation
        assert_eq!(doubled.get(), 10);
        
        // Test reactive update
        counter.set(10);
        assert_eq!(doubled.get(), 20);
    }
}
```

## Common Patterns and Best Practices

### 1. Keep State Local
Only lift state up when it needs to be shared:

```rust
// Good: Local state for local concerns
struct LocalCounter;
impl View for LocalCounter {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);  // Local to this view
        // ...
    }
}
```

### 2. Use Computed for Expensive Operations
```rust
// Good: Cache expensive computations
let result = input.computed(|input| expensive_operation(input));

// Avoid: Recomputing on every access  
let result = input.map(|input| expensive_operation(input));
```

### 3. Handle Errors Gracefully
```rust
let result = async_operation()
    .map(|result| {
        match result {
            Ok(data) => DataView { data }.into(),
            Err(error) => ErrorView { error }.into(),
        }
    });
```

### 4. Minimize State Scope
```rust
// Good: Separate concerns
struct UserForm {
    name: Binding<String>,
    email: Binding<String>,
    is_submitting: Binding<bool>,
}

// Avoid: One large state object
struct AppState {
    everything: EverythingStruct,
}
```

## Summary

Reactive state management in WaterUI provides:
- Automatic UI updates
- Type-safe state operations
- Performance optimization through caching
- Composable reactive computations
- Easy testing and debugging

Key takeaways:
- Use `binding()` for mutable state
- Use `.signal()` and `.map()` for derived values
- Use `.computed()` for expensive operations
- Keep state local when possible
- Handle async operations gracefully
- Test reactive logic thoroughly

In the next chapter, we'll explore component composition and learn how to build complex UIs from simple, reusable parts.