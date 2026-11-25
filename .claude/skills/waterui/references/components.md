# Component catalog

Signatures verified against the WaterUI source and compiled. Everything here is reachable
from `use waterui::prelude::*;` unless an explicit import is shown.

**Feature gates.** `waterui`'s default features are `gpu`, `assets`, `dialog`, `snackbar`,
`highlight`, `locale-datetime` and `locale-list`. Everything else is opt-in, chiefly `image`,
`video`, `video-gpu`, `webview`, `flow-markdown` (also gates `RichText` and
`include_markdown!`), `markdown-math`, `navigation-restoration`, `haptics`, `inspector`, `remote`, `ts`,
`widgets`, and the component features `canvas`, `chart`, `barcode` and `particle`. `all`
turns on the full set except `ts`, `markdown-math` and `widgets`. A missing module here is
usually a missing feature in `Cargo.toml`, not a wrong path:

```toml
waterui = { version = "…", features = ["webview"] }
```

**Component crates.** Charts, barcodes and particles are independent crates that the
facade re-exports behind features — `waterui::chart::LineChart`,
`waterui::barcode::Barcode`, `waterui::particle::ParticleSystem`, and `waterui::canvas` for
the drawing API. A direct dependency on the crate (`waterui-chart`) names the same types.
Maps are not a facade feature: `waterui_map::Map` needs `waterui-map` in `Cargo.toml`, like
the icon packs. "Could not find `chart` in `waterui`" therefore means the feature is off:

```toml
waterui = { version = "…", features = ["chart", "barcode", "particle", "canvas"] }
waterui-map = "…"
```

One more manifest fact worth knowing: `default-features = false` is a real pattern for
lean builds, but then *every* needed feature must be listed explicitly. Inside a Cargo
workspace, feature unification from sibling crates can make an under-declared manifest
compile anyway — a green build there is not evidence the manifest is right.

## Contents

- Naming conventions
- Layout containers
- Absolute placement and overlays
- Anchored overlays (popovers, tooltips)
- Scrolling
- Controls
- Menus, commands, context menus
- Text
- Lists and collections
- Forms and pickers
- Overlays: snackbars, cards, suspense, full screen
- Badges, avatars, tables, focus
- Accessibility modifiers
- Dialogs (alerts, confirmations)

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
prelude. `Toggle::new`, `TextField::new` and `Picker::new` take the same `impl IntoLabel`
as their lowercase forms; `Button::new`, `Slider::new` and `Stepper::new` take a finished
`Label`, so write `slider("Volume", &level)` rather than `Slider::new("Volume", &level)`.

There is no `Type::custom(...)` anywhere; if you find yourself looking for one, `new` is
it. `progress(..)` takes its label as a `.label(..)` modifier rather than a
first argument — the one control that does.

## Layout containers

```rust
hstack((a, b, c))        // horizontal
vstack((a, b, c))        // vertical
zstack((back, front))    // depth, later children on top
```

Common modifiers on stacks:

```rust
.spacing(8.0)
.leading()   // or .alignment(Leading); hstack aligns vertically: .top() / .centered() / .bottom()
.padding()  /  .padding_with(16.0)  /  .padding_with((10.0, 16.0))
```

