<div align="center">
  <h1>WaterUI</h1>
  <p><strong>Native-first, fine-grained reactive UI for Rust.</strong></p>
  <img src="https://assets.waterui.dev/images/logo.png" alt="WaterUI logo" width="150" />
  <p>
    <a href="https://crates.io/crates/waterui"><img src="https://img.shields.io/crates/v/waterui.svg" alt="crates.io version" /></a>
    <a href="https://docs.rs/waterui"><img src="https://docs.rs/waterui/badge.svg" alt="docs.rs documentation" /></a>
    <a href="https://github.com/water-rs/waterui/blob/main/LICENSE-MIT"><img src="https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg" alt="MIT or Apache 2.0 license" /></a>
    <a href="https://codecov.io/gh/water-rs/waterui"><img src="https://img.shields.io/codecov/c/github/water-rs/waterui?logo=codecov" alt="code coverage" /></a>
  </p>
</div>

`WaterUI` lets you describe an application once in Rust and realize it through the backend that fits each platform. It bridges semantic components to `UIKit`/`AppKit`, Android View, and GTK4 where suitable native primitives exist, and provides purpose-built shared renderers for platforms or components that need a portable realization.

The framework is built around four ideas:

- **Native first.** A button, text field, list, or other semantic component uses the platform's canonical UI model when the platform provides one.
- **Precise reactivity.** `Binding`, `Computed`, and signal-aware component inputs update the affected value or semantic object without rebuilding an unrelated subtree.
- **One development tool.** The `water` CLI creates projects, manages platform support code, runs applications, inspects toolchains, and renders previews.
- **Portable rendering where it belongs.** Hydrolysis is the GPU renderer for high-end and web targets; Dew is the dirty-area CPU renderer for constrained devices.

## Quick start

Install the CLI:

```bash
cargo install waterui-cli
```

Create a playground and run it on the current host:

```bash
water create my-playground --mode playground
cd my-playground
water run
```

Playground mode keeps the project focused on application code. `water run`, `water package`, and `water preview` create and manage native support projects outside the source tree when they are needed.

Edit `src/lib.rs`:

```rust,ignore
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;

#[preview]
fn main() -> impl View {
    let count = Binding::i32(0);

    vstack((
        text("Hello, WaterUI!").size(28),
        text!("Count: {count}"),
        stepper("Count", &count),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
```

That is the complete user-owned entry point. The application crate depends on `waterui`; it does **not** declare `waterui-ffi` or call `waterui_ffi::export!()`. The CLI owns the generated FFI companion and backend integration.

Run a specific target when needed:

```bash
water run --platform macos
water run --platform ios
water run --platform android
water run --platform linux
```

Use `water doctor` to check the toolchain required by a target and `water devices` to see available devices and simulators.

## Preview a view

The `#[preview]` attribute makes a view directly addressable by the preview system:

```bash
water preview main --output preview.png
water preview main --frame 800x600 --output preview.png
```

Preview uses a native support application for Apple and Android targets, or Hydrolysis for direct offscreen rendering. The same preview surface also supports semantic interaction tests and GPU performance measurements through `water preview test` and `water preview perf`.

## Build a standalone app

Playgrounds are best for iteration. App mode creates platform projects that belong to the application and can be customized, signed, and packaged:

```bash
water create my-app --backends apple,android
cd my-app
water run --platform ios
```

Other backend configurations include:

```bash
water create linux-app --backends gtk4
water create hydrolysis-app --backends hydrolysis
water create esp32-app --backends esp32
```

The generated `Water.toml` is the source of truth for package metadata, enabled backends, permissions, themes, and an optional local `waterui_path`. Add or remove an app backend with `water backend`.

## Programming model

### Views are semantic values

Every component implements `View`. Layouts and modifiers compose those values into a tree:

```rust
use waterui::prelude::*;

fn profile_header() -> impl View {
    hstack((
        text("Ada").bold(),
        spacer(),
        button("Edit"),
    ))
    .padding()
}
```

Components keep their semantic identity while styles choose their presentation. For example, a `Toggle`, `Picker`, or `List` remains the same component when its visual style changes.

### State flows through signals

Use `Binding<T>` for mutable state and `Computed<T>` for derived state. Pass them into signal-aware APIs so only their dependents update:

```rust
use waterui::prelude::slider::slider;
use waterui::prelude::*;

fn progress_editor() -> impl View {
    let value = Binding::f64(0.25);
    let percent = value.map(|value| value * 100.0);

    vstack((
        slider("Progress", &value).range(0.0..=1.0),
        text!("Progress: {percent:.0}%"),
    ))
}
```

