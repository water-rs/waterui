---
name: waterui
description: Build cross-platform apps with WaterUI. Use when writing views, handling state, styling UI, or debugging WaterUI Rust code. Covers reactive bindings, layout, components, and the water CLI.
---

# WaterUI App Development

Build views with reactive state. When unsure, search `examples/*/src/lib.rs` for existing patterns.

## CRITICAL: Runtime And Testing Semantics

- WaterUI is fine-grained reactive with reconstruction semantics. If parent-driven control flow rebuilds a component instance, that instance's local state resetting is expected and correct.
- Do not fix rebuild-driven resets by caching hidden state across rebuilds. If state must survive, lift it into explicit reactive ownership at the right level.
- Prefer `Binding`, `Computed`, and other signal inputs for dynamic behavior. Avoid reading `.get()` in view bodies when a reactive API can accept the signal directly.
- Hydrolysis widget chrome is provided by a backend-neutral `WidgetTheme`. For Material Design 3 output, install `hydrolysis_m3::install(&mut env)` before running or previewing a Hydrolysis app.
- Hydrolysis Material 3 colors use Material You system roles for WaterUI theme tokens. Use `hydrolysis_m3::MaterialColorSource::new(source_color)` to configure source color, variant, contrast level, spec version, and platform; call `.schemes()` to generate paired light/dark `MaterialColorSchemes`, then install one side with `install_with_source`, `install_with_color_schemes`, or `install_with_colors`.
- For Material-specific view colors, use `hydrolysis_m3::color::*` role tokens such as `Primary`, `OnPrimary`, `PrimaryContainer`, `SurfaceContainerHighest`, and `OnSurfaceVariant`. These resolve from the installed `MaterialColorScheme`, while WaterUI's portable `theme_color::*` tokens map semantic roles such as `Accent`, `AccentContainer`, `Tertiary`, and `TertiaryContainer` to the active platform theme.
- Use `water preview --backend hydrolysis --theme material3` for Hydrolysis Material 3 visual checks. Pass `--expr` when previewing an inline Rust expression; the CLI writes the expression into generated Rust preview code and rustc compiles it normally with `waterui::prelude::*` and `waterui` in scope.

## CRITICAL: Reactive-First Pattern

**WaterUI is a reactive framework. ALWAYS pass Bindings directly to APIs instead of using `.get()` or `watch`.**

Most WaterUI APIs accept `impl Signal` or `impl IntoSignalF32` - pass bindings directly for automatic reactivity:

```rust
// ✅ CORRECT - Pass binding directly, updates automatically
Photo::new(url).blur(blur_value.clone())       // blur updates as slider moves
view.visible(is_visible.clone())               // visibility reacts to state
view.opacity(opacity_value.clone())            // opacity animates reactively
view.disabled(is_loading.clone())              // disabled state follows loading
text!("Count: {count}")                        // text updates automatically

// ❌ WRONG - Static value, requires manual refresh
Photo::new(url).blur(blur_value.get())         // blur frozen at initial value
view.visible(is_visible.get())                 // visibility never changes
watch(count.clone(), |c| text(format!("{c}"))) // unnecessary indirection
```

**Rule: If an API accepts a value that might change, check if it accepts `impl Signal` and pass the binding.**

## Quick Start

```rust
use waterui::prelude::*;

fn main() -> impl View {
    let count = Binding::i32(0);

    vstack((
        text!("Count: {count}").headline(),
        button("+1")
            .action(|State(count): State<Binding<i32>>| count.set(count.get() + 1))
            .state(&count),
    ))
}
```

## Views

Functions and closures are views:
```rust
fn card(title: &str) -> impl View {
    vstack((text(title).title(), Divider))
}

// Use directly - no wrapper needed
vstack((card("Hello"), card("World")))
```

Conditional rendering:
```rust
// Show or hide (Option<impl View> is a View)
is_new.map(|b| b.then(|| badge("New")))

// Binary choice (if-else)
when(is_logged_in, || dashboard()).otherwise(|| login_form())

// Multi-branch (if-elif-else)
when(state.equal_to(0), || "Loading")
    .or(state.equal_to(1), || "Ready")
    .otherwise(|| "Error")
```

