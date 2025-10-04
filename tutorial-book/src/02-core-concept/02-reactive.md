# Nami - Reactive System of WaterUI

Reactive state management is the heart of interactive WaterUI applications. When your data changes, the UI automatically updates to reflect those changes. This chapter teaches you how to master WaterUI's reactive system powered by the **nami** crate.

## Understanding the Foundation: Signal Trait

Everything in nami's reactive system implements the `Signal` trait. This trait represents **any value that can be observed and computed**:

```rust,ignore
pub trait Signal: Clone + 'static {
    type Output;
    
    // Get the current value
    fn get(&self) -> Self::Output;
    
    // Watch for changes (used internally by the UI system)
    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard;
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

// Even literals implement Signal automatically! (but not all!)
let literal_string = "Hello World";   // Already a Signal!
let literal_number = 100;             // Already a Signal!
```

### 2. Binding - Mutable Reactive State

`Binding<T>` is for **mutable** reactive state that can be changed and will notify the UI:

```rust,ignore
use waterui::prelude::*;

// Create mutable reactive state
let counter = binding(0);
let name = binding("Alice".to_string());

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