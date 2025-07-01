# WaterUI Text Components

Comprehensive text rendering and formatting capabilities for the WaterUI framework.

## Overview

`waterui-text` provides text components with support for fonts, attributed text, links, internationalization, and locale-aware formatting. It handles everything from simple text display to complex rich text with multiple styles.

## Components

### Text

The main text rendering component:

```rust
use waterui_text::Text;

let text = Text::new("Hello, World!")
    .font(Font::system().size(16.0))
    .color(Color::BLACK)
    .alignment(TextAlignment::Center);
```

### Attributed Text

Rich text with multiple styles:

```rust
use waterui_text::attributed::{AttributedText, TextAttribute};

let rich_text = AttributedText::new()
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

### Links

Interactive text elements:

```rust
use waterui_text::link::Link;

let link = Link::new("Visit our website")
    .url("https://example.com")
    .color(Color::BLUE)
    .on_click(|url| open_browser(url));
```

## Font System

### Font Configuration

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

## Text Attributes

Rich text formatting options:

- **Font attributes**: Size, weight, style, family
- **Color attributes**: Foreground and background colors
- **Decoration**: Underline, strikethrough, overline
- **Spacing**: Letter spacing, line height
- **Effects**: Shadow, outline, glow

## Localization Support

### Locale-Aware Formatting

```rust
use waterui_text::locale::{Locale, Formatter};

let locale = Locale::new("en_US");
let formatter = Formatter::new(locale);

// Number formatting
let number = formatter.format_number(1234.56);  // "1,234.56"

// Date formatting
let date = formatter.format_date(date, DateStyle::Medium);  // "Jan 15, 2025"

// Currency formatting  
let currency = formatter.format_currency(99.99, "USD");  // "$99.99"
```

### Text Direction

Automatic handling of LTR and RTL text:

```rust
let arabic_text = Text::new("مرحبا بالعالم")  // Arabic text
    .direction(TextDirection::Auto)              // Auto-detect RTL
    .alignment(TextAlignment::Natural);          // Natural alignment
```

## Macros

Convenient text creation:

```rust
use waterui_text::macros::*;

// Simple text
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

Calculate text dimensions for layout:

```rust
let size = Text::measure(
    "Sample text",
    Font::system().size(14.0),
    Some(200.0), // max width
);
```

### Text Truncation

Handle overflow text:

```rust
let truncated = Text::new("Very long text...")
    .truncation(TruncationMode::Tail)      // Add "..." at end
    .max_lines(2)                          // Limit to 2 lines
    .line_break_mode(LineBreakMode::WordWrap);
```

### Typography Scales

Predefined text styles:

```rust
use waterui_text::typography::*;

let heading = Text::new("Title").typography(Typography::HEADING_1);
let body = Text::new("Content").typography(Typography::BODY);  
let caption = Text::new("Note").typography(Typography::CAPTION);
```

## Integration

### With I18n Plugin

```rust
use waterui_plugins_i18n::I18n;

let mut i18n = I18n::new();
i18n.insert("en", "hello", "Hello, World!");
i18n.insert("es", "hello", "¡Hola, Mundo!");

let localized = Text::new(i18n_key("hello"));
```

## Performance

- **Text Caching**: Automatic caching of rendered text
- **Lazy Rendering**: Text rendered only when needed
- **Efficient Updates**: Only re-render when content or style changes

## Dependencies

- `waterui-core`: Core framework functionality
- `waterui-reactive`: Reactive data binding
- `waterui-str`: Efficient string handling

## Example

```rust
use waterui_text::*;

struct ArticleView {
    title: String,
    content: String,
}

impl View for ArticleView {
    fn body(self, env: &Environment) -> impl View {
        vstack![
            Text::new(self.title)
                .typography(Typography::HEADING_1)
                .color(Color::PRIMARY),
                
            Text::new(self.content)
                .typography(Typography::BODY)
                .max_lines(None)
                .line_break_mode(LineBreakMode::WordWrap),
        ]
    }
}
```
