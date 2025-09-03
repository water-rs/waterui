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
use waterui::prelude::*;

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
use waterui::prelude::*;
use waterui_text::{text, Text};

fn text_component_examples() -> impl View {
    let count = binding(0);
    let name = binding("Alice".to_string());

    vstack((
        // Basic Text component
        "Styleable text content",

        // Reactive text with text! macro
        text!("Count: {}", count),
        text!("Hello, {}!", name),

        // Text component with styling
        "Styled text"
            .size(20.0)
            .font(Font::default().weight(FontWeight::Bold)),

        button("Increment")
            .action({
                let count = count.clone();
                move |_| count.update(|c| c + 1)
            }),
    ))
}
```

## Text Styling with the Text Component

Only the `Text` component (created with `text()` function or `Text::new()`) supports styling. Labels (`&str`, `String`, `Str`) are rendered without styling.

### Font Properties

```rust
use waterui::prelude::*;
use waterui_text::{text, Font, FontWeight};

fn font_styling_demo() -> impl View {
    vstack((
        // Labels - no styling available
        "Default label text",

        // Text components - styling available
        text("Large text")
            .size(24.0),

        text("Small text")
            .size(12.0),

        text("Bold text")
            .font(Font::default().weight(FontWeight::Bold)),

        text("Styleable text")
            .font(Font::default()
                .size(18.0)
                .weight(FontWeight::SemiBold)),
    ))
}
```

### When to Use Labels vs Text Components

Choose the right approach based on your needs:

```rust
use waterui::prelude::*;
use waterui_text::{text, Font, FontWeight};

fn choosing_text_type() -> impl View {
    vstack((
        // Use labels for simple, static text
        "Static heading",
        "Simple description text",

        // Use Text component for styled text
        text("Styled heading")
            .font(Font::default()
                .size(20.0)
                .weight(FontWeight::Bold)),

        // Use text! macro for reactive content
        {
            let count = binding(42);
            text!("Dynamic count: {}", count)
        },
    ))
}
```

## Reactive Text with the text! Macro

The `text!` macro creates reactive Text components that automatically update when underlying data changes:

```rust
use waterui::prelude::*;
use waterui_text::text;

fn reactive_text_demo() -> impl View {
    let count = binding(0);
    let name = binding("Alice".to_string());
    let temperature = binding(22.5);

    vstack((
        // Reactive formatted text
        text!("Count: {}", count),
        text!("Hello, {}!", name),
        text!("Temperature: {:.1}°C", temperature),

        // Reactive with computed expressions using s! macro
        text!("Status: {}", s!(if count > 5 { "High" } else { "Low" })),

        hstack((
            button("Increment").action({
                let count = count.clone();
                move |_| count.update(|c| c + 1)
            }),
            button("Reset").action({
                let count = count.clone();
                move |_| count.set(0)
            }),
        )),
    ))
}
```

### Formatting Best Practices

Always use `text!` macro for reactive text, never `format!` macro which loses reactivity:

```rust
use waterui::prelude::*;
use waterui_text::text;

fn formatting_best_practices() -> impl View {
    let user_count = binding(42);
    let status = binding("Active".to_string());

    vstack((
        // ✅ CORRECT: Use text! for reactive content
        text!("Users: {} ({})", user_count, status),

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
use waterui::prelude::*;
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

### Font Customization

```rust
fn rich_text_demo() -> impl View {
    vstack((
        rich_text([
            text_span("Regular text, ")
                .color(Color::primary()),
            text_span("bold text, ")
                .font_weight(FontWeight::Bold)
                .color(Color::blue()),
            text_span("italic text, ")
                .font_style(FontStyle::Italic)
                .color(Color::green()),
            text_span("and underlined text.")
                .underline(true)
                .color(Color::red()),
        )),

        rich_text([
            text_span("Temperature: "),
            text_span("25°C")
                .font_weight(FontWeight::Bold)
                .color(Color::orange()),
            text_span(" ("),
            text_span("Normal")
                .color(Color::green()),
            text_span(")"),
        )),
    ))
    .spacing(15.0)
}
```

## Performance Considerations

### Efficient Reactive Updates

```rust
use waterui::prelude::*;
use waterui_text::text;

fn efficient_text_updates() -> impl View {
    let counter = binding(0);

    vstack((
        // ✅ GOOD: Reactive text with text! macro
        text!("Counter: {}", counter),

        // ✅ GOOD: Computed reactive text
        text!("Status: {}", s!(match counter {
            0..=10 => "Low",
            11..=50 => "Medium",
            _ => "High"
        })),

        button("Increment").action({
            let counter = counter.clone();
            move |_| counter.update(|c| c + 1)
        }),
    ))
}
```

## Summary

WaterUI's text system provides:

- **Labels (`&str`, `String`, `Str`)**: Simple, unstyled text for static content
- **Text Component**: Reactive, styleable text with font customization
- **text! macro**: Reactive formatted text that updates automatically
- **Font API**: Comprehensive typography control for Text components

### Key Guidelines

- Use **labels** for simple, static text without styling
- Use **Text component** when you need styling or reactive updates
- Use **text! macro** for formatted reactive content
- Never use `format!` with reactive values - it breaks reactivity
- Choose the simplest approach that meets your needs

### Quick Reference

```rust
use waterui::prelude::*;
use waterui_text::{text, Text, Font, FontWeight};

fn text_reference() -> impl View {
    let count = binding(42);

    vstack((
        // Label: static, no styling
        "Simple text",

        // Text: static, with styling
        text("Styled text").size(18.0),

        // Text: reactive, formatted
        text!("Count: {}", count),

        // Text: reactive, custom formatting
        Text::display(count),
        Text::format(count, |n| format!("#{:03}", n)),
    ))
}
```

Next: [Forms](10-forms.md)