## State

```rust
// Use type-specific constructors (Binding::new does NOT exist)
let toggle = Binding::bool(false);
let count = Binding::i32(0);
let value = Binding::f64(1.5);
let name = Binding::container(String::new());  // heap types (String, Vec, etc.)
let text = Binding::container(Str::from("hello")); // Str type

// Pass by reference to child views
fn section(count: &Binding<i32>) -> impl View { ... }
```

## Reactive Transforms

Methods on signals (no `.clone()` needed for transforms):
```rust
count.not()                    // bool negation
count.select(a, b)             // if-else
count.equal_to(5)              // equality check
count.gt(0)                    // comparisons: lt, le, ge
count.is_empty()               // for strings/collections
count.map(|v| v * 2)           // custom transform
count.zip(&other).map(|(a,b)| a + b)  // combine signals
```

Convert to Computed: `signal.computed()`

## Reactive Modifiers

**Pass bindings directly to modifiers for real-time updates:**

```rust
let opacity = Binding::f64(1.0);
let blur = Binding::f64(0.0);
let is_visible = Binding::bool(true);
let is_disabled = Binding::bool(false);
let scale_factor = Binding::f64(1.0);

view
    .opacity(opacity.clone())           // reactive opacity
    .visible(is_visible.clone())        // reactive visibility
    .disabled(is_disabled.clone())      // reactive disabled state
    .scale(scale_factor.clone(), scale_factor.clone())  // reactive scale

// Filters also accept reactive values
Photo::new(url)
    .blur(blur.clone())                 // blur updates in real-time
    .saturation(saturation.clone())     // saturation updates in real-time
    .brightness(brightness.clone())     // brightness updates in real-time
```

## Event Handlers

**IMPORTANT: Inject handler state with `.state()` after `.action()` or
`.action_async()`; never clone bindings just to capture them. `State<T>` extracts
the injected value from the action environment.**

```rust
// Single state
button("Click")
    .action(|State(count): State<Binding<i32>>| count.set(count.get() + 1))
    .state(&count)

// Multiple states are separate typed extractor parameters
button("Reset")
    .action(
        |State(x): State<Binding<i32>>, State(y): State<Binding<i32>>| {
            x.set(0);
            y.set(0);
        },
    )
    .state(&x)
    .state(&y)

// Four states example
button("Submit")
    .action(
        |State(url): State<Binding<Str>>,
         State(blur): State<Binding<f64>>,
         State(status): State<Binding<String>>,
         State(handler): State<DynamicHandler>| {
            // Use all four injected values
        },
    )
    .state(&url)
    .state(&blur)
    .state(&status)
    .state(&handler)

// Async
button("Load").action_async(|_| async { fetch().await })

// Lifecycle
view.on_appear(|| setup())
view.on_change(&signal, |new_val| handle(new_val))
```

## Text

**IMPORTANT: Use `text()` for static text and `text!` for reactive text. Never use `watch()` just to build text. Also never write `waterui::text!`; import the macro and use `text!` directly.**

```rust
// Static text - use text() function
text("Hello").title()       // semantic sizes: title, headline, body, caption, footnote, sub_headline

// Reactive text - use text! macro (auto-updates when bindings change)
text!("Count: {count}")              // single binding
text!("{a} + {b} = {sum}")           // multiple bindings
text!("Value: {value:.2}")           // with formatting
text!("{FOCUSED_READOUT}")           // const &str capture is fine if text! behavior is desired

// text! returns LocalizedText with font methods
text!("Status: {status}").sub_headline()
text!("Small: {value}").caption()
```

## Layout

```rust
hstack((a, b, c)).spacing(8.0)
vstack((a, b)).padding()
zstack((background, content))
scroll(content)
spacer()                    // flexible space
spacer().height(16.0)       // fixed space

// From iterator - use .collect() for dynamic layouts
let buttons: HStack<_> = items.iter().map(|i| button(i.label)).collect();
```

