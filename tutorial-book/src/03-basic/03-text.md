# Text and Typography

Text is the backbone of most interfaces. WaterUI gives you two complementary approaches:
lightweight **labels** for quick strings, and the configurable `Text` view for styled, reactive
content. Think of the split the same way Apple distinguishes between `Text` and bare strings in
SwiftUI, or Flutter differentiates `Text` from const literals.

## Quick Reference

| Need | Use | Notes |
| ---- | --- | ----- |
| Static copy, no styling | string literal / `String` / `Str` | Lowest overhead; respects the surrounding layout but cannot change font or colour. |
| Styled or reactive text | `Text` / `text!` macro | Full typography control and automatic updates when bound data changes. |
| Format existing signals | `text!("Total: {amount:.2}", amount)` | Uses the `nami::s!` formatter under the hood. |
| Display non-string signals | `Text::display(binding_of_number)` | Wraps any `Display` value, recalculating when the binding updates. |
| Custom formatter (locale-aware, currency, dates) | `Text::format(value, Formatter)` | See `waterui_text::locale` for predefined formatters. |

## Labels: Zero-Cost Strings

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::stack::vstack;

pub fn hero_copy() -> impl View {
    vstack((
        "WaterUI",                      // &'static str
        String::from("Rust-first UI"),  // Owned String
        Str::from("Lightning fast"),    // WaterUI's rope-backed string
    ))
}
```

Labels have no styling hooks and stay frozen after construction. Use them for static headings,
inline copy, or when you wrap them in other views (`button("OK")`).

## The `Text` View

`Text` is a configurable view exported by `waterui::component::text`. Create instances via the
`text` function, the `text!` macro, or constructors such as `Text::display`.

### Reactive Text with `text!`

```rust,ignore
use waterui::prelude::*;
use waterui::reactive::binding;

pub fn welcome_banner() -> impl View {
    let name = binding("Alice");
    let unread = binding(5);

    vstack((
        text!("Welcome back, {name}!"),
        text!("You have {unread} unread messages."),
    ))
}
```

`text!` captures any signals referenced in the format string and produces a reactive `Text` view.
Avoid `format!(…)` + `text(...)`; the one-off string will not update when data changes.

### Styling and Typography

`Text` exposes chainable modifiers that mirror SwiftUI:

```rust,ignore
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_text::font::FontWeight;

pub fn ticker(price: Binding<f32>) -> impl View {
    text!("${price:.2}")
        .size(20.0)
        .weight(FontWeight::Medium)
        .foreground(Color::srgb(64, 196, 99))
}
```

Available modifiers include:

- `.size(points)` – font size in logical pixels.
- `.weight(FontWeight::…)` or `.bold()` – typographic weight.
- `.italic(binding_of_bool)` – toggle italics reactively.
- `.font(Font)` – swap entire font descriptions (custom families, monospaced, etc).
- `.content()` returns the underlying `Computed<StyledStr>` for advanced pipelines.

Combine with `ViewExt` helpers for layout and colouring, e.g. `.padding()`, `.background(...)`, or
`.alignment(Alignment::Trailing)`.

### Displaying Arbitrary Values

```rust,ignore
use waterui::prelude::*;
use waterui::reactive::binding;

pub fn stats() -> impl View {
    let active_users = binding(42_857);
    let uptime = binding(99.982);

    vstack((
        Text::display(active_users),
        Text::format(uptime, waterui_text::locale::Percent::default()),
    ))
}
```

`Text::display` converts any `Signal<Output = impl Display>` into a reactive string. For complex
localised formatting (currency, dates), `Text::format` interoperates with the formatters in
`waterui_text::locale`.

### Working with `Binding<Option<T>>`

When the text source may be absent, leverage nami’s mapping helpers:

```rust,ignore
use nami::SignalExt;

let maybe_location = binding::<Option<String>>(None);
let fallback = maybe_location.unwrap_or_else(|| "Unknown location".to_string());
text(fallback);
```

`unwrap_or_else` yields a new `Binding<String>` that always contains a value, ensuring the view stays
reactive.

## Best Practices

- **Avoid `.get()` inside views** – Convert to signals with `.map`, `.zip`, or `binding::<T>` +
  turbofish when the compiler needs help inferring types.
- **Keep expensive formatting out of the view** – Precompute large strings in a `Computed` binding
  so the closure remains trivial.
- **Prefer `text!` for dynamic content** – It keeps formatting expressive and reduces boilerplate.
- **Use labels for performance-critical lists** – Large table rows with static copy render faster as
  bare strings.

## Troubleshooting

- **Text truncates unexpectedly** – Wrap it in `Frame::new(text!(…)).alignment(Alignment::Leading)`
  or place inside an `hstack` with `spacer()` to control overflow.
- **Styling missing on one platform** – Confirm the backend exposes the property; some early-stage
  renderers intentionally ignore unsupported font metrics.
- **Emoji or wide glyph clipping** – Ensure the containing layout provides enough height; padding or
  a frame often resolves baseline differences between fonts.

With these building blocks you can express everything from static headings to live, localised
metrics without imperatively updating the UI. Let your data bindings drive the text, and WaterUI
handles the rest.
