# Component catalog

Signatures verified against the WaterUI source and compiled. Everything here is reachable
from `use waterui::prelude::*;` unless an explicit import is shown.

**Feature gates.** `waterui`'s default features are `gpu`, `assets`, `media`, `inspector`,
and `snackbar`. `webview`, `flow-markdown`, and `navigation-restoration` are opt-in — a
missing module here is usually a missing feature in `Cargo.toml`, not a wrong path:

```toml
waterui = { version = "…", features = ["webview"] }
```

**Component crates.** Charts, maps, barcodes and particles are not `waterui` features at
all: each is a crate of its own, added to `Cargo.toml` directly and imported under its own
name — `waterui_chart::LineChart`, `waterui_map::Map`, `waterui_barcode::Barcode`,
`waterui_particle::ParticleSystem` — exactly like the icon packs. There is no
`waterui::chart` module, so "could not find `chart` in `waterui`" means a missing
dependency, never a missing feature:

```toml
waterui-chart = "0.1"
waterui-map = "0.1"
waterui-barcode = "0.1"
waterui-particle = "0.1"
```

One more manifest fact worth knowing: `default-features = false` is a real pattern for
lean builds, but then *every* needed feature must be listed explicitly. Inside a Cargo
workspace, feature unification from sibling crates can make an under-declared manifest
compile anyway — a green build there is not evidence the manifest is right.

## Contents

- Naming conventions
- Layout containers
- Absolute placement and overlays
- Scrolling
- Controls
- Menus, commands, context menus
- Text
- Lists and collections
- Forms and pickers
- Overlays: snackbars, cards, suspense, full screen
- Accessibility modifiers

Media, web views, GPU graphics, charts, and maps have their own reference:
[media.md](media.md).

## Naming conventions

Two entry points per component, deliberately:

- **`Type::new(...)`** is the general constructor. It accepts the most general shape the
  component can render — arbitrary label views, raw configuration.
- **`lowercase(...)`** is the ergonomic constructor. It takes narrower, friendlier input
  (`impl IntoLabel`, `impl IntoText`) so string literals flow into the localized text
  pipeline and pick up correct default accessibility semantics.

The types behind the everyday lowercase constructors: `button` → `Button`, `toggle` →
`Toggle`, `slider` → `Slider`, `stepper` → `Stepper`, `field` → **`TextField`** (not
`Field`), `progress` → `Progress`, `picker` → `Picker`, `label` → `Label`. All are in the
prelude, and the control constructors take the same `(impl IntoLabel, &Binding<T>)` shape
as their lowercase forms.

There is no `Type::custom(...)` anywhere; if you find yourself looking for one, `new` is
it. Two named exceptions: `FilePicker::open(..)` is that type's constructor (it opens
files), and `progress(..)` takes its label as a `.label(..)` modifier rather than a first
argument — the one control that does.

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
.padding()  /  .padding_with(16.0)  /  .padding_with(EdgeInsets::symmetric(10.0, 16.0))
```

`.padding_with` takes `impl IntoComputed<EdgeInsets>`: a bare number, an `EdgeInsets`, or
a signal of one. Argument orders are traps worth memorizing —
`EdgeInsets::new(top, bottom, leading, trailing)` (not CSS order) and
`EdgeInsets::symmetric(vertical, horizontal)`.

Children are a tuple for a fixed set. For a runtime-length static set, collect — the
iterator item must be a single concrete type, so heterogeneous helpers return `AnyView`:

```rust
let row: HStack<_> = tabs.iter().map(|t| button(t.label())).collect();
let tiles: VStack<_> = (0..6).map(photo_tile).collect();   // photo_tile: fn(usize) -> AnyView
```

Spacing and separation:

```rust
spacer()                    // flexible, absorbs leftover space
spacer_min(12.0)            // flexible with a floor
spacer().height(16.0)       // fixed vertical gap; .width(12.0) for horizontal
Divider                     // a separator line (a unit struct, no call)
```

Frame modifiers exist per-axis as well as paired:

```rust
.size(w, h) / .width(w) / .height(h)               // plain f32
.min_size(w, h) / .max_size(w, h)
.min_width(w) / .min_height(h)                     // single-axis floors (plain f32)
.max_width(w) / .max_height(h)                     // accept signals (IntoSignalF32)
.max_width(f32::INFINITY)                          // idiom: stretch to full container width
```

`.min_width`/`.min_height` are how you reserve stage space for a scaled or rotated child
that overflows its intrinsic size.

Safe areas: content is inset from notches and home indicators by default; a full-bleed
layer opts out with `.ignore_safe_area(EdgeSet::ALL)` (per-edge: `EdgeSet` has
`top`/`leading`/`bottom`/`trailing` fields and `ALL`/`NONE` consts).

Grid — note the import and the `row` name collision with the list row builder:

```rust
use waterui::layout::grid::{grid as layout_grid, row as grid_row};

