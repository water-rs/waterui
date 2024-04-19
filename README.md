<div align="center">
<h1>WaterUI</h1>
 <strong>
   Bring your app to all plaform, once for all.
 </strong>
</div>

WaterUI is a expeirmental UI framework written in Rust. Enable you to build your app in a single codebase,
then available in all platform, even embedded devices.

# Features
- Type-safe, declarative and reactive API
- First-class async support (require std)
- Platform-independence core
- `no-std` support
- Optional macro


# TODO

- [ ] Better error handling
- [ ] Support async and error handling without std.
- [ ] Icon component
- [ ] Hot reloading
- [ ] Cli
- [ ] Muti-window support

# Quick start

```rust
  use waterui::{
      component::{button, text, vstack},
      Binding, ComputeExt, Environment, View,
  };

  pub struct Counter;

  impl View for Counter {
      fn body(self, _env: Environment) -> impl View {
          let count = Binding::constant(0);
          vstack((
              text(count.display()),
              button("Click me!").action(move |_| {
                  count.add(1);
              }),
          ))
      }
  }

```