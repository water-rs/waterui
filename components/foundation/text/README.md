# waterui-text

Text and typography components for WaterUI with rich styling, fonts, markdown, and syntax highlighting.

## Overview

`waterui-text` provides comprehensive text rendering and formatting capabilities for the WaterUI framework. It handles everything from simple text display to complex styled text with multiple font properties, inline-focused markdown styling, and syntax highlighting for code snippets across 40+ programming languages.

The crate is designed around reactive primitives, automatically updating text when underlying data changes. All text rendering delegates to native platform widgets (UIKit/AppKit on Apple, Android View on Android), ensuring platform-native appearance and accessibility.

Core features include semantic font styles (body, title, headline), granular styling control (bold, italic, underline, colors), inline markdown parsing for styled text runs, and production-ready syntax highlighting via `syntect`.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-text = "0.1.0"
```

Or use the main WaterUI crate which re-exports text components:

```toml
[dependencies]
waterui = "0.2"
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

`StyledStr` represents rich text with multiple styling attributes. It stores text as chunks, each with independent font, color, and decoration properties. This enables inline formatting like **bold** and _italic_ within a single text component.

### Font System

Fonts are resolved through the `Environment`, allowing dynamic theming. The crate provides semantic font styles:

- `Body` (16pt, Normal)
- `Title` (24pt, SemiBold)
- `Headline` (32pt, Bold)
- `Subheadline` (20pt, SemiBold)
- `Caption` (12pt, Normal)
- `Footnote` (10pt, Light)

### Markdown Support

`StyledStr::from_markdown()` focuses on inline semantics (headings/emphasis/strong/strikethrough/inline code) and lightweight block separators. For full block-level markdown rendering, use `RichText` via `include_markdown!()` in the main `waterui` crate.

## Examples

### Basic Text with Styling

```rust
use waterui_text::{text, font::{FontWeight, Body}};
use waterui::color::Color;

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

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let custom_name = binding("");
let custom_count = binding(5);
let custom_slider = binding(0.5);

// Reactive text updates automatically when bindings change
vstack((
    text!("Username: {name}", name = custom_name),
    text!("Count: {count}", count = custom_count),
    text!("Progress: {value}", value = custom_slider),
))
```

### Markdown Rendering

```rust
use waterui::prelude::*;

fn main() -> impl View {
    scroll(include_markdown!("example.md").padding())
}
```

### Localized Text

```rust
use waterui_text::Text;

// Static keys resolve through the environment's translation catalog and
// update when the native platform locale changes.
let title = Text::localized("settings.title");

// Runtime strings remain verbatim.
let server_message = Text::verbatim(message);
```

### Styled Text Construction

```rust
use waterui_text::styled::{StyledStr, Style};
use waterui_text::font::{Font, FontWeight, Title};
use waterui::color::Color;

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
let mut highlighter = DefaultHighlighter::new();

// Highlight code
let code = Str::from("fn main() { println!(\"Hello\"); }");
let highlighted = highlight_text(Language::Rust, &code, &mut highlighter);
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
- `highlight_text(lang, text, highlighter)` - Highlight code into styled text chunks

### Localization

- `Text::localized(key)` - Resolve a static translation key reactively
- `Text::localized_with(resolver)` - Build locale-aware text configuration
- `Formatter<T>` and `Text::format` - Format reactive values without rebuilding a view

## Features

This crate has no optional features. All functionality is included by default.

## Dependencies

Key dependencies that shape the API:

- **waterui-core** - Provides `View` trait, `Environment`, and reactive primitives
- **nami** - Fine-grained reactivity system (`Binding`, `Computed`, `Signal`)
- **waterui-graphics** - Color types for text and background styling
- **pulldown-cmark** - Markdown parsing engine
- **syntect** - Syntax highlighting for code blocks
- **two-face** - Extended syntax definitions including Swift
