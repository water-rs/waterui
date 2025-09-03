# WaterUI Best Practices

This chapter summarizes the recommended patterns and best practices for building WaterUI applications efficiently and maintainably.

## View Creation Best Practices

### Use Functions for Stateless Components

**✅ Recommended**: Use functions returning `impl View` for components without internal state:

```rust,ignore
// Clean and simple - no View trait needed!
fn user_card(name: &str, email: &str) -> impl View {
    vstack((
        text(name).weight(.bold),
        text(email).color(Color::secondary()),
    ))
    .padding(10.0)
    .background(Color::card())
}

// Usage - functions are automatically views!
user_card("Alice", "alice@example.com")
```

**Why?** 
- Simpler syntax
- Less boilerplate
- Functions and closures automatically implement `View` trait
- Perfect for lazy initialization

### Use View Trait Only When Needed

**✅ Use View trait when**:
- Your component has configuration stored in struct fields
- You need to pass complex state between methods
- You're building a reusable widget library

```rust,ignore
// Only use View trait when struct holds configuration
struct PaginatedList {
    items: Vec<String>,
    items_per_page: usize,
    current_page: usize,
}

impl View for PaginatedList {
    fn body(self, _env: &Environment) -> impl View {
        // Complex logic using struct fields
        let start = self.current_page * self.items_per_page;
        let end = (start + self.items_per_page).min(self.items.len());
        
        vstack(
            self.items[start..end]
                .iter()
                .map(|item| text(item))
                .collect()
        )
    }
}
```

## Reactive Programming Best Practices

### Use the `s!` Macro for Reactive Computations

**✅ Recommended**: Use `s!` macro for clean reactive expressions:

```rust,ignore
use nami::s;

let width = binding(100);
let height = binding(50);

// Clean and readable
let area = s!(width * height);
let perimeter = s!((width + height) * 2);
let aspect_ratio = s!(width as f64 / height as f64);

// Works with method calls too
let name = binding("alice".to_string());
let capitalized = s!(name.to_uppercase());
let first_char = s!(name.chars().next());
```

**❌ Avoid**: Using `.get()` for reactive computations:

```rust,ignore
// WRONG - breaks reactivity!
let area = width.get() * height.get(); // This is not reactive!

// Also wrong - unnecessarily verbose
let area = width.get()
    .zip(height.get())
    .map(|(w, h)| w * h);
```

### Use the `text!` Macro for Reactive Text

**✅ Recommended**: Use `text!` macro for reactive formatted text:

```rust,ignore
let name = binding("Alice");
let score = binding(100);

// Clean reactive text formatting
text!("Hello, {}!", name)
text!("Score: {} points", score)
text!("{} scored {} points", name, score)
```

**❌ Avoid**: Using `.get()` with `format!` - breaks reactivity:

```rust,ignore
// WRONG - breaks reactivity!
text(format!("Hello, {}!", name.get()))

// Also wrong - unnecessarily verbose
text(name.get().map(|n| format!("Hello, {}!", n)))
```

## State Management Best Practices

### Keep State Local

**✅ Recommended**: Keep state as local as possible:

```rust,ignore
fn counter_button() -> impl View {
    let count = binding(0);  // Local state
    
    button(text!("Clicked {} times", count))
        .action({
            let count = count.clone();
            move |_| count.update(|c| c + 1)
        })
}
```

### Group Related State

**✅ Recommended**: Use structs to group related state:

```rust,ignore
#[derive(Clone)]
struct FormData {
    name: String,
    email: String,
    age: u32,
}

fn contact_form() -> impl View {
    let form = binding(FormData {
        name: String::new(),
        email: String::new(),
        age: 18,
    });
    
    // Single source of truth for form state
    vstack((
        text_field(form.clone().map(
            |f| f.name.clone(),
            |mut f, name| { f.name = name; f }
        )),
        // ...
    ))
}
```

## Component Composition Best Practices

### Use Builder Pattern for Complex Components

**✅ Recommended**: Provide a fluent API for configuration:

```rust,ignore
fn button(label: impl Into<String>) -> ButtonBuilder {
    ButtonBuilder {
        label: label.into(),
        style: ButtonStyle::Primary,
        disabled: false,
        action: None,
    }
}

struct ButtonBuilder {
    label: String,
    style: ButtonStyle,
    disabled: bool,
    action: Option<Box<dyn Fn()>>,
}

impl ButtonBuilder {
    fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }
    
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    
    fn action(mut self, action: impl Fn() + 'static) -> Self {
        self.action = Some(Box::new(action));
        self
    }
    
    fn build(self) -> impl View {
        // Build the actual button view
    }
}

// Usage
button("Save")
    .style(ButtonStyle::Success)
    .disabled(s!(form.is_empty()))
    .action(|| save_data())
```

### Prefer Composition Over Inheritance

**✅ Recommended**: Compose views from smaller components:

```rust,ignore
fn card(content: impl View) -> impl View {
    content
        .padding(15.0)
        .background(Color::white())
        .corner_radius(8.0)
        .shadow(2.0)
}

fn header(title: &str) -> impl View {
    text(title)
        .size(24.0)
        .weight(.bold)
        .padding_bottom(10.0)
}

// Compose them together
fn user_profile(user: &User) -> impl View {
    card(vstack((
        header(&user.name),
        text(&user.bio),
        text(&user.email).color(Color::secondary()),
    )))
}
```

## Performance Best Practices

### Use `computed()` for Expensive Operations

**✅ Recommended**: Cache expensive computations:

```rust,ignore
let data = binding(large_dataset);

// Cached - only recomputes when data changes
let analysis = data.computed(|data| {
    expensive_analysis(data)  // Only runs when needed
});

// Use the computed value multiple times without recomputation
text!("Result: {}", analysis)
text!("Summary: {}", s!(analysis.summary()))
```

### Avoid Creating Views in Loops

**✅ Recommended**: Pre-create views when possible:

```rust,ignore
// Good: Create views once
let items: Vec<impl View> = data.iter()
    .map(|item| item_view(item))
    .collect();

vstack(items)
```

**❌ Avoid**: Creating new closures in render:

```rust,ignore
// Avoid: Creates new closures every render
vstack(
    data.iter().map(|item| {
        button(&item.name).action(move || {  // New closure each time
            println!("Clicked {}", item.id);
        })
    })
)
```

## Error Handling Best Practices

### Use Explicit Error States

**✅ Recommended**: Model errors explicitly in your state:

```rust,ignore
#[derive(Clone)]
enum DataState<T> {
    Loading,
    Success(T),
    Error(String),
}

fn data_view() -> impl View {
    let state = binding(DataState::Loading);
    
    when(state.clone(), |state| match state {
        DataState::Loading => loading_spinner().into_view(),
        DataState::Success(data) => data_list(data).into_view(),
        DataState::Error(msg) => error_message(msg).into_view(),
    })
}
```

## Testing Best Practices

### Test Views as Functions

**✅ Recommended**: Structure views for easy testing:

```rust,ignore
// Easy to test
fn calculate_total(items: &[Item)) -> f64 {
    items.iter().map(|i| i.price).sum()
}

fn invoice_view(items: Vec<Item>) -> impl View {
    let total = calculate_total(&items);  // Testable logic separated
    
    vstack((
        // ... item list ...
        text!("Total: ${:.2}", total),
    ))
}

#[test]
fn test_calculate_total() {
    let items = vec![
        Item { price: 10.0 },
        Item { price: 20.0 },
    ];
    assert_eq!(calculate_total(&items), 30.0);
}
```

## Summary

Key takeaways:
- **Use functions** for stateless views, View trait only when needed
- **Use `s!` macro** for reactive computations
- **Use `text!` macro** for reactive text
- **Keep state local** and group related state
- **Compose views** from smaller components
- **Cache expensive operations** with `computed()`
- **Model errors explicitly** in your state
- **Structure for testability** by separating logic from views

Following these best practices will lead to cleaner, more maintainable, and more performant WaterUI applications!