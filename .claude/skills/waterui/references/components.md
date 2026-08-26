# Component catalog

Signatures verified against the WaterUI source and compiled. Everything here is reachable
from `use waterui::prelude::*;` unless an explicit import is shown.

**Feature gates.** `waterui`'s default features are `gpu`, `assets`, `media`, `inspector`,
and `snackbar`. `webview`, `chart`, `barcode`, `map`, `particle`, `flow-markdown`, and
`navigation-restoration` are opt-in — a missing module here is usually a missing feature in
`Cargo.toml`, not a wrong path:

```toml
waterui = { version = "…", features = ["chart", "map", "barcode"] }
```

## Contents

- Naming conventions
- Layout containers
- Scrolling
- Controls
- Text
- Lists and collections
- Forms and pickers
- Overlays: snackbars, cards, suspense, full screen
- Media
- Graphics and codes
- Data: charts and maps
- Embedded web content
- Accessibility modifiers

## Naming conventions

Two entry points per component, deliberately:

- **`Type::new(...)`** is the general constructor. It accepts the most general shape the
  component can render — arbitrary label views, raw configuration.
- **`lowercase(...)`** is the ergonomic constructor. It takes narrower, friendlier input
  (`impl IntoLabel`, `impl IntoText`) so string literals flow into the localized text
  pipeline and pick up correct default accessibility semantics.

Prefer the lowercase form in app code. Reach for `Type::new` when the label must be a
composed view rather than text.

There is no `Type::custom(...)` anywhere; if you find yourself looking for one, `new` is it.

## Layout containers

```rust
hstack((a, b, c))        // horizontal
vstack((a, b, c))        // vertical
zstack((back, front))    // depth, later children on top
```

Common modifiers on stacks:

```rust
.spacing(8.0)
.alignment(HorizontalAlignment::Leading)   // VerticalAlignment on hstack
.padding()  /  .padding_with(EdgeInsets::symmetric(10.0, 16.0))
```

Children are a tuple for a fixed set. For a runtime-length static set, collect:

```rust
let row: HStack<_> = tabs.iter().map(|t| button(t.label())).collect();
```

Spacing and separation:

```rust
spacer()                    // flexible, absorbs leftover space
spacer_min(12.0)            // flexible with a floor
spacer().height(16.0)       // fixed gap
Divider                     // a separator line (a unit struct, no call)
```

Grid:

```rust
grid(columns, rows)         // rows: impl IntoIterator<Item = GridRow>
```

Absolute placement — `AbsoluteLayout` hands every child the full bounds of its parent.
That is what a window-filling overlay layer needs; a content-sized `zstack` mis-anchors
its children as soon as the window is larger than the content.

## Scrolling

```rust
scroll(content)              // vertical
scroll_horizontal(content)
scroll_both(content)
```

Programmatic scrolling goes through a `ScrollController`, which is explicit and
repeatable. `List` addresses item indices; `ScrollView` addresses content coordinates.

```rust
let rows = ScrollController::<usize>::new(0);
let list = List::for_each(records, row_view).scroll_controller(&rows);
rows.scroll_to(50_000);                       // does not materialize rows 0..50_000

let offset = ScrollController::<Point>::new(Point::zero());
let view = scroll(content).scroll_controller(&offset);
offset.scroll_to(Point::new(0.0, 2_400.0));
```

## Controls

Every control's first argument is its accessibility label. That is a requirement of the
API, not a convention.

```rust
button("Save")                              // Button<fn(&Environment)>
    .action(handler)                        // -> Button<impl FnMut(&Environment)>
    .action_async(|| async { … })
    .style(ButtonStyle::Plain)              // also .bordered() .bordered_prominent() .plain()
    .state(&value)                          // inject handler state (repeatable)

toggle("Wi-Fi", &enabled)                   // &Binding<bool>
slider("Volume", &level).range(0.0..=1.0)   // &Binding<f64>
stepper("Quantity", &count)                 // &Binding<i32>
field("Email", &address)                    // &Binding<Str>
progress(fraction)                          // impl IntoComputed<f64>
loading()                                   // indeterminate progress
```

`Label` composes text with an icon, and is what `impl IntoLabel` resolves to:

```rust
use waterui::component::label::label;

label("Compose").icon(mdi::pencil())
label("Delete").icon(mdi::trash()).trailing()   // icon after the text
```

