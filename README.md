# WaterUI

WaterUI is a UI framework written in Rust, providing a fun coding experience for app designers.

# Features
- Reactive api, easy to use and thread-safe
- Native appearance
- Build your own component!

# Overview
- View : The core trait for declaring your component, they keep their own stationary state in struct
- Reactive<T> : Read-only source of trust, updated automatically
- Binding<T> : Creating a two-way connection between stored data and view
- Environment : Provide environment value for view, it can be set manually before call a view

Let's build a simple example - a counter in WaterUI to provide you a general view!
 
`View` is the core trait of WaterUI, it declares your UI :
```rust
pub trait View{
    fn body(self) -> impl View;
}
```

The build-in components have implement `View` trait, and you can align your ui logic,
building your own component by just implementing `View`

```rust
struct Counter;

impl View for Counter {
    fn body(self, _env: Environment) -> impl View {
        let count: Binding<u64> = Binding::from(0);
        vstack((
            text(count.display()),
            button("Click it!", move || {
                count.increment(1);
            }),
        ))
    }
}

```

This is an example of a counter in WaterUI. We create a binding named `count` to store the number.

`text()` can input any type implementing `IntoReactive<AttributedString>`, we just call `.display()` for this binding,
it creates a `Reactive<String>`, which implements `IntoReactive<AttributedString>`.

We align our view logic, and retur