`.padding()` is 14 pt on every edge. `.padding_with` takes `impl IntoComputed<EdgeInsets>`: a
bare number (every edge), a `(vertical, horizontal)` pair, a `[top, leading, bottom,
trailing]` array, an `EdgeInsets`, or a signal of one. `.padding_horizontal(x)` /
`.padding_vertical(x)` pad one axis. Do not spell `EdgeInsets::all(x)` / `symmetric(v, h)` /
`new(..)` at a padding call — the number, pair and array are those constructors. The orders
are traps worth memorizing — `[top, leading, bottom, trailing]` (SwiftUI's order, not CSS)
and `(vertical, horizontal)`.

Alignment is a token or a method, never the qualified constant: `vstack(..).leading()` or
`.alignment(Leading)`, `hstack(..).top()`, `zstack(..).top_leading()`. The tokens
(`Leading`, `Trailing`, `Center`, `Top`, `Bottom`, `FirstBaseline`, `LastBaseline`, and the
corners `TopLeading` …) convert only into the axis they are legal for, so
`vstack(..).alignment(Top)` does not compile.

Children are a tuple for a fixed set. For a runtime-length static set, collect — the
iterator item must be a single concrete type, so a helper whose branches differ returns
a `OneOfN`:

```rust
let row: HStack<_> = tabs.iter().map(|t| button(t.label()).action(|| {})).collect();
let tiles: VStack<_> = (0..6).map(photo_tile).collect();   // photo_tile: fn(usize) -> impl View
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
.size(w, h) / .width(w) / .height(h)
.min_size(w, h) / .max_size(w, h)
.min_width(w) / .min_height(h)                     // single-axis floors
.max_width(w) / .max_height(h)                     // single-axis caps
.max_width(f32::INFINITY)                          // idiom: stretch to full container width
```

Every sizing shortcut on a view takes a literal or signal (`IntoSignalF32`) and returns a
`Frame`, so `.height(height_signal)` and `.width(120.0).max_height(height_signal)` are both
reactive; `.alignment(...)` accepts `Alignment` or an alignment token (`TopLeading`, `Center`,
…). `.min_width`/`.min_height` are how you reserve stage space for a scaled or rotated child
that overflows its intrinsic size. Views also take `.ideal_width(w)` / `.ideal_height(h)`
(signals): the size requested when the parent proposes nothing. Unlike `.width(w)`, which pins
the minimum, ideal and maximum together, it leaves the frame free to be smaller or larger when
the parent does propose a size.

Two more sizing modifiers:

```rust
.aspect_ratio(16.0 / 9.0)     // width : height; shrinks to fit inside the offer
.layout_priority(1)           // a stack compresses its lowest-priority children first
```

`.horizontal_alignment_guide(HorizontalAlignment::Leading, |d| d.size.width / 2.0)` and
`.vertical_alignment_guide(VerticalAlignment::Center, |d| ..)` move where a view's guide
sits, so a stack aligned on that guide lines the view up there; the size and placement of the
view itself are unchanged. The closure is `Fn(&ViewDimensions) -> f32` and returns points;
`d.horizontal(guide)` / `d.vertical(guide)` read another guide's current value.

Safe areas: content is inset from notches and home indicators by default; a full-bleed
layer opts out with `.ignore_safe_area(EdgeSet::ALL)` (`EdgeSet` has `top`/`leading`/
`bottom`/`trailing` fields and the consts `ALL`, `NONE`, `TOP`, `BOTTOM`, `HORIZONTAL`,
`VERTICAL`). The safe area has two regions per edge — `container` (bars, cutouts, window
chrome) and `keyboard` (the software keyboard) — and `.ignore_safe_area` takes an
`Into<IgnoreSafeArea>`: `SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM)` lays a view under
the keyboard but above the container inset, `SafeAreaRegions::CONTAINER` / `ALL` name the
other combinations, and a bare `EdgeSet` ignores every region.

Visual transforms never affect layout. `.scale(factor)` / `.scale_xy(x, y)` and `.rotation(degrees)` pivot on the
centre; `.scale_from(factor, Anchor::TOP_LEFT)` / `.scale_xy_from(x, y, Anchor::TOP_LEFT)` and `.rotation_from(45.0, Anchor::TOP_LEFT)`
pivot on an `Anchor` (in the prelude; normalized, `(0, 0)` top-left: `CENTER`, `TOP_LEFT`,
`TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, or `Anchor::new(x, y)`). Factors and angles take
signals; the anchor is a plain value.

Grid — note the import and the `row` name collision with the list row builder. Rows go in
a tuple, which lets each row hold different views; an array or `Vec` works when every row
has the same view types:

```rust
use waterui::layout::grid::{grid as layout_grid, row as grid_row};

layout_grid(3, (
    grid_row((a, b, c)),
    grid_row((d, e)),          // short rows are fine
))
.spacing(10.0)
```

## Absolute placement and overlays

`absolute((a, b, c))` fills its parent and hands every child the full bounds — what a
window-filling overlay layer needs; a content-sized `zstack` mis-anchors its children as
soon as the window is larger than the content. Children place themselves with the
`PositionExt` methods and `PinConstraints` (both in the prelude):

```rust
absolute((
    map_view().pin(PinConstraints::all(0.0)),                 // stretch to fill
    status_panel().size(220.0, 64.0).position_in_offset(
        UnitPoint::TOP_LEADING, UnitPoint::TOP_LEADING,       // anchor on child, position in parent
        16.0, 16.0,                                           // offsets — these are signal slots
    ),
    controls().position_in_anchor(UnitPoint::BOTTOM_TRAILING, UnitPoint::BOTTOM_TRAILING),
    dot().pin(PinConstraints::new().trailing(12.0).bottom(12.0).width(28.0).height(28.0)),
))
```

For a two-view stack where one just floats above the other, the `overlay` free function
is simpler — sizing follows the base:

```rust
overlay(player, buffering_indicator).height(360.0)
```

## Anchored overlays (popovers, tooltips)

`.anchored_overlay(...)` presents content next to the view it modifies — above all other
content in the window — while its `Binding<bool>` is true. The backend places it against
the anchor's declared edge, flips to the opposite edge when the preferred side has no
room, and clamps it inside the window. By default a pointer or touch down outside it writes
`false` back to the binding (the event still reaches its target):

```rust
use waterui::metadata::anchored_overlay::{
    AnchoredOverlay, AnchorEdge, EdgeAlignment, Clamp, Dismissal,
};

let open = binding(false);
let placed = binding(AnchorEdge::Top);
button("Options")
    .action({ let open = open.clone(); move || open.toggle() })
    .anchored_overlay(
        AnchoredOverlay::new(&open, move || popover_body()) // a fresh view per presentation
            .edge(AnchorEdge::Top)                     // Top / Bottom / Leading / Trailing
            .alignment(EdgeAlignment::Center)          // Start / Center / End along the edge
            .gap(4.0)                                  // points away from the anchor
            .flip(true)                                // flip to the opposite edge on overflow
            .clamp(Clamp::Window { margin: 2.0 })      // stay inside the window
            .dismissal(Dismissal::OutsideInteraction)  // or Manual — only the binding closes it
            .placed_edge(&placed),                     // the edge actually used, after any flip
    )
```

Leading/trailing follow the layout direction; `Start`/`End` are leading/trailing along
the top and bottom edges and top/bottom along the leading and trailing edges. Unlike
`.context_menu`, the overlay takes a `move ||` builder producing arbitrary `impl View`
content — every presentation calls it, so state shared between presentations lives in
`Binding`s the caller owns and the builder captures. `placed_edge` lets the content grow
out of, or point at, the anchor from the right side after a flip. This is the popover and
tooltip primitive.

## Scrolling

```rust
scroll(content)              // vertical
scroll_horizontal(content)
scroll_both(content)
```

Programmatic scrolling goes through a `ScrollController`, which is explicit and
repeatable. `List` addresses item indices; `ScrollView` addresses content coordinates.
A request carries an optional animation: `scroll_to` jumps, `animate_to` moves
along the animation's curve. An `animate_to` on a `List` further than 100 rows
away jumps to within 100 rows of the target first and animates only the final
stretch — it never flies the viewport through the rows between. A controller's
`.position()` is a read-only signal of the last position the backend reported
(`None` until the first layout) with `offset()`, `at_start()` and `at_end()` — a
follow-tail view reads `at_end`. `ScrollView::report_offset(&Binding<Point>)` writes the
content offset as it moves, for chrome that follows the scroll; the binding is written
only, never read.

```rust
let rows = ScrollController::<usize>::new(0);
let list = List::for_each(records, row_view).scroll_controller(&rows);
rows.scroll_to(50_000);                          // jump — does not materialize rows 0..50_000
rows.animate_to(50_000, Animation::default());   // glide with the platform's smooth scroll
let at_end = rows.position().map(|p| p.is_some_and(|p| p.at_end()));

let offset = ScrollController::<Point>::new(Point::zero());
let reported = Binding::new(Point::zero());
let view = scroll(content).scroll_controller(&offset).report_offset(&reported);
let lifted = reported.map(|o| o.y > 0.0);        // e.g. lift a top bar once content passes under it
offset.scroll_to(Point::new(0.0, 2_400.0));
offset.animate_to(Point::new(0.0, 2_400.0), Animation::ease_in_out(Duration::from_millis(400)));
```

## Controls

Every control's first argument is its accessibility label. That is a requirement of the
API, not a convention. To hide a label *visually* while keeping it for assistive tech and
for `waterui-testing` queries, use the display-mode API — never drop the label:

```rust
slider("Sensitivity", &level).range(0.5..=3.0).hide_label()   // exists on every labeled control
button(label("Locate").icon(lucide::locate_fixed())).label_style(LabelDisplayMode::IconOnly).action(|| {})
toolbar_row().install(LabelDisplayMode::IconOnly)             // scope a mode to a whole subtree
```

`LabelDisplayMode` variants: `Automatic`, `TitleAndIcon`, `TitleOnly`, `IconOnly`,
`Hidden`. `.hide_label()` is the shorthand for `Hidden`. `IconOnly` on a label with no icon
renders its title instead, so install it freely on a toolbar and give each label an
`.icon(..)`. The label text remains the test-query key even when hidden.

```rust
button("Save")                              // Button<_, NoAction>: not a View until .action / .action_async is set
    .action(handler)                        // -> Button<_, impl FnMut(&Environment)>
    .action_async(|| async { … })
    .style(ButtonStyle::Plain)              // Automatic | Plain | Link | Borderless | Bordered | BorderedProminent
                                            // | Glass | GlassProminent (Liquid Glass capsules on Apple;
                                            //   bordered / bordered-prominent elsewhere)
    .size(ControlSize::Large)               // ExtraSmall..=ExtraLarge; buttons default to Small
    .state(&value)                          // inject handler state (repeatable)

button(text!("{edit_label}")).action(|| {}) // a text! satisfies IntoLabel: reactive button titles need no watch

toggle("Wi-Fi", &enabled)                   // &Binding<bool>
Toggle::new("Wi-Fi", &enabled).style(ToggleStyle::Switch)     // Automatic | Switch | Checkbox; .switch() / .checkbox()
slider("Volume", &level).range(0.0..=1.0)   // &Binding<T: SliderValue> (f32, f64, ints); range/step accept signals
slider("Volume", &level).size(ControlSize::Large)      // sliders default ExtraSmall
    .value_formatter(|v| Str::from(format!("{v:.0}"))) // shown above the thumb while dragging
slider("Volume", &level).step(0.1).min_value_label("Quiet").max_value_label("Loud")
stepper("Quantity", &count)                 // &Binding<T: StepperValue> (i32, u32, i64, usize, ...)
stepper("Items", &count).range(0..=100).step(5)   // range and step accept signals
stepper("Items", &count).value_formatter(|n| format!("{n} items"))   // inline value text; T: Into<StyledStr>
field("Email", &address)                    // &Binding<Str>
field("Search", &query).on_submit(handler)  // Return in a line-limited field runs it
TextField::new("Username", &name).prompt("Enter your username")   // placeholder ≠ label
TextField::new("Notes", &notes).line_limit(None::<NonZeroUsize>) // multi-line: Return inserts a line break
use waterui::component::text_field::{ContentType, KeyboardType};   // NOT in the prelude
// .content_type declares what the field means, for autofill — it is what
// makes Apple offer the received SMS code; pair it with .keyboard(..)
TextField::new("Verification code", &code)
    .keyboard(KeyboardType::Number)
    .content_type(ContentType::OneTimeCode)
// .selection_menu adds commands to the selected-text menu; a command takes the
// field's live selection as a handler parameter
TextField::new("Note", &note).selection_menu((
    "Bold".action(|selection: TextSelection| {
        let marked = format!("**{}**", selection.selected_text());
        selection.replace(&marked);
    }),
))
progress(fraction)                          // impl IntoComputed<f64>
progress(fraction).label("Downloading")     // its label is a modifier — the one exception
progress(fraction).style(ProgressStyle::Circular)  // or Linear / Loading (shorthands: .circular(), .linear())
loading()                                   // indeterminate progress
link("Docs", "https://waterui.dev")         // a Link-style button that opens the URL
```

Style shorthands on `Button` — `.plain()`, `.link()`, `.borderless()`, `.bordered()`,
`.bordered_prominent()` — equal `.style(..)` and work before or after `.action(..)`.
`.borderless()` is the idiomatic toolbar-button style; `Link` shows the
pointing-hand cursor on pointer platforms. `TextField::line_limit(impl IntoComputed<Option<NonZeroUsize>>)` caps the
lines (pass `None` for unlimited multi-line input, or a signal to toggle it); `TextField::new(label, &value)` accepts both `&Binding<Str>` and `&Binding<StyledStr>` via `TextFieldValue`. `Switch` and `checkbox` map to different accessibility roles (`SWITCH` vs `CHECKBOX`), which matters to
tests. `ToggleStyle::Automatic` reads the ambient `ToggleStyle` installed in the environment
(`.with(ToggleStyle::...)`) first, then the backend's platform default (`Switch` on
Hydrolysis and iOS, `Checkbox` on macOS AppKit). On Hydrolysis, `Role::toggle(ToggleStyle::Automatic)`
maps to `Role::SWITCH`. A switch with a visible label fills the
width it is offered (label leading, switch trailing); a checkbox and a toggle with a
hidden label are content-sized.

### Custom button and toggle styles

Implement `CustomButtonStyle`, `PrimitiveButtonStyle`, or `CustomToggleStyle`, and pass an
instance either to a single control with `.style(..)` or to an entire subtree with
`.button_style(..)` / `.toggle_style(..)`. `CustomButtonStyle` and `CustomToggleStyle`
receive the semantic `label()` / `into_label()`, interaction-state signals (`is_pressed()`,
`is_hovered()`, `is_focused()`, `is_disabled()`, `interaction_state()`), `role()` / `is_on()`,
and automatically preserve tap activation, disabled gating, and accessibility semantics;
`PrimitiveButtonStyle` receives `trigger()` so the style owns its gesture.
An explicit `.style(..)` on a control wins over an enclosing subtree hook, and an inner
`.button_style(ButtonStyle::Bordered)` or `.toggle_style(ToggleStyle::Checkbox)` overrides
an outer custom style.

```rust
use waterui::gesture::TapGesture;
use waterui::theme::color::{Accent, Error, SurfaceVariant};
use waterui::shape::Capsule;

struct PillButtonStyle;

impl CustomButtonStyle for PillButtonStyle {
    fn make_body<L: View>(&self, configuration: ButtonStyleConfiguration<L>) -> impl View + use<L> {
        let base = if configuration.role() == Some(ButtonRole::Destructive) {
            Color::new(Error)
        } else {
            Color::new(SurfaceVariant)
        };
        let bg = configuration
            .is_pressed()
            .select(Color::new(Accent), base);
        configuration
            .into_label()
            .padding_with((8.0, 16.0))
            .background(bg)
            .clip(Capsule)
    }
}

struct DoubleTapButtonStyle;

impl PrimitiveButtonStyle for DoubleTapButtonStyle {
    fn make_body<L: View>(
        &self,
        configuration: PrimitiveButtonStyleConfiguration<L>,
    ) -> impl View + use<L> {
        let trigger = configuration.trigger().clone();
        configuration
            .into_label()
            .padding_with((8.0, 16.0))
            .gesture(
                TapGesture::repeat(NonZeroU32::new(2).unwrap()),
                move |_: TapEvent| {
                    trigger.trigger();
                },
            )
    }
}

struct PillToggleStyle;

impl CustomToggleStyle for PillToggleStyle {
    fn make_body<L: View>(&self, configuration: ToggleStyleConfiguration<L>) -> impl View + use<L> {
        let bg = configuration
            .is_on()
            .select(Color::new(Accent), Color::new(SurfaceVariant));
        hstack((
            configuration.into_label(),
            spacer(),
            ().size(44.0, 26.0)
                .background(bg)
                .clip(Capsule),
        ))
    }
}

let styled = button("Delete")
    .role(ButtonRole::Destructive)
    .style(PillButtonStyle)
    .action(|| {});

let scoped = vstack((
    button("Save").action(|| {}),
    button("Reset").style(DoubleTapButtonStyle).action(|| {}),
    toggle("Wi-Fi", &enabled),
))
.button_style(PillButtonStyle)
.toggle_style(PillToggleStyle);
```

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

`.icon(..)` accepts any cloneable view, not only an icon-set glyph. On Apple
platforms, `waterui_apple::SfSymbol::HOUSE` (and the rest of Apple's typed
catalog) is realized natively by the Apple backend and projected directly into
menus, toolbars, and tab bars; other icons are rasterized when placed in
platform chrome. `.icon(..)` panics on a `Label::new` label.

## Menus, commands, context menus

`Menu` is a view (a popup surface) whose items are a `MenuView`: tuples, arrays, `Vec`,
or `Option` of `Command`, `Button`, `Divider`, `MenuItem`, or a nested `Menu`. The trigger
button takes `.style(ButtonStyle)` and `.size(ControlSize)`:

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
button. A `Button` also converts into `MenuItem` and `Command`, so one definition can serve
a toolbar, a menu, and a keyboard shortcut. `Command` has its own builder chain:

```rust
"Copy"
    .action(copy_handler)
    .state(&clipboard)                         // handler state, per command
    .disabled(nothing_selected)                // signal of bool
    .selected(checked)                         // signal of bool: a checked / toggled row
    .subtitle("Copy the selection")            // secondary line under the label
    .shortcut(Shortcut::new('c').command())
"Delete".action(delete_handler).destructive()  // red in every menu; == .role(ButtonRole::Destructive)
"Delete".action(delete_handler).shortcut(Shortcut::new(ShortcutNamedKey::Delete))
Shortcut::new('s').command().shift()           // ⇧⌘S; also .option() and .control()
```

A shortcut key is a `char` or a `ShortcutNamedKey` (or `ShortcutKey::try_from(NamedKey)`). `.command()` is the platform's primary
modifier: ⌘ on Apple, Ctrl elsewhere. When a chord is live depends on where the command
sits: a mounted `Menu` arms its chords whether or not its popup is open, a `.context_menu`
arms them only while it is open, and `App::menu_bar` arms them app-wide. A chord without
Ctrl, Alt or Super does not fire while a text field has focus — that press is typing.

`App::menu_bar(..)` takes a `Menu`, or a tuple, array or `Vec` of them. macOS and Windows
draw it as the system menu bar and iOS feeds it to the system main menu; Linux, the web
and headless hosts draw nothing but keep the shortcuts armed:

```rust
use waterui::app::App;

App::new(env)
    .window(Window::new("", move || content()))
    .menu_bar(Menu::new("File", (
    "Save".action(save).shortcut(Shortcut::new('s').command()),
    Divider,
    MenuItem::CloseWindow,
    MenuItem::Quit,
)))
```

Two items stand for platform commands instead of carrying an action: `MenuItem::Quit`
and `MenuItem::CloseWindow`. Each renders with the platform's own label and chord, acting
through the termination machine or the window's own close path. macOS's menu bar never
shows `Quit` — the standard application menu already has it — but a menu a window mounts
does. A platform that cannot quit or close a window omits them, so a portable menu declares
them unconditionally. The close chord is decided once for the whole application from its
declared menu bar, and every Close Window item carries it. ⌘W is not reserved — an
application may bind it on its own commands (a tabbed app's Close Tab): the chord is
then ⇧⌘W when no declared command binds ⇧⌘W, and no chord at all when the application
binds both.

`.context_menu(items)` attaches a long-press / right-click menu to ANY view and takes the
same `MenuView` content. Attach `.state(..)` to each command, not to the menu:

```rust
text("Long press me").padding().context_menu((
    "Copy".action(copy_handler).state(&clipboard),
    Divider,
    "Paste".action(paste_handler).state(&clipboard),
))
```

For more than rows, pass a `ContextMenu`: `.preview(view)` shows content while the menu is
open (a lifted preview on iOS), and `.accessory(view)` adds an interactive view alongside the
commands — a native row in the macOS `NSMenu` (pointer-only, no text entry), a popover
with the commands on iOS. Choosing an item dismisses the menu; acting in the accessory does
not, so call `DismissContextMenu` (extracted with `Use`) to close it:

```rust
text("Hello").context_menu(
    ContextMenu::new((
        "Reply".action(|| {}),
        "Delete".command().action(|| {}).destructive(),
    ))
    .accessory(button("Like").action(|dismiss: DismissContextMenu| dismiss.dismiss())),
)
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
.title() .headline() .subheadline() .body() .caption() .footnote()
.font_size(18.0) .bold(true) .italic(true) .font(font::Caption) .foreground(color)
.weight(font::FontWeight::SemiBold) .monospaced(true) .underline(true) .strikethrough(true) .color(color)
```

`.bold(..)`, `.monospaced(..)`, `.italic(..)`, `.underline(..)`, and `.strikethrough(..)` all take `impl IntoSignal<bool>` — a `bool` literal or a reactive signal of one.
`.line_limit(impl IntoComputed<Option<NonZeroUsize>>)` truncates past that many lines (pass `None` or a signal to expand/collapse). `.text_align(HorizontalAlignment::Center)`
sets the paragraph alignment of multiline text (a signal works too) and
`.background_color(color)` paints behind the text run.

`StyledStr` (`waterui::text::StyledStr`; what `TextField::new` and `text(..)` also accept) takes
`.strikethrough(bool)` beside `.underline(bool)`, and `.on_tap(payload, handler)` makes the
whole string a tappable span: the handler receives the payload (`impl IntoComputed<E>`,
read at tap time, so a `Binding` reports its latest value); a second `.on_tap` replaces the
first: `text(StyledStr::plain("Terms").on_tap("terms", |page: &'static str| open(page)))`.

`.font(..)` takes a slot struct from the prelude's `font` module — `font::Title`,
`Headline`, `Subheadline` (note the spelling; the method is `.subheadline()`), `Body`,
`Caption`, `Footnote` — the form to use when the style is chosen dynamically. On `Text`,
`.font_size(..)` sets the font size and is reactive (`impl IntoSignal<f64>`), leaving the
two-argument frame `.size(w, h)` available directly on `Text`.

A `font::Font` built from a slot carries the same overrides, and every one of them —
`.family(..)`, `.font_size(..)`, `.weight(..)`, `.design(..)`, `.line_height(..)`,
`.letter_spacing(..)` — takes a plain value or a signal of one. A family that follows a
setting is a signal, never a `watch(..)`:

```rust
let family: Binding<Option<Str>> = Binding::new(None); // e.g. a user setting
text("~/src").font(font::Font::from(font::Body).family(family.clone()))
```

`.family(..)` takes a name (`"Menlo"`, a `Str`, a `String`), a signal of one, or a signal
of `Option<Str>` whose `None` (`font::FontFamily::Inherited`) keeps the family the slot
resolves to. Changing the signal re-resolves the font in place; the text is not rebuilt.

Code blocks (feature `highlight`, on by default) are a syntax-highlighted block with a
header naming the language and a copy button. `code(language, content)` and `Code::new`
accept any `impl Into<CodeLanguage>` — a typed `Language` or any string token such as
`"rust"` or `"mermaid"` — and any `impl IntoComputed<Str>` for `content` (static text or a
reactive `Binding<Str>` / `Computed<Str>` that re-highlights in place without `watch`).
Unrecognized language tokens render as `Language::Plaintext` without panicking, keep the
original token as `CodeConfig::info`, and display it in the header. `Code` owns the copy;
`.on_copied(..)` is where the caller tells the user:

```rust
use waterui::prelude::highlight::Language;
use waterui::widget::{Code, code};

code(Language::Rust, "fn main() {}")                   // typed language, static text
code(Language::Rust, source)                           // reactive Binding<Str> / Computed<Str>
code("rust", "fn main() {}")                           // recognized token -> Rust header
code(token, fence_text)                                // any token: unknown renders as plaintext with `token` header
    .on_copied({ let copied = copied.clone(); move |_| copied.set(true) })
```

Richer text (`RichText` and `include_markdown!` need feature `flow-markdown`):

```rust
use waterui::widget::{RichText, rich_text};

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
are `Text | Heading | ListItem | Quote | CodeBlock | Table | Image | Link | Hr`, and a
policy is `None`, `Fade(Animation)` or `Typewriter { .. }`. `max_pending_bytes(n)` bounds
the buffer kept for partial parsing and `table_policy(FlowTablePolicy::NoAnimationReadablePending)`
keeps an unfinished table readable. The config is a move-builder — reassign it in loops.

## Lists and collections

`ForEach` is the plain reactive sequence; `List` is the platform list control (lazy row
realization, swipe-to-delete, reordering, sections, selection).

```rust
use waterui::component::lazy::Lazy;
use waterui::component::list::{ListDelete, ListMove};   // NOT in the prelude
use waterui::views::ForEach;
use waterui::Identifiable;                              // the trait the derive expands to is NOT in the prelude

#[derive(Clone, Identifiable)]
struct Record { #[id] id: u64, title: Str }

// `ForEach` is a `Views` collection, not a `View`: a container consumes it.
Lazy::for_each(records.clone(), |r| text(r.title))       // == Lazy::vstack(ForEach::new(..))
Lazy::hstack(ForEach::new(records.clone(), record_card)) // record_card: fn(Record) -> impl View

List::for_each(records.clone(), |r| text(r.title))
    .editing(is_editing.clone())          // impl IntoComputed<bool>
    .on_delete(|ListDelete(id): ListDelete<u64>, mut s: AppState| {
        if let Some(i) = s.rows.snapshot().iter().position(|r| r.id == id) {
            let _ = s.rows.remove(i);
        }
    })
    .on_move(|ListMove(m): ListMove<u64>, mut s: AppState| {
        let moved = s.rows.remove(m.from());
        s.rows.insert(m.to(), moved);
    })
    .scroll_controller(&controller)
```

`Lazy` stacks (`vstack`, `hstack`, `for_each`, and `vstack_spaced(spacing, ..)` /
`hstack_spaced`) wrap themselves in a scroll view. `ListDelete<Id>(id)` carries the deleted
item's identity; `ListMove<Id>(m)` exposes `.id()`, `.target()` (the item occupying the
destination slot before the move), and part-local `.from()` / post-removal `.to()`. Inside
`List::content`, attach `.on_delete` and `.on_move` directly to each `ForEach` (via
`ForEachExt` in the prelude) or `.on_delete` to an individual `Row`; static rows are not
deletable unless opted in, and moves stay inside their owning part. `AppState` is an owned
`#[state]` type (SKILL.md rule 3), so handlers take it bare.

For a reactive collection rendered as an ordinary (non-lazy) stack that still takes stack
modifiers, `VStack`/`HStack` have `for_each` too; membership animation is declared per row
with `.transition(...)` plus `.animation(...)` (a removed row's ghost holds its slot
while it fades). `.transition(...)` must be the outermost modifier of the row — or of a
`when`/`watch` branch, or of the conditional itself; one beneath `.padding()`, `.frame()`
or any other modifier is never picked up and panics, so put it last:

```rust
use waterui::PropertyTransition;                        // NOT in the prelude
use waterui::layout::stack::VStack;

let row_view = |record: Record| text(record.title)       // `Record` from above; `Row` is the list row type
    .transition(PropertyTransition::opacity())
    .animation(Animation::ease_in_out(Duration::from_millis(250)));
let drawer = VStack::for_each(rows.clone(), row_view).spacing(4.0);
```

Heterogeneous / sectioned static content — rows are *builders*, so the list can rebuild a
row when it is realized again. `row` and `detail_row` take **two** arguments (label,
value) and may sit directly in the content tuple:

```rust
List::content((
    Section::new("Recent")
        .footer("Sub-pages push onto this tab's own stack.")
        .content((
            || text("Today"),
            row("Streak", "14 days"),                    // Row is valid content directly
            detail_row("Last entry", "Yesterday"),
        )),
    || text("Footer"),
))
```

A `Row` also takes `.value_color(color)`, `.detail()` / `.inline()` (the two-line and
one-line layouts) and `.deletable(bool)`. A section's content may also be a `ForEach` over
a reactive collection, next to or instead
of static rows — `Section::new("Today").content(ForEach::new(today, |item| text(item.title)))`.
Its rows are inserted and removed in place as the collection changes, the header sits on
whichever row currently opens the section, and a section with no rows draws neither header
nor footer until rows come back. A grouped, filtered list is therefore one `List::content`
with a `Section` per group, each over its own `SignalCollection`; sections do not nest.

Row modifiers: `.list_row_deletable(signal)`, `.list_row_insets(insets)`; `.list_min_row_height(h)` on
any ancestor sets the row-height floor for every list inside (`0.0` sizes rows to their
content). Selection lives on the
list itself and holds the app's own values — `.selection(&Binding<Option<T>>)` or
`.multi_selection(&Binding<BTreeSet<T>>)` on both `List` and `ListBuilder`, `T: Ord + Clone`.
A `ForEach` row — every row of `List::for_each`, or a `ForEach` inside `List::content` —
is selected by its collection id: `Identifiable::Id` for a `StateList`/`Vec`/`SignalCollection`,
the index for an array or static slice. A static row of `List::content` is selected by the
value it is tagged with, like SwiftUI's `.tag(_:)` — `(|| text("Inbox")).tag(Mailbox::Inbox)`
or `row("Archive", "12").tag(Mailbox::Archive)` — and an untagged static row is not
selectable. Every selectable row of one list is selected by the same type; a selection
of another type does not compile, and neither does a selection over a `List::content`
whose rows are all untagged. Rows carrying the same value are selected together.
At most one mode per list: a later call replaces an earlier one.
`List::for_each` takes any collection whose items are `Clone`; `List::content` takes the
structural tree above; `List::new(views)` takes pre-built `Views`. An
enum row type implements `Identifiable` by hand (`type Id; fn id(&self)`),
keeping the id ranges of different variants disjoint.

A row whose content sits under `.disabled(true)` is inert to selection the way a
disabled `Button` cannot be pressed: pointer, keyboard and accessibility input leave
the selection unchanged, and the row's accessibility node reports disabled without
offering the click action. There is no per-item selectable flag — selection follows
the row content's own disabled state.

When the row set is *derived* — filtered or sorted from other state — do not fall back to
`watch`: wrap the derived signal in `SignalCollection` (see
[reactivity.md](reactivity.md), Reactive collections).

## Forms and pickers

`#[form]` on a struct derives `Default`, `Clone`, `Debug`, `FormBuilder`, and `Project` —
so the struct gains a generated form UI, per-field bindings, and an inference-free
constructor. Field types with a built-in control are `Str` and `String` (text field),
`bool` (toggle), `i32` (stepper), `f32` / `f64` (slider), and `Color` (color picker); other
widths have no `FormBuilder` and need a conversion or a hand-written impl:

```rust
#[form]
struct Settings {
    /// Doc comments become field labels. Text fields bind `Str` or `String`.
    display_name: Str,
    volume: f64,
    notifications: bool,
}

let settings = Settings::binding();              // Binding<Settings>, no annotation needed
form(&settings)                                  // whole generated form
field("Name", &settings.project().display_name)  // or drive one field yourself
```

Validation wraps a validatable control (`TextField`, `Stepper`, `Slider`): the binding
accepts only values the validator passes, and the failure reason shows under the control.
`Required` and a `Range` are validators; combine with `.and(..)` / `.or(..)`. A text field is backed by styled text, so lift a plain-text
validator with `Plain(..)`. `SecureField` (`secure(label, &Binding<Secure>)`) edits a
password without exposing it in `Debug`:

```rust
use waterui::form::valid::{Plain, Required, ValidatableView};

ValidatableView::new(field("Name", &name), Plain(Required))
```

Pickers:

```rust
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
`.segmented()` is the shorthand for `PickerStyle::Segmented`.

Date and color pickers bind typed values. The civil date types are `jiff`'s, re-exported
from `waterui::form::picker::date::{Date, DateTime, Time}` — no `jiff` dependency of your
own is needed to name them or call their constructors, which are fallible:

```rust
use waterui::form::picker::date::Date;

DatePicker::new("Date", &date)                   // generic over T: DatePickable (Date, Time, DateTime)
    .range(min..=max)                            // RangeInclusive<T> of the bound type; clamps immediately
DatePicker::new("Alarm", &time).seconds()        // .seconds() only exists for Time / DateTime
DatePicker::new("Due", &dt).components(DateTimePickerComponents::DateHourMinuteAndSecond)

Calendar::new("Trip Date", &date)                        // month owned by config; `.visible_month(&shown)` optional
    .range(start..=end)
    .decorated(marked_days.clone())                    // impl IntoComputed<BTreeSet<Date>> — passive dots

MultiDatePicker::new("Available", &date_set)             // Binding<BTreeSet<Date>>; `.visible_month(&shown)` optional

ColorPicker::new("Accent", &color).with_alpha().with_hdr()     // Binding<Color>
FilePicker::new("Select Files", &urls).max_count(5)            // Binding<Vec<Url>>; `.import()` copies into app storage
```

Seeding a `Binding<Color>` from a constant: `binding(Color::from(Srgb::from_hex("#4A84F6")))`.
A `Binding<BTreeSet<Date>>` needs the turbofish: `binding(BTreeSet::<Date>::new())`.

## Overlays: snackbars, cards, suspense

Attach `.snackbar_host()` to a root view to install a plain `SnackbarManager` (and
`CodeCopyFeedback` when `flow-markdown` is enabled) into its subtree; it is a `#[state]`-marked
framework type, so descendant handlers take it bare. For snackbars with icons of view type `V`,
attach `.snackbar_host_with_icons::<V>()` (or wrap the subtree in `SnackbarManager::host(|m| ...)`)
and extract `m: SnackbarManager<V>`.

```rust
use core::time::Duration;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};

button("Save").action(|m: SnackbarManager| {
    m.show(
        Snackbar::new("Item moved to trash")
            .action("Undo", || restore())
            .position(SnackbarPosition::BottomCenter)   // BottomCenter | TopCenter | BottomLeading | BottomTrailing
            .closeable()
            .duration(Duration::from_secs(5)),          // default 3 s; Duration::ZERO = until dismissed
    );
});
```

Placements are independent stacks: a top snackbar never evicts a bottom one, and multiple
at one placement stack and reflow. `.icon(|| view)` attaches a typed leading icon builder
(`Snackbar<V>`, so modified non-`Clone` views like `|| mdi::delete().padding()` work directly),
`.state(&v)` injects state for the `.action(..)` and
`.on_dismiss(..)` handlers; `.on_dismiss(|reason: SnackbarDismissReason| ..)` runs once per
presentation with why it went (`Action`, `Timeout`, `Closed`, `Replaced`, `Programmatic`),
and `manager.dismiss()` dismisses the current one.

Code that is not a WaterUI handler — a callback handed to another component, a
self-drawn view's `on_input` — cannot take the manager as a parameter. Read it from the
environment while the view is built and move it into the callback:

```rust
use waterui::env::use_env;

use_env(|manager: SnackbarManager| {
    text("Copied?").on_tap(move || manager.show(Snackbar::new("Copied")))
})
```

```rust
use waterui::widget::disclosure;                       // not in the prelude

card(content).title("Summary").subtitle("This week")   // Card; .style(CardStyle::Outlined) | Elevated | Filled
suspense(async { load().await })                       // takes a future, not a closure; .loading(view) overrides the spinner
disclosure("Header", || body())                        // Disclosure::new(header, ..).expanded(&open) controls it
view.floating()                                        // themed elevated surface (FloatingStyle-aware)
```

`.secure()` on any view marks it as content that must not appear in screenshots. Only the
iOS backend enforces it today; macOS and Hydrolysis (Android included) render the content
unprotected.

An error an author wants rendered as a view uses `waterui::error` (re-exported as
`waterui::widget::error`): `ErrorView::new(err)` (or `ErrorView::from(err)` /
`error_view(err)`) renders the error's `Display` text by default through
`DefaultErrorView`, and `.builder(|err| view)` or `ErrorView::with_builder(err, |err| view)`
customizes it statically with no `Environment` lookup or type erasure;
`result.error_view(|e| text(e.to_string()))` (`ResultExt`) turns a `Result`'s error into a
typed `View`, and `result.default_error_view()` turns it into an `ErrorView<E>`, so a
body can return `Result<impl View, ErrorView<_>>` or `Result<impl View, impl View>`.

## Badges, avatars, tables, focus

```rust
use waterui::shape::RoundedRectangle;

text("Inbox").badge(3_u32)                      // 0 hides; BadgeValue::{Hidden, Dot, Count(u32), Text(Text)}; badge::Badge::new(v, view).color(..)
avatar("Grace Hopper")                          // initials when there is no image; the name is always spoken
    .image(photo_url)                           // a signal of Url: the fallback shows until it decodes
    .shape(RoundedRectangle::new(16.0))
    .side(64.0)
    .ring(Color::accent(), 2.0)
field("Email", &email).focused(&focus, Field::Email)    // focused while `focus` holds Some(Field::Email)
```

`avatar(name)` shows the initials; `Avatar::new(name, || view)` takes a fallback view
instead. An avatar renders SDR by default, unlike the framework's HDR default;
`.dynamic_range(DynamicRange::High)` (in the prelude) opts one back
in when the picture is the subject, such as a full-width profile header. `.focused(&Binding<Option<T>>, value)` ties a control to a shared focus binding
(`T: Eq + Clone`): setting the binding moves focus, and focusing a control writes its value.
`Table::new(data, (TableColumn::new("Name", |p: &Person| text(p.name.clone())).sortable(Sort::Name), TableColumn::new("Avatar", |p: &Person| avatar(p.name.clone()))))` builds a row-major table over an identifiable `Collection` with arbitrary view-valued cells; chain `.selection(&Binding<Option<C::Id>>)` or `.multi_selection(&Binding<BTreeSet<C::Id>>)` for identity-based row selection and `.sort_order(&Binding<Option<TableSort<K>>>)` when at least one column is marked `.sortable(key)`. Convenience constructors `table(data, columns)` and `col(label, cell)` are exported in the prelude alongside `Table`, `TableColumn`, `TableSort`, and `SortDirection`.

## Accessibility modifiers

```rust
use waterui::accessibility::{AccessibilityRole, AccessibilityState};

.a11y_label("Delete message")          // override the derived label
.a11y_id("inbox.row.3")                // stable automation id — reaches XCUITest and Android
.a11y_role(AccessibilityRole::Button)
.a11y_hidden(true)                     // or a reactive boolean signal
.a11y_state(active.map(|a| AccessibilityState::new().selected(a)))
```

The idiom for a composed row that should read as a single node: `.a11y_hidden(true)` on
the inner content, then `.a11y_role(..)` + `.a11y_label(..)` on the wrapper — or better,
build it with `Label::new(semantic_text, || content)`, which does this for you.

Three more modifiers carry explicit semantics. `.a11y_state(..)` accepts either a
constant `AccessibilityState` or a reactive signal of one. `.a11y_children(
AccessibilityChildren::ExcludeDescendants)` hides every descendant and exposes only this
node (`Automatic`, the default, leaves it to the backend) — the one-modifier form of the
hidden-inner idiom above. `.a11y_geometry(AccessibilityGeometry::Allocation)` reports the
whole frame the parent assigned instead of the view's own rectangle (a canvas whose
drawing fills it, a gap allocated to a spacer), and `AccessibilityGeometry::Region(rect)`
an explicit `Rect`; the default is `Response`. Hit testing, focus and drawing are
unaffected. Both enums live in `waterui::accessibility`.

`.a11y_id` is the escape hatch for views that are hard to label meaningfully; the same
identifier is what `waterui-testing` queries with `.identifier(..)`. Prefer a real label
where one exists — the label serves users, the id only serves tests.

## Dialogs (alerts, confirmations)

`.dialog(...)` presents a window-modal alert while its `Binding<bool>` is true — the
platform's native alert on Apple (`NSAlert` sheet on AppKit, `UIAlertController` on
UIKit), and elsewhere the self-drawn card on a modal layer that dims the window with
the `Scrim` theme colour and leaves the wrapped content inert for pointer, keyboard
and the accessibility tree. Actions declare their role — `Default`, `Cancel`,
`Destructive` — and the platform does the ordering: the primary (first `Default`)
action sits rightmost and answers Return, `Cancel` is visually separated on
iOS, and declaration order holds within each role. Tapping an action runs its handler
then writes `false` to the binding; writing `false` yourself dismisses without a
handler. Escape, Android back and the scrim tap are the *cancel path*: they run the
`Cancel` action's handler, and are no-ops when no `Cancel` exists. At most one
`Cancel` action is allowed — a second panics in a debug build and is logged in a release
build; no actions at all gains a single
acknowledgement in the platform's wording. One dialog presents per window at a time; a
second one presented meanwhile waits until the first closes.

```rust
use waterui::dialog::{Dialog, DialogAction};

let presented = binding(false);
button("Delete")
    .action({ let presented = presented.clone(); move || presented.set(true) })
    .dialog(
        Dialog::new(&presented, "Delete photo?")        // title takes impl IntoText
            .message("This cannot be undone.")          // optional, also IntoText
            .action(DialogAction::cancel("Cancel", || {}))
            .action(DialogAction::destructive("Delete", |_: Environment| trash_photo())),
    )
```

## Sheets (in-window modal presentation)

`.sheet(...)` presents arbitrary view content window-modal while its `Binding<bool>` is
true — `UISheetPresentationController` (or full-screen modal `UIViewController`) on iOS,
`NSWindow` sheet (`beginSheet:`) on macOS, and a self-drawn modal sheet layer with `Scrim`
backdrop on Hydrolysis. While open, content beneath is inert to pointer, keyboard, and
assistive technology; focus enters the sheet on present and returns to the previously
focused control on dismiss. Configure heights with `.detents(...)` (`SheetDetent::Medium`,
`Large`, `Fraction(f32)`, `Height(f32)`, `FullScreen`), opt out of user dismissal with
`.dismissible(false)`, and handle Escape / back / scrim / swipe dismissal with
`.on_dismiss(...)`. Handlers inside the sheet extract `SheetDismiss` to close it
programmatically.

```rust
use waterui::sheet::{Sheet, SheetDetent, SheetDismiss};

let open = binding(false);
button("Filters")
    .action({ let open = open.clone(); move || open.set(true) })
    .sheet(
        Sheet::new(&open, || {
            vstack((
                text("Filter options"),
                button("Done").action(|sheet: SheetDismiss| sheet.dismiss()),
            ))
        })
        .detents([SheetDetent::Medium, SheetDetent::Large])
        .label("Filter options")
        .on_dismiss(|| {}),
    )
```