Menus and commands:

```rust
use waterui::component::menu::{Command, Menu, MenuItem, Shortcut, ShortcutModifiers};
```

A `Button` converts into `MenuItem` and `Command`, so one definition can serve a toolbar,
a menu, and a keyboard shortcut.

## Text

```rust
text("Static string")
text!("Reactive {value}")
text!("{n} items", n = count.clone())
```

Both carry the full font API:

```rust
.title() .headline() .sub_headline() .body() .caption() .footnote()
.size(18.0) .bold() .italic() .font(font) .foreground(color)
```

Richer text lives in `waterui::widget`:

```rust
use waterui::widget::{Code, RichText, code, rich_text};
```

`FlowMarkdown` (feature `flow-markdown`) renders streaming markdown with per-element
animation — built for LLM output that arrives token by token.

## Lists and collections

`ForEach` is the plain reactive sequence; `List` is the platform list control (lazy row
realization, swipe-to-delete, reordering, sections, selection).

```rust
use waterui::component::lazy::Lazy;
use waterui::views::ForEach;

#[derive(Clone, Identifiable)]
struct Record { #[id] id: u64, title: Str }

// `ForEach` is a `Views` collection, not a `View`: a container consumes it.
Lazy::for_each(records.clone(), |r| text(r.title))       // == Lazy::vstack(ForEach::new(..))
Lazy::hstack(ForEach::new(records.clone(), row))

List::for_each(records.clone(), |r| ListItem::new(text(r.title)))
    .editing(is_editing.clone())          // impl IntoComputed<bool>
    .on_delete(|ListDelete(i), State(s): State<AppState>| { let _ = s.rows.remove(i); })
    .on_move(|ListMove(m), State(s): State<AppState>| { /* m.from(), m.to() */ })
    .scroll_controller(&controller)

// Heterogeneous / sectioned static content. Rows are *builders*, so the list can
// rebuild a row when it is realized again.
List::content((
    Section::new("Recent")
        .footer("Sub-pages push onto this tab's own stack.")
        .content((
            || ListItem::new(text("Today")),
            || ListItem::new(text("Yesterday")),
        )),
    || ListItem::new(text("Footer")),
))
```

`ListItem` modifiers: `.deletable(signal)`, `.selected(signal)`, `.section(section)`.
`Section::new(label).footer(text).content(rows)` builds a section.
Convenience row builders: `row("Title")`, `detail_row("Title", "Detail")`.

`List::for_each` requires `C::Item: Identifiable`; `List::content` takes a structural
tree of rows, sections, tuples, arrays, and `Option`s, so it is the one to use when rows
are heterogeneous or carry section markers.

## Forms and pickers

`#[form]` on a struct derives `Default`, `Clone`, `Debug`, `FormBuilder`, and `Project` —
so the struct gains both a generated form UI and per-field bindings.

```rust
#[form]
struct Settings {
    /// Doc comments become field labels.
    display_name: String,
    volume: f64,
    notifications: bool,
}

let settings: Binding<Settings> = binding(Settings::default());
form(&settings)                                  // whole generated form
field("Name", &settings.project().display_name)  // or drive one field yourself
```

Pickers:

```rust
use waterui::form::picker::{Picker, PickerStyle};
use waterui::form::picker::color::ColorPicker;
use waterui::form::picker::date::{DatePicker, DatePickerType};
use waterui::form::picker::file::FilePicker;
use waterui::form::picker::multi_date::MultiDatePicker;

// Options are views carrying their value via .tag(..). The picker is generic over
// your own type; it never asks you to construct an Id.
let options: Vec<_> = Fruit::all().map(|(f, name)| text(name).tag(f)).collect();

Picker::new("Sort by", options, &selection)      // &Binding<Fruit>
    .style(PickerStyle::Menu)                    // Automatic | Menu | Radio
```

Picker style is an attribute, never a separate type — there is no `RadioPicker`. The same
holds for toggles (switch vs checkbox) and lists (plain vs inset vs sidebar).

`DatePicker` binds typed `jiff` date/time values rather than strings.

## Overlays: snackbars, cards, suspense, full screen

Every `Window` installs a `SnackbarManager`; reach it from any handler.

