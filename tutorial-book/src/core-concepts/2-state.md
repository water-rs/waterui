# Reactive State Management

Reactive state management is the heart of interactive WaterUI applications. When your data changes, the UI automatically updates to reflect those changes. This chapter teaches you how to master WaterUI's reactive system powered by the **nami** crate.

## Understanding the Foundation: Signal Trait

Everything in nami's reactive system implements the `Signal` trait. This trait represents **any value that can be observed and computed**:

```rust,ignore
pub trait Signal: Clone + 'static {
    type Output;
    
    // Get the current value
    fn get(&self) -> Self::Output;
    
    // Watch for changes (used internally by the UI system)
    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> impl WatcherGuard;
}
```

**Key insight**: A `Signal` represents a reactive value that knows how to:
1. **Compute** its current value (`get()`)
2. **Notify** observers when it changes (`watch()`)

## Types of Signals

There are several types that implement `Signal`, each serving different purposes:

### 1. Constants - Never Change

```rust,ignore
use nami::constant;

let fixed_name = constant("WaterUI");  // Never changes
let fixed_number = constant(42);       // Never changes

// Even literals implement Signal automatically!
let literal_string = "Hello World";   // Already a Signal!
let literal_number = 100;             // Already a Signal!
```

### 2. Binding - Mutable Reactive State

`Binding<T>` is for **mutable** reactive state that can be changed and will notify the UI:

```rust,ignore
use waterui::{binding, Binding};

// Create mutable reactive state
let counter: Binding<i32> = binding(0);
let name: Binding<String> = binding("Alice".to_string());

// Set new values (triggers UI updates)
counter.set(42);
name.set("Bob".to_string());
```

### 3. Computed Signals - Derived from Other Signals

These are created by transforming other signals using SignalExt methods:

```rust,ignore
use nami::SignalExt;

let first_name = binding("Alice".to_string());
let last_name = binding("Smith".to_string());

// Create computed signals that update automatically
let full_name = first_name.zip(last_name).map(|(first, last)| {
    format!("{} {}", first, last)
});

let name_length = first_name.map(|name| name.len());
```

## ⚠️ WARNING: The Dangers of `.get()`

**`.get()` is the #1 reactivity killer!** Here's why it's dangerous:

```rust,ignore
let name = binding("Alice".to_string());
let age = binding(25);

// ❌ WRONG: Using .get() breaks reactivity
let broken_message = format!("Hello {}, you are {}", name.get(), age.get());
text(broken_message); // This will NEVER update when name or age change!

// ✅ CORRECT: Keep reactive chain intact  
let reactive_message = s!("Hello {name}, you are {age}");
text(reactive_message); // This updates automatically when name or age change
```

**When you call `.get()`:**
- You extract a **snapshot** of the current value
- The reactive connection is **permanently broken** 
- UI will **never update** even when the original signal changes
- You lose all the benefits of the reactive system

**Only use `.get()` when you absolutely need the raw value outside reactive contexts** (like debugging, logging, or interfacing with non-reactive APIs).

## The Golden Rule of Reactivity

**🔑 Avoid `.get()` and reactivity is preserved automatically!**

The reactive system in nami is designed to "just work" - as long as you don't call `.get()`, reactivity flows naturally through your computations. The system automatically tracks dependencies when you use:

- Methods on bindings: `counter.increment(1)`, `items.push(value)`, `text.append(" World")`  
- SignalExt methods: `.map()`, `.zip()`, `.cached()`, `.computed()`
- The `s!` macro for strings: `s!("Hello {name}!")`

## Working with Bindings - Mutable Signals

Now that you understand signals, let's dive into `Binding<T>` - the mutable reactive state container:

### Basic Operations

```rust,ignore
let counter = binding(0);

// Set new values (triggers UI updates)
counter.set(42);

// Bindings automatically provide their current value in reactive contexts
// No need to extract values with .get() - just use the binding directly!
```

### Type-Specific Convenience Methods

Nami provides specialized methods for different types to make common operations more ergonomic:

#### Integer Bindings

```rust,ignore
let counter = binding(0);

// Convenient arithmetic operations
counter.increment(1);     // counter += 1
counter.decrement(2);     // counter -= 2
counter.set(10);
```

#### Boolean Bindings

```rust,ignore
let is_enabled = binding(false);

// Toggle between true/false
is_enabled.toggle();

// Logical NOT operation
let is_disabled = !is_enabled; // Creates a new reactive binding
```

#### String Bindings

```rust,ignore
let text = binding(String::from("Hello"));

// Append text
text.append(" World");
text.clear();  // Empty the string
```

