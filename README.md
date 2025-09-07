# WaterUI 🌊

A cross-platform UI framework for Rust with cross-platform native rendering

# Counter example

```rust
use waterui::prelude::*;

pub fn counter() -> impl View {
    let count = Binding::int(0);
    let doubled = count.map(|n| n * 2);
    
    vstack((
        text!("Count: {count}"),
        text!("Doubled: {doubled}")
            .font_size(20)
            .foreground_color(Color::gray()),
        
        hstack((
            button("Increment")
                .action_with(&count,|count| count.increment(1)),
            button("Reset")
                .action_with(&count,|count| count.set(0))
                .foreground_color(Color::red()),
        ))
        .spacing(10),
    ))
    .padding(20)
    .spacing(15)
}

```

## ✨ Features

- True native rendering - Uses SwiftUI on Apple platforms (yes, even visionOS/watchOS/widgets!)
- Vue-like fine-grained reactivity - Allows efficient updates without virtual DOM
- Type-safe from top to bottom - Leverage Rust's type system fully
- Declarative & reactive - Familiar to SwiftUI/React developers
- Cross-platform - Windows, Linux, macOS, iOS, Android (Web coming soon)

## 🧑‍🏫 Demo (SwiftUI backend)

Check it [here](./demo).

## 📚 Documentation
- [Tutorial book](https://water-rs.github.io/waterui/)
- [API reference](https://docs.rs/waterui)