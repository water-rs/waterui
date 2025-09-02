# Reactive State Management

Reactive state management is the heart of interactive WaterUI applications. When your data changes, the UI automatically updates to reflect those changes. This chapter teaches you how to master WaterUI's reactive system.

## Core Reactive Types

### Binding - Mutable State

```rust,ignore
use waterui::{binding, Binding};

// Create mutable reactive state
let counter: Binding<i32> = binding(0);

// Read current value
let current = counter.get();

// Set new value
counter.set(42);

// Update based on current value
counter.update(|n| n + 1);
```

### The `s!` Macro - Best Practice for Reactive Computations

The `s!` macro from nami is the recommended way to create reactive computations:

```rust,ignore
use nami::s;
use waterui::{binding, text!};

let a = binding(5);
let b = binding(10);

// Best practice: Use s! macro for reactive computations
let sum = s!(a + b);           // Automatically updates when a or b change
let product = s!(a * b);       // Clean, readable syntax
let complex = s!(a * 2 + b / 2); // Complex expressions work too!

// Use in UI with text! macro
text!("Sum: {}", sum)
text!("Product: {}", product)

// The s! macro also works with method calls
let name = binding("Alice".to_string());
let uppercase = s!(name.to_uppercase());
let length = s!(name.len());
```

### Signal - Derived Values (Alternative Approach)

While `s!` is preferred, you can also use the signal API directly:

```rust,ignore
let counter = binding(10);

// Manual approach (more verbose)
let doubled = counter.signal().map(|&n| n * 2);
let formatted = doubled.map(|&n| format!("Value: {}", n));

// Better: Use s! and text! macros
let doubled = s!(counter * 2);
text!("Value: {}", doubled)

### Computed - Cached Expensive Operations

```rust,ignore
let data = binding(vec![1, 2, 3, 4, 5));

// Only recomputes when data changes
let sum = data.computed(|nums| {
    nums.iter().sum::<i32>()  // Expensive operation
});
```

## Summary

WaterUI's reactive state management provides automatic UI updates, type-safe operations, and performance through caching.

Next: [Component Composition](06-composition.md)
