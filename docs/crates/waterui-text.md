# WaterUI Text Components

**Version**: 0.1.0  
**Location**: `components/text/`

## Overview

`waterui-text` provides comprehensive text rendering and formatting capabilities for the WaterUI framework. It includes support for fonts, attributed text, links, internationalization, and locale-aware formatting.

## Core Components

### Text Component

The main text rendering component with configurable styling:

```rust
use waterui_text::{Text, TextConfig};

let text = Text::new("Hello, World!")
    .font(Font::system().size(16.0))
    .color(Color::BLACK);
```

### TextConfig

Configuration structure for text components:

```rust
pub struct TextConfig {
    /// The text content to be displayed
    pub content: Computed<Str>,
    /// The font styling to apply to the text
    pub font: Computed<Font>,
}
```

## Font System

### Font Types and Styling

```rust
use waterui_text::font::Font;

// System fonts
let system_font = Font::system()
    .size(14.0)
    .weight(FontWeight::Bold)
    .style(FontStyle::Italic);

// Custom fonts
let custom_font = Font::custom("Helvetica")
    .size(16.0)
    .weight(FontWeight::Medium);

// Font families with fallbacks
let font_family = Font::family(&["SF Pro", "Helvetica", "Arial"])
    .size(12.0);
```

### Font Properties

- **Size**: Point size for text rendering
- **Weight**: Thin, Light, Regular, Medium, Bold, Heavy
- **Style**: Normal, Italic, Oblique
- **Family**: Font family names with fallback support
- **Features**: OpenType features and variations

## Attributed Text

Rich text formatting with multiple styles within a single text component:

```rust
use waterui_text::attributed::{AttributedText, TextAttribute};

let attributed = AttributedText::new()
    .append("Normal text ")
    .append_with_attributes(
        "bold text",
        &[TextAttribute::Weight(FontWeight::Bold)]
    )
    .append(" and ")
    .append_with_attributes(
        "italic text",
        &[TextAttribute::Style(FontStyle::Italic)]
    );
```

### Text Attributes

- **Font attributes**: Size, weight, style, family
- **Color attributes**: Foreground and background colors
- **Decoration**: Underline, strikethrough, overline
- **Spacing**: Letter spacing, line height
- **Effects**: Shadow, outline, glow

## Link Components

Interactive text elements for navigation and actions:

```rust
use waterui_text::link::{Link, LinkStyle};

let link = Link::new("Visit our website")
    .url("https://example.com")
    .style(LinkStyle {
        color: Color::BLUE,
        hover_color: Color::DARK_BLUE,
        underline: true,
    })
    .on_click(|url| {
        // Handle link activation
        open_url(url);
    });
```

### Link Features

- **URL handling**: Support for web URLs and custom schemes
- **Hover states**: Visual feedback on interaction
- **Accessibility**: Screen reader and keyboard support
- **Custom actions**: Execute code instead of navigation

## Localization Support

### Locale-Aware Formatting

```rust
use waterui_text::locale::{Locale, Formatter};

let locale = Locale::new("en_US");
let formatter = Formatter::new(locale);

// Number formatting
let number_text = formatter.format_number(1234.56);
// Result: "1,234.56"

// Date formatting  
let date_text = formatter.format_date(date, DateStyle::Medium);
// Result: "Jan 15, 2025"

// Currency formatting
let currency_text = formatter.format_currency(99.99, "USD");
// Result: "$99.99"
```

### Text Direction Support

Automatic handling of left-to-right (LTR) and right-to-left (RTL) text:

```rust
let text = Text::new("مرحبا بالعالم")  // Arabic text
    .direction(TextDirection::Auto)      // Automatically detect RTL
    .alignment(TextAlignment::Natural);  // Use natural alignment for direction
```

## Macros

Convenient macros for creating text elements:

```rust
use waterui_text::macros::*;

// Simple text creation
let simple = text!("Hello, World!");

// Text with formatting
let formatted = text! {
    font: Font::system().size(16.0),
    color: Color::RED,
    "Styled text"
};

// Attributed text
let rich = attributed_text! {
    "Normal " + bold("bold") + " and " + italic("italic")
};
```

## Advanced Features

### Text Measurement

Calculate text dimensions for layout purposes:

```rust
let text_size = Text::measure(
    "Sample text",
    Font::system().size(14.0),
    Some(200.0), // max width
);

println!("Text size: {}x{}", text_size.width, text_size.height);
```

### Text Truncation

Handle text that exceeds available space:

```rust
let truncated = Text::new("Very long text that might not fit")
    .truncation(TruncationMode::Tail)  // Add "..." at the end
    .max_lines(2)                      // Limit to 2 lines
    .line_break_mode(LineBreakMode::WordWrap);
```

### Typography Scales

Predefined text styles for consistent typography:

```rust
use waterui_text::typography::*;

let heading = Text::new("Main Title")
    .typography(Typography::HEADING_1);

let body = Text::new("Body content")
    .typography(Typography::BODY);

let caption = Text::new("Small text")
    .typography(Typography::CAPTION);
```

## Integration with I18n Plugin

Seamless integration with the internationalization plugin:

```rust
use waterui_plugins_i18n::I18n;

// Set up i18n
let mut i18n = I18n::new();
i18n.insert("en", "hello", "Hello, World!");
i18n.insert("es", "hello", "¡Hola, Mundo!");

// Use localized text
let localized_text = Text::new(i18n_key("hello"))
    .locale(current_locale);
```

## Dependencies

- `waterui-core`: Core framework functionality
- `waterui-reactive`: Reactive data binding
- `waterui-str`: Efficient string handling
- `alloc`: Memory allocation (no-std compatible)

## Performance Considerations

### Text Caching

The framework automatically caches text rendering for performance:

```rust
// Text with the same content and styling is cached
let text1 = Text::new("Same content").font(font.clone());
let text2 = Text::new("Same content").font(font.clone());
// Second instance reuses cached rendering
```

### Lazy Rendering

Text is only rendered when needed:

```rust
// Text measurement and rendering is deferred until display
let text = Text::new(expensive_computation())
    .font(dynamic_font);
// computation() and font resolution happen only when text is shown
```

## Best Practices

1. **Use typography scales**: Maintain consistent text styling across your app
2. **Cache expensive text**: Store complex attributed text in reactive bindings
3. **Consider performance**: Be mindful of frequent text updates in animations
4. **Support accessibility**: Provide proper contrast and sizing options
5. **Handle localization**: Design for different text lengths and directions
6. **Test with real content**: Use actual content during development, not placeholder text
7. **Optimize fonts**: Only load fonts that are actually used in your application
