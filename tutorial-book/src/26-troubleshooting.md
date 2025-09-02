# Troubleshooting

This chapter provides solutions to common problems encountered when developing WaterUI applications, from compilation errors to runtime issues and performance problems.

## Common Compilation Errors

### Dependency Resolution Issues

#### Error: "cannot find crate `waterui`"

```bash
Error: cannot find crate `waterui` in registry
```

**Solution:**

```toml
# Cargo.toml - Ensure correct version and features
[dependencies]
waterui = { version = "0.1", features = ["gtk4"] }
nami = "0.1"

# Or use path dependency for local development
waterui = { path = "../waterui" }
```

```bash
# Update dependencies
cargo update

# Clean and rebuild
cargo clean
cargo build
```

#### Error: "feature `gtk4` not found"

```bash
Error: feature `gtk4` is not available in crate `waterui`
```

**Solution:**

```toml
# Check available features
[dependencies]
waterui = { version = "0.1", default-features = false, features = ["gtk4"] }

# For web target
[target.'cfg(target_arch = "wasm32")'.dependencies]
waterui = { version = "0.1", features = ["web"] }

# For desktop target
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
waterui = { version = "0.1", features = ["gtk4"] }
```

### Macro Expansion Errors

#### Error: "cannot find macro `s!`"

```rust,ignore
// ❌ Missing import
fn broken_component() -> impl View {
    let count = binding(0);
    text!("Count: {}", s!(count)) // Error: cannot find macro `s!`
}
```

**Solution:**

```rust,ignore
// ✅ Correct imports
use waterui::*;
use nami::*; // This provides the s! macro

fn working_component() -> impl View {
    let count = binding(0);
    text!("Count: {}", s!(count))
}
```

#### Error: "text! macro not found"

```rust,ignore
// ❌ Missing waterui import
use nami::*;

fn broken_text() -> impl View {
    text!("Hello") // Error: text! macro not found
}
```

**Solution:**

```rust,ignore
// ✅ Import both waterui and nami
use waterui::*; // Provides text! macro
use nami::*;    // Provides s! macro

fn working_text() -> impl View {
    text!("Hello")
}
```

### Type System Issues

#### Error: "expected `impl View`, found `Option<T>`"

```rust,ignore
// ❌ Incorrect conditional rendering
fn broken_conditional(show: Binding<bool>) -> impl View {
    if show.get() {
        Some(text!("Visible"))
    } else {
        None // Type mismatch
    }
}
```

**Solution:**

```rust,ignore
// ✅ Use s! macro for reactive conditionals
fn working_conditional(show: Binding<bool>) -> impl View {
    s!(if show {
        Some(text!("Visible"))
    } else {
        None
    })
}

// Or use consistent Option types
fn alternative_conditional(show: Binding<bool>) -> impl View {
    if show.get() {
        Some(text!("Visible"))
    } else {
        Some(empty_view()) // Both branches return Some
    }
}
```

#### Error: "cannot infer type for generic parameter"

```rust,ignore
// ❌ Type inference failure
fn broken_generic() -> impl View {
    let items = binding(vec![)); // Empty vec, type unknown
    vstack(s!(items.iter().map(|item| text!(item)).collect()))
}
```

**Solution:**

```rust,ignore
// ✅ Explicit type annotation
fn working_generic() -> impl View {
    let items: Binding<Vec<String>> = binding(vec![)); // Explicit type
    vstack(s!(items.iter().map(|item| text!(item)).collect()))
}

// Or initialize with typed data
fn alternative_generic() -> impl View {
    let items = binding(vec!["item1".to_string())); // Type inferred
    vstack(s!(items.iter().map(|item| text!(item)).collect()))
}
```

## Runtime Errors

### Reactive State Issues

#### Problem: "Signal updates not propagating"

```rust,ignore
// ❌ Incorrect signal usage
fn broken_reactive() -> impl View {
    let count = binding(0);
    let doubled = count.get() * 2; // Not reactive!
    
    vstack((
        text!("Count: {}", count),
        text!("Doubled: {}", doubled), // Won't update
        button("+1", move || count.set(count.get() + 1)),
    ))
}
```

**Solution:**

