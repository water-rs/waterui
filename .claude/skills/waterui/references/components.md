# Component catalog

Signatures verified against the WaterUI source and compiled. Everything here is reachable
from `use waterui::prelude::*;` unless an explicit import is shown.

**Feature gates.** `waterui`'s default features are `gpu`, `assets`, `dialog`, `snackbar`,
`highlight`, `locale-datetime` and `locale-list`. Everything else is opt-in, chiefly `media`,
`video`, `video-gpu`, `webview`, `flow-markdown` (also gates `RichText` and
`include_markdown!`), `markdown-math`, `navigation-restoration`, `inspector`, `remote`, `ts`,
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
it. Named exceptions: `FilePicker::open(..)` / `FilePicker::import(..)` are that type's
constructors, and `progress(..)` takes its label as a `.label(..)` modifier rather than a
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
.size(w, h) / .width(w) / .height(h)
.min_size(w, h) / .max_size(w, h)
.min_width(w) / .min_height(h)                     // single-axis floors
.max_width(w) / .max_height(h)                     // single-axis caps
.max_width(f32::INFINITY)                          // idiom: stretch to full container width
```

On a view these take a plain `f32` and return a `Frame`; chained on that `Frame` they take
signals (`IntoSignalF32`), so `.width(120.0).max_height(height_signal)` makes the cap
reactive. `.min_width`/`.min_height` are how you reserve stage space for a scaled or
rotated child that overflows its intrinsic size. A `Frame` also takes `.ideal_width(w)` /
`.ideal_height(h)` (signals): the size it asks for when its parent proposes nothing.
Unlike `.width(w)`, which pins the minimum, ideal and maximum together, it leaves the frame
free to be smaller or larger when the parent does propose a size.

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

Visual transforms never affect layout. `.scale(x, y)` and `.rotation(degrees)` pivot on the
centre; `.scale_from(x, y, Anchor::TOP_LEFT)` and `.rotation_from(45.0, Anchor::TOP_LEFT)`
pivot on an `Anchor` (in the prelude; normalized, `(0, 0)` top-left: `CENTER`, `TOP_LEFT`,
`TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, or `Anchor::new(x, y)`). Factors and angles take
signals; the anchor is a plain value.

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
let reported = Binding::container(Point::zero());
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
button(label("Locate").icon(lucide::locate_fixed())).label_style(LabelDisplayMode::IconOnly)
toolbar_row().install(LabelDisplayMode::IconOnly)             // scope a mode to a whole subtree
```

`LabelDisplayMode` variants: `Automatic`, `TitleAndIcon`, `TitleOnly`, `IconOnly`,
`Hidden`. `.hide_label()` is the shorthand for `Hidden`. `IconOnly` on a label with no icon
renders its title instead, so install it freely on a toolbar and give each label an
`.icon(..)`. The label text remains the test-query key even when hidden.

```rust
button("Save")                              // Button<NoAction>
    .action(handler)                        // -> Button<impl FnMut(&Environment)>
    .action_async(|| async { … })
    .style(ButtonStyle::Plain)              // Automatic | Plain | Link | Borderless | Bordered | BorderedProminent
                                            // | Glass | GlassProminent (Liquid Glass capsules on Apple;
                                            //   bordered / bordered-prominent elsewhere)
    .size(ControlSize::Large)               // ExtraSmall..=ExtraLarge; buttons default to Small
    .state(&value)                          // inject handler state (repeatable)

button(text!("{edit_label}"))               // a text! satisfies IntoLabel: reactive button titles need no watch

toggle("Wi-Fi", &enabled)                   // &Binding<bool>
Toggle::new("Wi-Fi", &enabled).style(ToggleStyle::Switch)     // Automatic | Switch | Checkbox; .switch() / .checkbox()
slider("Volume", &level).range(0.0..=1.0)   // &Binding<f64>; range is RangeInclusive<f64>
slider("Volume", &level).size(ControlSize::Large)      // sliders default ExtraSmall
    .value_indicator(|v| Str::from(format!("{v:.0}"))) // shown above the thumb while dragging