Programmatic scrolling is explicit and repeatable through `ScrollController`.
`List` targets item indices, while `ScrollView` targets content coordinates:

```rust
let list_scroll = ScrollController::<usize>::new(0);
let list = List::for_each(rows, row_view).scroll_controller(&list_scroll);
list_scroll.scroll_to(50_000);

let content_scroll = ScrollController::<Point>::new(Point::zero());
let viewport = scroll(lazy_content).scroll_controller(&content_scroll);
content_scroll.scroll_to(Point::new(0.0, 2_400.0));
```

`List::for_each` and `ScrollView` containing a lazy stack materialize only the
visible window, including after a deep programmatic jump. Use `List::content`
when section markers or heterogeneous static rows are required.

## Colors

```rust
// Built-in (zero-sized, efficient)
Blue, Green, Red, Orange, Purple, Cyan, Yellow, Pink, Grey

// Custom
const BRAND: Srgb = Srgb::from_hex("#3B82F6");

// Usage - colors are Views
view.background(Blue)
view.foreground(BRAND)
Blue.size(80.0, 80.0)       // colored rectangle
BRAND.with_opacity(0.5)
```

Theme colors: `Foreground`, `MutedForeground`, `Accent`, `AccentContainer`,
`AccentForeground`, `Tertiary`, `TertiaryContainer`, `Background`, `Surface`,
`SurfaceVariant`, `Border`

## Icons

Icons come from packaged icon-set crates — pick **one set per app** and depend on it:
`waterui-icons-material-icon` (Material Symbols), `waterui-icons-lucide`,
`waterui-icons-fontawesome7`, `waterui-icons-sf-symbol` (Apple only).

```rust
use waterui_icons_material_icon as mdi;

mdi::check_circle()                   // an icon view
mdi::delete().size(20.0, 20.0)        // size it
mdi::flag().foreground(Accent)        // theme color
mdi::calendar_today().tint(Srgb::from_hex("#4A84F6"))  // explicit tint
```

Match the icon set to the design language: a Material 3 (`hydrolysis_m3`) app uses
**Material** icons, not Lucide. `SystemIcon`/SF Symbols are Apple-only — for
portable code depend on a cross-platform set instead.

## Modifiers

```rust
.padding() / .padding_with(EdgeInsets::all(16.0))
.background(color) / .foreground(color)
.size(w, h) / .width(w) / .height(h)
.scale(x, y) / .rotation(degrees) / .offset(x, y)
.border(color, width) / .shadow() / .clip(shape)
.disabled(bool_signal) / .visible(bool_signal)  // accept signals!
.opacity(f64_signal)                             // accepts signal!
```

## Components

| Category | Components |
|----------|------------|
| Layout | `hstack`, `vstack`, `zstack`, `scroll`, `spacer`, `grid` |
| Controls | `button`, `toggle`, `Slider`, `Stepper`, `TextField`, `Menu`, `Picker` |
| Navigation | `NavigationStack`, `NavigationLink`, `NavigationSplitView`, `TabView` |
| Collections | `List`, `ForEach` (see below) |
| Overlays | `Snackbar` / `SnackbarManager`, `FullScreenOverlayManager` |
| Media | `Photo`, `VideoPlayer`, `MediaPicker` |
| Data | `Chart`, `Map`, `form` (`#[derive(FormBuilder)]`) |
| Platform | `WebView` |
| Graphics | `Canvas`, `Barcode::qr()`, `Icon` sets (see Icons) |

### Embedded browsers

Use `WebView` for ordinary embedded web content. It uses the platform system
web view by default, except on Linux where the default is WaterUI's bundled WPE
runtime. Select an explicit engine in `Water.toml` when the application requires
one:

```toml
webview_backend = "default" # default | system | wpe | cef
```

`system` selects the platform web view, including WebKitGTK on Linux. `wpe`
selects the bundled WPE runtime on Linux. `cef` selects the bundled CEF runtime
on macOS, Linux, and Windows and is independent of the selected WaterUI backend;
the browser is still embedded as a WaterUI view. Dew deliberately excludes CEF
because its constrained-device targets cannot carry the Chromium runtime.