```rust,ignore
// ✅ Use s! macro for reactive computations
fn working_reactive() -> impl View {
    let count = binding(0);
    let doubled = s!(count * 2); // Reactive computation
    
    vstack((
        text!("Count: {}", count),
        text!("Doubled: {}", doubled), // Will update
        button("+1", move || count.set(count.get() + 1)),
    ))
}
```

#### Problem: "Memory leaks from circular references"

```rust,ignore
// ❌ Circular reference
struct ProblematicComponent {
    self_ref: Binding<Option<ProblematicComponent>>,
}

impl ProblematicComponent {
    fn new() -> Self {
        let component = Self {
            self_ref: binding(None),
        };
        component.self_ref.set(Some(component.clone())); // Circular reference!
        component
    }
}
```

**Solution:**

```rust,ignore
// ✅ Use weak references or restructure
use std::sync::{Arc, Weak};

struct SafeComponent {
    parent: Option<Weak<SafeComponent>>,
    children: Binding<Vec<Arc<SafeComponent>>>,
}

impl SafeComponent {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            parent: None,
            children: binding(vec![)),
        })
    }
    
    fn add_child(self: &Arc<Self>, child: Arc<SafeComponent>) {
        // Set weak reference to parent
        child.parent = Some(Arc::downgrade(self));
        
        // Add to children
        self.children.update(|children| children.push(child));
    }
}
```

### UI Rendering Issues

#### Problem: "Components not updating visually"

```rust,ignore
// ❌ Modifying state without using binding methods
struct BrokenState {
    items: Vec<String>,
}

fn broken_update(state: Binding<BrokenState>) -> impl View {
    vstack((
        text!("Items: {}", state.items.len()),
        button("Add Item", move || {
            // ❌ This doesn't trigger updates
            let mut current = state.get();
            current.items.push("New item".to_string());
            // Missing: state.set(current);
        }),
    ))
}
```

**Solution:**

```rust,ignore
// ✅ Use binding update methods
fn working_update(state: Binding<BrokenState>) -> impl View {
    vstack((
        text!("Items: {}", s!(state.items.len())),
        button("Add Item", move || {
            // ✅ Use update method for atomic changes
            state.update(|state| {
                state.items.push("New item".to_string());
            });
        }),
    ))
}
```

#### Problem: "Layout not responding to content changes"

```rust,ignore
// ❌ Static layout calculations
fn broken_layout() -> impl View {
    let content = binding("Short".to_string());
    let fixed_width = 100.0; // Static width
    
    text_field("Content", content.clone())
        .frame_width(fixed_width) // Doesn't adapt to content
}
```

**Solution:**

```rust,ignore
// ✅ Dynamic layout calculations
fn working_layout() -> impl View {
    let content = binding("Short".to_string());
    let dynamic_width = s!(content.len() as f64 * 8.0 + 20.0); // Adaptive width
    
    text_field("Content", content.clone())
        .frame_width(dynamic_width)
        .frame_min_width(100.0)
        .frame_max_width(400.0)
}
```

## Performance Issues

### Slow Rendering

#### Problem: "App becomes sluggish with large lists"

```rust,ignore
// ❌ Rendering all items at once
fn slow_large_list(items: Binding<Vec<String>>) -> impl View {
    scroll_view(
        vstack(s!(items.iter().map(|item| {
            // Creates thousands of views
            expensive_item_view(item)
        }).collect()))
    )
}

fn expensive_item_view(item: &str) -> impl View {
    // Complex view with many nested components
    vstack((
        text!(item),
        image(format!("https://api.placeholder.com/150x150?text={}", item)),
        hstack((
            button("Edit", || {}),
            button("Delete", || {}),
            button("Share", || {}),
        )),
    ))
}
```

**Solution:**

```rust,ignore
// ✅ Implement virtualization
fn fast_large_list(items: Binding<Vec<String>>) -> impl View {
    let visible_range = binding(0..50); // Show only 50 items
    let scroll_offset = binding(0.0);
    
    scroll_view(
        vstack(s!({
            let range = visible_range.get();
            let visible_items = &items.get()[range];
            visible_items.iter().map(|item| {
                optimized_item_view(item)
            }).collect()
        }))
    )
    .on_scroll({
        let visible_range = visible_range.clone();
        move |offset: f64| {
            let start_index = (offset / 60.0) as usize; // Assuming 60px per item
            let end_index = start_index + 50;
            visible_range.set(start_index..end_index);
        }
    })
}

fn optimized_item_view(item: &str) -> impl View {
    // Simplified view with lazy loading
    hstack((
        text!(item),
        lazy_image(format!("https://api.placeholder.com/150x150?text={}", item)),
        action_buttons(),
    ))
    .frame_height(60.0) // Fixed height for virtualization
}
```