Use reactive collections with `ForEach` or `List` when membership changes. `watch` is reserved for an intentional structural replacement; it is not the routine way to update text, control values, styles, or collection items.

Field-backed collection identity can be derived. For a fixed set of items, pass
an array directly instead of allocating a `Vec`:

```rust
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::Identifiable;

#[derive(Clone, Identifiable)]
struct Contact {
    #[id]
    id: u64,
    name: &'static str,
    role: &'static str,
}

fn contacts() -> impl View {
    let contacts = [
        Contact {
            id: 1,
            name: "Alice Chen",
            role: "Software Engineer",
        },
        Contact {
            id: 2,
            name: "Bob Smith",
            role: "Product Manager",
        },
        Contact {
            id: 3,
            name: "Carol Williams",
            role: "Designer",
        },
    ];

    List::for_each(contacts, |contact| {
        ListItem::new(
            vstack((
                text(contact.name).headline(),
                text(contact.role).sub_headline().foreground(MutedForeground),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 16.0)),
        )
    })
}
```

### Environment carries application context

`Environment` propagates themes, locale data, services, and backend capabilities through the tree:

```rust,ignore
use waterui::app::App;
use waterui::prelude::*;

pub fn app(mut env: Environment) -> App {
    env.install(Theme::new().color_scheme(ColorScheme::Dark));
    App::new(main, env)
}
```

Backends consume shared theme slots, so ordinary view code receives platform-correct foreground, background, surface, border, accent, and font defaults.

## Rendering backends

`WaterUI` separates component semantics from their realization:

| Target | Backend | Realization |
| --- | --- | --- |
| iOS and macOS | Apple | `UIKit` and `AppKit` |
| Android | Android | Android View |
| Linux | GTK4 | GTK4 widgets |
| macOS, Linux, Windows, and web | Hydrolysis | Self-drawn GPU renderer |
| ESP32-S3 and ESP32-C3 | Dew | Dirty-area CPU renderer |

A shared renderer is a deliberate backend, not a silent fallback for a failed native path. Components without a suitable platform primitive use their shared realization directly.

## Crate features

The `waterui` crate exposes feature-granular capabilities so applications only link what they use.

Default crate features:

- `gpu`
- `assets`
- `media` and `video`
- `webview`
- `flow-markdown`

Opt-in capabilities:

- `chart`
- `barcode`
- `map`
- `particle`
- `navigation-restoration`

Generated projects select the features appropriate for their configured targets. The `all` feature is available for development and broad integration testing.

## Examples

The repository includes focused examples built with the same public API:

- [Gallery](examples/gallery/) — a broad component showcase
- [Form](examples/form/) — derived forms and reactive field projection
- [Navigation](examples/navigation/) — stacks, tabs, and navigation state
- [Flow Markdown](examples/flow_markdown/) — incrementally rendered streaming Markdown
- [Map](examples/map/) — map semantics and platform realization
- [Video player](examples/video_player/) — playback state and controls
- [Filter](examples/filter/) — GPU image filters and visual gallery export

From this repository, install the development CLI and run an example with its local `waterui_path`:

```bash
cargo install --path cli
cd examples/gallery
water run --platform macos
```

## Repository map

- [`core/`](core/) — view, environment, accessibility, layout contracts, and reactive integration
- [`components/foundation/`](components/foundation/) — layouts, text, controls, forms, navigation, shapes, and icons
- [`components/visual/`](components/visual/) — graphics, images, canvas, SVG, and filters
- [`components/multimedia/`](components/multimedia/) — media and video
- [`components/data/`](components/data/) — charts and maps
- [`backends/`](backends/) — Apple, Android, GTK4, Hydrolysis, Dew, and backend contracts
- [`cli/`](cli/) — the `water` command and project generators
- [`ffi/`](ffi/) — backend-facing integration used by generated companion crates
- [`testing/`](testing/) — semantic UI testing through the `WaterUI` accessibility tree
- [`examples/`](examples/) — runnable applications and previews

## Documentation

- [API reference](https://docs.rs/waterui)
- [WaterUI book](https://book.waterui.dev)
- [CLI guide](cli/README.md)
- [Roadmap](docs/ROADMAP.md)

## Contributing

Contributions target the `dev` branch; `main` is reserved for releases. Open an issue before starting a substantial change so the API and architecture can be discussed first. Read [AGENTS.md](AGENTS.md) before making repository changes.

AI-assisted contributions are welcome when a human understands and reviews the result. Fully autonomous pull requests without human review are not accepted.

## License

`WaterUI` is available under either the [Apache License 2.0](LICENSE-APACHE) or the [MIT License](LICENSE-MIT).