slider("Volume", &level).step(0.1).min_value_label("Quiet").max_value_label("Loud")
stepper("Quantity", &count)                 // &Binding<i32>
stepper("Items", &count).range(0..=100).step(5)   // range: impl RangeBounds<i32>; step takes a signal
stepper("Items", &count).value_formatter(|n| format!("{n} items"))   // inline value text; T: Into<StyledStr>
field("Email", &address)                    // &Binding<Str>
field("Search", &query).on_submit(handler)  // Return in a line-limited field runs it
TextField::new("Username", &name).prompt("Enter your username")   // placeholder ≠ label
TextField::new("Notes", &notes).disable_line_limit()   // multi-line: Return inserts a line break
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
progress(fraction).circular()               // or .linear()
loading()                                   // indeterminate progress
link("Docs", "https://waterui.dev")         // a Link-style button that opens the URL
```

Style shorthands on `Button` — `.plain()`, `.link()`, `.borderless()`, `.bordered()`,
`.bordered_prominent()` — equal `.style(..)` and work before or after `.action(..)`.
`.borderless()` is the idiomatic toolbar-button style; `Link` shows the
pointing-hand cursor on pointer platforms. `TextField::line_limit(NonZeroUsize)` caps the
lines; `TextField::styled(label, &Binding<StyledStr>)` binds styled text. Switch and
checkbox map to different accessibility roles (`SWITCH` vs `CHECKBOX`), which matters to
tests. `ToggleStyle::Automatic` is the platform's style — a switch on iOS and Android, a
checkbox on macOS — so a test that must pass on every host queries a default toggle
with `Role::toggle(ToggleStyle::Automatic)`. A switch with a visible label fills the
width it is offered (label leading, switch trailing); a checkbox and a toggle with a
hidden label are content-sized.

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
`label("Home").system_icon(SystemIcon::new("house"))` (`waterui::icon::SystemIcon`) names an
SF Symbol: it renders on Apple platforms only and is deliberately unsupported elsewhere
(styling.md, Icons), so portable code uses `.icon(..)`. It panics on a `Label::new` label.

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
"Delete".action(delete_handler).destructive()  // red in every menu; == .role(CommandRole::Destructive)
"Delete".action(delete_handler).shortcut(Shortcut::new(NamedKey::Delete))
Shortcut::new('s').command().shift()           // ⇧⌘S; also .option() and .control()
```

A shortcut key is a `char` or a W3C `NamedKey`. `.command()` is the platform's primary
modifier: ⌘ on Apple, Ctrl elsewhere. When a chord is live depends on where the command
sits: a mounted `Menu` arms its chords whether or not its popup is open, a `.context_menu`
arms them only while it is open, and `App::menu_bar` arms them app-wide. A chord without
Ctrl, Alt or Super does not fire while a text field has focus — that press is typing.

`App::menu_bar(..)` takes a `Menu`, or a tuple, array or `Vec` of them. macOS and Windows
draw it as the system menu bar and iOS feeds it to the system main menu; Linux, the web
and headless hosts draw nothing but keep the shortcuts armed:

```rust
use waterui::app::App;

App::new(move || content(), env).menu_bar(Menu::new("File", (
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
    .accessory(button("Like").action(|Use(dismiss): Use<DismissContextMenu>| dismiss.dismiss())),
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
.title() .headline() .sub_headline() .body() .caption() .footnote()
.size(18.0) .bold() .italic(true) .font(font::Caption) .foreground(color)
.weight(font::FontWeight::SemiBold) .monospaced() .underline(true) .color(color)
```

`.bold()` takes no argument; `.italic(..)` takes a bool — a signal of one works too.
`.line_limit(NonZeroUsize)` truncates past that many lines. `.text_align(HorizontalAlignment::Center)`
sets the paragraph alignment of multiline text (a signal works too) and
`.background_color(color)` paints behind the text run.

`StyledStr` (`waterui::text::StyledStr`; what `TextField::styled` and `text(..)` also accept) takes
`.strikethrough(bool)` beside `.underline(bool)`, and `.on_tap(payload, handler)` makes the
whole string a tappable span: the handler receives the payload (`impl IntoComputed<E>`,
read at tap time, so a `Binding` reports its latest value); a second `.on_tap` replaces the
first: `text(StyledStr::plain("Terms").on_tap("terms", |page: &'static str| open(page)))`.

`.font(..)` takes a slot struct from the prelude's `font` module — `font::Title`,
`Headline`, `Subheadline` (note the spelling; the method is `.sub_headline()`), `Body`,
`Caption`, `Footnote` — the form to use when the style is chosen dynamically. On `Text`,
`.size(..)` is the *font* size, is reactive (`impl IntoSignal<f64>`), and **shadows** the
two-argument frame `.size(w, h)` — to give a text a frame, use `.width(..)`/`.height(..)`
or size its container.

A `font::Font` built from a slot carries the same overrides, and every one of them —
`.family(..)`, `.size(..)`, `.weight(..)`, `.design(..)`, `.line_height(..)`,
`.letter_spacing(..)` — takes a plain value or a signal of one. A family that follows a
setting is a signal, never a `watch(..)`:

```rust
let family: Binding<Option<Str>> = Binding::container(None); // e.g. a user setting
text("~/src").font(font::Font::from(font::Body).family(family.clone()))
```

