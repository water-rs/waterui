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

// ❌ Old reactive text (verbose)
text(name.get().map(|n| format!("Hello, {}!", n)))

// ✅ Better reactive text (use text! macro)
text!("Hello, {}!", name)

// Styled text
"Important!"
    .size(24.0)
    .weight(.bold)
    .color(Color::red())
```

### Control Views
```rust,ignore
// Button
button("Click me")
    .action(|_| println!("Clicked!"))

// Text field
let input = binding(String::new());
text_field(input)
    .placeholder("Enter text...")

// Toggle switch
let enabled = binding(false);
toggle(enabled)
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

### Function Views (Recommended for Stateless Components)

**Best Practice**: Use functions returning `impl View` for components without internal state:

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

Only use the View trait when your component needs to store state:

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
                .action({
                    let count = count.clone();
                    let step = self.step;
                    move |_| count.update(|c| c + step)
                }),
        ))
    }
}

// Usage
CounterWidget { 
    initial_value: 0,
    step: 5,
}
```

### Parameterized Views

Views can accept various types of parameters:

```rust,ignore
struct ProfileCard {
    user: User,
    show_email: bool,
    on_edit: Box<dyn Fn() + 'static>,
}

impl View for ProfileCard {
    fn body(self, _env: &Environment) -> impl View {
        vstack((
            text(self.user.name)
                .size(20.0)
                .weight(.bold),
                
            if self.show_email {
                Some(text(self.user.email).color(Color::secondary()))
            } else {
                None
            },
            
            button("Edit Profile")
                .action(move |_| (self.on_edit)()),
        ))
        .spacing(10.0)
        .padding(15.0)
        .background(Color::card_background())
        .corner_radius(8.0)
    }
}
```

### Stateful Custom Views

Views can manage their own internal state:

```rust,ignore
struct ExpandableSection {
    title: String,
    content: String,
}

impl View for ExpandableSection {
    fn body(self, _env: &Environment) -> impl View {
        let is_expanded = binding(false);
        
        vstack((
            // Header with expand/collapse button
            hstack((
                text(self.title)
                    .weight(.bold),
                spacer(),
                button(if is_expanded { "▲" } else { "▼" })
                .action({
                    let is_expanded = is_expanded.clone();
                    move |_| is_expanded.update(|expanded| !expanded)
                }),
            ))
            .padding(10.0),
            
            // Expandable content
            when(is_expanded, || {
                text(self.content.clone())
                    .padding(10.0)
                    .background(Color::light_gray())
            }),
        ))
        .background(Color::white())
        .border(Color::gray(), 1.0)
        .corner_radius(4.0)
    }
}
```

## View Composition Patterns

### Container Views

Views that wrap and enhance other views:

```rust,ignore
struct Card<V> {
    content: V,
    title: Option<String>,
}

impl<V: View> View for Card<V> {
    fn body(self, _env: &Environment) -> impl View {
        vstack((
            self.title.map(|title| {
                title
                    .weight(.bold)
                    .padding(10.0)
                    .background(Color::card_header())
            }),
            
            self.content
                .padding(15.0),
        ))
        .background(Color::card_background())
        .corner_radius(8.0)
        .shadow(2.0)
    }
}

// Usage
Card {
    title: Some("User Profile".to_string()),
    content: UserProfileView { user },
}
```

### Conditional Views

Views that render different content based on conditions:

```rust,ignore
struct ConditionalView<T, C, E> {
    condition: T,
    content: C,
    else_content: E,
}

impl<T, C, E> View for ConditionalView<T, C, E> 
where
    T: Signal<Output = bool>,
    C: View,
    E: View,
{
    fn body(self, _env: &Environment) -> impl View {
        self.condition.map(move |condition| {
            if condition {
                AnyView::new(self.content)
            } else {
                AnyView::new(self.else_content)
            }
        })
    }
}

// Helper function
fn conditional<T, C, E>(condition: T, content: C, else_content: E) -> ConditionalView<T, C, E> {
    ConditionalView { condition, content, else_content }
}

// Usage
conditional(
    user_logged_in,
    DashboardView { user },
    LoginView {},
)
```

### List Views

Views that render collections of data:

```rust,ignore
struct ListView<T, F> {
    items: Signal<Vec<T>>,
    item_builder: F,
}

impl<T, F, V> View for ListView<T, F>
where
    T: Clone + 'static,
    F: Fn(T) -> V + 'static,
    V: View,
{
    fn body(self, _env: &Environment) -> impl View {
        scroll(
            self.items.map(move |items| {
                items.into_iter()
                    .map(|item| (self.item_builder)(item))
                    .collect::<Vec<_>>()
            })
        )
    }
}

// Usage
ListView {
    // ❌ Wrong: Using .get() breaks reactivity
    // items: todos.get(),
    
    // ✅ Better: Use reactive iteration
    items: todos,
    item_builder: |todo| TodoItem { todo },
}
```

## Advanced View Patterns

### Generic Views

Views that work with multiple types:

```rust,ignore
struct DataView<T> {
    data: Signal<Option<T>>,
    loading_view: Box<dyn Fn() -> AnyView + 'static>,
    content_builder: Box<dyn Fn(T) -> AnyView + 'static>,
    empty_view: Box<dyn Fn() -> AnyView + 'static>,
}

impl<T: Clone + 'static> View for DataView<T> {
    fn body(self, _env: &Environment) -> impl View {
        self.data.map(move |data| {
            match data {
                Some(content) => (self.content_builder)(content),
                None => (self.empty_view)(),
            }
        })
    }
}
```

### Async Views

Views that handle asynchronous operations:

```rust,ignore
struct AsyncView<T> {
    future: Pin<Box<dyn Future<Output = T> + Send + 'static>>,
    initial_state: AsyncState<T>,
}