```rust
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};

button("Save").action(|State(m): State<SnackbarManager>| {
    m.show(
        Snackbar::new("Item moved to trash")
            .icon(mdi::delete())
            .action("Undo", || restore())
            .position(SnackbarPosition::BottomCenter)   // Top/Bottom × Center/Leading/Trailing
            .closeable()
            .duration(Duration::from_secs(5)),          // Duration::ZERO = until dismissed
    );
});
```

Placements are independent stacks: a top snackbar never evicts a bottom one, and multiple
at one placement stack and reflow.

```rust
card(content).title("Summary").subtitle("This week")   // Card
suspense(async { load().await })                       // takes a future, not a closure
accordion(header, || body())                           // disclosure
```

`FullScreenOverlayManager` presents modal, window-filling content.

## Media

```rust
use waterui::media::{Photo, Url};

Photo::new("https://waterui.dev/logo.png")   // impl IntoComputed<Url>; Url: From<&'static str>
    .blur(radius.clone())
    .saturation(sat.clone())
    .on_event(|event| match event {
        PhotoEvent::Loaded => …,
        PhotoEvent::Error(msg) => …,
    })

video_player(item)                       // impl Into<MediaItem>
MediaPicker::new(&selection)             // &Binding<Option<media_picker::Selected>>
```

A runtime string is not a `Url` — parse it, and report the failure rather than letting a
bad address reach the loader as a local path:

```rust
let Ok(parsed) = entered.get().as_str().parse::<Url>() else { return };
Photo::new(parsed)
```

## Graphics and codes

```rust
Canvas::new(|ctx: &mut DrawingContext| { /* immediate-mode drawing */ })

use waterui::barcode::Barcode;           // feature = "barcode"
Barcode::qr("https://waterui.dev").size(120.0, 120.0)
Barcode::code128("012345").size(160.0, 60.0)

use waterui::svg::Svg;
Svg::new(source)

ParticleSystem::new(..)                  // feature = "particle"
GpuSurface::new(renderer)                // custom wgpu rendering
```

Shapes are views that fill the space they are given; `.fill()` and `.clip()` are the two
ways to use them — the styling reference covers them.

## Data: charts and maps

```rust
use waterui::chart::{BarChart, ChartExt, DataPoint, LineChart, PieChart};  // feature = "chart"
// plus Area, Scatter, Bubble, Radar, Heatmap, Contour, Candlestick, Gauge, Depth,
// each with a matching lowercase constructor (`bar_chart`, `pie_chart`, …).
```

```rust
use waterui::map::{Annotation, Coordinate, Map, Region};

let center = Coordinate::from_degrees(37.33, -122.03).expect("valid coordinate");
Map::new(Region::new(center, 0.05, 0.05))         // impl IntoComputed<Region>
Annotation::new(center, "Office")
```

`Coordinate::from_degrees` returns a `Result` — an out-of-range latitude or longitude is an
error you handle, not a value that silently clamps.

Maps need network access and, for user location, location permission — declare both in
`Water.toml`; the project reference covers permissions.

## Embedded web content

```rust
use waterui::webview::{ScriptInjectionTime, Url, WebView, WebViewEvent, WebViewProxy};

WebView::open("https://waterui.dev")
    .redirects_enabled(allow.clone())
    .user_agent(ua.clone())
    .on_event(handle_event)
```

The whole web view is described before it exists — the builder records the URL, redirect
policy, injected scripts, and handlers the page can call back into. Drive a live page
through `WebViewProxy`, which is extracted inside a handler exactly the way `State<T>` is.

Engine selection is a project setting (`webview_backend` in `Water.toml`), not a code
decision. `waterui-chromium` is a separate, heavier component for when the application
needs a full Chromium surface, headless pages, screenshots, or DevTools Protocol access.

## Accessibility modifiers

```rust
.a11y_label("Delete message")          // override the derived label
.a11y_id("inbox.row.3")                // stable automation id — reaches XCUITest and Android
.a11y_role(role)
.a11y_hidden(true)                     // decorative only
.a11y_state(state) / .a11y_state_signal(signal)
.a11y_children(..)
```

`.a11y_id` is the escape hatch for views that are hard to label meaningfully; the same
identifier is what `waterui-testing` queries with `.identifier(..)`. Prefer a real label
where one exists — the label serves users, the id only serves tests.
