//! Snippets from `.claude/skills/waterui/SKILL.md`, in file order.
//!
//! Transcription conventions are documented once, in the crate README.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// SKILL.md § "## Quick start" — rust block 8/16
//
// At module scope because the skill presents these as the imports later
// snippets in the same file rely on.
// ---------------------------------------------------------------------------
use waterui::Identifiable; // the derive macro
use waterui::animation::Animation; // animation curves
use waterui::component::lazy::Lazy; // reactive stacks over a collection
use waterui::cursor::CursorStyle;
use waterui::drag_drop::DragData;
use waterui::env::with; // scope a value to a subtree
use waterui::gesture::{DragGesture, LongPressGesture, TapGesture};
use waterui::reactive::binding; // the general Binding constructor
use waterui::reactive::collection::List as ReactiveList;
use waterui::task::{sleep, spawn_local}; // async utilities
use waterui::views::ForEach; // the collection itself
use waterui::widget::condition::when; // conditionals

use waterui::media::Photo;

/// Glue: the row type SKILL.md's rule-2 and rule-3 snippets refer to.
#[derive(Clone, Identifiable)]
pub struct GlueRow {
    #[id]
    pub id: u64,
    pub title: Str,
}

/// Glue: the `row_view` free function rules 2 and 3 hand to a container.
fn glue_row_view(row: GlueRow) -> ListItem {
    ListItem::new(text(row.title))
}

// ---------------------------------------------------------------------------
// SKILL.md § "### 1. Pass the signal, never a snapshot of it" — rust block 1/16
// Listing: four independent one-line examples.
// ---------------------------------------------------------------------------
pub fn skill_block_01() {
    let fade = Binding::f32(1.0);
    let blur = Binding::f64(4.0);
    let count = Binding::i32(0);
    let url = "https://waterui.dev/logo.png";

    let view = Divider;
    let _ = {
        view.opacity(fade.clone()) // reacts
    };
    let view = Divider;
    let _ = {
        view.opacity(fade.get()) // frozen forever — a plain f32
    };
    let _ = {
        Photo::new(url).blur(blur.clone()) // reacts
    };
    let _ = {
        text!("Count: {count}") // reacts
    };
}

// ---------------------------------------------------------------------------
// SKILL.md § "### 2. `watch` is not the reactive primitive — it is the escape
// hatch" — rust block 2/16
// Listing: three independent replacements for `watch`.
// ---------------------------------------------------------------------------
pub fn skill_block_02() {
    let status = Binding::container(Str::from("idle"));
    let blur = Binding::f64(4.0);
    let url = "https://waterui.dev/logo.png";
    let rows = ReactiveList::<GlueRow>::new();
    let row_view = glue_row_view;

    let _ = {
        text!("{status}") // reactive text — not watch + format!
    };
    let _ = {
        Photo::new(url).blur(blur.clone()) // reactive value — pass the signal
    };
    let _ = {
        Lazy::for_each(rows.clone(), row_view) // dynamic set of views — a collection
    };
}

