# Projects, the `water` CLI, and platforms

## Contents

- Installing the CLI
- Creating a project
- Framework channels
- Project shape
- `Cargo.toml`: features that matter
- `Water.toml`
- Permissions: declaring and requesting
- Assets and the app icon
- Launch screen
- Home-screen widgets
- Running and building
- Platforms and backends
- Logging and debugging

## Installing the CLI

Install the prebuilt binary; do not build the CLI from source. `cargo install
waterui-cli` compiles the CLI and its whole dependency graph, which takes minutes and
is never what a user of the framework wants.

```bash
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/water-rs/waterui/releases/download/cli-latest/waterui-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/water-rs/waterui/releases/download/cli-latest/waterui-cli-installer.ps1 | iex"
```

`water update` replaces a binary the release installer put there with the newest release;
`water update --check` only reports what is available. A `water` installed by Homebrew or
`cargo` is not rewritten: the command prints the owning package manager's update command.
A project pins its framework version separately from the CLI — see
[Framework channels](#framework-channels).

## Creating a project

Never hand-scaffold a WaterUI project. The generated layout is the source of truth and the
CLI keeps it consistent with the backends it builds.

```bash
water create "My App"
water create "My App" --bundle-id dev.example.myapp
water create "My App" --channel nightly            # dev | nightly | stable (default)
water create "My App" --waterui-path ~/src/waterui   # scaffold against a local framework checkout
water create "My App" --template web               # a web/ Vite frontend mounted by include_web!
water create "My App" --template web --package-manager pnpm --vite-template vanilla-ts
water init                                         # turn the current directory into a project
water init --web new                               # ... scaffolding a web/ frontend
water init --web ../site --web-mode reference      # ... adopting an existing one (copy | reference)
```

There are no modes or package types to choose: every project is entry-owning — WaterUI
owns the program entry and the CLI generates and manages every backend project, so there
is no Xcode project or Gradle wrapper in your tree to maintain. `--package-manager` and
`--vite-template` are valid only with a web frontend. The `[web] package_manager` key in
`Water.toml` (`bun` default, `pnpm`, `npm`, `yarn`) records which one the CLI drives.

## Framework channels

A project pins its framework exactly, separately from the CLI. Three channels:

| Channel | Resolves to |
|---|---|
| `stable` (default) | published packages and the native backends bundled with the CLI |
| `nightly` | an immutable revision certified by the complete nightly suite |
| `dev` | an exact, compilation-checked commit of the integration branch |

```bash
water channel                     # show the current selection
water channel nightly             # select, or update to the channel's newest revision
water channel dev --rev <sha>     # pin dev to one commit (--rev is valid only with dev)
```

The CLI owns the `[framework]` table in `Water.toml` and the project's `Water.lock`; never
edit either by hand and never re-pin git revisions in `Cargo.toml` yourself. A
framework-revision or lock-mismatch error names the `water channel` command that
reconciles the project: run exactly that. A project built against a local checkout
(`waterui_path`) has no channel; `water channel` then reports the local path.

## Project shape

```
my-app/
├── Water.toml           # WaterUI project manifest
├── Water.lock           # CLI-managed with Water.toml's [framework]; never hand-edited
├── .mcp.json            # lets an MCP client find `water mcp` (references/mcp.md)
├── Cargo.toml
├── assets/
│   ├── Icon.svg         # single source for every platform icon
│   └── Launch.svg       # optional: launch screen artwork
├── i18n/                # optional: translation catalogs, one TOML per locale
│   └── en-US.toml       #   (see references/i18n.md)
├── web/                 # optional: Vite frontend mounted by include_web! (--template web)
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
builder:

```rust
pub fn app(env: Environment) -> App {
    let state = AppState::new();
    App::new(move || content(state.clone()), env)
}
```

Cloning a value into the builder does not make it reachable from handlers. A handler
extracts state — as `State<T>`, or as a bare parameter of a `#[state]`-marked type — only
once it is installed with `.state(&value)`: on a view for that subtree's handlers, or with
`App::state(&value)` for the menu-bar, quit-request and terminate handlers that run under
the application's environment.

Take `mut env` when the app installs things (a `Theme`, `MapGpuOptions`, an `ApiClient`).

## `Cargo.toml`: features that matter

Generated projects carry a `dev` feature that forwards to `waterui/dynamic_linking`, and a
`media` feature that forwards to `waterui/media` and is off by default (video playback links a
stack that needs VA-API and PipeWire at build time on Linux; add it to `default` when the app
plays video):

```toml
[features]
dev = ["waterui/dynamic_linking"]
media = ["waterui/media"]
```

Keep the `dev` feature. `dynamic_linking` links the app against a shared framework runtime,
which is what `water preview` and the fast dev loop reuse — the difference between
a seconds-scale and a minutes-scale edit-to-render cycle. Every compiled example in the
WaterUI repository declares the same `dev` line; a hand-added crate that omits it gets
cold static builds, and the Apple preview refuses to build it. It is a build strategy,
not a capability — release builds ignore it. Windows links statically: a project that
enables `waterui/dynamic_linking` there fails the Windows build, so never put it in
`default` or reach for `--all-features` (use `--features all`, which omits it).

The `waterui` crate's `default` features are `gpu`, `assets`, `dialog`, `snackbar`,
`highlight`, `locale-datetime` and `locale-list`; a lean build sets
`default-features = false` and names what it needs. `all` enables every capability feature
except `dynamic_linking`, `ts` / `ts-ota`, `markdown-math` and `widgets`; it includes
`video-gpu` (so `video`), `inspector` and `inspector-signals`. The inspector's reactive-graph
view needs `inspector-signals` (implies `inspector`).

Component features (`webview`, `flow-markdown`, `canvas`, `chart`, `barcode`, `particle`, …)
re-export their crate as a `waterui` module (`waterui::chart`, `waterui::barcode`, …) and are
covered in `references/components.md`, along with `waterui-map`, which is a crate of its own.

## `Water.toml`

```toml
[package]
name = "My App"
bundle_identifier = "dev.example.myapp"
# assets_path = "assets"                  # default
# accessory = false                       # macOS: build as a headless accessory app
# embedded = false                        # true: the crate is a library a host app embeds;
                                          #   `water build` yields its artifact, run/package refuse

[platforms.linux]                         # optional
backend = "hydrolysis"

[hydrolysis]                              # optional
painter = "gpu"                           # Android painter: "gpu" (default) or "hwui"

[web]                                     # optional: only the CLI reads it
package_manager = "bun"                   # bun (default) | pnpm | npm | yarn

[[assets.font]]                           # optional: one table per font family to bundle
name = "Inter"                            # alone: the CLI's built-in registry
# local_path = "fonts/Inter.ttf"          # or a file in the project, or
# remote_path = "https://…"               #   a URL `water fetch` pre-seeds into the font cache
# platforms = ["ios", "android"]          # default: every platform

[theme]
background = "#0B0B0F"
surface = "#15151C"
surface_variant = "#1E1E28"
border = "#2A2A36"
foreground = "#F5F5F7"
muted_foreground = "#A0A0AE"
accent = "#4A84F6"
accent_foreground = "#FFFFFF"

[launch]
background = "#0B0B0F"           # optional: defaults to theme.background
background_dark = "#000000"      # optional: defaults to background

[permissions.internet]
enable = true
description = "Required to download map tiles"

[permissions.location]
enable = true
description = "Required to show user location on the map"
```

The `[theme]` slots seed the same tokens described in `references/styling.md`, so setting
them once here themes the whole app on every backend.

The optional `[platforms.<platform>]` tables declare the project's backend for each
platform; see [Platforms and backends](#platforms-and-backends). `water create` writes
no declarations. A font declaration sets at most one source. A build fetches a declared
font missing from the font cache; `water fetch` (optionally `--backend <b>`) downloads them
ahead of time, so an `--offline` build, which fails on a missing font, can succeed. Dependency
crates declare their own assets and fonts; `[assets]` is only for the app's own.

`[framework]` and the project's `Water.lock` are written by `water channel`, `water create`
and `water init` ([Framework channels](#framework-channels)); do not edit them.

Apple builds target the framework's deployment floor unless the project raises it. An API
that needs a newer OS (`WebView::navigation_policy`, and so `include_web!`, need OS 27) does
not compile below it, and the error names this table:

```toml
[apple.deployment-targets]
ios = "27.0"      # any of macos, ios, tvos, watchos, visionos; below the framework floor is an error
macos = "27.0"
```

`water create` writes it for a project that bundles a web frontend.

When developing against a local WaterUI checkout, add `waterui_path = "../.."` at the top
level so backends resolve locally instead of from the registry.

Some capability crates need configuration only the app knows. Such a crate requests it, and
packaging fails naming the key until the app supplies it under `[app_values]`:

```toml
[app_values]
firebase_config = "google-services.json"            # Android: copied into the Gradle module
apple_pay_merchant_ids = ["merchant.com.example"]   # Apple: the in-app-payments entitlement
cast_receiver_app_id = "CC1AD845"                    # Android: @string/waterui_cast_receiver_app_id
```

Release and distribution packaging read `[signing]` (macOS Developer ID, Android release
keystore); an Android release package without `[signing.android]` fails unless packaged
`--unsigned`. The table carries no secrets: passwords come from the environment, and an
unknown key (a committed `store_password`) fails to parse.

```toml
[signing.macos]
team_id = "ABCDE12345"                  # matches the Developer ID Application certificate
notary_profile = "my-notary"            # from `xcrun notarytool store-credentials`

[signing.android]
keystore = "release.jks"                # relative to the project root
key_alias = "upload"                    # passwords: WATERUI_ANDROID_STORE_PASSWORD / _KEY_PASSWORD
```

## Permissions: declaring and requesting

Permission keys: `internet`, `camera`, `microphone`, `location`, `coarse_location`,
`storage`, `write_storage`, `photo_library`, `contacts`, `calendars`, `bluetooth`,
`bluetooth_admin`, `vibrate`, `wake_lock`, `activity_recognition`, `nearby_wifi_devices`.
The CLI translates each into the right platform declaration (`AndroidManifest` entries,
Info.plist usage strings), so the `description` is what the user actually reads in the
system prompt — write it for them.

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

Match `PermissionStatus` with a wildcard arm. On Android a request can return
`PermissionStatus::NotDetermined` until the host activity applies the callback result;
check again rather than treating it as a denial. On denial, leave the gated feature closed
and tell the user how to retry — do not fall back to pretending it works.

## Assets and the app icon

Put one square `Icon.svg` (or `Icon.png`) at the root of `assets/`. The CLI generates every
platform format from it: iOS full-bleed, macOS rounded rect, Android adaptive layers,
Windows `.ico`, Linux `.desktop`. Exactly one root-level `Icon.*` is allowed. New projects
start with the WaterUI logo, so replacing that one file rebrands the app everywhere.

Everything else under `assets/` is bundled as a regular asset. To mount another directory
as a named bundle, write `include_bundle!("fonts-extra", as = extra)` (path relative to the
crate root; needs the `assets` feature): it expands to `pub mod extra` with a `BUNDLE`
constant and one accessor per file, and the CLI stages the directory at package time.

## Launch screen

The launch screen is what the platform shows from the tap on the icon until the app's
first frame. No Rust runs while it is visible — the wasm is still downloading, the dylib
is still loading — so it is declared once in `Water.toml` and staged by the CLI, never
built as a view. `[launch]` carries the background for the light and dark appearance;
one optional square `Launch.svg` (or `Launch.png`) at the root of `assets/` is the
artwork, sized and placed by each platform's own rules. Colors are `#RRGGBB`, and a
malformed one fails when the manifest is read.

Without `[launch]`, the background is `[theme] background`. Defaults follow each platform's
own convention rather than one shared look:

| Platform | Nothing configured | With `Launch.*` |
| --- | --- | --- |
| iOS | system background, no artwork (a launch screen resembles the first screen) | centered, inside the safe area |
| Android | the OS default: the launcher icon on the window background | shown as the splash icon, which Android masks to a circle |
| Web (Hydrolysis) | theme background, the app icon, and a download progress bar | replaces the icon |
| macOS, GTK, desktop Hydrolysis | nothing | ignored |

Desktop platforms have no launch screen and take nothing from `[launch]`: the window
appears with the app's first frame already in it (Hydrolysis keeps it hidden until then,
macOS and GTK create it after the tree exists), so there is no moment for one to show.

## Home-screen widgets

Enable the `widgets` feature (outside `all`). A widget is a function that returns a dated
snapshot, not a view: Apple packaging (`water run`, `water package`) builds a WidgetKit extension that renders
the serialized snapshot natively. The authoring surface is the closed `waterui::widgets`
schema: `text`, `vstack` / `hstack` / `zstack`, `fill`, `link(url, content)` and
`intent(&action, label)`. A WaterUI `View` in widget position does not compile.

```rust
use waterui::widgets::*;

#[waterui::widget_intent(name = "Mark done")]
pub struct MarkDone {
    id: u32,
}

impl WidgetIntent for MarkDone {
    fn perform(self) {
        waterui::log::info!("done {}", self.id);
    }
}

#[waterui::widget(name = "Tasks", description = "Open tasks", families = [systemSmall, systemMedium])]
pub fn tasks(context: WidgetContext) -> impl WidgetContent {
    let title = Font::style(if context.family() == WidgetFamily::SystemSmall {
        FontStyle::Headline
    } else {
        FontStyle::Title
    });
    vstack((
        text("Tasks").font(title),
        intent(&MarkDone { id: 1 }, text("Mark done")),
    ))
    .alignment(HorizontalAlignment::Leading)
}
```

`name` and `description` are required; `kind` defaults to the function name and `families`
to `[systemSmall, systemMedium, systemLarge]`. The `WidgetContext` parameter is optional
(by value or by reference). Return a `Timeline` of `TimelineEntry`s with a `ReloadPolicy`
for dated entries; any plain `WidgetContent` becomes one entry dated now. `#[widget_intent]`
declares the action type, and `WidgetIntent::perform` runs in the extension's process.

## Running and building

```bash
water run                              # host platform
water run --platform ios
water run --platform android
water run --platform macos
water run --platform linux --backend hydrolysis
water run --device <id>                # a specific simulator or device (name, UDID, serial or AVD)
water run --logs debug                 # stream device logs at debug and above
water run --native-logs                # include native platform logs too — noisy
water run --env NAME                   # export NAME from this shell into the launched app (repeatable)
water run --release                    # also --profiling (optimized + symbols) and --debug (fully unoptimized)
water run --platform android --painter hwui  # Android + hydrolysis only: the painter (gpu | hwui)
water run --no-dev-server              # an include_web! mount renders the staged bundle, not the dev server
water run --tui                        # run in this terminal on the experimental TUI backend

water build --platform ios             # compile the Rust library for a platform (--platform is required)
water package --platform ios           # package the production (release) build; --debug for an unoptimized one
water package --platform ios --device <udid>  # provision the package for a specific device
water package --platform android --arch arm64 # Android needs --arch (comma-separated)
water package --platform android --arch arm64 --distribution  # store build (AAB / App Store); --unsigned leaves it unsigned
water devices                          # list simulators and devices (--platform to narrow)
water doctor                           # check the toolchain; --fix repairs what it can
water clean                            # --backend, --recursive, --global-cache
water gc build-cache                   # garbage-collect stale build caches; --dry-run reports only
water gc build-cache --shared-target   # instead remove the shared Cargo target dir (~/.water/build_cache/target) now
water inspector                        # launch the inspector app
water completions fish                 # shell completion script
```

`--offline` (build, run, package, preview, mcp, inspector) fails on a declared font missing
from the cache instead of fetching it. `-y` skips the confirmation the experimental backends
(`gtk4`, `winui`) require; a non-interactive run needs it. The global
`--json` flag prints machine-readable output. Previews, tests, benchmarks and the agent session are
`water preview` and `water preview test` (`references/testing.md`), `water bench`
(`references/testing.md`) and `water mcp` (`references/mcp.md`); `water device capture|tap|swipe|text|describe`
drives a connected device; `water device tap|swipe|text --diff [--diff-output <png>]` takes before/after screenshots around the gesture and reports the difference.

`water run` does not exit while the app is alive. **A `water run` that "completes" means
the app crashed** — read the log tail rather than treating it as success.

## Platforms and backends

Declare a backend once in `Water.toml` with `[platforms.linux]` and
`backend = "hydrolysis"`, for example. The platform names are `ios`,
`ios-simulator`, `macos`, `android`, `linux`, `windows` and `web`; backend names are
`apple`, `gtk4`, `hydrolysis` and `winui`, exactly as in CLI flags. Unknown names and
backends unsupported on the selected platform are manifest errors. `--platform`
takes the same names, except `water run` has no `ios-simulator` (`ios` runs a simulator or a
paired device) and `water build` has no `web` (`water package --platform web` and
`water run --platform web` do).

`water build`, `water run`, `water package`, and CLI/MCP previews use the declaration.
An explicit `--backend` (MCP: `backend`) may equal it, but a different value is an error
before building. Without a declaration an explicit backend wins; without either,
the defaults below apply. `water create` leaves the tables absent.

| Platform | Default backend | Also possible |
|---|---|---|
| macOS | `apple` (AppKit) | `hydrolysis` |
| iOS (+ simulator) | `apple` (UIKit) | — |
| Android | `hydrolysis` | — |
| Linux | `gtk4` | `hydrolysis` |
| Windows | `hydrolysis` | `winui` |
| Web | `hydrolysis` (WASM + WebGPU) | — |

`apple` and `hydrolysis` are the mainline backends; `gtk4` (the Linux default) and `winui` are
experimental, with no stability guarantee, and the CLI asks for confirmation (`-y`) before
building with one. On Android, `[hydrolysis] painter` (or `--painter`) picks `gpu` or `hwui`.

Previews support only `apple` on iOS/macOS and `hydrolysis` on macOS/Linux/Windows/Android.
A resolved backend without preview support is an error, not a fallback. For Linux
previews, declare `hydrolysis` or pass `--backend hydrolysis` on an undeclared project.
`water preview test` requires Hydrolysis and rejects a conflicting declaration too.

Native backends bridge to real platform widgets. `hydrolysis` is WaterUI's own
renderer: it is GPU-required and targets high-refresh modern hardware. Choosing a
self-drawn renderer is a deliberate decision, never a fallback for a native path that
failed.

TypeScript views (`tsx!`) run on an embedded JavaScript engine that differs by platform:
JavaScriptCore on iOS, macOS, tvOS and visionOS; QuickJS-NG on Android, Linux and Windows;
unsupported on watchOS — the platform ships no JavaScriptCore and embedding QuickJS-NG
there is not supported. The asymmetry is documented, not faked: the `ts` feature does
not build on watchOS, a compile error rather than a fallback.

The same view code runs on all of them. Platform-specific behavior belongs in the backend,
not in conditional app code. Apple-only code (a component's AppKit or UIKit realization) that
must choose between a newer and an older realization by OS version uses
`apple_available!(27.0)`: `objc2::available!` for iOS, macOS, tvOS, watchOS and visionOS
(one number covers them all). The version must be one `waterui-apple-os` lists, the crate
must depend on `waterui-apple-os`, and the expansion builds for Apple targets only; once the
deployment target meets the version the call site warns `#[deprecated]`, the cue to delete
the older branch.

## Logging and debugging

Use `tracing`, never `println!` — printed output does not reach the device log pipeline.
You do not need `tracing` in your own `Cargo.toml`: the whole crate is re-exported as
`waterui::log`:

```rust
use waterui::log::{debug, info};

debug!(?value, "recomputed layout");
info!("saved");
```

```bash
water run --logs debug
```

`water inspector` attaches to a running debug build and shows its frame timings, the live
accessibility tree with each node's bounds, main-thread occupancy, logs, and the reactive
graph. When a layout looks wrong, read the
accessibility bounds rather than eyeballing the picture — bounds tell you which container
mis-sized a child; a screenshot only tells you something is off.
