# WaterUI Text Components

Minimal, reactive text for WaterUI.

## Basics

```rust,ignore
use waterui_text::Text;

// Plain text
let t = Text::new("Hello, World!");

// Size via font convenience
let large = Text::new("Large").size(18.0);
```

## Reactive Text with `text!`

```rust,ignore
use nami::binding;
use waterui_text::text; // function

// Macro is exported as `text!`
let name = binding("World");
let greet = text!("Hello, {}!", name); // updates on change
```

Tip: prefer `text!(...)` with reactive values; it formats reactively. For non-reactive values, `Text::new("...")` is fine.

## Fonts

```rust,ignore
use waterui_text::{Text, font::Font};

let text = Text::new("Body").font(Font::default()).size(14.0);
```

## Integration

```rust,ignore
use waterui_layout::stack::vstack;
use waterui_text::text;
use waterui_core::{View, Environment};

struct Article { title: String, body: String }

impl View for Article {
    fn body(self, _env: &Environment) -> impl View {
        vstack((
            Text::new(self.title).size(18.0),
            Text::new(self.body),
        ))
    }
}
```