#[derive(Clone)]
enum AsyncState<T> {
    Loading,
    Loaded(T),
    Error(String),
}

impl<T: Clone + 'static> View for AsyncView<T> {
    fn body(self, _env: &Environment) -> impl View {
        let state = binding(self.initial_state);
        
        // Spawn the future
        let state_clone = state.clone();
        task::spawn(async move {
            match self.future.await {
                Ok(result) => state_clone.set(AsyncState::Loaded(result)),
                Err(e) => state_clone.set(AsyncState::Error(e.to_string())),
            }
        });
        
        state.get().map(|state| {
            match state {
                AsyncState::Loading => loading_spinner().into(),
                AsyncState::Loaded(data) => content_view(data).into(),
                AsyncState::Error(error) => error_view(error).into(),
            }
        })
    }
}
```

## View Performance

Understanding View performance is crucial for building responsive applications:

### Performance Characteristics

1. **View Creation**: Views are created fresh on each update. Keep creation lightweight.
2. **Reactive Updates**: Only views that depend on changed signals re-render.
3. **Memory Usage**: Views are temporary and get garbage collected after rendering.

### Performance Best Practices

#### 1. Keep View Creation Cheap

```rust,ignore
// Good: Simple, fast creation
impl View for MyView {
    fn body(self, _env: &Environment) -> impl View {
        text(self.message)
    }
}

// Avoid: Expensive computation in body()
impl View for MyView {
    fn body(self, _env: &Environment) -> impl View {
        let result = expensive_computation();  // Don't do this!
        text(result)
    }
}
```

#### 2. Use Computed Values for Expensive Operations

```rust,ignore
// Good: Compute once, reuse
let expensive_result = data.computed(|data| expensive_computation(data));
text(expensive_result.map(|result| format!("Result: {}", result)))

// Avoid: Recomputing every time
text(data.map(|data| {
    let result = expensive_computation(data);  // Computed every update!
    format!("Result: {}", result)
}))
```

#### 3. Minimize Signal Dependencies

```rust,ignore
// Good: Only depends on name
text(user.get().map(|user| user.name.clone()))

// Avoid: Depends on entire user object
text(user.get().map(|user| user.name.clone()))  // Same effect, but...
// If you access user.email elsewhere, both will update when user changes
```

#### 4. Use AnyView Judiciously

```rust,ignore
// Good: Type-erased when necessary
fn conditional_view(condition: bool) -> AnyView {
    if condition {
        complex_view().into()
    } else {
        simple_view().into()
    }
}

// Avoid: Unnecessary type erasure
fn simple_view() -> AnyView {  // No need for AnyView here
    "Hello".into()
}
```

## Testing Custom Views

Testing Views involves verifying their structure and behavior:

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_welcome_message_content() {
        let view = WelcomeMessage {
            name: "Test User".to_string(),
        };
        
        // In a real test framework, you'd render the view
        // and verify its structure
        let env = Environment::new();
        let rendered = view.body(&env);
        
        // Test would verify the rendered structure contains
        // expected text and styling
    }
    
    #[test]
    fn test_expandable_section_interaction() {
        let view = ExpandableSection {
            title: "Test Section".to_string(),
            content: "Test Content".to_string(),
        };
        
        // Test would simulate user interaction and verify
        // that the section expands and collapses correctly
    }
}
```

## Common View Patterns

### 1. Builder Pattern

```rust,ignore
struct Button {
    text: String,
    style: ButtonStyle,
    action: Option<Box<dyn Fn() + 'static>>,
    disabled: bool,
}

impl Button {
    fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: ButtonStyle::Primary,
            action: None,
            disabled: false,
        }
    }
    
    fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }
    
    fn action(mut self, action: impl Fn() + 'static) -> Self {
        self.action = Some(Box::new(action));
        self
    }
    
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl View for Button {
    fn body(self, _env: &Environment) -> impl View {
        // Implementation...
    }
}

// Usage
Button::new("Save")
    .style(ButtonStyle::Primary)
    .action(|| save_data())
    .disabled(form_invalid)
```

### 2. Configuration Structs

```rust,ignore
#[derive(Default)]
struct ListConfig {
    item_spacing: f64,
    show_separators: bool,
    selection_enabled: bool,
    max_height: Option<f64>,
}

struct ConfigurableList<T> {
    items: Vec<T>,
    config: ListConfig,
    item_builder: Box<dyn Fn(&T) -> AnyView>,
}

// Usage
ConfigurableList {
    items: my_items,
    config: ListConfig {
        item_spacing: 10.0,
        show_separators: true,
        ..Default::default()
    },
    item_builder: Box::new(|item| item_view(item)),
}
```

## Summary

In this chapter, you learned:

- ✅ The View trait is simple but powerful: just one method returning another View
- ✅ Custom views enable code reuse and encapsulation
- ✅ Views can be parameterized, stateful, and generic
- ✅ Composition patterns like containers, conditionals, and lists are fundamental
- ✅ Performance depends on keeping View creation lightweight and using reactive patterns correctly
- ✅ Testing Views requires thinking about structure and behavior

Key takeaways:
- Views are the building blocks of all UIs in WaterUI
- Simple composition patterns enable complex UIs
- Performance comes from understanding the rendering model
- Custom views make your code more maintainable and reusable

In the next chapter, we'll explore the Environment system and learn how to pass configuration and dependencies through your view hierarchy in a type-safe way.

Ready to learn about dependency injection? Let's dive into [The Environment System](04-environment.md)!