Depend on `waterui-chromium` when the application needs a visible Chromium
surface, headless pages, screenshots, or raw/typed Chrome DevTools Protocol
access. Chromium is an independent advanced component rather than a `WebView`
backend, and links its bundled CEF runtime regardless of whether the surrounding
WaterUI renderer is native or self-drawn.

## Collections (dynamic lists)

For a **changing set** of views, use `ForEach`/`List` — NOT `watch`. The collection
diffs by `Identifiable` id, so adding/removing one item updates precisely instead of
rebuilding the whole subtree.

```rust
#[derive(Clone, Identifiable)]
struct Row {
    #[id]
    id: u64,
    title: Str,
}

// A fixed collection stays on the stack.
let rows = [
    Row { id: 1, title: "First".into() },
    Row { id: 2, title: "Second".into() },
];
List::for_each(rows, |row| ListItem::new(text(row.title)))

// Reactive collection backed by nami's List<T> (push/remove updates the UI)
let rows = nami::collection::List::<Row>::new();
rows.push(Row { id: 1, title: "Hello".into() });
ForEach::new(rows.clone(), |row| text(row.title))
```

A fixed, known set is an array (`[a, b, c]`) or a tuple stack
(`vstack((a, b, c))`). Pass an array directly when an API accepts it through
`Collection`, `IntoIterator`, or another generic collection trait. Do not allocate
with `vec!` unless the collection has a runtime-dependent length, needs mutation,
or the API specifically requires a `Vec`.

## Snackbars (transient bottom/top messages)

Every `Window` auto-installs a `SnackbarManager`; reach it from a handler via
`State<SnackbarManager>` and call `.show(...)`:

```rust
button("Save").action(|State(m): State<SnackbarManager>| {
    m.show(Snackbar::new("File saved"));
});

m.show(
    Snackbar::new("Item moved to trash")
        .icon(mdi::delete())                       // optional leading icon
        .action("Undo", || { /* runs, then dismisses */ })
        .position(SnackbarPosition::TopCenter)     // BottomCenter (default)/TopCenter/BottomLeading/BottomTrailing
        .closeable()                               // trailing ✕ close button
        .duration(Duration::from_secs(5)),         // Duration::ZERO = stay until dismissed
);
```

Different placements are independent stacks (a top snackbar never evicts a bottom
one). Multiple at the same placement stack and reflow automatically.

## Testing (`#[waterui::test]` + waterui-testing)

Accessibility-first UI tests run under plain `cargo test`. Add
`waterui-testing` and `hydrolysis-m3` as dev-dependencies.

```rust
use waterui_testing::{Role, SemanticApp, UiBuilder};

// Mounting form: the macro mounts a no-arg view function.
#[waterui::test(login_view, theme = hydrolysis_m3::install, viewport = (360, 320))]
fn login_flow(app: &mut SemanticApp) {
    app.query().role(Role::BUTTON).label("Login").tap();
    app.query().label("Welcome").assert_exists();
}

// Manual-mount form: the test owns Bindings the view closes over.
#[waterui::test(theme = hydrolysis_m3::install)]
fn stepper_updates(ui: UiBuilder) {
    let value = Binding::i32(2);
    let value_for_view = value.clone();
    let mut app = ui.mount(move || stepper("Limited", &value_for_view));
    app.query().label("Limited").increment();
    assert_eq!(value.get(), 3);
}
```

- Interactions (`tap`, `set_text`, `increment`, `focus`, drags, keys) return `()`
  and panic when the runtime rejects the action — the call itself is the assertion.
  After each interaction the session settles to quiescence automatically.
- Query by semantics: `app.query().role(Role::SWITCH).label("Wi-Fi")`, then
  `.assert_exists()` / `.single()` / `.all()`. Tag hard-to-label views with
  `.a11y_id("settings.wifi")` and query `.identifier("settings.wifi")` — the same
  identifier reaches XCUITest (`accessibilityIdentifier`) and Android automation.