layout_grid(3, [
    grid_row((a, b, c)),
    grid_row((d, e)),          // short rows are fine
])
.spacing(10.0)
```

## Absolute placement and overlays

`absolute((a, b, c))` fills its parent and hands every child the full bounds — what a
window-filling overlay layer needs; a content-sized `zstack` mis-anchors its children as
soon as the window is larger than the content. Children place themselves with the
`PositionExt` methods (in the prelude):

```rust
use waterui::layout::PinConstraints;   // not in the prelude (PositionExt is)

absolute((
    map_view().pin(PinConstraints::all(0.0)),                 // stretch to fill
    status_panel().size(220.0, 64.0).position_in_offset(
        UnitPoint::TOP_LEADING, UnitPoint::TOP_LEADING,       // anchor on child, position in parent
        16.0, 16.0,                                           // offsets — these are signal slots
    ),
    controls().position_in(UnitPoint::BOTTOM_TRAILING),
))
```

For a two-view stack where one just floats above the other, the `overlay` free function
is simpler — sizing follows the base:

```rust
overlay(player, buffering_indicator).height(360.0)
```

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
API, not a convention. To hide a label *visually* while keeping it for assistive tech and
for `waterui-testing` queries, use the display-mode API — never drop the label:

```rust
slider("Sensitivity", &level).range(0.5..=3.0).hide_label()   // exists on every labeled control
button(label("Locate").icon(lucide::locate_fixed())).label_style(LabelDisplayMode::IconOnly)
toolbar_row().install(LabelDisplayMode::IconOnly)             // scope a mode to a whole subtree
```

`LabelDisplayMode` variants: `Automatic`, `TitleAndIcon`, `TitleOnly`, `IconOnly`,
`Hidden`. `.hide_label()` is the shorthand for `Hidden`. `IconOnly` panics at render time
on a label with no icon, so install it only on subtrees whose labels all carry `.icon(..)`.
The label text remains the test-query key even when hidden.

```rust
button("Save")                              // Button<fn(&Environment)>
    .action(handler)                        // -> Button<impl FnMut(&Environment)>
    .action_async(|| async { … })
    .style(ButtonStyle::Plain)              // Automatic | Plain | Link | Borderless | Bordered | BorderedProminent
    .state(&value)                          // inject handler state (repeatable)

button(text!("{edit_label}"))               // a text! satisfies IntoLabel: reactive button titles need no watch

