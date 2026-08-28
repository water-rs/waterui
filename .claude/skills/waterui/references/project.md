# Projects, the `water` CLI, and platforms

## Contents

- Creating a project
- Project shape
- `Cargo.toml`: features that matter
- `Water.toml`
- Permissions: declaring and requesting
- Assets and the app icon
- Running and building
- Platforms and backends
- Logging and debugging
- Embedded targets (Dew)

## Creating a project

Never hand-scaffold a WaterUI project. The generated layout is the source of truth and the
CLI keeps it consistent with the backends it builds.

```bash
water create "My App"                       # app mode (default)
water create "My App" --mode playground     # playground mode
water create "My App" --bundle-id dev.example.myapp
water create "My App" --backends apple,android,hydrolysis
```

`--mode playground` is the right default for experiments and examples: the CLI owns the
native project entirely, so there is no Xcode project or Gradle wrapper to maintain. App
mode gives you those files to edit when the app needs real native integration.

## Project shape

```
my-app/
├── Water.toml           # WaterUI project manifest
├── Cargo.toml
├── assets/
│   └── Icon.svg         # single source for every platform icon
├── i18n/                # optional: translation catalogs, one TOML per locale
│   └── en-US.toml       #   (see references/i18n.md)
└── src/
    └── lib.rs
```

`src/lib.rs` exposes an entry point:

```rust
use waterui::app::App;
use waterui::prelude::*;

pub fn app(env: Environment) -> App {
    App::new(root_view, env)
}
```

For state that must exist before the first view, build it in `app` and clone into the
builder — this is also how you inject app-wide state that handlers reach with `State<T>`:

```rust
pub fn app(env: Environment) -> App {
    let state = AppState::new();
    App::new(move || content(state.clone()), env)
}
```

Take `mut env` when the app installs things (a `Theme`, `MapGpuOptions`, an `ApiClient`).

## `Cargo.toml`: features that matter

Generated projects carry a `dev` feature that forwards to `waterui/dynamic_linking`:

```toml
[features]
dev = ["waterui/dynamic_linking"]
```

Keep it. `dynamic_linking` builds the app as a dylib against a shared framework copy,
which is what `water preview` and the fast dev loop link against — the difference between
a seconds-scale and a minutes-scale edit-to-render cycle. Every compiled example in the
WaterUI repository declares exactly this stanza; a hand-added crate that omits it gets
cold static builds and a preview pipeline that cannot load it. It is a build strategy,
not a capability — release builds ignore it.

Component features (`chart`, `map`, `webview`, …) are covered in
`references/components.md`, including the direct-crate alternative
(`waterui-chart` instead of `waterui = { features = ["chart"] }`).

## `Water.toml`

```toml
[package]
type = "app"                              # "app" | "playground"
name = "My App"
bundle_identifier = "dev.example.myapp"
# assets_path = "assets"                  # default
# accessory = false                       # macOS: build as a headless accessory app

[theme]
background = "#0B0B0F"
surface = "#15151C"
surface_variant = "#1E1E28"
border = "#2A2A36"
foreground = "#F5F5F7"
muted_foreground = "#A0A0AE"
accent = "#4A84F6"
accent_foreground = "#FFFFFF"

[permissions.internet]
enable = true
description = "Required to download map tiles"

[permissions.location]
enable = true
description = "Required to show user location on the map"
```

The `[theme]` slots seed the same tokens described in `references/styling.md`, so setting
them once here themes the whole app on every backend.

When developing against a local WaterUI checkout, add `waterui_path = "../.."` at the top
level so backends resolve locally instead of from the registry.

## Permissions: declaring and requesting

Permission keys: `internet`, `camera`, `microphone`, `location`, `coarse_location`,
`storage`, `write_storage`, `photo_library`, `contacts`, `calendars`, `bluetooth`,
`bluetooth_admin`, `vibrate`, `wake_lock`. The CLI translates each into the right platform
declaration (`AndroidManifest` entries, Info.plist usage strings), so the `description`
is what the user actually reads in the system prompt — write it for them.

Missing `internet` is a common and confusing failure: Android denies DNS outright, so every
request fails with a resolution error rather than a permission error.