- Waits mirror XCTest: `app.wait_for_existence(&selector, timeout)`,
  `app.wait_for(&[expectation], WaitOptions::new(timeout))`.
- Visual tests: add the `offscreen` flag (parameter becomes `&mut OffscreenApp`)
  or call `ui.mount_offscreen(...)`; `app.pump_for(duration)` advances the
  virtual animation clock exactly, and `app.capture_snapshot(suite, case, stage)`
  writes PNGs under `WATERUI_TEST_ARTIFACTS_DIR`.
- Keyboard with modifiers: `app.press_named_key_with("Tab", modifiers)`.
  Gesture pacing: `query.drag_by_with(dx, dy, DragOptions { steps: 12, frame_per_step: true })`.

## CLI Commands

```bash
water create my-app              # new project
water run --platform ios         # run on simulator
water run --platform android
water run --platform macos
water preview my_view            # preview #[preview] function
water run --logs debug           # with debug output
```

## Preview System

Use the `#[preview]` macro to enable instant view previews:

```rust
#[preview]
fn my_card() -> impl View {
    text!("Hello Preview!")
}
```

Run previews with `water preview my_card --platform macos`.

## Common Patterns

```rust
// Reactive blur with slider (real-time updates)
let blur = Binding::f64(0.0);
vstack((
    Photo::new(url).blur(blur.clone()),  // blur reacts to slider
    Slider::new(0.0..=10.0, &blur),
    text!("Blur: {blur:.1}"),
))

// Animated toggle
let scale = active.select(1.2 as f32, 1.0).with(Animation::spring(300.0, 15.0));

// Conditional visibility (reactive)
.visible(items.map(|i| !i.is_empty()).computed())

// List rendering
List::for_each(&items, |item| item_view(item))

// Static layout from slice/array via FromIterator
fn tab_buttons(tabs: &[Tab], selected: &Binding<Tab>) -> HStack<(Vec<AnyView>,)> {
    tabs.iter()
        .map(|&tab| {
            button(tab.label())
                .action(move |State(selected): State<Binding<Tab>>| selected.set(tab))
                .state(selected)
        })
        .collect()
}

// Conditional views - prefer when().otherwise() over match
when(is_dark, || dark_theme()).otherwise(|| light_theme())
when(!is_loading, || content()).otherwise(|| spinner())

// Multi-branch conditionals
when(state.equal_to(0), || loading_view())
    .or(state.equal_to(1), || ready_view())
    .or(state.equal_to(2), || error_view())
    .otherwise(|| unknown_view())

// For many branches or complex matching, use match + .anyview()
fn render(mode: Mode) -> AnyView {
    match mode {
        Mode::A => view_a().anyview(),
        Mode::B => view_b().anyview(),
        Mode::C => view_c().anyview(),
    }
}

// Form from struct
#[derive(FormBuilder)]
struct Settings { name: String, volume: f64 }
form(&settings_binding)

// Dynamic view for URL changes (Photo with reactive blur)
let url_input: Binding<Str> = Binding::container(initial_photo_url);
let blur = Binding::f64(0.0);
let status: Binding<String> = Binding::container(String::from("Loading..."));
let (handler, photo_view) = Dynamic::new();

// Load button - only Dynamic for URL change, blur is reactive
button("Load")
    .action(
        |State(url): State<Binding<Str>>,
         State(blur): State<Binding<f64>>,
         State(status): State<Binding<String>>,
         State(handler): State<DynamicHandler>| {
            let photo = Photo::new(url.get())
                .on_event({
                    let status = status.clone();
                    move |event| match event {
                        PhotoEvent::Loaded => status.set(String::from("Loaded")),
                        PhotoEvent::Error(msg) => status.set(format!("Error: {msg}")),
                    }
                })
                .blur(blur.clone());  // Pass binding for reactive blur!
            handler.set(photo);
        },
    )
    .state(&url_input)
    .state(&blur)
    .state(&status)
    .state(&handler);

vstack((
    text!("{status}"),
    photo_view,
    Slider::new(0.0..=10.0, &blur),  // Slider controls blur in real-time
))
```