toggle("Wi-Fi", &enabled)                   // &Binding<bool>
Toggle::new("Wi-Fi", &enabled).style(ToggleStyle::Switch)     // Automatic | Switch | Checkbox
slider("Volume", &level).range(0.0..=1.0)   // &Binding<f64>; range is RangeInclusive<f64>
stepper("Quantity", &count)                 // &Binding<i32>
stepper("Items", &count).range(0..=100).step(5)   // range: impl RangeBounds<i32>; step takes a signal
field("Email", &address)                    // &Binding<Str>
TextField::new("Username", &name).prompt("Enter your username")   // placeholder ≠ label
progress(fraction)                          // impl IntoComputed<f64>
progress(fraction).label("Downloading")     // its label is a modifier — the one exception
loading()                                   // indeterminate progress
```

Style shorthands on `Button` — `.plain()`, `.link()`, `.borderless()`, `.bordered()`,
`.bordered_prominent()` — must come *before* `.action(..)`, which changes the button's
type parameter. `.borderless()` is the idiomatic toolbar-button style; `Link` shows the
pointing-hand cursor on pointer platforms. Switch and checkbox map to different
accessibility roles (`SWITCH` vs `CHECKBOX`), which matters to tests.

`Label` composes text with an icon; `Label::new` makes a whole composed row read as one
accessibility node:

```rust
use waterui::component::label::label;       // also in the prelude
use waterui_icons_material_icon as mdi;     // icons come from an icon-set crate (styling.md)

label("Compose").icon(mdi::pencil())
label("Delete").icon(mdi::delete()).trailing()      // icon after the text; .leading() is the default side
label("Mode").icon(mdi::tune()).display_mode(LabelDisplayMode::TitleAndIcon)

// General form: semantic text + arbitrary content, ONE node for assistive tech.
// The content is a builder closure — it may run again, so clone what it captures.
Label::new(text!("{sender}, {subject}"), move || message_row(message.clone()))
```

`.icon(..)` accepts any cloneable view, not only an icon-set glyph.

## Menus, commands, context menus

`Menu` is a view (a popup surface) whose items are a `MenuView`: tuples, arrays, `Vec`,
or `Option` of `Command`, `Button`, `Divider`, or a nested `Menu`:

```rust
Menu::new("Choose an Option", (
    "Option A"
        .action(|State(sel): State<Binding<String>>| sel.set("A".into()))
        .state(&selected),
    Divider,                                   // becomes a separator inside a menu
    Menu::new("More", ( "Nested".action(nested_handler), )),
))
```

That `"Option A".action(..)` is `CommandExt`, blanket-implemented for every
`impl IntoLabel`: the receiver is the *label* and the result is a `Command` — not a
button. `Command` has its own builder chain: `.state(&value)`, `.disabled(signal)`,
`.selected(signal)`, `.shortcut(Shortcut)`. A `Button` also converts into `MenuItem` and
`Command`, so one definition can serve a toolbar, a menu, and a keyboard shortcut.

`.context_menu(items)` attaches a long-press / right-click menu to ANY view and takes the
same `MenuView` content. Attach `.state(..)` to each command, not to the menu:

```rust
text("Long press me").padding().context_menu((
    "Copy".action(copy_handler).state(&clipboard),
    Divider,
    "Paste".action(paste_handler).state(&clipboard),
))
```

## Text

```rust
text("Static string")
text!("Reactive {value}")
text!("{n} items", n = count.clone())
```

`text!` is also the translation pipeline — the whole literal is a catalog key, plurals use
a `{#count}` selector, and slot arguments are moved into the macro (pre-clone signals you
still need). See [i18n.md](i18n.md).

Both carry the full font API:

```rust
.title() .headline() .sub_headline() .body() .caption() .footnote()
.size(18.0) .bold() .italic(true) .font(font::Caption) .foreground(color)
```

`.bold()` takes no argument; `.italic(..)` takes a bool — a signal of one works too.

`.font(..)` takes a slot struct from the prelude's `font` module — `font::Title`,
`Headline`, `Subheadline` (note the spelling; the method is `.sub_headline()`), `Body`,
`Caption`, `Footnote` — the form to use when the style is chosen dynamically. On `Text`,
`.size(..)` is the *font* size, is reactive (`impl IntoSignal<f64>`), and **shadows** the
two-argument frame `.size(w, h)` — to give a text a frame, use `.width(..)`/`.height(..)`
or size its container.

Richer text:

```rust
use waterui::widget::{Code, RichText, code, rich_text};

include_markdown!("guide.md")               // compile-time: expands to RichText::from_markdown(include_str!(..))
RichText::from_markdown(runtime_str)        // runtime markdown -> RichText
```