`.family(..)` takes a name (`"Menlo"`, a `Str`, a `String`), a signal of one, or a signal
of `Option<Str>` whose `None` (`font::FontFamily::Inherited`) keeps the family the slot
resolves to. Changing the signal re-resolves the font in place; the text is not rebuilt.

Code blocks (feature `highlight`, on by default) are a syntax-highlighted block with a
header naming the language and a copy button. `code(language, content)` and `Code::new`
take a typed `Language`, or a token such as `"rust"` — and **panic** when a token names no
language, so a token from runtime data (a fence info string, a server-supplied tag) must be
parsed first, falling back to `Language::Plaintext`, with the original token kept through
`.info(..)`. `Code` owns the copy; `.on_copied(..)` is where the caller tells the user:

```rust
use waterui::prelude::highlight::Language;
use waterui::widget::{Code, code};

code(Language::Rust, "fn main() {}")                   // typed: cannot fail
code("rust", "fn main() {}")                           // a literal token: panics if unknown
code(token.parse::<Language>().unwrap_or(Language::Plaintext), fence_text)
    .info(token)                                       // keep the token as written
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

List::for_each(records.clone(), |r| ListItem::new(text(r.title)))
    .editing(is_editing.clone())          // impl IntoComputed<bool>
    .on_delete(|ListDelete(i), mut s: AppState| { let _ = s.rows.remove(i); })
    .on_move(|ListMove(m), mut s: AppState| {
        let moved = s.rows.remove(m.from());
        s.rows.insert(m.to(), moved);
    })
    .scroll_controller(&controller)
```

`Lazy` stacks (`vstack`, `hstack`, `for_each`, and `vstack_spaced(spacing, ..)` /
`hstack_spaced`) wrap themselves in a scroll view. `ListDelete(index)` destructures to a
`usize`; `ListMove(m)` exposes `.from()`/`.to()`. `AppState` is an owned `#[state]` type
(SKILL.md rule 3), so handlers take it bare.

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
            || ListItem::new(text("Today")),
            row("Streak", "14 days"),                    // Row is valid content directly
            detail_row("Last entry", "Yesterday"),
        )),
    || ListItem::new(text("Footer")),
))
```

A `Row` also takes `.value_color(color)`, `.detail()` / `.inline()` (the two-line and
one-line layouts) and `.deletable(bool)`. A section's content may also be a `ForEach` over
a reactive collection, next to or instead
of static rows — `Section::new("Today").content(ForEach::new(today, |item| ListItem::new(text(item.title))))`.
Its rows are inserted and removed in place as the collection changes, the header sits on
whichever row currently opens the section, and a section with no rows draws neither header
nor footer until rows come back. A grouped, filtered list is therefore one `List::content`
with a `Section` per group, each over its own `SignalCollection`; sections do not nest.

`ListItem` modifiers: `.deletable(signal)`, `.insets(insets)`; `.list_min_row_height(h)` on
any ancestor sets the row-height floor for every list inside (`0.0` sizes rows to their
content). Selection lives on the
list itself and holds the app's own values — `.selection(&Binding<Option<T>>)` or
`.multi_selection(&Binding<BTreeSet<T>>)` on both `List` and `ListBuilder`, `T: Ord + Clone`.
A `ForEach` row — every row of `List::for_each`, or a `ForEach` inside `List::content` —
is selected by its collection id: `Identifiable::Id` for a `List`/`Vec`/`SignalCollection`,
the index for an array or static slice. A static row of `List::content` is selected by the
value it is tagged with, like SwiftUI's `.tag(_:)` — `(|| ListItem::new(text("Inbox"))).tag(Mailbox::Inbox)`
or `row("Archive", "12").tag(Mailbox::Archive)` — and an untagged static row is not
selectable. Every selectable row of one list is selected by the same type; a selection
of another type does not compile, and neither does a selection over a `List::content`
whose rows are all untagged. Rows carrying the same value are selected together.
At most one mode per list: a later call replaces an earlier one.
`List::for_each` takes any collection whose items are `Clone`; `List::content` takes the
structural tree above; `List::new(views)` takes pre-built `Views<View = ListItem>`. An
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
`.segmented()` is the shorthand for `PickerStyle::Segmented`.

Date and color pickers bind typed values. The civil date types are `jiff`'s, re-exported
from `waterui::form::picker::date::{Date, DateTime, Time}` — no `jiff` dependency of your
own is needed to name them or call their constructors, which are fallible:

```rust
use waterui::form::picker::date::Date;

DatePicker::new("Date", &date)                   // T: DatePickable (Date, Time, DateTime) picks the presentation
    .ty(DatePickerType::DateHourMinuteAndSecond) // override; also Date, HourAndMinute, HourMinuteAndSecond, DateHourAndMinute
    .range(min..=max)                            // clamps the bound value immediately