## Extension Traits

WaterUI uses `*Ext` traits. When unsure, search `trait.*Ext` in codebase.

**SignalExt** (from nami, works on `Binding`/`Computed`):
```rust
// Core
.map(|v| ...), .zip(&other), .computed(), .cached(), .distinct(), .with(metadata)

// Bool → Signal<bool>
.not(), .select(if_true, if_false), .then_some(value)

// Comparison → Signal<bool>
.equal_to(v), .gt(v), .lt(v), .ge(v), .le(v), .condition(|v| ...)

// Option<T>
.is_some(), .is_none(), .unwrap_or(default), .map_some(|v| ...)

// String
.is_empty(), .contains("pattern")
```

**ViewExt**: `.anyview()`, `.visible()`, `.padding()`, `.background()`, etc.

**AnimationExt**: `.animated()`, `.with(Animation::spring(...))`

## Gotchas

**No `Binding::new()`** - use type-specific constructors:
```rust
// WRONG
let count = Binding::new(0);

// CORRECT
let count = Binding::i32(0);
let value = Binding::f64(1.5);
let flag = Binding::bool(false);
let name = Binding::container(String::new());
```

**No `_f32` suffix** - use `as f32` cast:
```rust
// WRONG
.select(1.0_f32, 0.3)

// CORRECT
.select(1.0 as f32, 0.3)
```

**No `.get()` for reactive values** - pass binding directly:
```rust
// WRONG - static, won't update
Photo::new(url).blur(blur.get())
view.opacity(opacity.get())

// CORRECT - reactive, updates automatically
Photo::new(url).blur(blur.clone())
view.opacity(opacity.clone())
```

**Don't reach for `watch()`** — it rebuilds and replaces the whole subtree on every
change (losing that subtree's internal state), so it's almost never what you want.
Three things replace nearly every use:
```rust
// Reactive text → text!        (not watch(status, |m| text(m)))
text!("{status}")
// Reactive value → pass the binding to the API    (not watch(blur, |b| ...blur(b)))
Photo::new(url).blur(blur.clone())
// Dynamic set of views → ForEach/List             (not watch over a Vec)
ForEach::new(rows.clone(), |row| row_view(row))
```
Only reach for `watch` for a genuinely one-off structural swap with no signal-aware
API and no collection — check those three first.

**No manual `.clone()` for button states** - inject with `.state()` after
configuring the action:
```rust
// WRONG
let count_clone = count.clone();
button("Click").action(move || count_clone.set(...))

// CORRECT
button("Click")
    .action(|State(count): State<Binding<i32>>| count.set(...))
    .state(&count)
```

**Two-param transforms:**
```rust
.scale(x, y)    // not .scale(uniform)
.offset(x, y)
.size(w, h)
```

**`text!` returns `LocalizedText`** - supports all font methods:
```rust
// LocalizedText has: .title(), .headline(), .sub_headline(), .body(), .caption(), .footnote()
// Plus: .size(), .bold(), .italic(), .font()
text!("{status}").sub_headline()
text!("{value}").caption()
text!("{note}").footnote()
```

## Embedded (dew backend, experimental)

WaterUI runs on microcontrollers through the `waterui-dew` backend
(CPU rasterization, no GPU, dirty-region flushes sized for SPI panels).
The same views, bindings, and `text!` reactivity work unchanged.

Develop against the desktop panel simulator — the full embedded rendering
flow in a native window, no cross-compilation:

```bash
cargo run -p waterui-dew --example watch_sim --features embedded-simulator
```

Headless snapshot: `waterui_dew::render_view_png(builder, env, w, h)`.

Currently supported views: stacks/padding, colors, spacers, text.
Unsupported views panic fast with a clear message instead of rendering
wrong. ESP32-S3 firmware lives in `examples/embedded/dew-esp32s3/`
(see its README for QEMU and real-hardware flows and the current Xtensa
toolchain blocker).