// ---------------------------------------------------------------------------
// SKILL.md § "### 3. Inject handler state with `.state()`" — rust block 3/16
// ---------------------------------------------------------------------------
pub fn skill_block_03() -> impl View {
    let count = Binding::i32(0);

    button("Increment")
        .action(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
        .state(&count)
}

// ---------------------------------------------------------------------------
// SKILL.md § "### 3. Inject handler state with `.state()`" — rust block 4/16
// ---------------------------------------------------------------------------
pub fn skill_block_04() -> impl View {
    let query = Binding::container(Str::from(""));
    let history = Binding::container(Vec::<Str>::new());

    button("Search")
        .action(
            |State(q): State<Binding<Str>>, State(hist): State<Binding<Vec<Str>>>| {
                hist.get_mut().push(q.get());
            },
        )
        .state(&query) // -> first parameter
        .state(&history) // -> second parameter
}

// ---------------------------------------------------------------------------
// SKILL.md § "### 3. Inject handler state with `.state()`" — rust block 5/16
// ---------------------------------------------------------------------------
pub mod skill_block_05 {
    use super::{GlueRow as Row, ReactiveList, glue_row_view as row_view};
    use waterui::prelude::*;

    #[derive(Clone)]
    pub struct Editor {
        rows: ReactiveList<Row>,
        editing: Binding<bool>,
    }

    fn toggle_editing(State(state): State<Editor>) {
        state.editing.set(!state.editing.get());
    }

    fn content(state: Editor) -> impl View {
        vstack((
            button("Edit").action(toggle_editing),
            List::for_each(state.rows.clone(), row_view),
        ))
        .state(&state) // injected once, visible to every handler below
    }

    /// Glue: constructs an `Editor` and hands it to the snippet's `content`.
    pub fn use_it() -> impl View {
        content(Editor {
            rows: ReactiveList::new(),
            editing: Binding::bool(false),
        })
    }
}

// ---------------------------------------------------------------------------
// SKILL.md § "### 4. A changing set of views is a collection, not a `watch`"
// — rust block 6/16
// Prefix is a real statement sequence; the last two lines are a listing.
// ---------------------------------------------------------------------------
pub fn skill_block_06() {
    use waterui::Identifiable;
    use waterui::component::lazy::Lazy;
    use waterui::reactive::collection::List as ReactiveList; // the derive is NOT in the prelude

    #[derive(Clone, Identifiable)]
    struct Row {
        #[id]
        id: u64,
        title: Str,
    }

    let seed_rows = vec![Row {
        id: 1,
        title: Str::from("Hello"),
    }];

    let rows = ReactiveList::from(seed_rows); // bulk-seed; .push/.insert/.remove diff by id

    let _ = {
        Lazy::for_each(rows.clone(), |row| text(row.title)) // reactive sequence in a stack
    };
    let _ = {
        // platform list: lazy, editable
        List::for_each(rows.clone(), |row| ListItem::new(text(row.title))) // [ellipsis filled]
    };
}

// ---------------------------------------------------------------------------
// SKILL.md § "## Quick start" — rust block 7/16
// ---------------------------------------------------------------------------
pub mod skill_block_07 {
    use waterui::app::App;
    use waterui::prelude::*;

    fn counter() -> impl View {
        let count = Binding::i32(0);

        vstack((
            text!("Count: {count}").headline(),
            button("+1")
                .action(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
                .state(&count),
        ))
        .spacing(8.0)
        .padding()
    }

    pub fn app(env: Environment) -> App {
        App::new(counter, env)
    }
}

// ---------------------------------------------------------------------------
// SKILL.md § "### Views" — rust block 9/16
// ---------------------------------------------------------------------------
pub fn skill_block_09() -> impl View {
    fn card(title: &'static str) -> impl View {
        vstack((text(title).title(), Divider))
    }

    vstack((card("Hello"), card("World"), "a bare literal is a view"))
}

// ---------------------------------------------------------------------------
// SKILL.md § "### Views" — rust block 10/16
// ---------------------------------------------------------------------------
pub mod skill_block_10 {
    use waterui::prelude::*;

    pub struct ColorSwatch {
        pub color: Binding<Color>,
    }

    impl View for ColorSwatch {
        fn body(self, _env: &Environment) -> impl View {
            signal_color(self.color).size(64.0, 32.0) // a Color is itself a view
        }
    }
}

// ---------------------------------------------------------------------------
// SKILL.md § "### State" — rust block 11/16
// ---------------------------------------------------------------------------
pub fn skill_block_11() {
    use waterui::media::media_picker::Selected;

    #[derive(Clone)]
    enum Pane {
        Inbox,
    }

    #[form]
    struct Settings {
        display_name: Str,
        volume: f64,
        notifications: bool,
    }

    let count = Binding::i32(0); // bool f32 f64 i32 i64 isize u32 u64 usize
    let flag = Binding::bool(false);
    let name = Binding::container(String::new()); // any Clone type
    let sel: Binding<Option<Selected>> = Binding::default(); // empty optional

    let pane: Binding<Pane> = binding(Pane::Inbox); // general form, needs an inferable type
    let settings = Settings::binding(); // #[form] types: inference-free

    let _ = (count, flag, name, sel, pane, settings);
}

// ---------------------------------------------------------------------------
// SKILL.md § "### Text" — rust block 12/16
// Listing: four independent one-line examples.
// ---------------------------------------------------------------------------
pub fn skill_block_12() {
    struct Mail {
        unread: Binding<i32>,
    }
    impl Mail {
        fn count(&self) -> Binding<i32> {
            self.unread.clone()
        }
    }

    let count = Binding::i32(0);
    let blur = Binding::f64(1.25);
    let mail = Mail {
        unread: Binding::i32(3),
    };

    let _ = {
        text("Settings").title() // title/headline/sub_headline/body/caption/footnote
    };
    let _ = {
        text!("Count: {count}") // updates automatically
    };
    let _ = {
        text!("{unread} unread", unread = mail.count()) // aliasing an expression into a slot
    };
    let _ = {
        text!("Blur: {blur:.1}") // format specs work
    };
}

// ---------------------------------------------------------------------------
// SKILL.md § "### Layout" — rust block 13/16
// Listing: six one-line examples plus one `let`.
// ---------------------------------------------------------------------------
pub fn skill_block_13() {
    struct Item {
        label: &'static str,
    }

    let content = text("content");
    let background = text("background");
    let items = [Item { label: "One" }, Item { label: "Two" }];

    let (a, b, c) = (text("a"), text("b"), text("c"));
    let _ = { hstack((a, b, c)).spacing(8.0) };
    let (a, b) = (text("a"), text("b"));
    let _ = {
        vstack((a, b))
            .alignment(HorizontalAlignment::Leading)
            .padding()
    };
    let _ = { zstack((background, content)) };
    let content = text("content");
    let _ = { scroll(content) };
    let _ = {
        spacer() // flexible gap
    };
    let _ = {
        spacer().height(16.0) // fixed gap
    };

    let buttons: HStack<_> = items.iter().map(|i| button(i.label)).collect();

    let _ = buttons;
}

// ---------------------------------------------------------------------------
// SKILL.md § "### Conditionals" — rust block 14/16
// Listing: independent conditional forms.
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn skill_block_14() {
    struct RowFlags {
        flagged: bool,
    }
    fn flag_icon() -> impl View {
        text("flag")
    }
    fn dashboard() -> impl View {
        text("dashboard")
    }
    fn login() -> impl View {
        text("login")
    }
    fn loading() -> impl View {
        text("loading")
    }
    fn ready() -> impl View {
        text("ready")
    }
    fn error() -> impl View {
        text("error")
    }
    fn new_marker() -> impl View {
        text("new")
    }

    let row = RowFlags { flagged: true };
    let logged_in = Binding::bool(false);
    let state = Binding::i32(0);
    let is_new = Binding::bool(true);

    use waterui::widget::condition::when; // not in the prelude

    // A plain bool: Option<impl View> is itself a View.
    let _ = { row.flagged.then(|| flag_icon()) };

    // A reactive bool: use when(...), or keep the view and drive .visible(..).
    let _ = { when(logged_in.clone(), || dashboard()).otherwise(|| login()) };
    let _ = {
        when(state.equal_to(0), || loading())
            .or(state.equal_to(1), || ready())
            .otherwise(|| error())
    };

    let _ = {
        new_marker().visible(is_new.clone()) // keep the view, drive its visibility
    };
}

// ---------------------------------------------------------------------------
// SKILL.md § "### Modifiers" — rust block 15/16
//
// A signature listing: every line is a `/`-separated set of alternatives, each
// a bare method fragment. Each fragment is applied to a fresh receiver so the
// method itself is proven to exist. `(..)` markers are ellipsis placeholders.
//
// The receiver is `Divider` rather than `text(..)`: the skill itself warns
// (§ "### Text") that `Text::size(..)` is the font size and shadows the
// two-argument frame `.size(w, h)`.
// ---------------------------------------------------------------------------
pub fn skill_block_15() {
    use waterui::accessibility::AccessibilityRole;
    use waterui::gesture::Gesture;
    use waterui::layout::EdgeSet;

    let (w, h) = (100.0_f32, 40.0_f32);
    let (x, y) = (1.5_f32, 2.5_f32);
    let degrees = 30.0_f32;
    let width = 2.0_f32;
    let shape = waterui::shape::Circle;

    let view = Divider;
    let _ = { view.padding() };
    let view = Divider;
    let _ = { view.padding_with(16.0) };
    let view = Divider;
    let _ = { view.padding_with(EdgeInsets::all(16.0)) };

    let color = Color::srgb_hex("#3B82F6");
    let view = Divider;
    let _ = { view.background(color) };
    let color = Color::srgb_hex("#3B82F6");
    let view = Divider;
    let _ = { view.foreground(color) };
    let overlaid = text("overlay");
    let view = Divider;
    let _ = { view.overlay(overlaid) };

    let view = Divider;
    let _ = { view.size(w, h) };
    let view = Divider;
    let _ = { view.width(w) };
    let view = Divider;
    let _ = { view.height(h) };
    let view = Divider;
    let _ = { view.min_width(w) };
    let view = Divider;
    let _ = { view.max_width(w) };
    let view = Divider;
    let _ = {
        view.min_size(w, h) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.max_size(w, h) // [ellipsis filled]
    };

    let view = Divider;
    let _ = { view.scale(x, y) };
    let view = Divider;
    let _ = { view.rotation(degrees) };
    let view = Divider;
    let _ = {
        view.offset(x, y) // two arguments, not one
    };

    let color = Color::srgb_hex("#3B82F6");
    let view = Divider;
    let _ = { view.border(color, width) };
    let shadow = Shadow::default();
    let view = Divider;
    let _ = { view.shadow(shadow) };
    let view = Divider;
    let _ = { view.clip(shape) };

    // `.opacity` / `.blur` take numeric signals; `.visible` / `.disabled` take
    // bool signals. The listing spells the free variable `signal` for all of
    // them, so each line gets the signal its own modifier requires.
    let signal = Binding::f32(1.0);
    let view = Divider;
    let _ = { view.opacity(signal) };
    let signal = Binding::bool(true);
    let view = Divider;
    let _ = { view.visible(signal) };
    let signal = Binding::bool(true);
    let view = Divider;
    let _ = { view.disabled(signal) };

    let signal = Binding::f32(2.0);
    let view = Divider;
    let _ = { view.blur(signal) };
    let view = Divider;
    let _ = {
        view.brightness(1.2) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.contrast(1.1) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.saturation(0.8) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.grayscale(0.5) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.hue_rotation(30.0) // [ellipsis filled]
    };

    let view = Divider;
    let _ = {
        view.a11y_label("Delete") // [ellipsis filled]
    };
    let view = Divider;
    let _ = { view.a11y_id("settings.wifi") };
    let view = Divider;
    let _ = {
        view.a11y_role(AccessibilityRole::Button) // [ellipsis filled]
    };

    let signal = Binding::bool(true);
    let view = Divider;
    let _ = {
        view.on_appear(|| ()) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.on_change(&signal, |_| ()) // [ellipsis filled]
    };
    let view = Divider;
    let _ = {
        view.on_tap(|| ()) // [ellipsis filled]
    };
    let g = Gesture::from(TapGesture::new());
    let handler = || ();
    let view = Divider;
    let _ = { view.gesture(g, handler) };
    let items = ("Copy".action(|| ()),);
    let view = Divider;
    let _ = { view.context_menu(items) };

    let style = CursorStyle::PointingHand;
    let view = Divider;
    let _ = { view.cursor(style) };
    let view = Divider;
    let _ = { view.ignore_safe_area(EdgeSet::ALL) };
    let view = Divider;
    let _ = { view.floating() };
}

// ---------------------------------------------------------------------------
// SKILL.md § "### The Environment" — rust block 16/16
// Listing: seeding alternatives, then three independent reading examples.
// ---------------------------------------------------------------------------
pub mod skill_block_16 {
    use waterui::locale::{Locale, locales};
    use waterui::prelude::*;

    use waterui::env::{use_env, with};

    #[derive(Clone)]
    pub struct ApiClient {
        base_url: Str,
    }
    waterui::impl_extractor!(ApiClient); // makes it a handler/`use_env` parameter

    fn send(_client: &ApiClient) {}

    fn glue_client() -> ApiClient {
        ApiClient {
            base_url: Str::from("https://waterui.dev"),
        }
    }

    fn glue_subtree_view() -> impl View {
        text("subtree")
    }

    pub fn seeding() {
        // Seeding, usually in `app(env)`:
        {
            let mut env = Environment::new();
            let client = glue_client();
            env.insert(client); // in place
        }
        {
            let mut env = Environment::new();
            let client = glue_client();
            env.with(client); // in place, chains
        }
        {
            let env = Environment::new();
            let client = glue_client();
            let scoped = env.extending(client); // non-mutating overlay
            let _ = scoped;
        }
        {
            let subtree_view = glue_subtree_view();
            let _ = {
                // free fn: scope a value to one subtree
                with(subtree_view, LayoutDirection::RightToLeft)
            };
        }
    }

    pub fn reading_from_a_view() -> impl View {
        // Reading, from a view:
        use_env(|client: ApiClient| text!("API: {url}", url = Binding::container(client.base_url)))
    }

    pub fn reading_from_a_handler() -> impl View {
        // Reading, from a handler — same extractors, no ceremony:
        button("Send").action(|client: ApiClient| send(&client))
    }

    pub fn reading_optional() {
        let env = Environment::new();

        // Reading, where absence is legitimate — the non-panicking form:
        let locale = env
            .get::<Locale>()
            .cloned()
            .unwrap_or_else(|| locales::EN.clone());

        let _ = locale;
    }
}

/// Glue: keeps the module-scope imports from block 8 that no other snippet in
/// this file consumes proven to resolve.
pub fn skill_block_08_imports_are_live() {
    let _ = DragGesture::new(5.0);
    let _ = LongPressGesture::new(500);
    let _ = TapGesture::new();
    let _ = CursorStyle::Arrow;
    let _ = DragData::text("payload");
    let _ = Animation::default();
    let _ = ForEach::new(ReactiveList::<GlueRow>::new(), glue_row_view);
    let _ = binding::<i32>(0);
    let _ = when(Binding::bool(true), || text("a")).otherwise(|| text("b"));
    let _ = with(text("x"), 1_i32);
    spawn_local(async { sleep(core::time::Duration::from_millis(1)).await }).detach();
}
