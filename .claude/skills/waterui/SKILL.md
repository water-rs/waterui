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
- Use `waterui-testing` for UI component coverage when testing WaterUI components; it validates the Hydrolysis accessibility tree as both interaction behavior and accessibility output.
- Every UI component should expose a meaningful accessibility tree. If a component is not testable through `waterui-testing`, improve the component or renderer rather than relying on weak assertions.
- Preview renders should use the real WaterUI renderer so visual checks reflect actual platform output.
- WaterUI layout lengths are logical view units, not physical pixels. Treat Material metrics as dp-equivalent design units in Hydrolysis themes.
- Hydrolysis widget chrome is provided by a backend-neutral `WidgetTheme`. For Material Design 3 output, install `hydrolysis_m3::install(&mut env)` before running or previewing a Hydrolysis app.
- Hydrolysis Material 3 colors use Material You system roles for both WaterUI theme tokens and backend widget chrome. Use `hydrolysis_m3::MaterialColorScheme::from_seed(...)` plus `install_with_colors(...)` when a preview or app needs a custom source color; use `hydrolysis_m3::install_with_seed_mode(...)` for explicit light/dark generated schemes. Button, toggle, slider, progress, picker, input, navigation, list, table, and scroll chrome should read roles from that scheme.
- For Material-specific view colors, use `hydrolysis_m3::color::*` role tokens such as `Primary`, `OnPrimary`, `PrimaryContainer`, `SurfaceContainerHighest`, and `OnSurfaceVariant`. These resolve from the installed `MaterialColorScheme`, while WaterUI's generic `theme_color::*` tokens remain mapped to the closest Material roles.
- Hydrolysis Material 3 installs Material Web v0.192 Roboto typography into WaterUI's semantic font slots, so `.body()`, `.title()`, `.headline()`, `.sub_headline()`, `.caption()`, and `.footnote()` inherit Material type scale in `--theme material3` previews and apps.
- Hydrolysis Material 3 button chrome applies the Material `label-large` font through the backend widget theme, so ordinary `button("Save")` labels match Material typography without losing their semantic label or disabled accessibility state.
- Hydrolysis Material 3 text-style buttons (`.plain()` / `.borderless()`) keep Material text-button touch metrics: 40 logical-unit container height, 64 logical-unit minimum width, and 12 logical-unit horizontal label padding, even though their normal chrome remains transparent.
- Hydrolysis Material 3 progress indicators follow Material Web v0.192 metrics: linear track and active indicator are 4 logical units high, circular progress is 48 logical units square, and circular active indicator stroke width is 4 logical units.
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
            .with_state(&count)
            .action(|c| c.set(c.get() + 1)),
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

**IMPORTANT: Always use `.with_state()` - never clone bindings manually!**

```rust
// Single state - receives Binding directly
button("Click")
    .with_state(&count)
    .action(|c| c.set(c.get() + 1))

// Multiple states → nested tuple (((a, b), c), d)
button("Reset")
    .with_state(&x)
    .with_state(&y)
    .action(|(x, y)| { x.set(0); y.set(0); })

// Four states example
button("Submit")
    .with_state(&url)
    .with_state(&blur)
    .with_state(&status)
    .with_state(&handler)
    .action(|(((url, blur), status), handler)| {
        // Use all four bindings
    })

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

Theme colors: `Foreground`, `MutedForeground`, `Accent`, `Background`, `Surface`, `Border`

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
| Controls | `button`, `toggle`, `Slider`, `Stepper`, `TextField`, `Menu` |
| Navigation | `NavigationStack`, `NavigationLink`, `TabView` |
| Media | `Photo`, `VideoPlayer`, `MediaPicker` |
| Data | `Chart`, `Map` |
| Platform | `WebView` |
| Graphics | `Canvas`, `Barcode::qr()` |

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
        .map(|&tab| button(tab.label()).with_state(selected).action(move |s| s.set(tab)))
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
let url_input = Binding::container(Str::from("https://example.com/image.jpg"));
let blur = Binding::f64(0.0);
let status = Binding::container(String::from("Loading..."));
let (handler, photo_view) = Dynamic::new();

// Load button - only Dynamic for URL change, blur is reactive
button("Load")
    .with_state(&url_input)
    .with_state(&blur)
    .with_state(&status)
    .with_state(&handler)
    .action(|(((url, blur), status), handler)| {
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
    });

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

**No `watch()` for text**:
```rust
// WRONG
watch(status.clone(), |msg| text(msg))

// CORRECT: reactive
text!("{status}")

// CORRECT: static
text("Ready")
```

**No `watch()` when reactive API exists** - pass binding directly:
IMPORTANT: `watch` would rebuild the entire subtree on every change and lost internal state, only a few scenarios require `watch`, always check if the API accepts `impl Signal` first
```rust
// WRONG - unnecessary watch
watch(blur.clone(), |b| Photo::new(url).blur(b))

// CORRECT - pass binding directly
Photo::new(url).blur(blur.clone())
```

**No manual `.clone()` for button states** - use `.with_state()`:
```rust
// WRONG
let count_clone = count.clone();
button("Click").action(move || count_clone.set(...))

// CORRECT
button("Click").with_state(&count).action(|c| c.set(...))
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
