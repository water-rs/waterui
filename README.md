# WaterUI

WaterUI is a UI framework written in Rust, providing a fun coding experience for app designers.

# Features
- Reactive api, easy to use and thread-safe
- Native appearance
- Build your own component!

# Overview

- You declare your component with `View` trait, they keep their own stationary state in struct
- And the component update their state automatically with `Reactive<T>`

`View` is the core trait of WaterUI, it declares your UI :

```rust
pub trait View{
    fn body(self) -> BoxView;
}
```
The build-in components have implement `View` trait, and you can build your own component by just implementing `View`



```rust
struct Counter{
  counter:Reactive<u64>
}

#[view]
impl View for Counter{
    fn body(self) -> VStack{
        vstack((
          text(self.counter.to(|n| n.to_string())),
          button("click it!",move ||{
            *self.counter.get_mut() += 1;
          })
        ))
    }
}
```

This is an example of counter in WaterUI. It uses `Reactive<u64>` and `Reactive<String>`,
we use `.to()` to transform `Reactive<u64>` to `Reactive<String>`


`text()` can input `&Reactive<String>` because it implements `IntoReactive<AttributedString>`


If it doesn't depend any `Reactive`, `Reactive<T>` behaves just like `Arc<RwLock<T>>`,
you can set it value and other value depend it will be updated automatically. Just like `Reacitve<u64>` in this example.
