# Assets

Files placed here are bundled into {{ ctx.app_display_name }} and reachable from Rust
through the `assets!` macro, which resolves paths relative to this directory:

```rust
use waterui::assets;

let logo = assets!("logo.png");
```

The assets planner walks this directory recursively, so subdirectories work too
(`assets!("icons/logo.png")`). The directory is scanned at build time — adding a
file is enough, there is nothing to register.

`Icon.svg` is the app icon; it starts out as the WaterUI logo. Replace it with
your own square SVG or PNG (any root-level `Icon.*` image is picked up) and the
CLI generates every platform format from it: full-bleed squares for iOS, the
rounded-rectangle shape for macOS, and adaptive-icon layers for Android.

This README exists to document the workflow. Feel free to delete it.