`include_markdown!` resolves the path against the calling source file and is used bare
(never `waterui::include_markdown!`), like `text!`.

`FlowMarkdown` (feature `flow-markdown`) renders *streaming* markdown with per-element
animation — built for LLM output. `flow_markdown(source)` takes
`impl IntoComputed<Str>`, so pass the `Binding<Str>` itself and keep appending to it;
`.configuration(..)` also takes a signal, so animation settings retune live:

```rust
use waterui::prelude::flow_markdown::FlowMarkdownConfig;

let config = FlowMarkdownConfig::default()
    .stream(FlowStreamMode::AppendOnly)             // source only ever grows: the LLM fast path
    .preset(FlowAnimationPreset::AssistantDefault)  // | Minimal | None
    .token_fade_in(Some(Animation::ease_out(Duration::from_millis(180))));
flow_markdown(markdown.clone()).configuration(config)
```

`.configuration(..)` takes a *signal* of config — derive one from your control bindings
to retune animation live. A fixed config goes in bare, as above: `FlowMarkdownConfig` is
itself a constant signal, so it needs no `Computed::constant` wrapper. Per-element
overrides:
`.override_animation(FlowElementKind::Text, FlowAnimationPolicy::Typewriter { cps: 40,
batch_ms: 24, fade_in: None })` — `fade_in` is an `Option<Animation>`, not a bool; kinds
are `Text | Heading | ListItem | Quote | Link`. The config is a move-builder — reassign
it in loops.

## Lists and collections

`ForEach` is the plain reactive sequence; `List` is the platform list control (lazy row
realization, swipe-to-delete, reordering, sections, selection).

```rust
use waterui::component::lazy::Lazy;
use waterui::component::list::{ListDelete, ListMove};   // NOT in the prelude
use waterui::views::ForEach;
use waterui::Identifiable;                              // the derive is NOT in the prelude

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
```

`ListDelete(index)` destructures to a `usize`; `ListMove(m)` exposes `.from()`/`.to()`.

For a reactive collection rendered as an ordinary (non-lazy) stack that still takes stack
modifiers, `VStack`/`HStack` have `for_each` too, and `collection_transition` animates
membership changes (insert/remove fade and collapse along the stack axis):

```rust
use waterui::layout::collection_transition;
use waterui::layout::stack::VStack;

let drawer = VStack::for_each(rows.clone(), row_view).spacing(4.0);
collection_transition(drawer, Animation::ease_in_out(Duration::from_millis(250)))
```

Heterogeneous / sectioned static content — rows are *builders*, so the list can rebuild a
row when it is realized again. `row` and `detail_row` take **two** arguments (label,
value) and may sit directly in the content tuple:

```rust
List::content((
    Section::new("Recent")
        .footer("Sub-pages push onto this tab's own stack.")
        .content((
            || ListItem::new(text("Today")),
            row("Streak", "14 days"),                    // Row is valid content directly
            detail_row("Last entry", "Yesterday"),
        )),
    || ListItem::new(text("Footer")),
))
```

`ListItem` modifiers: `.deletable(signal)`, `.selected(signal)`, `.section(section)`.
`List::for_each` requires `C::Item: Identifiable`; `List::content` takes the structural
tree above. An enum row type implements `Identifiable` by hand (`type Id; fn id(&self)`),
keeping the id ranges of different variants disjoint.

When the row set is *derived* — filtered or sorted from other state — do not fall back to
`watch`: wrap the derived signal in `SignalCollection` (see
[reactivity.md](reactivity.md), Reactive collections).

## Forms and pickers

`#[form]` on a struct derives `Default`, `Clone`, `Debug`, `FormBuilder`, and `Project` —
so the struct gains a generated form UI, per-field bindings, and an inference-free
constructor:

```rust
#[form]
struct Settings {
    /// Doc comments become field labels. Text fields bind `Str`, not `String`.
    display_name: Str,
    volume: f64,
    notifications: bool,
}

let settings = Settings::binding();              // Binding<Settings>, no annotation needed
form(&settings)                                  // whole generated form
field("Name", &settings.project().display_name)  // or drive one field yourself
```

Pickers:

```rust
use waterui::form::picker::{Picker, PickerItem, PickerStyle, picker};
use waterui::form::picker::color::ColorPicker;
use waterui::form::picker::date::{DatePicker, DatePickerType};
use waterui::form::picker::file::FilePicker;
use waterui::form::picker::multi_date::MultiDatePicker;
use waterui::form::Calendar;

// Options are views carrying their value via .tag(..). The tag type is T: Ord + Clone.
// PickerItem<T> is the nameable item type — an array of them works without collect().
fn sizes() -> [PickerItem<&'static str>; 3] {
    [text("Small").tag("S"), text("Medium").tag("M"), text("Large").tag("L")]
}

picker("Size", sizes(), &choice)                 // ergonomic constructor
Picker::new(text!("Sort by"), options, &choice)  // labels may be localized text!
    .style(PickerStyle::Menu)                    // Automatic | Menu | Radio | Segmented
    .hide_label()
```

Picker style is an attribute, never a separate type — there is no `RadioPicker`. The same
holds for toggles (switch vs checkbox) and lists (plain vs inset vs sidebar).

Date and color pickers bind typed values; `jiff` is **your** dependency (WaterUI does not
re-export it), and its constructors are fallible:

```rust
// Cargo.toml: jiff = "…"
use jiff::civil::Date;

DatePicker::new("Date", &date)                   // T: DatePickable picks the presentation
    .ty(DatePickerType::DateHourMinuteAndSecond) // override; also HourMinuteAndSecond etc.
    .range(min..=max)                            // clamps the bound value immediately

Calendar::new("Trip Date", &date, &visible_month)      // TWO bindings: selection + shown month
    .range(start..=end)
    .decorated(marked_days.clone())                    // impl IntoComputed<BTreeSet<Date>> — passive dots

MultiDatePicker::new("Available", &date_set, &visible_month)   // Binding<BTreeSet<Date>>

ColorPicker::new("Accent", &color).with_alpha().with_hdr()     // Binding<Color>
FilePicker::open("Select Files", &urls).max_count(5)           // Binding<Vec<Url>>; constructor is `open`
```

Seeding a `Binding<Color>` from a constant: `binding(Color::from(Srgb::from_hex("#4A84F6")))`.
A `Binding<BTreeSet<Date>>` needs the turbofish: `binding(BTreeSet::<Date>::new())`.

## Overlays: snackbars, cards, suspense, full screen

Every `Window` installs a `SnackbarManager`; reach it from any handler.

```rust
use core::time::Duration;
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
use waterui::widget::accordion;                        // not in the prelude

card(content).title("Summary").subtitle("This week")   // Card
suspense(async { load().await })                       // takes a future, not a closure
accordion(header, || body())                           // disclosure
view.floating()                                        // themed elevated surface (FloatingStyle-aware)
```

`FullScreenOverlayManager` presents modal, window-filling content.

## Accessibility modifiers

```rust
use waterui::accessibility::{AccessibilityRole, AccessibilityState};

.a11y_label("Delete message")          // override the derived label
.a11y_id("inbox.row.3")                // stable automation id — reaches XCUITest and Android
.a11y_role(AccessibilityRole::Button)
.a11y_hidden(true)                     // decorative only
.a11y_state_signal(active.map(|a| AccessibilityState::new().selected(a)))
```

The idiom for a composed row that should read as a single node: `.a11y_hidden(true)` on
the inner content, then `.a11y_role(..)` + `.a11y_label(..)` on the wrapper — or better,
build it with `Label::new(semantic_text, || content)`, which does this for you.

`.a11y_id` is the escape hatch for views that are hard to label meaningfully; the same
identifier is what `waterui-testing` queries with `.identifier(..)`. Prefer a real label
where one exists — the label serves users, the id only serves tests.
