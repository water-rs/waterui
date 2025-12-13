# waterui-text

Text and typography components for WaterUI with rich styling, fonts, markdown, and syntax highlighting.

## Overview

`waterui-text` provides comprehensive text rendering and formatting capabilities for the WaterUI framework. It handles everything from simple text display to complex styled text with multiple font properties, markdown rendering with full formatting support, and syntax highlighting for code snippets across 40+ programming languages.

The crate is designed around reactive primitives, automatically updating text when underlying data changes. All text rendering delegates to native platform widgets (UIKit/AppKit on Apple, Jetpack Compose on Android), ensuring platform-native appearance and accessibility.

Core features include semantic font styles (body, title, headline), granular styling control (bold, italic, underline, colors), markdown parsing with inline and block elements, and production-ready syntax highlighting via `syntect`.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-text = "0.1.0"
```

Or use the main WaterUI crate which re-exports text components:

```toml
[dependencies]
waterui = "0.1.0"
```

## Quick Start

```rust
use waterui_text::text;

// Simple text
let greeting = text("Hello, World!");

// Styled text with method chaining
let title = text("Welcome").bold().title().foreground(Color::blue());

// Reactive text that updates automatically
let count = binding(0);
let counter_text = text!("Count: {}", count);
```

## Core Concepts

### Text Component

The `Text` struct is the primary component for displaying read-only text. It automatically sizes itself to fit content and wraps when constrained by width. Text never stretches to fill extra space—it behaves like a label.

### StyledStr

`StyledStr` represents rich text with multiple styling attributes. It stores text as chunks, each with independent font, color, and decoration properties. This enables inline formatting like **bold** and *italic* within a single text component.

### Font System

Fonts are resolved through the `Environment`, allowing dynamic theming. The crate provides semantic font styles:
- `Body` (16pt, Normal)
- `Title` (24pt, SemiBold)
- `Headline` (32pt, Bold)
- `Subheadline` (20pt, SemiBold)
- `Caption` (12pt, Normal)
- `Footnote` (10pt, Light)

### Markdown Support

The `StyledStr::from_markdown()` function parses markdown into styled text, supporting headings, emphasis, strong, code blocks, lists, tables, links, and horizontal rules.

## Examples

### Basic Text with Styling

```rust
use waterui_text::{text, font::{FontWeight, Body}};
use waterui_color::Color;

// Simple text
text("Plain text")

// Bold text with custom size
text("Large Title")
    .bold()
    .size(32.0)

// Custom font and color
text("Custom Style")
    .font(Body)
    .weight(FontWeight::SemiBold)
    .foreground(Color::red())

// Text with background
text("Highlighted")
    .background_color(Color::yellow())
    .foreground(Color::black())
```

### Reactive Text with Formatting

From `/Users/lexoliu/Coding/waterui/examples/form/src/lib.rs`:

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let custom_name = binding("");
let custom_count = binding(5);
let custom_slider = binding(0.5);

// Reactive text updates automatically when bindings change
vstack((
    hstack(("Username: ", waterui::text!("{}", custom_name))),
    hstack(("Count: ", waterui::text!("{}", custom_count))),
    hstack(("Progress: ", waterui::text!("{}", custom_slider))),
))
```

### Markdown Rendering

From `/Users/lexoliu/Coding/waterui/examples/markdown/src/lib.rs`:

```rust
use waterui::prelude::*;

#[hot_reload]
fn main() -> impl View {
    scroll(include_markdown!("example.md").padding())
}
```

### Custom Formatting with Locales

```rust
use waterui_text::{Text, locale::{DateFormatter, Locale}};
use time::Date;

// Create a date formatter respecting locale
let formatter = DateFormatter { locale: Locale("en-US".into()) };
let date = binding(Date::from_calendar_date(2025, 1, 1).unwrap());

// Format date with custom formatter
let formatted = Text::format(date, formatter);
```

### Styled Text Construction

```rust
use waterui_text::styled::{StyledStr, Style};
use waterui_text::font::{Font, FontWeight, Title};
use waterui_color::Color;

// Build styled text from chunks
let mut styled = StyledStr::empty();
styled.push("Normal ", Style::default());
styled.push("Bold ", Style::default().bold());
styled.push("Red", Style::default().foreground(Color::red()));

// Parse markdown
let markdown = StyledStr::from_markdown("# Heading\n\nParagraph with **bold** and *italic*.");

// Apply styling to all chunks
let blue_text = styled.foreground(Color::blue());
```

### Syntax Highlighting

```rust
use waterui_text::highlight::{DefaultHighlighter, Language, highlight_text};
use waterui_core::Str;

// Create highlighter
let highlighter = DefaultHighlighter::new();

// Highlight code asynchronously
let code = Str::from("fn main() { println!(\"Hello\"); }");
let highlighted = highlight_text(Language::Rust, code, highlighter).await;
```

## API Overview

### Main Types

- `Text` - Primary text display component with styling methods
- `text(content)` - Convenience function to create text components
- `text!(format, args...)` - Macro for reactive formatted text
- `StyledStr` - Rich text with multiple style chunks
- `Style` - Text attributes (font, color, italic, underline, strikethrough)
- `Font` - Font configuration with semantic styles
- `FontWeight` - Font weight enumeration (Thin to Black)

### Text Methods

- `.bold()` - Apply bold weight
- `.italic(bool)` - Toggle italic style
- `.underline(bool)` - Toggle underline decoration
- `.size(f64)` - Set font size in points
- `.weight(FontWeight)` - Set font weight
- `.font(Font)` - Set complete font configuration
- `.foreground(Color)` - Set text color
- `.background_color(Color)` - Set background color
- `.body()`, `.title()`, `.headline()`, etc. - Apply semantic font styles

### StyledStr Methods

- `StyledStr::plain(text)` - Create plain styled text
- `StyledStr::from_markdown(md)` - Parse markdown into styled text
- `.push(text, style)` - Add styled chunk
- `.bold()`, `.italic(bool)`, `.underline(bool)` - Apply styling to all chunks
- `.foreground(color)`, `.background_color(color)` - Color all chunks
- `.to_plain()` - Extract plain text without styling

### Syntax Highlighting

- `Language` - Enum of supported languages (Rust, Swift, Python, Javascript, etc.)
- `DefaultHighlighter` - Syntect-based highlighter with 40+ languages
- `highlight_text(lang, text, highlighter)` - Async highlighting function

### Localization

- `Formatter<T>` - Trait for locale-aware formatting
- `DateFormatter` - Date formatting with locale support
- `Locale` - Locale identifier wrapper

## Features

This crate has no optional features. All functionality is included by default.

## Dependencies

Key dependencies that shape the API:

- **waterui-core** - Provides `View` trait, `Environment`, and reactive primitives
- **nami** - Fine-grained reactivity system (`Binding`, `Computed`, `Signal`)
- **waterui-color** - Color types for text and background styling
- **pulldown-cmark** - Markdown parsing engine
- **syntect** - Syntax highlighting for code blocks
- **two-face** - Extended syntax definitions including Swift