Declaring a permission is half the story — camera, microphone, and location must also be
**requested at runtime** through the `waterkit-permission` crate (a direct dependency),
from an async handler, never from a view body:

```rust
use waterkit_permission::{Permission, PermissionStatus, check, request};

button("Enable microphone")
    .action_async(|State(granted): State<Binding<bool>>| async move {
        let status = check(Permission::Microphone).await;       // infallible
        let status = if matches!(status, PermissionStatus::Granted) {
            status
        } else {
            match request(Permission::Microphone).await {       // fallible: Result
                Ok(s) => s,
                Err(_) => return,
            }
        };
        granted.set(matches!(status, PermissionStatus::Granted));
    })
    .state(&granted)
```

Match `PermissionStatus` with a wildcard arm. On denial, leave the gated feature closed
and tell the user how to retry — do not fall back to pretending it works.

## Assets and the app icon

Put one square `Icon.svg` (or `Icon.png`) at the root of `assets/`. The CLI generates every
platform format from it: iOS full-bleed, macOS rounded rect, Android adaptive layers,
Windows `.ico`, Linux `.desktop`. Exactly one root-level `Icon.*` is allowed. New projects
start with the WaterUI logo, so replacing that one file rebrands the app everywhere.

Everything else under `assets/` is bundled as a regular asset.

## Running and building

```bash
water run                              # host platform
water run --platform ios
water run --platform android
water run --platform macos
water run --platform linux --backend hydrolysis
water run --device <id>                # a specific simulator or device
water run --logs debug                 # stream device logs at debug and above
water run --native-logs                # include native platform logs too — noisy

water build <target>                   # compile the Rust library for a platform
water package                          # package artifacts for distribution
water devices                          # list simulators and devices
water doctor                           # check the toolchain
water clean
water gc                               # garbage-collect stale build caches
water inspector                        # launch the inspector app
```

`water run` does not exit while the app is alive. **A `water run` that "completes" means
the app crashed** — read the log tail rather than treating it as success.

## Platforms and backends

| Platform | Default backend | Also possible |
|---|---|---|
| macOS, iOS, tvOS, watchOS, visionOS (+ simulators) | `apple` (UIKit/AppKit) | `hydrolysis` |
| Android | `android` (Android View) | `hydrolysis` |
| Linux | `gtk4` | `hydrolysis` |
| Windows | `hydrolysis` | — |
| Web | WASM + WebGPU | — |
| ESP32-S3 | `dew` | — |

Native backends bridge to real platform widgets. `hydrolysis` and `dew` are WaterUI's own
renderers: `hydrolysis` is GPU-required and targets high-refresh modern hardware; `dew` is
CPU-first for constrained devices. Choosing a self-drawn renderer is a deliberate decision,
never a fallback for a native path that failed.

The same view code runs on all of them. Platform-specific behavior belongs in the backend,
not in conditional app code.

## Logging and debugging

Use `tracing`, never `println!` — printed output does not reach the device log pipeline.
You do not need `tracing` in your own `Cargo.toml`: the whole crate is re-exported as
`waterui::log`:

```rust
waterui::log::debug!(?value, "recomputed layout");
waterui::log::info!("saved");
```

```bash
water run --logs debug
```

`water inspector` opens an inspect-element view of a running debug build: it shows the live
view tree, layout bounds, and the accessibility tree. When a layout looks wrong, read the
accessibility bounds rather than eyeballing the picture — bounds tell you which container
mis-sized a child; a screenshot only tells you something is off.

## Embedded targets (Dew)

WaterUI runs on microcontrollers through the Dew backend: CPU rasterization, no GPU, and
dirty-region flushes sized for SPI panels, so peak pixel memory is one band rather than a
full frame. The same views, bindings, and `text!` reactivity work unchanged.

Develop against the desktop panel simulator — the full embedded rendering path in a native
window, no cross-compilation:

```bash
cargo run -p waterui-dew --example watch_sim --features embedded-simulator
```

Headless snapshot: `waterui_dew::render_view_png(builder, env, w, h)`.

Dew supports a deliberately narrow set of views (stacks, padding, colors, spacers, text,
navigation and shape primitives). An unsupported view panics immediately with a clear
message rather than rendering something wrong — treat that panic as the accurate answer
about what the target can do, not as a bug to route around.
