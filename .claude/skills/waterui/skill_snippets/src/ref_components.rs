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
// components.md § "## Layout containers" — rust block 1/28
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
// components.md § "## Layout containers" — rust block 2/28
// Method-fragment listing applied to a stack receiver.
// ---------------------------------------------------------------------------
pub fn components_block_02() {
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.spacing(8.0) };
    let stack = vstack((text("a"), text("b")));
    let _ = {
        stack.alignment(HorizontalAlignment::Leading) // VerticalAlignment on hstack
    };
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.padding() };
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.padding_with(16.0) };
    let stack = vstack((text("a"), text("b")));
    let _ = { stack.padding_with(EdgeInsets::symmetric(10.0, 16.0)) };
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 3/28
// ---------------------------------------------------------------------------
pub fn components_block_03() {
    struct Tab;
    impl Tab {
        fn label(&self) -> &'static str {
            "tab"
        }
    }
    fn photo_tile(i: usize) -> AnyView {
        text(format!("tile {i}")).anyview()
    }
    let tabs = [Tab, Tab];

    let row: HStack<_> = tabs.iter().map(|t| button(t.label())).collect();
    let tiles: VStack<_> = (0..6).map(photo_tile).collect(); // photo_tile: fn(usize) -> AnyView

    let _ = (row, tiles);
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 4/28
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
// components.md § "## Layout containers" — rust block 5/28
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
    let _ = { view.min_width(w) };
    let view = Divider;
    let _ = { view.min_height(h) };

    let view = Divider;
    let _ = { view.max_width(w) };
    let view = Divider;
    let _ = { view.max_height(h) };

    let view = Divider;
    let _ = {
        view.max_width(f32::INFINITY) // idiom: stretch to full container width
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Layout containers" — rust block 6/28
// ---------------------------------------------------------------------------
pub fn components_block_06() {
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
// components.md § "## Absolute placement and overlays" — rust block 7/28
// ---------------------------------------------------------------------------
pub fn components_block_07() -> impl View {
    fn map_view() -> impl View {
        text("map")
    }
    fn status_panel() -> impl View {
        text("status")
    }
    fn controls() -> impl View {
        text("controls")
    }

    use waterui::layout::PinConstraints; // not in the prelude (PositionExt is)

    absolute((
        map_view().pin(PinConstraints::all(0.0)), // stretch to fill
        status_panel().size(220.0, 64.0).position_in_offset(
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_LEADING, // anchor on child, position in parent
            16.0,
            16.0, // offsets — these are signal slots
        ),
        controls().position_in(UnitPoint::BOTTOM_TRAILING),
    ))
}

// ---------------------------------------------------------------------------
// components.md § "## Absolute placement and overlays" — rust block 8/28
// ---------------------------------------------------------------------------
pub fn components_block_08() -> impl View {
    let player = text("player");
    let buffering_indicator = text("buffering");

    overlay(player, buffering_indicator).height(360.0)
}

// ---------------------------------------------------------------------------
// components.md § "## Scrolling" — rust block 9/28
// Listing: three scroll constructors.
// ---------------------------------------------------------------------------
pub fn components_block_09() {
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
// components.md § "## Scrolling" — rust block 10/28
// ---------------------------------------------------------------------------
pub fn components_block_10() {
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
    rows.scroll_to(50_000); // does not materialize rows 0..50_000

    let offset = ScrollController::<Point>::new(Point::zero());
    let view = scroll(content).scroll_controller(&offset);
    offset.scroll_to(Point::new(0.0, 2_400.0));

    let _ = (list, view);
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" — rust block 11/28
// Listing: three label-display-mode forms.
// ---------------------------------------------------------------------------
pub fn components_block_11() {
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
// components.md § "## Controls" — rust block 12/28
// A chain on `button(..)`, then eleven independent control constructors.
// ---------------------------------------------------------------------------
pub fn components_block_12() {
    fn handler() {}
    let value = Binding::i32(0);
    let edit_label = Binding::container(Str::from("Edit"));
    let enabled = Binding::bool(true);
    let level = Binding::f64(0.5);
    let count = Binding::i32(1);
    let address = Binding::container(Str::from(""));
    let name = Binding::container(Str::from(""));
    let fraction = 0.5_f64;

    let _ = {
        button("Save") // Button<fn(&Environment)>
            .action(handler) // -> Button<impl FnMut(&Environment)>
            .action_async(|| async {}) // [ellipsis filled]
            .style(ButtonStyle::Plain) // Automatic | Plain | Link | Borderless | Bordered | BorderedProminent
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
        // Automatic | Switch | Checkbox
        Toggle::new("Wi-Fi", &enabled).style(ToggleStyle::Switch)
    };
    let _ = {
        slider("Volume", &level).range(0.0..=1.0) // &Binding<f64>; range is RangeInclusive<f64>
    };
    let _ = {
        stepper("Quantity", &count) // &Binding<i32>
    };
    let _ = {
        // range: impl RangeBounds<i32>; step takes a signal
        stepper("Items", &count).range(0..=100).step(5)
    };
    let _ = {
        field("Email", &address) // &Binding<Str>
    };
    let _ = {
        // placeholder ≠ label
        TextField::new("Username", &name).prompt("Enter your username")
    };
    let _ = {
        progress(fraction) // impl IntoComputed<f64>
    };
    let _ = {
        progress(fraction).label("Downloading") // its label is a modifier — the one exception
    };
    let _ = {
        loading() // indeterminate progress
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" (prose): the `Button` style shorthands, which
// "must come *before* `.action(..)`", and the `LabelDisplayMode` variants.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_button_style_shorthands() {
    let _ = button("a").plain().action(|| ());
    let _ = button("a").link().action(|| ());
    let _ = button("a").borderless().action(|| ());
    let _ = button("a").bordered().action(|| ());
    let _ = button("a").bordered_prominent().action(|| ());

    let _ = LabelDisplayMode::Automatic;
    let _ = LabelDisplayMode::TitleAndIcon;
    let _ = LabelDisplayMode::TitleOnly;
    let _ = LabelDisplayMode::IconOnly;
    let _ = LabelDisplayMode::Hidden;
}

// ---------------------------------------------------------------------------
// components.md § "## Controls" — rust block 13/28
// ---------------------------------------------------------------------------
pub fn components_block_13() {
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
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 14/28
// ---------------------------------------------------------------------------
pub fn components_block_14() -> impl View {
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
// components.md § "## Menus, commands, context menus" (prose): the `Command`
// builder chain. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_command_builder_prose() {
    use waterui::component::menu::{Shortcut, ShortcutModifiers};

    let flag = Binding::bool(true);
    let value = Binding::i32(0);

    let _: Option<ShortcutModifiers> = None;
    let _ = "Copy"
        .action(|| ())
        .state(&value)
        .disabled(flag.clone())
        .selected(flag)
        .shortcut(Shortcut::new("c").command());
}

// ---------------------------------------------------------------------------
// components.md § "## Menus, commands, context menus" — rust block 15/28
// ---------------------------------------------------------------------------
pub fn components_block_15() -> impl View {
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
// components.md § "## Text" — rust block 16/28
// Listing: three text constructors.
// ---------------------------------------------------------------------------
pub fn components_block_16() {
    let value = Binding::i32(1);
    let count = Binding::i32(2);

    let _ = { text("Static string") };
    let _ = { text!("Reactive {value}") };
    let _ = { text!("{n} items", n = count.clone()) };
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 17/28
// Method-name listing; each is applied to its own `Text` receiver.
// ---------------------------------------------------------------------------
pub fn components_block_17() {
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

    // The other font slots named in the prose.
    let _ = text("t").font(font::Title);
    let _ = text("t").font(font::Headline);
    let _ = text("t").font(font::Subheadline);
    let _ = text("t").font(font::Body);
    let _ = text("t").font(font::Footnote);
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 18/28
// ---------------------------------------------------------------------------
pub fn components_block_18() {
    let runtime_str = "# Heading";

    use waterui::widget::{Code, RichText, code, rich_text};

    let _ = {
        // compile-time: expands to RichText::from_markdown(include_str!(..))
        include_markdown!("guide.md")
    };
    let _ = {
        RichText::from_markdown(runtime_str) // runtime markdown -> RichText
    };

    let _: Option<Code> = None;
    let _ = code(
        waterui::prelude::highlight::Language::Plaintext,
        "fn main() {}",
    );
    let _ = rich_text(Vec::new());
}

// ---------------------------------------------------------------------------
// components.md § "## Text" — rust block 19/28
//
// Re-transcribed after the framework fix: `FlowMarkdownConfig` is now itself a
// constant signal, so the config goes in bare — no `Computed::constant` wrapper.
// ---------------------------------------------------------------------------
pub fn components_block_19() -> impl View {
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
// components.md § "## Text" (prose): the per-element override and the
// `FlowElementKind` / `FlowAnimationPreset` variants.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_flow_overrides_prose() {
    use waterui::prelude::flow_markdown::FlowMarkdownConfig;

    let _ = FlowMarkdownConfig::default().override_animation(
        FlowElementKind::Text,
        FlowAnimationPolicy::Typewriter {
            cps: 40,
            batch_ms: 24,
            fade_in: None,
        },
    );

    let _ = FlowElementKind::Heading;
    let _ = FlowElementKind::ListItem;
    let _ = FlowElementKind::Quote;
    let _ = FlowElementKind::Link;
    let _ = FlowAnimationPreset::Minimal;
    let _ = FlowAnimationPreset::None;
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" — rust block 20/28
// Item declarations, then a listing of container forms.
// ---------------------------------------------------------------------------
pub mod components_block_20 {
    use waterui::layout::scroll::ScrollController;
    use waterui::prelude::*;
    use waterui::reactive::collection::List as ReactiveList;

    #[derive(Clone)]
    pub struct AppState {
        pub rows: ReactiveList<Record>,
    }

    use waterui::Identifiable;
    use waterui::component::lazy::Lazy;
    use waterui::component::list::{ListDelete, ListMove}; // NOT in the prelude
    use waterui::views::ForEach; // the derive is NOT in the prelude

    #[derive(Clone, Identifiable)]
    pub struct Record {
        #[id]
        id: u64,
        title: Str,
    }

    fn row(r: Record) -> Text {
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
        let _ = { Lazy::hstack(ForEach::new(records.clone(), row)) };

        let _ = {
            List::for_each(records.clone(), |r| ListItem::new(text(r.title)))
                .editing(is_editing.clone()) // impl IntoComputed<bool>
                .on_delete(|ListDelete(i), State(s): State<AppState>| {
                    let _ = s.rows.remove(i);
                })
                .on_move(|ListMove(m), State(_s): State<AppState>| {
                    let _ = (m.from(), m.to()); // [ellipsis filled]
                })
                .scroll_controller(&controller)
        };
    }
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" — rust block 21/28
// ---------------------------------------------------------------------------
pub fn components_block_21() -> impl View {
    use core::time::Duration;
    use waterui::animation::Animation;
    use waterui::reactive::collection::List as ReactiveList;

    fn row_view(r: Record) -> Text {
        text(r.title)
    }
    let rows = ReactiveList::<Record>::new();

    use waterui::layout::collection_transition;
    use waterui::layout::stack::VStack;

    let drawer = VStack::for_each(rows.clone(), row_view).spacing(4.0);
    collection_transition(drawer, Animation::ease_in_out(Duration::from_millis(250)))
}

// ---------------------------------------------------------------------------
// components.md § "## Lists and collections" — rust block 22/28
// ---------------------------------------------------------------------------
pub fn components_block_22() -> impl View {
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
// components.md § "## Lists and collections" (prose): the `ListItem` modifiers.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_list_item_prose() {
    let signal = Binding::bool(true);
    let section = ListSection::default();

    let _ = ListItem::new(text("a")).deletable(signal.clone());
    let _ = ListItem::new(text("a")).selected(signal);
    let _ = ListItem::new(text("a")).section(section);
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" — rust block 23/28
// ---------------------------------------------------------------------------
pub mod components_block_23 {
    use waterui::prelude::*;

    #[form]
    struct Settings {
        /// Doc comments become field labels. Text fields bind `Str`, not `String`.
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
// components.md § "## Forms and pickers" — rust block 24/28
// ---------------------------------------------------------------------------
pub mod components_block_24 {
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

        // The remaining style variants named in the trailing comment.
        let _ = PickerStyle::Automatic;
        let _ = PickerStyle::Radio;
        let _ = PickerStyle::Segmented;

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
// components.md § "## Forms and pickers" — rust block 25/28
// ---------------------------------------------------------------------------
pub fn components_block_25() {
    use alloc::collections::BTreeSet;
    use waterui::form::Calendar;
    use waterui::form::picker::color::ColorPicker;
    use waterui::form::picker::date::{DatePicker, DatePickerType};
    use waterui::form::picker::file::FilePicker;
    use waterui::form::picker::multi_date::MultiDatePicker;
    use waterui::media::Url;
    use waterui::reactive::binding;

    // Cargo.toml: jiff = "…"
    use jiff::civil::Date;

    let date = binding(Date::constant(2026, 3, 20));
    let (min, max) = (Date::constant(2026, 1, 1), Date::constant(2026, 12, 31));
    let (start, end) = (min, max);
    let visible_month = binding(Date::constant(2026, 3, 1));
    let marked_days: Binding<BTreeSet<Date>> = binding(BTreeSet::<Date>::new());
    let date_set: Binding<BTreeSet<Date>> = binding(BTreeSet::<Date>::new());
    let color: Binding<Color> = binding(Color::from(waterui::color::Srgb::from_hex("#4A84F6")));
    let urls: Binding<Vec<Url>> = binding(Vec::<Url>::new());

    let _ = {
        DatePicker::new("Date", &date) // T: DatePickable picks the presentation
            .ty(DatePickerType::DateHourMinuteAndSecond) // override; also HourMinuteAndSecond etc.
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
        // Binding<Vec<Url>>; constructor is `open`
        FilePicker::open("Select Files", &urls).max_count(5)
    };

    let _ = DatePickerType::HourMinuteAndSecond;
}

// ---------------------------------------------------------------------------
// components.md § "## Forms and pickers" (prose): the two seeding idioms.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn components_picker_seeding_prose() {
    use alloc::collections::BTreeSet;
    use jiff::civil::Date;
    use waterui::color::Srgb;
    use waterui::reactive::binding;

    let _: Binding<Color> = binding(Color::from(Srgb::from_hex("#4A84F6")));
    let _: Binding<BTreeSet<Date>> = binding(BTreeSet::<Date>::new());
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays" — rust block 26/28
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    unused_must_use,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn components_block_26() {
    fn restore() {}

    use core::time::Duration;
    use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};

    button("Save").action(|State(m): State<SnackbarManager>| {
        m.show(
            Snackbar::new("Item moved to trash")
                .icon(mdi::delete())
                .action("Undo", || restore())
                .position(SnackbarPosition::BottomCenter) // Top/Bottom × Center/Leading/Trailing
                .closeable()
                .duration(Duration::from_secs(5)), // Duration::ZERO = until dismissed
        );
    });
}

// ---------------------------------------------------------------------------
// components.md § "## Overlays" — rust block 27/28
// Listing: four overlay/composition helpers.
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn components_block_27() {
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
        card(content).title("Summary").subtitle("This week") // Card
    };
    let _ = {
        suspense(async { load().await }) // takes a future, not a closure
    };
    let _ = {
        accordion(header, || body()) // disclosure
    };
    let _ = {
        view.floating() // themed elevated surface (FloatingStyle-aware)
    };
}

// ---------------------------------------------------------------------------
// components.md § "## Accessibility modifiers" — rust block 28/28
// Method-fragment listing applied to fresh receivers.
// ---------------------------------------------------------------------------
pub fn components_block_28() {
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
