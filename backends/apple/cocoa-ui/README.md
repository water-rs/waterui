# cocoa-ui

A safe Rust API over Apple's user-interface frameworks, `AppKit` on macOS and
`UIKit` on iOS, built on the [`objc2`](https://crates.io/crates/objc2) crates.

It owns the parts of an application the frameworks run themselves — the
application and its delegate, windows and scenes, a host view whose layout,
resizing and hit testing are Rust closures, and the system notifications around
them — and exposes them as ordinary Rust values and closures:

- main-thread types are created and used only with a `MainThreadMarker`;
- native objects are owned through `Retained`;
- no `unsafe` function or `msg_send!` appears in the public API;
- a panic in any closure the frameworks call is logged and aborts the process,
  instead of unwinding into Objective-C.

The crate knows nothing about any particular UI toolkit: it is an imperative
foundation for Rust applications and toolkits that drive the native frameworks
directly. WaterUI's Apple backend is built on it.

## License

Licensed under either of the MIT license or the Apache License, Version 2.0, at
your option.
