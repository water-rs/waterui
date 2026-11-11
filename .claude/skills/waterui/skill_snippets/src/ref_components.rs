//! Snippets from `.claude/skills/waterui/references/components.md`, in file
//! order. Transcription conventions are documented in the crate README.

extern crate alloc;

use waterui::Identifiable;
use waterui::prelude::*;

use waterui_icons_lucide as lucide;
use waterui_icons_material_icon as mdi;

/// Glue: the record type components.md's collection snippets refer to.
#[derive(Clone, Identifiable)]
pub struct Record {
    #[id]
    pub id: u64,
    pub title: Str,
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 1/39
// Listing: three container constructors.
// ---------------------------------------------------------------------------
pub fn components_block_01() {
    let (a, b, c) = (text("a"), text("b"), text("c"));
    let _ = {
        hstack((a, b, c)) // horizontal
    };
    let (a, b, c) = (text("a"), text("b"), text("c"));
    let _ = {
        vstack((a, b, c)) // vertical
    };
    let (back, front) = (text("back"), text("front"));
    let _ = {
        zstack((back, front)) // depth, later children on top
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 2/39
// Method-fragment listing applied to a stack receiver.
// ---------------------------------------------------------------------------
pub fn components_block_02() {
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.spacing(8.0) };
    let stack = vstack((text("a"), text("b")));
    let _ = {
        stack.leading() // or .alignment(Leading); hstack aligns vertically: .top() / .centered() / .bottom()
    };
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.padding() };
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.padding_with(16.0) };
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.padding_with((10.0, 16.0)) };
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 3/39
// ---------------------------------------------------------------------------
pub fn components_block_03() {
    struct Tab;
    impl Tab {
        fn label(&self) -> &'static str {
            "tab"
        }
    }
    // The doc mandates `fn(usize) -> AnyView`: heterogeneous helpers erase so
    // the collect below can unify them.
    #[allow(unknown_lints, needless_anyview)]
    fn photo_tile(i: usize) -> AnyView {
        text!("tile {i}").anyview()
    }
    let tabs = [Tab, Tab];

    let row: HStack<_> = tabs.iter().map(|t| button(t.label())).collect();
    let tiles: VStack<_> = (0..6).map(photo_tile).collect(); // photo_tile: fn(usize) -> AnyView

    let _ = (row, tiles);
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 4/39
// Listing: four spacing/separator forms.
// ---------------------------------------------------------------------------
pub fn components_block_04() {
    let _ = {
        spacer() // flexible, absorbs leftover space
    };
    let _ = {
        spacer_min(12.0) // flexible with a floor
    };
    let _ = {
        spacer().height(16.0) // fixed vertical gap; .width(12.0) for horizontal
    };
    let _ = {
        Divider // a separator line (a unit struct, no call)
    };
    // The `.width(12.0)` alternative named in the trailing comment.
    let _ = spacer().width(12.0);
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 5/39
// Frame-modifier listing applied to fresh receivers.
// ---------------------------------------------------------------------------
pub fn components_block_05() {
    let (w, h) = (100.0_f32, 40.0_f32);

    let view = Divider;
    let _ = { view.size(w, h) };
    let view = Divider;
    let _ = { view.width(w) };
    let view = Divider;
    let _ = { view.height(h) };

    let view = Divider;
    let _ = { view.min_size(w, h) };
    let view = Divider;
    let _ = { view.max_size(w, h) };

    let view = Divider;
    let _ = {
        view.min_width(w) // single-axis floors
    };
    let view = Divider;
    let _ = { view.min_height(h) };

    let view = Divider;
    let _ = {
        view.max_width(w) // single-axis caps
    };
    let view = Divider;
    let _ = { view.max_height(h) };

    let view = Divider;
    let _ = {
        view.max_width(f32::INFINITY) // idiom: stretch to full container width
    };

    // Prose: chained on a `Frame`, the per-axis modifiers take signals.
    let height_signal = Binding::f32(80.0);
    let _ = Divider.width(120.0).max_height(height_signal);

    // Prose (not counted): a `Frame` also takes `.ideal_width(w)` / `.ideal_height(h)`.
    let _ = Divider.max_width(f32::INFINITY).ideal_width(w);
    let _ = Divider.max_height(h).ideal_height(h);
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 6/39
// Listing: two sizing modifiers applied to fresh receivers.
// ---------------------------------------------------------------------------
pub fn components_block_06() {
    let view = Divider;
    let _ = {
        view.aspect_ratio(16.0 / 9.0) // width : height; shrinks to fit inside the offer
    };
    let view = Divider;
    let _ = {
        view.layout_priority(1) // a stack compresses its lowest-priority children first
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" (prose): alignment guides. Not
// counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_alignment_guide_prose() {
    let _ =
        text("x").horizontal_alignment_guide(HorizontalAlignment::Leading, |d| d.size.width / 2.0);
    let _ = text("x").vertical_alignment_guide(VerticalAlignment::Center, |d| {
        d.vertical(VerticalAlignment::Bottom)
    });
    let _ = text("x").horizontal_alignment_guide(HorizontalAlignment::Leading, |d| {
        d.horizontal(HorizontalAlignment::Trailing)
    });
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" (prose): visual transforms. Not
// counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_transforms_prose() {
    // `Anchor` comes from the prelude, as the prose says.
    let _ = text("x").scale(1.5, 1.5);
    let _ = text("x").rotation(45.0);
    let _ = text("x").scale_from(0.5, 0.5, Anchor::TOP_LEFT);
    let _ = text("x").rotation_from(45.0, Anchor::TOP_LEFT);
    let _ = [
        Anchor::CENTER,
        Anchor::TOP_RIGHT,
        Anchor::BOTTOM_LEFT,
        Anchor::BOTTOM_RIGHT,
        Anchor::new(0.25, 0.75),
    ];
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 7/39
// ---------------------------------------------------------------------------
pub fn components_block_07() {
    let (a, b, c) = (text("a"), text("b"), text("c"));
    let (d, e) = (text("d"), text("e"));

    use waterui::layout::grid::{grid as layout_grid, row as grid_row};

    let _ = {
        layout_grid(
            3,
            [
                grid_row((a, b, c)),
                grid_row((d, e)), // short rows are fine
            ],
        )
        .spacing(10.0)
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Absolute placement and overlays" — rust block 8/39
// ---------------------------------------------------------------------------
pub fn components_block_08() -> impl View {
    fn map_view() -> impl View {
        text("map")
    }
    fn status_panel() -> impl View {
        text("status")
    }
    fn controls() -> impl View {
        text("controls")
    }
    fn dot() -> impl View {
        text("dot")
    }

    absolute((
        map_view().pin(PinConstraints::all(0.0)), // stretch to fill
        status_panel().size(220.0, 64.0).position_in_offset(
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_LEADING, // anchor on child, position in parent
            16.0,
            16.0, // offsets — these are signal slots
        ),
        controls().position_in_anchor(UnitPoint::BOTTOM_TRAILING, UnitPoint::BOTTOM_TRAILING),
        dot().pin(
            PinConstraints::new()
                .trailing(12.0)
                .bottom(12.0)
                .width(28.0)
                .height(28.0),
        ),
    ))
}

// ---------------------------------------------------------------------------
// components.md § "## Absolute placement and overlays" — rust block 9/39
// ---------------------------------------------------------------------------
pub fn components_block_09() -> impl View {
    let player = text("player");
    let buffering_indicator = text("buffering");

    overlay(player, buffering_indicator).height(360.0)
}

// ---------------------------------------------------------------------------
// components.md § "## Anchored overlays (popovers, tooltips)" — rust block 10/39
// ---------------------------------------------------------------------------
#[allow(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn components_block_10() -> impl View {
    fn popover_body() -> impl View {
        text("popover")
    }

    use waterui::metadata::anchored_overlay::{
        AnchorEdge, AnchoredOverlay, Clamp, Dismissal, EdgeAlignment,
    };

    let open = binding(false);
    let placed = binding(AnchorEdge::Top);
    button("Options")
        .action({
            let open = open.clone();
            move || open.toggle()
        })
        .anchored_overlay(
            AnchoredOverlay::new(&open, move || popover_body()) // a fresh view per presentation
                .edge(AnchorEdge::Top) // Top / Bottom / Leading / Trailing
                .alignment(EdgeAlignment::Center) // Start / Center / End along the edge
                .gap(4.0) // points away from the anchor
                .flip(true) // flip to the opposite edge on overflow
                .clamp(Clamp::Window { margin: 2.0 }) // stay inside the window
                .dismissal(Dismissal::OutsideInteraction) // or Manual — only the binding closes it
                .placed_edge(&placed), // the edge actually used, after any flip
        )
}

// ---------------------------------------------------------------------------
// components.md § "## Scrolling" — rust block 11/39
// Listing: three scroll constructors.
// ---------------------------------------------------------------------------
pub fn components_block_11() {
    let content = text("content");
    let _ = {
        scroll(content) // vertical
    };
    let content = text("content");
    let _ = { scroll_horizontal(content) };
    let content = text("content");
    let _ = { scroll_both(content) };
}

// ---------------------------------------------------------------------------
// components.md § "## Scrolling" — rust block 12/39
// ---------------------------------------------------------------------------
pub fn components_block_12() {
    use core::time::Duration;
    use waterui::animation::Animation;
    use waterui::layout::scroll::ScrollController;

    fn row_view(r: Record) -> ListItem {
        ListItem::new(text(r.title))
    }
    let records = vec![Record {
        id: 1,
        title: Str::from("One"),
    }];
    let content = text("content");

    let rows = ScrollController::<usize>::new(0);
    let list = List::for_each(records, row_view).scroll_controller(&rows);
    rows.scroll_to(50_000); // jump — does not materialize rows 0..50_000
    rows.animate_to(50_000, Animation::default()); // glide with the platform's smooth scroll
    let at_end = rows.position().map(|p| p.is_some_and(|p| p.at_end()));

    let offset = ScrollController::<Point>::new(Point::zero());
    let reported = Binding::container(Point::zero());
    let view = scroll(content)
        .scroll_controller(&offset)
        .report_offset(&reported);
    let lifted = reported.map(|o| o.y > 0.0); // e.g. lift a top bar once content passes under it
    offset.scroll_to(Point::new(0.0, 2_400.0));
    offset.animate_to(
        Point::new(0.0, 2_400.0),
        Animation::ease_in_out(Duration::from_millis(400)),
    );

    let _ = (list, view, at_end, lifted);
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" — rust block 13/39
// Listing: three label-display-mode forms.
// ---------------------------------------------------------------------------
// components.md writes `lucide::` qualified so the icon set stays visible.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn components_block_13() {
    fn toolbar_row() -> impl View {
        hstack((
            button(label("Locate").icon(lucide::locate_fixed())),
            Divider,
        ))
    }
    let level = Binding::f64(1.0);

    let _ = {
        // exists on every labeled control
        slider("Sensitivity", &level).range(0.5..=3.0).hide_label()
    };
    let _ = {
        button(label("Locate").icon(lucide::locate_fixed())).label_style(LabelDisplayMode::IconOnly)
    };
    let _ = {
        toolbar_row().install(LabelDisplayMode::IconOnly) // scope a mode to a whole subtree
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" — rust block 14/39
// A chain on `button(..)`, then independent control constructors.
// ---------------------------------------------------------------------------
pub fn components_block_14() {
    fn handler() {}
    let value = Binding::i32(0);
    let edit_label = Binding::container(Str::from("Edit"));
    let enabled = Binding::bool(true);
    let level = Binding::f64(0.5);
    let count = Binding::i32(1);
    let address = Binding::container(Str::from(""));
    let query = Binding::container(Str::from(""));
    let name = Binding::container(Str::from(""));
    let notes = Binding::container(Str::from(""));
    let code = Binding::container(Str::from(""));
    let note = Binding::container(Str::from(""));
    let fraction = 0.5_f64;

    let _ = {
        button("Save") // Button<NoAction>
            .action(handler) // -> Button<impl FnMut(&Environment)>
            .action_async(|| async {}) // [ellipsis filled]
            .style(ButtonStyle::Plain) // Automatic | Plain | Link | Borderless | Bordered | BorderedProminent
            // | Glass | GlassProminent (Liquid Glass capsules on Apple;
            //   bordered / bordered-prominent elsewhere)
            .size(ControlSize::Large) // ExtraSmall..=ExtraLarge; buttons default to Small
            .state(&value) // inject handler state (repeatable)
    };

    let _ = {
        // a text! satisfies IntoLabel: reactive button titles need no watch
        button(text!("{edit_label}"))
    };

    let _ = {
        toggle("Wi-Fi", &enabled) // &Binding<bool>
    };
    let _ = {
        // Automatic | Switch | Checkbox; .switch() / .checkbox()
        Toggle::new("Wi-Fi", &enabled).style(ToggleStyle::Switch)
    };
    let _ = {
        slider("Volume", &level).range(0.0..=1.0) // &Binding<f64>; range is RangeInclusive<f64>
    };
    let _ = {
        // sliders default ExtraSmall;
        // value_indicator shows the formatted value above the thumb while dragging
        slider("Volume", &level)
            .size(ControlSize::Large)
            .value_indicator(|v| Str::from(format!("{v:.0}")))
    };
    let _ = {
        slider("Volume", &level)
            .step(0.1)
            .min_value_label("Quiet")
            .max_value_label("Loud")
    };
    let _ = {
        stepper("Quantity", &count) // &Binding<i32>
    };
    let _ = {
        // range: impl RangeBounds<i32>; step takes a signal
        stepper("Items", &count).range(0..=100).step(5)
    };
    let _ = {
        stepper("Items", &count).value_formatter(|n| format!("{n} items")) // inline value text; T: Into<StyledStr>
    };
    let _ = {
        field("Email", &address) // &Binding<Str>
    };
    let _ = {
        // Return in a line-limited field runs it
        field("Search", &query).on_submit(handler)
    };
    let _ = {
        // placeholder ≠ label
        TextField::new("Username", &name).prompt("Enter your username")
    };
    let _ = {
        // multi-line: Return inserts a line break
        TextField::new("Notes", &notes).disable_line_limit()
    };
    use waterui::component::text_field::{ContentType, KeyboardType}; // NOT in the prelude
    let _ = {
        // .content_type declares what the field means, for autofill — it is
        // what makes Apple offer the received SMS code; pair with .keyboard(..)
        TextField::new("Verification code", &code)
            .keyboard(KeyboardType::Number)
            .content_type(ContentType::OneTimeCode)
    };
    let _ = {
        // .selection_menu adds commands to the selected-text menu; a command
        // takes the field's live selection as a handler parameter
        TextField::new("Note", &note).selection_menu(("Bold".action(|selection: TextSelection| {
            let marked = format!("**{}**", selection.selected_text());
            selection.replace(&marked);
        }),))
    };
    let _ = {
        progress(fraction) // impl IntoComputed<f64>
    };
    let _ = {
        progress(fraction).label("Downloading") // its label is a modifier — the one exception
    };
    let _ = {
        progress(fraction).circular() // or .linear()
    };
    let _ = {
        loading() // indeterminate progress
    };
    let _ = {
        link("Docs", "https://waterui.dev") // a Link-style button that opens the URL
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" (prose): the `Button` style shorthands, which
// "work before or after `.action(..)`", the `LabelDisplayMode` variants, the
// `TextField` line limit and styled binding, and `ToggleStyle` shorthands.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_button_style_shorthands() {
    use core::num::NonZeroUsize;
    use waterui::text::styled::StyledStr;

    let _ = button("a").plain().action(|| ());
    let _ = button("a").link().action(|| ());
    let _ = button("a").borderless().action(|| ());
    let _ = button("a").bordered().action(|| ());
    let _ = button("a").bordered_prominent().action(|| ());
    let _ = button("a").action(|| ()).bordered_prominent();

    let _ = LabelDisplayMode::Automatic;
    let _ = LabelDisplayMode::TitleAndIcon;
    let _ = LabelDisplayMode::TitleOnly;
    let _ = LabelDisplayMode::IconOnly;
    let _ = LabelDisplayMode::Hidden;

    let enabled = Binding::bool(true);
    let _ = Toggle::new("a", &enabled).switch();
    let _ = Toggle::new("a", &enabled).checkbox();

    let text_value = Binding::container(Str::from(""));
    let styled_value = Binding::container(StyledStr::default());
    if let Some(lines) = NonZeroUsize::new(3) {
        let _ = TextField::new("a", &text_value).line_limit(lines);
    }
    let _ = TextField::styled("a", &styled_value);
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" — rust block 15/39
// ---------------------------------------------------------------------------
// components.md writes `mdi::` qualified so the icon set stays visible.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn components_block_15() {
    fn message_row(_message: Str) -> impl View {
        text("row")
    }
    let message = Str::from("body");
    let sender = Binding::container(Str::from("Ada"));
    let subject = Binding::container(Str::from("Hello"));

    use waterui::component::label::label; // also in the prelude
    use waterui_icons_material_icon as mdi; // icons come from an icon-set crate (styling.md)

    let _ = { label("Compose").icon(mdi::pencil()) };
    let _ = {
        // icon after the text; .leading() is the default side
        label("Delete").icon(mdi::delete()).trailing()
    };
    let _ = {
        label("Mode")
            .icon(mdi::tune())
            .display_mode(LabelDisplayMode::TitleAndIcon)
    };

    // General form: semantic text + arbitrary content, ONE node for assistive tech.
    // The content is a builder closure — it may run again, so clone what it captures.
    let _ = {
        Label::new(text!("{sender}, {subject}"), move || {
            message_row(message.clone())
        })
    };

    // `.leading()` — the default side, named in the trailing comment.
    let _ = label("Compose").icon(mdi::pencil()).leading();

    // Prose (not counted): `label("Home").system_icon(SystemIcon::new("house"))`.
    let _ = label("Home").system_icon(waterui::icon::SystemIcon::new("house"));
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 16/39
// ---------------------------------------------------------------------------
pub fn components_block_16() -> impl View {
    fn nested_handler() {}
    let selected = Binding::container(String::new());

    Menu::new(
        "Choose an Option",
        (
            "Option A"
                .action(|State(sel): State<Binding<String>>| sel.set("A".into()))
                .state(&selected),
            Divider, // becomes a separator inside a menu
            Menu::new("More", ("Nested".action(nested_handler),)),
        ),
    )
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 17/39
// Listing: the `Command` builder chain and `Shortcut`, one expression each.
// ---------------------------------------------------------------------------
pub fn components_block_17() {
    fn copy_handler() {}
    fn delete_handler() {}
    let clipboard = Binding::container(Str::from(""));
    let nothing_selected = Binding::bool(true);
    let checked = Binding::bool(false);

    let _ = {
        "Copy"
            .action(copy_handler)
            .state(&clipboard) // handler state, per command
            .disabled(nothing_selected) // signal of bool
            .selected(checked) // signal of bool: a checked / toggled row
            .subtitle("Copy the selection") // secondary line under the label
            .shortcut(Shortcut::new('c').command())
    };
    let _ = {
        "Delete".action(delete_handler).destructive() // red in every menu; == .role(CommandRole::Destructive)
    };
    let _ = {
        "Delete"
            .action(delete_handler)
            .shortcut(Shortcut::new(NamedKey::Delete))
    };
    let _ = {
        Shortcut::new('s').command().shift() // ⇧⌘S; also .option() and .control()
    };

    // The alternatives named in the trailing comments.
    let _ = "Delete"
        .action(delete_handler)
        .role(CommandRole::Destructive);
    let _ = Shortcut::new('s').command().option();
    let _ = Shortcut::new('s').command().control();
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 18/39
// `save` and `content` stand for the caller's own handler and root view.
// ---------------------------------------------------------------------------
#[allow(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn components_block_18(env: Environment) -> waterui::app::App {
    fn content() -> impl View {
        text("demo")
    }
    fn save() {}

    use waterui::app::App;

    App::new(move || content(), env).menu_bar(Menu::new(
        "File",
        (
            "Save".action(save).shortcut(Shortcut::new('s').command()),
            Divider,
            MenuItem::CloseWindow,
            MenuItem::Quit,
        ),
    ))
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 19/39
// ---------------------------------------------------------------------------
pub fn components_block_19() -> impl View {
    fn copy_handler() {}
    fn paste_handler() {}
    let clipboard = Binding::container(Str::from(""));

    text("Long press me").padding().context_menu((
        "Copy".action(copy_handler).state(&clipboard),
        Divider,
        "Paste".action(paste_handler).state(&clipboard),
    ))
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 20/39
// ---------------------------------------------------------------------------
pub fn components_block_20() -> impl View {
    text("Hello").context_menu(
        ContextMenu::new((
            "Reply".action(|| {}),
            "Delete".command().action(|| {}).destructive(),
        ))
        .accessory(
            button("Like").action(|Use(dismiss): Use<DismissContextMenu>| dismiss.dismiss()),
        ),
    )
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 21/39
// Listing: three text constructors.
// ---------------------------------------------------------------------------
pub fn components_block_21() {
    let value = Binding::i32(1);
    let count = Binding::i32(2);

    let _ = { text("Static string") };
    let _ = { text!("Reactive {value}") };
    let _ = { text!("{n} items", n = count.clone()) };
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 22/39
// Method-name listing; each is applied to its own `Text` receiver.
// ---------------------------------------------------------------------------
pub fn components_block_22() {
    let color = Color::srgb_hex("#3B82F6");

    let _ = { text("t").title() };
    let _ = { text("t").headline() };
    let _ = { text("t").sub_headline() };
    let _ = { text("t").body() };
    let _ = { text("t").caption() };
    let _ = { text("t").footnote() };

    let _ = { text("t").size(18.0) };
    let _ = { text("t").bold() };
    let _ = { text("t").italic(true) };
    let _ = { text("t").font(font::Caption) };
    let _ = { text("t").foreground(color) };

    let color = Color::srgb_hex("#3B82F6");
    let _ = { text("t").weight(font::FontWeight::SemiBold) };
    let _ = { text("t").monospaced() };
    let _ = { text("t").underline(true) };
    let _ = { text("t").color(color) };

    // The other font slots named in the prose.
    let _ = text("t").font(font::Title);
    let _ = text("t").font(font::Headline);
    let _ = text("t").font(font::Subheadline);
    let _ = text("t").font(font::Body);
    let _ = text("t").font(font::Footnote);

    // `.line_limit(NonZeroUsize)` named in the prose.
    if let Some(lines) = core::num::NonZeroUsize::new(2) {
        let _ = text("t").line_limit(lines);
    }

    // Prose (not counted): `.text_align(HorizontalAlignment::Center)` and
    // `.background_color(color)`.
    let _ = text("t").text_align(HorizontalAlignment::Center);
    let _ = text("t").background_color(Color::srgb_hex("#3B82F6"));

    // Prose (not counted): `StyledStr` strikethrough and tappable span.
    use waterui::text::StyledStr;
    fn open(_page: &'static str) {}
    let _ = StyledStr::plain("Struck").strikethrough(true);
    let _ = text(StyledStr::plain("Terms").on_tap("terms", |page: &'static str| open(page)));
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 23/39
// ---------------------------------------------------------------------------
pub fn components_block_23() -> impl View {
    let family: Binding<Option<Str>> = Binding::container(None); // e.g. a user setting
    text("~/src").font(font::Font::from(font::Body).family(family.clone()))
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 24/39
// Listing: `Code` constructors. `token` and `fence_text` stand for runtime
// data; `copied` for the caller's own state.
// ---------------------------------------------------------------------------
pub fn components_block_24(token: &'static str, fence_text: &'static str) {
    let copied = Binding::bool(false);

    use waterui::prelude::highlight::Language;
    use waterui::widget::{Code, code};

    let _ = {
        code(Language::Rust, "fn main() {}") // typed: cannot fail
    };
    let _ = {
        code("rust", "fn main() {}") // a literal token: panics if unknown
    };
    let _ = {
        code(
            token.parse::<Language>().unwrap_or(Language::Plaintext),
            fence_text,
        )
        .info(token) // keep the token as written
        .on_copied({
            let copied = copied.clone();
            move |_| copied.set(true)
        })
    };

    let _: Option<Code> = None;
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 25/39
// ---------------------------------------------------------------------------
pub fn components_block_25() {
    let runtime_str = "# Heading";

    use waterui::widget::{RichText, rich_text};

    let _ = {
        // compile-time: expands to RichText::from_markdown(include_str!(..))
        include_markdown!("guide.md")
    };
    let _ = {
        RichText::from_markdown(runtime_str) // runtime markdown -> RichText
    };

    let _ = rich_text(Vec::new());
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 26/39
//
// `FlowMarkdownConfig` is itself a constant signal, so the config goes in
// bare — no `Computed::constant` wrapper.
// ---------------------------------------------------------------------------
pub fn components_block_26() -> impl View {
    use core::time::Duration;
    use waterui::animation::Animation;

    let markdown = Binding::container(Str::from("# Streaming"));

    use waterui::prelude::flow_markdown::FlowMarkdownConfig;

    let config = FlowMarkdownConfig::default()
        .stream(FlowStreamMode::AppendOnly) // source only ever grows: the LLM fast path
        .preset(FlowAnimationPreset::AssistantDefault) // | Minimal | None
        .token_fade_in(Some(Animation::ease_out(Duration::from_millis(180))));
    flow_markdown(markdown.clone()).configuration(config)
}

// ---------------------------------------------------------------------------
// components.md § "## Text" (prose): the per-element override, the
// `FlowElementKind` / `FlowAnimationPreset` / `FlowAnimationPolicy` variants,
// `max_pending_bytes` and `table_policy`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_flow_overrides_prose() {
    use core::time::Duration;
    use waterui::animation::Animation;
    use waterui::prelude::flow_markdown::FlowMarkdownConfig;

    let _ = FlowMarkdownConfig::default().override_animation(
        FlowElementKind::Text,
        FlowAnimationPolicy::Typewriter {
            cps: 40,
            batch_ms: 24,
            fade_in: None,
        },
    );
    let _ = FlowMarkdownConfig::default().override_animation(
        FlowElementKind::Heading,
        FlowAnimationPolicy::Fade(Animation::ease_out(Duration::from_millis(180))),
    );
    let _ = FlowMarkdownConfig::default()
        .override_animation(FlowElementKind::Link, FlowAnimationPolicy::None);
    let _ = FlowMarkdownConfig::default()
        .max_pending_bytes(64 * 1024)
        .table_policy(FlowTablePolicy::NoAnimationReadablePending);

    let _ = FlowElementKind::ListItem;
    let _ = FlowElementKind::Quote;
    let _ = FlowElementKind::CodeBlock;
    let _ = FlowElementKind::Table;
    let _ = FlowElementKind::Image;
    let _ = FlowElementKind::Hr;
    let _ = FlowAnimationPreset::Minimal;
    let _ = FlowAnimationPreset::None;
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" — rust block 27/39
// Item declarations, then a listing of container forms.
// ---------------------------------------------------------------------------
pub mod components_block_27 {
    use waterui::layout::scroll::ScrollController;
    use waterui::prelude::*;
    use waterui::reactive::collection::List as ReactiveList;

    #[state]
    #[derive(Clone)]
    pub struct AppState {
        pub rows: ReactiveList<Record>,
    }

    use waterui::Identifiable; // the trait the derive expands to is NOT in the prelude
    use waterui::component::lazy::Lazy;
    use waterui::component::list::{ListDelete, ListMove}; // NOT in the prelude
    use waterui::views::ForEach;

    #[derive(Clone, Identifiable)]
    pub struct Record {
        #[id]
        id: u64,
        title: Str,
    }

    fn record_card(r: Record) -> Text {
        text(r.title)
    }

    pub fn body() {
        let records = ReactiveList::<Record>::new();
        let is_editing = Binding::bool(false);
        let controller = ScrollController::<usize>::new(0);

        // `ForEach` is a `Views` collection, not a `View`: a container consumes it.
        let _ = {
            // == Lazy::vstack(ForEach::new(..))
            Lazy::for_each(records.clone(), |r| text(r.title))
        };
        let _ = {
            // record_card: fn(Record) -> impl View
            Lazy::hstack(ForEach::new(records.clone(), record_card))
        };

        let _ = {
            List::for_each(records.clone(), |r| ListItem::new(text(r.title)))
                .editing(is_editing.clone()) // impl IntoComputed<bool>
                .on_delete(|ListDelete(i), mut s: AppState| {
                    let _ = s.rows.remove(i);
                })
                .on_move(|ListMove(m), mut s: AppState| {
                    let moved = s.rows.remove(m.from());
                    s.rows.insert(m.to(), moved);
                })
                .scroll_controller(&controller)
        };

        // Prose: `Lazy` spaced stacks and `List::new` over pre-built rows.
        let _ = Lazy::vstack_spaced(8.0, ForEach::new(records.clone(), record_card));
        let _ = Lazy::hstack_spaced(8.0, ForEach::new(records.clone(), record_card));
        let _ = List::new(ForEach::new(records, |r: Record| {
            ListItem::new(text(r.title))
        }));
    }
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" — rust block 28/39
// ---------------------------------------------------------------------------
pub fn components_block_28() {
    use core::time::Duration;
    use waterui::animation::Animation;
    use waterui::reactive::collection::List as ReactiveList;

    let rows = ReactiveList::<Record>::new();

    use waterui::PropertyTransition; // NOT in the prelude
    use waterui::layout::stack::VStack;

    // `Record` from above; `Row` is the list row type
    let row_view = |record: Record| {
        text(record.title)
            .transition(PropertyTransition::opacity())
            .animation(Animation::ease_in_out(Duration::from_millis(250)))
    };
    let drawer = VStack::for_each(rows.clone(), row_view).spacing(4.0);

    let _ = drawer;
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" — rust block 29/39
// ---------------------------------------------------------------------------
pub fn components_block_29() -> impl View {
    List::content((
        Section::new("Recent")
            .footer("Sub-pages push onto this tab's own stack.")
            .content((
                || ListItem::new(text("Today")),
                row("Streak", "14 days"), // Row is valid content directly
                detail_row("Last entry", "Yesterday"),
            )),
        || ListItem::new(text("Footer")),
    ))
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" (prose): a `ForEach` section and
// the `Row` modifiers. Not counted as a rust block. Glue: `today` stands for
// the prose's reactive collection.
// ---------------------------------------------------------------------------
pub fn components_reactive_section_prose(
    today: waterui::reactive::collection::List<Record>,
) -> impl View {
    use waterui::views::ForEach;

    let _ = row("a", "b")
        .value_color(Color::srgb_hex("#3B82F6"))
        .detail()
        .inline()
        .deletable(true);

    List::content(
        Section::new("Today").content(ForEach::new(today, |item: Record| {
            ListItem::new(text(item.title))
        })),
    )
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" (prose): the `ListItem` modifiers
// and `list_min_row_height`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_list_item_prose(records: waterui::reactive::collection::List<Record>) {
    use waterui::layout::padding::EdgeInsets;
    use waterui::views::ForEach;

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Mailbox {
        Inbox,
        Archive,
    }

    let signal = Binding::bool(true);
    let selection = Binding::container(None::<Mailbox>);
    let multi = Binding::container(alloc::collections::BTreeSet::<Mailbox>::new());
    let record_selection = Binding::container(None::<u64>);

    let _ = ListItem::new(text("a")).deletable(signal.clone());
    let _ = ListItem::new(text("a")).insets(EdgeInsets::all(4.0));
    let _ = text("a").list_min_row_height(0.0);
    let _ = List::for_each(records.clone(), |r: Record| ListItem::new(text(r.title)))
        .selection(&record_selection);
    let _ = List::content((
        (|| ListItem::new(text("Inbox"))).tag(Mailbox::Inbox),
        row("Archive", "12").tag(Mailbox::Archive),
    ))
    .selection(&selection);
    let _ = List::content((
        row("Archive", "12").tag(Mailbox::Archive),
        row("Storage", "3 GB"), // untagged: not selectable
    ))
    .multi_selection(&multi);
    let _ = List::content(
        Section::new("Records").content(ForEach::new(records, |r: Record| {
            ListItem::new(text(r.title))
        })),
    )
    .selection(&record_selection);
    // A row under `.disabled(true)` is inert to selection; there is no
    // per-item selectable flag — selection follows the content's disabled state.
    let _ = ListItem::new(text("a").disabled(true));
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" — rust block 30/39
// ---------------------------------------------------------------------------
pub mod components_block_30 {
    use waterui::prelude::*;

    #[form]
    struct Settings {
        /// Doc comments become field labels. Text fields bind `Str` or `String`.
        display_name: Str,
        volume: f64,
        notifications: bool,
    }

    pub fn body() {
        let settings = Settings::binding(); // Binding<Settings>, no annotation needed
        let _ = {
            form(&settings) // whole generated form
        };
        let _ = {
            // or drive one field yourself
            field("Name", &settings.project().display_name)
        };
    }
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" — rust block 31/39
// ---------------------------------------------------------------------------
pub fn components_block_31() -> impl View {
    let name = Binding::container(Str::from(""));

    use waterui::form::valid::{Plain, Required, ValidatableView};

    ValidatableView::new(field("Name", &name), Plain(Required))
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" (prose): `SecureField` and `.and(..)`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_form_validation_prose() {
    use waterui::form::secure::Secure;
    use waterui::form::valid::{Required, Validator};

    let password = Binding::container(Secure::default());
    let _ = secure("Password", &password);
    // `Required` validates several text types, so `.and` names the one it combines over.
    let _ = Validator::<Str>::and(Required, Required);
    let _ = <Required as Validator<Str>>::validate;
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" — rust block 32/39
// ---------------------------------------------------------------------------
pub mod components_block_32 {
    use waterui::prelude::*;
    use waterui::reactive::binding;

    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Fruit {
        Apple,
        Pear,
    }

    use waterui::form::Calendar;
    use waterui::form::picker::color::ColorPicker;
    use waterui::form::picker::date::{DatePicker, DatePickerType};
    use waterui::form::picker::file::FilePicker;
    use waterui::form::picker::multi_date::MultiDatePicker;
    use waterui::form::picker::{Picker, PickerItem, PickerStyle, picker};

    // Options are views carrying their value via .tag(..). The tag type is T: Ord + Clone.
    // PickerItem<T> is the nameable item type — an array of them works without collect().
    fn sizes() -> [PickerItem<&'static str>; 3] {
        [
            text("Small").tag("S"),
            text("Medium").tag("M"),
            text("Large").tag("L"),
        ]
    }

    pub fn body() {
        let choice: Binding<&'static str> = binding("S");
        let options = sizes();
        let _ = Fruit::Apple;

        let _ = {
            picker("Size", sizes(), &choice) // ergonomic constructor
        };
        let _ = {
            Picker::new(text!("Sort by"), options, &choice) // labels may be localized text!
                .style(PickerStyle::Menu) // Automatic | Menu | Radio | Segmented
                .hide_label()
        };

        // The remaining style variants named in the trailing comment, and the
        // `.segmented()` shorthand named in the prose.
        let _ = PickerStyle::Automatic;
        let _ = PickerStyle::Radio;
        let _ = PickerStyle::Segmented;
        let _ = Picker::new("Size", sizes(), &choice).segmented();

        // Keep the other picker imports live so they are proven to resolve.
        let _: Option<ColorPicker> = None;
        let _: Option<DatePickerType> = None;
        let _: Option<FilePicker> = None;
        let _ = (
            core::marker::PhantomData::<DatePicker>,
            core::marker::PhantomData::<MultiDatePicker>,
            core::marker::PhantomData::<Calendar>,
        );
    }
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" — rust block 33/39
// ---------------------------------------------------------------------------
pub fn components_block_33() {
    use alloc::collections::BTreeSet;
    use waterui::color::Srgb;
    use waterui::form::Calendar;
    use waterui::form::picker::color::ColorPicker;
    use waterui::form::picker::date::{DatePicker, DatePickerType};
    use waterui::form::picker::file::FilePicker;
    use waterui::form::picker::multi_date::MultiDatePicker;
    use waterui::media::Url;
    use waterui::reactive::binding;

    use waterui::form::picker::date::Date;

    let date = binding(Date::constant(2026, 3, 20));
    let (min, max) = (Date::constant(2026, 1, 1), Date::constant(2026, 12, 31));
    let (start, end) = (min, max);
    let visible_month = binding(Date::constant(2026, 3, 1));
    let marked_days: Binding<BTreeSet<Date>> = binding(BTreeSet::<Date>::new());
    let date_set: Binding<BTreeSet<Date>> = binding(BTreeSet::<Date>::new());
    let color: Binding<Color> = binding(Color::from(Srgb::from_hex("#4A84F6")));
    let urls: Binding<Vec<Url>> = binding(Vec::<Url>::new());

    let _ = {
        DatePicker::new("Date", &date) // T: DatePickable (Date, Time, DateTime) picks the presentation
            .ty(DatePickerType::DateHourMinuteAndSecond) // override; also Date, HourAndMinute, HourMinuteAndSecond, DateHourAndMinute
            .range(min..=max) // clamps the bound value immediately
    };

    let _ = {
        Calendar::new("Trip Date", &date, &visible_month) // TWO bindings: selection + shown month
            .range(start..=end)
            .decorated(marked_days.clone()) // impl IntoComputed<BTreeSet<Date>> — passive dots
    };

    let _ = {
        // Binding<BTreeSet<Date>>
        MultiDatePicker::new("Available", &date_set, &visible_month)
    };

    let _ = {
        ColorPicker::new("Accent", &color).with_alpha().with_hdr() // Binding<Color>
    };
    let _ = {
        // Binding<Vec<Url>>; `import(..)` copies into app storage
        FilePicker::open("Select Files", &urls).max_count(5)
    };

    let _ = DatePickerType::Date;
    let _ = DatePickerType::HourAndMinute;
    let _ = DatePickerType::HourMinuteAndSecond;
    let _ = DatePickerType::DateHourAndMinute;
    let _ = FilePicker::import("Import Files", &urls);
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" (prose): the two seeding idioms.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_picker_seeding_prose() {
    use alloc::collections::BTreeSet;
    use waterui::color::Srgb;
    use waterui::form::picker::date::Date;
    use waterui::reactive::binding;

    let _: Binding<Color> = binding(Color::from(Srgb::from_hex("#4A84F6")));
    let _: Binding<BTreeSet<Date>> = binding(BTreeSet::<Date>::new());
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays: snackbars, cards, suspense, full screen" — rust block 34/39
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    unused_must_use,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
// Verbatim: components.md writes `mdi::` qualified so the icon set stays visible.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn components_block_34() {
    fn restore() {}

    use core::time::Duration;
    use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};

    button("Save").action(|m: SnackbarManager| {
        m.show(
            Snackbar::new("Item moved to trash")
                .icon(mdi::delete())
                .action("Undo", || restore())
                .position(SnackbarPosition::BottomCenter) // BottomCenter | TopCenter | BottomLeading | BottomTrailing
                .closeable()
                .duration(Duration::from_secs(5)), // default 3 s; Duration::ZERO = until dismissed
        );
    });
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays: snackbars, cards, suspense, full screen" (prose): the other snackbar placements,
// `.state`, `.on_dismiss`, `SnackbarManager::dismiss` and the full-screen
// overlay manager. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_snackbar_prose() {
    use waterui::snackbar::{Snackbar, SnackbarDismissReason, SnackbarManager, SnackbarPosition};

    let value = Binding::i32(0);

    let _ = SnackbarPosition::TopCenter;
    let _ = SnackbarPosition::BottomLeading;
    let _ = SnackbarPosition::BottomTrailing;
    let _ = Snackbar::new("x")
        .state(&value)
        .on_dismiss(|reason: SnackbarDismissReason| {
            let _ = reason;
        });
    let _ = button("x").action(|manager: SnackbarManager| manager.dismiss());
    let _ = button("x").action(|overlay: FullScreenOverlayManager| {
        overlay.show(text("modal"));
        overlay.hide();
    });
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays: snackbars, cards, suspense, full screen" — rust block 35/39
// ---------------------------------------------------------------------------
pub fn components_block_35() -> impl View {
    use waterui::env::use_env;
    use waterui::snackbar::{Snackbar, SnackbarManager};

    use_env(|manager: SnackbarManager| {
        text("Copied?").on_tap(move || manager.show(Snackbar::new("Copied")))
    })
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays: snackbars, cards, suspense, full screen" — rust block 36/39
// Listing: four overlay/composition helpers.
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn components_block_36() {
    async fn load() -> Text {
        text("loaded")
    }
    fn body() -> impl View {
        text("body")
    }

    let content = text("content");
    let header = text("header");
    let view = Divider;

    use waterui::widget::accordion; // not in the prelude

    let _ = {
        // Card; .style(CardStyle::Outlined) | Elevated | Filled
        card(content).title("Summary").subtitle("This week")
    };
    let _ = {
        // takes a future, not a closure; .loading(view) overrides the spinner
        suspense(async { load().await })
    };
    let _ = {
        // disclosure; Accordion::with_toggle(&open, ..) controls it
        accordion(header, || body())
    };
    let _ = {
        view.floating() // themed elevated surface (FloatingStyle-aware)
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays: snackbars, cards, suspense, full screen" (prose): `CardStyle`, `Suspense::loading` and
// `Accordion::with_toggle`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_overlay_extras_prose() {
    use waterui::widget::Accordion;

    let open = Binding::bool(false);

    let _ = card(text("c")).style(CardStyle::Outlined);
    let _ = card(text("c")).style(CardStyle::Elevated);
    let _ = card(text("c")).style(CardStyle::Filled);
    let _ = suspense(async { text("x") }).loading(text("loading"));
    let _ = Accordion::with_toggle(&open, text("h"), || text("b"));
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays: snackbars, cards, suspense, full screen"
// (prose): `.secure()` and the error-view types. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_secure_and_error_prose() -> impl View {
    use std::io;
    use waterui::widget::error::{
        BoxedStdError, DefaultErrorView, Error, ResultExt, UseDefaultErrorView,
    };

    let _ = Divider.secure();

    let _ = Error::new(io::Error::other("failed"));
    let _ = Error::from_view(text("failed"));
    let loaded: Result<Str, io::Error> = Ok(Str::from("data"));
    let _: Result<Str, Error> = loaded.error_view(|e| text(e.to_string()));
    let _ = Environment::new().with(DefaultErrorView::new(|e: BoxedStdError| {
        text(e.to_string())
    }));
    let _ = UseDefaultErrorView::new(io::Error::other("failed"));

    // The independent runtime pair.
    let _ = waterui::error::ErrorView::from(io::Error::other("failed"));
    let _ = waterui::error::ErrorViewBuilder::new(
        |e: Box<dyn std::error::Error>, _env: &Environment| text(e.to_string()).anyview(),
    );

    // A body may return `Result<impl View, Error>`.
    let body: Result<_, Error> = Ok(text("ok"));
    body
}

// ---------------------------------------------------------------------------
// components.md § "## Badges, avatars, tables, focus" — rust block 37/39
// Listing: three independent widgets. `photo_url` and `Field` are glue.
// ---------------------------------------------------------------------------
pub fn components_block_37(photo_url: waterui::Url) {
    use waterui::shape::RoundedRectangle;

    #[derive(Clone, PartialEq, Eq)]
    enum Field {
        Email,
    }
    let email = Binding::container(Str::from(""));
    let focus = Binding::container(None::<Field>);

    let _ = {
        text("Inbox").badge(3) // a count badge on any Clone view; badge::Badge::new(n, view).color(..)
    };
    let _ = {
        avatar("Grace Hopper") // initials when there is no image; the name is always spoken
            .image(photo_url) // a signal of Url: the fallback shows until it decodes
            .shape(RoundedRectangle::new(0.25))
            .size(64.0)
            .ring(Color::new(theme_color::Accent), 2.0)
    };
    let _ = {
        field("Email", &email).focused(&focus, Field::Email) // focused while `focus` holds Some(Field::Email)
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Badges, avatars, tables, focus" (prose): the
// general `Avatar` constructor. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_avatar_prose() {
    use waterui::widget::Avatar;

    let _ = Avatar::new("Katherine Johnson", || text("KJ").bold());

    // Prose (not counted): `.dynamic_range(ColorSpace::Hdr)`.
    use waterui::metadata::secure::ColorSpace;
    let _ = avatar("Grace Hopper").dynamic_range(ColorSpace::Hdr);
}

// ---------------------------------------------------------------------------
// components.md § "## Accessibility modifiers" — rust block 38/39
// Method-fragment listing applied to fresh receivers.
// ---------------------------------------------------------------------------
pub fn components_block_38() {
    let active = Binding::bool(true);

    use waterui::accessibility::{AccessibilityRole, AccessibilityState};

    let view = Divider;
    let _ = {
        view.a11y_label("Delete message") // override the derived label
    };
    let view = Divider;
    let _ = {
        view.a11y_id("inbox.row.3") // stable automation id — reaches XCUITest and Android
    };
    let view = Divider;
    let _ = { view.a11y_role(AccessibilityRole::Button) };
    let view = Divider;
    let _ = {
        view.a11y_hidden(true) // decorative only
    };
    let view = Divider;
    let _ = { view.a11y_state_signal(active.map(|a| AccessibilityState::new().selected(a))) };
}

// ---------------------------------------------------------------------------
// components.md § "## Accessibility modifiers" (prose): the constant state,
// children and geometry modifiers. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_a11y_explicit_prose() {
    use waterui::accessibility::{
        AccessibilityChildren, AccessibilityGeometry, AccessibilityState,
    };

    let _ = Divider.a11y_state(AccessibilityState::new().selected(true));
    let _ = Divider.a11y_children(AccessibilityChildren::ExcludeDescendants);
    let _ = Divider.a11y_children(AccessibilityChildren::Automatic);
    let _ = Divider.a11y_geometry(AccessibilityGeometry::Allocation);
    let _ = Divider.a11y_geometry(AccessibilityGeometry::Region(Rect::new(
        Point::zero(),
        Size::new(24.0, 24.0),
    )));
    let _ = AccessibilityGeometry::Response;
}

// ---------------------------------------------------------------------------
// components.md § "## Dialogs (alerts, confirmations)" — rust block 39/39
// `trash_photo` is the doc's stand-in for the caller's own handler.
// ---------------------------------------------------------------------------
pub fn components_block_39() {
    fn trash_photo() {}

    use waterui::dialog::{Dialog, DialogAction};

    let presented = binding(false);
    let _ = button("Delete")
        .action({
            let presented = presented.clone();
            move || presented.set(true)
        })
        .dialog(
            Dialog::new(&presented, "Delete photo?") // title takes impl IntoText
                .message("This cannot be undone.") // optional, also IntoText
                .action(DialogAction::cancel("Cancel", || {}))
                .action(DialogAction::destructive("Delete", |_: Environment| {
                    trash_photo()
                })),
        );
}
