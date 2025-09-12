# Text and Typography

Text is fundamental to any UI framework. WaterUI provides two distinct approaches for displaying text: **labels** for static text without styling, and the **Text component** for reactive text with rich styling capabilities.

## Understanding Labels vs Text Components

### Labels: Simple Static Text

In WaterUI, several types implement the `View` trait directly and are called "labels":

- `&'static str` - String literals
- `String` - Owned strings
- `Str` - WaterUI's optimized string type

Labels are rendered as simple, unstyled text and are perfect for static content:

```rust
use waterui::View;
use waterui::component::layout::stack::vstack;

fn label_examples() -> impl View {
    vstack((
        // String literal as label
        "Simple static text",

        // String variable as label
        String::from("Dynamic string as label"),

        // Multi-line text
        r#"Multi-line text with
line breaks"#,
    ))
}
```

### Text Component: Reactive and Styleable

The `Text` component provides reactive updates and rich styling options:

```rust
use waterui::View;
use waterui::reactive::binding;
use waterui::component::layout::stack::vstack;
use waterui::component::button::button;
use waterui_text::{text, Text};

fn text_component_examples() -> impl View {
    let count = binding(0);
    let name = binding("Alice".to_string());

    vstack((
        // Basic Text component
        "Styleable text content",

        // Reactive text with text! macro
        text!("Count: {count}"),
        text!("Hello, {name}!"),

        // Text component with styling
        Text::new("Styled text").size(20.0),

        button("Increment")
            .action_with(&count,|c| c.increment(1)),
    ))
}
```

## Text Styling with the Text Component

Only the `Text` component (created with `text()` function or `Text::new()`) supports styling. Labels (`&str`, `String`, `Str`) are rendered without styling.

### Font Properties

```rust
use waterui::View;
use waterui::component::layout::stack::vstack;
use waterui_text::text;

fn font_styling_demo() -> impl View {
    vstack((
        // Labels - no styling available
        "Default label text",

        // Text components - styling available
        text("Large text")
            .size(24.0),

        text("Small text")
            .size(12.0),

        // Bold/weight not yet available
        text("Styleable text").size(18.0),
    ))
}
```

### When to Use Labels vs Text Components

Choose the right approach based on your needs:

```rust
use waterui::{View,Binding};
use waterui::component::layout::stack::vstack;
use waterui_text::text;

fn choosing_text_type() -> impl View {
    vstack((
        // Use labels for simple, static text
        "Static heading",
        "Simple description text",

        // Use Text component for styled text
        text("Styled heading").size(20.0),

        // Use text! macro for reactive content
        {
            let count = Binding::int(42);
            text!("Dynamic count: {count}")
        },
    ))
}
```

## Reactive Text with the text! Macro

The `text!` macro creates reactive Text components that automatically update when underlying data changes:

```rust
use waterui::{View,Binding};
use waterui::reactive::{binding};
use waterui::component::layout::stack::{vstack, hstack};
use waterui::component::button::button;
use waterui_text::text;

fn reactive_text_demo() -> impl View {
    let name = binding(String::from("Alice"));
    let temperature:Binding<f64> = binding(22.5);

    vstack((
        // Reactive formatted text
        text!("Hello, {name}!"),
        text!("Temperature: {temperature:.1}°C"),

        // Reactive with computed expressions using s! macro
        text!("Status: {}", temperature.map(|t| t>30 ).select("High","Low")),

        hstack((
            button("Increment").action_with(&temperature,|t| t.increment(1.0)),
            button("Reset").action_with(&temperature,|t| t.set(22.5)),
        )),
    ))
}
```

You can see more convenicence method of `Binding` at nami's API reference
### Formatting Best Practices

Always use `text!` macro for reactive text, never `format!` macro which loses reactivity:

```rust
use waterui::View;
use waterui::component::layout::stack::vstack;
use waterui::reactive::binding;
use waterui_text::text;

fn formatting_best_practices() -> impl View {
    let user_count = binding(42);
    let status = binding(String::from("Active"));

    vstack((
        // ✅ CORRECT: Use text! for reactive content
        text!("Users: {user_count} ({status})"),

        // ❌ WRONG: .get() breaks reactivity!
        // text(format!("Users: {} ({})", user_count.get(), status.get()))
        // This creates static text that won't update when signals change!

        // ✅ CORRECT: Use labels for static text
        "Status Dashboard",

        // ✅ CORRECT: Use text() for static styleable content
        text("Styleable heading").size(18.0),
    ))
}
```

## Advanced Text Component Features

The Text component provides additional capabilities beyond basic labels:

### Text Display Options

```rust
use waterui::View;
use waterui::component::layout::stack::vstack;
use waterui::reactive::binding;
use waterui_text::{text, Text};

fn text_display_demo() -> impl View {
    vstack((
        // Display formatting with different value types
        Text::display(binding(42)),
        Text::display(binding(3.14159)),
        Text::display(binding(true)),

        // Custom formatting with formatters
        {
            let price = binding(29.99);
            Text::format(price, |value| format!("${:.2}", value))
        },
    ))
}
```