Calendar::new("Trip Date", &date, &visible_month)      // TWO bindings: selection + shown month
    .range(start..=end)
    .decorated(marked_days.clone())                    // impl IntoComputed<BTreeSet<Date>> — passive dots

MultiDatePicker::new("Available", &date_set, &visible_month)   // Binding<BTreeSet<Date>>

ColorPicker::new("Accent", &color).with_alpha().with_hdr()     // Binding<Color>
FilePicker::open("Select Files", &urls).max_count(5)           // Binding<Vec<Url>>; `import(..)` copies into app storage
```

Seeding a `Binding<Color>` from a constant: `binding(Color::from(Srgb::from_hex("#4A84F6")))`.
A `Binding<BTreeSet<Date>>` needs the turbofish: `binding(BTreeSet::<Date>::new())`.

## Overlays: snackbars, cards, suspense, full screen

Every `Window` installs a `SnackbarManager` through `.state()`; it is a `#[state]`-marked
framework type, so handlers take it bare.

```rust
use core::time::Duration;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};

button("Save").action(|m: SnackbarManager| {
    m.show(
        Snackbar::new("Item moved to trash")
            .icon(mdi::delete())
            .action("Undo", || restore())
            .position(SnackbarPosition::BottomCenter)   // BottomCenter | TopCenter | BottomLeading | BottomTrailing
            .closeable()
            .duration(Duration::from_secs(5)),          // default 3 s; Duration::ZERO = until dismissed
    );
});
```

Placements are independent stacks: a top snackbar never evicts a bottom one, and multiple
at one placement stack and reflow. `.state(&v)` injects state for the `.action(..)` and
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
use waterui::widget::accordion;                        // not in the prelude

card(content).title("Summary").subtitle("This week")   // Card; .style(CardStyle::Outlined) | Elevated | Filled
suspense(async { load().await })                       // takes a future, not a closure; .loading(view) overrides the spinner
accordion(header, || body())                           // disclosure; Accordion::with_toggle(&open, ..) controls it
view.floating()                                        // themed elevated surface (FloatingStyle-aware)
```

`FullScreenOverlayManager` (also installed per window, taken bare by a handler) presents
modal, window-filling content: `overlay.show(view)` and `overlay.hide()`.

`.secure()` on any view marks it as content that must not appear in screenshots. Only the
iOS backend enforces it today; macOS and Hydrolysis (Android included) render the content
unprotected.

An error an author wants rendered as a view uses `waterui::widget::error`: `Error::new(err)`
or `Error::from_view(view)` is itself a `View`, so a body may return `Result<impl View,
Error>`; `result.error_view(|e| text(e.to_string()))` (`ResultExt`) turns a `Result`'s error
into one. `Error::new(err)` and `UseDefaultErrorView::new(err)` render through the
environment entry `DefaultErrorView::new(|e: BoxedStdError| view)` installed above them, and
draw nothing when none is installed; `Error::from_view` draws its own view. A separate
`waterui::error::{ErrorView, ErrorViewBuilder}` pair — not the same-named items inside
`widget::error` — is an independent mechanism: `ErrorView::from(err)` renders through an
installed `ErrorViewBuilder::new(|e: Box<dyn std::error::Error>, env: &Environment| view.anyview())`,
or as plain text without one. The two do not read each other's environment entry, so pick
one per app.

## Badges, avatars, tables, focus

```rust
use waterui::shape::RoundedRectangle;

text("Inbox").badge(3)                          // a count badge on any Clone view; badge::Badge::new(n, view).color(..)
avatar("Grace Hopper")                          // initials when there is no image; the name is always spoken
    .image(photo_url)                           // a signal of Url: the fallback shows until it decodes
    .shape(RoundedRectangle::new(0.25))
    .size(64.0)
    .ring(Color::new(theme_color::Accent), 2.0)
field("Email", &email).focused(&focus, Field::Email)    // focused while `focus` holds Some(Field::Email)
```

`avatar(name)` shows the initials; `Avatar::new(name, || view)` takes a fallback view
instead. An avatar renders SDR by default, unlike the framework's HDR default;
`.dynamic_range(ColorSpace::Hdr)` (`waterui::metadata::secure::ColorSpace`) opts one back
in when the picture is the subject, such as a full-width profile header. `.focused(&Binding<Option<T>>, value)` ties a control to a shared focus binding
(`T: Eq + Clone`): setting the binding moves focus, and focusing a control writes its value.
`table(columns)` takes a signal of `Vec<TableColumn>` built with `col(label, rows)`, each
column a `Views` of `Text` rows; both come from `waterui::component::table` (in the
prelude, `table` names that module, not the function).

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

Three more modifiers carry explicit semantics. `.a11y_state(AccessibilityState::new()
.selected(true))` is the constant form of `a11y_state_signal`. `.a11y_children(
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