#### Problem: "Frequent re-computations"

```rust,ignore
// ❌ Expensive computation in signal
fn expensive_computation(data: Binding<Vec<i32>>) -> impl View {
    let result = s!({
        // Expensive operation runs on every data change
        data.iter().map(|&x| x * x).sum::<i32>()
    });
    
    text!("Sum of squares: {}", result)
}
```

**Solution:**

```rust,ignore
// ✅ Memoized computation
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

lazy_static! {
    static ref COMPUTATION_CACHE: Arc<Mutex<HashMap<u64, i32>>> = 
        Arc::new(Mutex::new(HashMap::new()));
}

fn memoized_computation(data: Binding<Vec<i32>>) -> impl View {
    let result = s!({
        let hash = calculate_hash(data.as_slice());
        
        if let Some(&cached) = COMPUTATION_CACHE.lock().unwrap().get(&hash) {
            cached
        } else {
            let result = data.iter().map(|&x| x * x).sum::<i32>();
            COMPUTATION_CACHE.lock().unwrap().insert(hash, result);
            result
        }
    });
    
    text!("Sum of squares: {}", result)
}

fn calculate_hash<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    t.hash(&mut hasher);
    hasher.finish()
}
```

### Memory Usage

#### Problem: "Memory usage keeps growing"

```rust,ignore
// ❌ Creating new bindings in render
fn memory_leak() -> impl View {
    // Creates new binding on every render!
    let counter = binding(0);
    
    vstack((
        text!("Count: {}", counter),
        button("+1", move || counter.update(|c| *c += 1)),
    ))
}
```

**Solution:**

```rust,ignore
// ✅ Move bindings outside render function
struct Counter {
    value: Binding<i32>,
}

impl Counter {
    fn new() -> Self {
        Self {
            value: binding(0),
        }
    }
}

impl View for Counter {
    fn body(self) -> impl View {
        vstack((
            text!("Count: {}", self.value),
            button("+1", {
                let value = self.value.clone();
                move || value.update(|c| *c += 1)
            }),
        ))
    }
}

fn fixed_memory_usage() -> impl View {
    Counter::new()
}
```

## Platform-Specific Issues

### GTK4 Backend Issues

#### Problem: "GTK initialization fails"

```bash
Error: Failed to initialize GTK: No display available
```

**Solution:**

```rust,ignore
// Check for display availability
use gtk4::prelude::*;

fn safe_gtk_init() -> Result<(), Box<dyn std::error::Error>> {
    // Set up environment for headless testing
    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        std::env::set_var("DISPLAY", ":99"); // Use virtual display
    }
    
    gtk4::init()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    safe_gtk_init()?;
    
    let app = gtk4::Application::new(
        Some("com.example.app"),
        gtk4::gio::ApplicationFlags::DEFAULT_FLAGS,
    );
    
    app.run();
    Ok(())
}
```

#### Problem: "CSS styling not applying"

```rust,ignore
// ❌ Incorrect CSS selector
fn broken_styling() -> impl View {
    text!("Styled text")
        .css("text { color: red; }") // Wrong selector
}
```

**Solution:**

```rust,ignore
// ✅ Correct CSS selector and application
fn working_styling() -> impl View {
    text!("Styled text")
        .style_class("custom-text")
        .css(".custom-text { color: red; font-weight: bold; }")
}

// Or use inline styles
fn inline_styling() -> impl View {
    text!("Styled text")
        .color(Color::red())
        .font_weight(FontWeight::Bold)
}
```

### Web Backend Issues

#### Problem: "WASM module fails to load"

```bash
Error: WebAssembly.instantiate(): Import #0 module="env" function="__linear_memory" error: global import must be a number or WebAssembly.Global object
```

**Solution:**

```toml
# Cargo.toml - Configure WASM properly
[lib]
crate-type = ["cdylib"]

[dependencies.wasm-bindgen]
version = "0.2"
features = [
  "serde-serialize",
]

[dependencies.web-sys]
version = "0.3"
features = [
  "console",
  "Document",
  "Element",
  "HtmlElement",
  "Window",
]
```