#### Vector Bindings

```rust,ignore
let items = binding(vec![1, 2, 3]);

// Collection operations
items.push(4);              // Add to end
items.insert(1, 99);        // Insert at index
let last = items.pop();      // Remove and return last
items.clear();               // Remove all elements

// For sortable vectors
let sortable = binding(vec![3, 1, 4, 1, 5]);
sortable.sort();             // Sort in-place
```

## Creating Computed Signals with SignalExt

All signals get powerful transformation methods through the `SignalExt` trait:

### Basic Transformations

```rust,ignore
use nami::SignalExt;

let numbers = binding(vec![1, 2, 3, 4, 5]);

// Transform the data
let doubled = numbers.map(|nums| {
    nums.iter().map(|&n| n * 2).collect::<Vec<_>>()
});

// Single value transformations
let count = numbers.map(|nums| nums.len());
let sum = numbers.map(|nums| nums.iter().sum::<i32>());
```

### Combining Multiple Signals

```rust,ignore
let a = binding(10);
let b = binding(20);

// Combine two signals
let sum = a.zip(b).map(|(x, y)| x + y);
let product = a.zip(b).map(|(x, y)| x * y);
let complex = a.zip(b).map(|(x, y)| x * 2 + y / 2);
```

### Performance Optimizations

```rust,ignore
let expensive_data = binding(vec![1, 2, 3, 4, 5]);

// Cache expensive computations (only recomputes when data changes)
let sum = expensive_data.cached().map(|nums| {
    // Expensive operation here
    nums.iter().sum::<i32>()
});
```

## The `s!` Macro - Reactive String Formatting

The `s!` macro from nami is a specialized macro for **string formatting** with automatic variable capture from reactive signals:

```rust,ignore
use nami::s;
use waterui::{binding, text};

let name = binding("Alice".to_string());
let age = binding(25);
let score = binding(95.5);

// ✅ s! macro for reactive string formatting with automatic capture
let greeting = s!("Hello {name}!");                    // Captures 'name' automatically
let info = s!("Name: {name}, Age: {age}");             // Multiple variables  
let detailed = s!("{name} is {age} years old");        // Clean, readable syntax

// Use with text to display
text(greeting);     // Automatically updates when 'name' changes
text(info);         // Updates when either 'name' or 'age' changes

// You can also use positional arguments
let positioned = s!("Hello {}, you are {} years old", name, age);

// The s! macro is specifically for string formatting - 
// for other reactive computations, use SignalExt methods
```

## Advanced Features

### Mutable Access Guard

```rust,ignore
let data = binding(vec![1, 2, 3]);

// Get mutable access that automatically updates on drop
let mut guard = data.get_mut();
guard.push(4);
guard.sort();
// Updates are sent when guard is dropped
```

### Filtered/Constrained Bindings

```rust,ignore
let temperature = binding(25);

// Create a binding constrained to a range
let safe_temp = temperature.range(0..=100);

// Custom filters
let even_numbers = binding(0);
let only_even = even_numbers.filter(|&n| n % 2 == 0);
```

### Debounced Bindings

```rust,ignore
use std::time::Duration;

let search_query = binding(String::new());

// Only update after user stops typing for 300ms
let debounced_search = search_query.debounced(Duration::from_millis(300));
```

### Working with Optional Values

```rust,ignore
let maybe_name: Binding<Option<String>> = binding(None);

// Provide default when None
let display_name = maybe_name.unwrap_or_else(|| "Anonymous".to_string());

// Transform the inner value if present
let maybe_upper = maybe_name.map(|opt| opt.map(|s| s.to_uppercase()));
```

## Key Principles

1. **Everything is a Signal** - Constants, bindings, computed values all implement the same trait
2. **Reactivity flows automatically** - Don't use `.get()` and the reactive chain stays intact
3. **Use binding methods** - `increment()`, `toggle()`, `push()`, etc. maintain reactivity
4. **Use SignalExt for transformations** - `.map()`, `.zip()`, `.cached()` preserve reactive flow
5. **Use `s!` macro for reactive strings** - Automatic variable capture with dependency tracking
6. **Use `text!` instead of `format!`** - `format!` breaks reactivity, `text!` preserves it
7. **Cache expensive computations** - Use `.cached()` for performance without losing reactivity

## Next Steps

This covers the fundamentals of nami's reactive system. For complete documentation of all available methods and advanced features, visit [docs.rs/nami](https://docs.rs/nami).

Next: [Component Composition](06-composition.md)