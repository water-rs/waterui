# Assets

Files placed here are bundled into {{ app_display_name }} and reachable from Rust
through the `assets!` macro, which resolves paths relative to this directory:

```rust
use waterui::assets;

let logo = assets!("logo.png");
```

The assets planner walks this directory recursively, so subdirectories work too
(`assets!("icons/logo.png")`). The directory is scanned at build time — adding a
file is enough, there is nothing to register.

This README only keeps the directory present in git. Delete it once you have
added real assets.