```rust,ignore
// Set panic hook for better error messages
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    
    // Initialize your app
    init_app();
}
```

#### Problem: "Large WASM bundle size"

```toml
# Cargo.toml - Optimize for size
[profile.release]
opt-level = "s"          # Optimize for size
lto = true               # Enable Link Time Optimization
codegen-units = 1        # Better optimization
panic = "abort"          # Smaller binary
strip = true             # Remove debug symbols

[profile.wasm-release]
inherits = "release"
opt-level = "z"          # Aggressive size optimization
```

```bash
# Post-processing optimization
wasm-pack build --target web --release
wasm-opt -Oz pkg/app_bg.wasm -o pkg/app_bg.wasm
```

## Debugging Techniques

### Debug Logging

```rust,ignore
use waterui::*;
use nami::*;

// Debug signal changes
fn debug_signals() -> impl View {
    let counter = binding(0);
    let doubled = s!({
        let value = counter * 2;
        println!("Computing doubled: {} -> {}", counter.get(), value);
        value
    });
    
    vstack((
        text!("Counter: {}", counter),
        text!("Doubled: {}", doubled),
        button("+1", {
            let counter = counter.clone();
            move || {
                let old_value = counter.get();
                counter.update(|c| *c += 1);
                println!("Counter updated: {} -> {}", old_value, counter.get());
            }
        }),
    ))
}

// Debug view rendering
fn debug_rendering<V: View>(name: &str, view: V) -> impl View {
    println!("Rendering component: {}", name);
    let start = std::time::Instant::now();
    let result = view;
    let elapsed = start.elapsed();
    println!("Component {} rendered in {:?}", name, elapsed);
    result
}
```

### Error Boundaries

```rust,ignore
use waterui::*;
use nami::*;

// Catch and display errors gracefully
fn error_boundary<V: View, F>(view: V, fallback: F) -> impl View 
where
    F: Fn(String) -> Box<dyn View>,
{
    // This would be implemented in the framework
    // For now, use Result-based error handling
    view
}

// Usage
fn safe_component() -> impl View {
    error_boundary(
        potentially_failing_component(),
        |error| Box::new(vstack((
            text!("Error: {}", error),
            button("Retry", || {
                // Retry logic
            }),
        )))
    )
}

fn potentially_failing_component() -> impl View {
    // Component that might fail
    text!("This might fail")
}
```

### Performance Profiling

```rust,ignore
use std::time::Instant;

// Profile component render times
macro_rules! profile {
    ($name:expr, $block:block) => {{
        let start = Instant::now();
        let result = $block;
        let duration = start.elapsed();
        if duration.as_millis() > 16 { // Slower than 60 FPS
            println!("SLOW: {} took {:?}", $name, duration);
        }
        result
    }};
}

// Usage
fn profiled_component() -> impl View {
    profile!("complex_calculation", {
        vstack((
            expensive_computation(),
            large_list_rendering(),
            complex_layout(),
        ))
    })
}
```

## Getting Help

### Diagnostic Information

When reporting issues, include:

1. **WaterUI Version**: `cargo tree | grep waterui`
2. **Rust Version**: `rustc --version`
3. **Platform**: OS and desktop environment
4. **Backend**: GTK4, Web, etc.
5. **Minimal Reproduction**: Smallest code that shows the issue

### Common Resources

- **Documentation**: Check the official WaterUI docs
- **Examples**: Look at example projects in the repository
- **Community**: Join the WaterUI Discord/forum
- **Issues**: Search existing GitHub issues

### Creating Bug Reports

```markdown
## Bug Report

### Description
Brief description of the issue

### Steps to Reproduce
1. Create a new WaterUI project
2. Add the following code...
3. Run with `cargo run`
4. Observe the error

### Expected Behavior
What should happen

### Actual Behavior
What actually happens

### Environment
- WaterUI version: 0.1.0
- Rust version: 1.70.0
- OS: Ubuntu 22.04
- Backend: GTK4

### Minimal Example
\```rust
use waterui::*;
use nami::*;

fn main() {
    // Minimal code that reproduces the issue
}
\```
```

By following these troubleshooting guidelines and solutions, you'll be able to resolve most common issues encountered during WaterUI development and create more robust applications.
