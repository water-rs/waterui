//! Snippets from `.claude/skills/waterui/references/reactivity.md`, in file
//! order. Transcription conventions are documented in the crate README.

use waterui::Identifiable;
use waterui::prelude::*;

/// Glue: the `Row` type reactivity.md's collection snippets refer to.
#[derive(Clone, Identifiable)]
pub struct Row {
    #[id]
    pub id: u64,
    pub title: Str,
}

/// Glue: the `row_view` free function reactivity.md hands to `Lazy`.
fn glue_row_view(row: Row) -> Text {
    text(row.title)
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Creating state" — rust block 1/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_01() {
    use waterui::media::media_picker::Selected;
    use waterui::window::WindowState;

    #[derive(Clone)]
    enum Pane {
        Inbox,
    }

    use waterui::reactive::binding;

    let count = Binding::i32(0); // typed constructors, primitives:
    let ratio = Binding::f64(1.5); // bool f32 f64 i32 i64 isize u32 u64 usize
    let flag = Binding::bool(false);
    let name = Binding::container(Str::from("Ada")); // any Clone type
    let items = Binding::container(Vec::<Row>::new());
    let status = Binding::container("Waiting…"); // Binding<&'static str> — fine for status text

    let sel: Binding<Option<Selected>> = Binding::default(); // empty optional selection
    let pane: Binding<Pane> = binding(Pane::Inbox); // general form
    let ws = binding::<WindowState>(WindowState::default()); // …or pin T with a turbofish

    let _ = (count, ratio, flag, name, items, status, sel, pane, ws);
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Creating state" (Writing) — rust block 2/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_02() {
    let count = Binding::container(Vec::<i32>::new());
    let flag = Binding::bool(false);
    let x = 7_i32;

    // The listing mixes receivers: `count.set(5)` / `*count.get_mut() += 1`
    // need a scalar, `count.with_mut(|v| v.push(x))` needs a container. Each
    // line therefore gets the receiver its own call requires.
    {
        let count = Binding::i32(0);
        count.set(5);
    }
    {
        let count = Binding::i32(0);
        *count.get_mut() += 1; // guard: writes back on drop
    }
    count.with_mut(|v| v.push(x)); // in-place mutation of a container
    flag.toggle(); // bool convenience
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Transforming signals" — rust block 3/20
//
// A method-name listing grouped by receiver kind. Each fragment is applied to a
// receiver of the kind its own comment header names, so every method in the
// list is individually proven to exist.
// ---------------------------------------------------------------------------
pub fn reactivity_block_03() {
    use core::time::Duration;
    use waterui::animation::Animation;

    // core
    let n = Binding::i32(2);
    let _ = { n.map(|v| v * 2) };
    let _ = { n.computed() };
    let _ = { n.cached() };
    let _ = { n.distinct() };
    let _ = {
        n.map_into::<i64>() // [ellipsis filled]
    };
    let _ = {
        n.inspect(|_| ()) // [ellipsis filled]
    };
    let metadata = Animation::linear(Duration::from_millis(100));
    let _ = { n.with(metadata) };

    // bool
    let b = Binding::bool(true);
    let other = Binding::bool(false);
    let _ = { b.not() };
    let _ = { b.and(&other) };
    let _ = { b.or(&other) };
    let (if_true, if_false) = (1.0_f32, 0.3_f32);
    let _ = { b.select(if_true, if_false) };
    let v = 5_i32;
    let _ = { b.then_some(v) };

    // comparison -> Signal<bool>
    let _ = { n.equal_to(5) };
    let _ = { n.gt(0) };
    let _ = { n.lt(9) };
    let _ = { n.ge(1) };
    let _ = { n.le(8) };
    // `.is_ascii()` in the skill's own closure body pins the receiver to a
    // signal of a character-like value.
    let ch = Binding::container('a');
    let _ = { ch.condition(|v| v.is_ascii()) };

    // numeric
    let _ = { n.negate() };
    let _ = { n.abs() };
    let _ = { n.sign() };
    let _ = { n.is_positive() };
    let _ = { n.is_negative() };
    let _ = { n.is_zero() };

    // Option<T>
    let opt = Binding::container(Some(1_i32));
    let d = 0_i32;
    let _ = { opt.is_some() };
    let _ = { opt.is_none() };
    let _ = { opt.unwrap_or(d) };
    let _ = { opt.unwrap_or_default() };
    let _ = {
        opt.map_some(|v| v + 1) // [ellipsis filled]
    };
    let _ = {
        opt.and_then_some(Some) // [ellipsis filled]
    };
    let nested = Binding::container(Some(Some(1_i32)));
    let _ = { nested.flatten() };
    let v = 1_i32;
    let _ = { opt.some_equal_to(v) };

    // Result<T, E>
    let res = Binding::container(Ok::<i32, Str>(1));
    let _ = { res.is_ok() };
    let _ = { res.is_err() };
    let _ = { res.ok() };
    let _ = { res.err() };
    let _ = {
        res.map_ok(|v| v + 1) // [ellipsis filled]
    };
    let _ = {
        res.map_err(|e| e) // [ellipsis filled]
    };

    // strings — note the str_ prefix; plain .is_empty()/.contains() are NOT signal methods
    let s = Binding::container(Str::from("query"));
    let _ = { s.str_is_empty() };
    let _ = { s.str_len() };
    let _ = { s.str_contains("query") };

    // time
    let _ = { n.debounce(Duration::from_millis(300)) };
    let _ = { n.throttle(Duration::from_millis(16)) };
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Transforming signals" (prose, not a fenced block): the
// binding-specific helpers `.range(0..=10)`, `.clamp(..)`, `.filter(..)`,
// `.bidirectional_select(a, b)`, `.unwrap_or(d)`, `.reverse()`, and the
// associated function `Binding::mapping(&source, getter, setter)` whose setter
// receives the source binding. Not counted as one of the 20 rust blocks.
// ---------------------------------------------------------------------------
pub fn reactivity_binding_helpers_prose() {
    let n = Binding::i32(5);
    let _ = n.range(0..=10);
    let _ = n.clamp(0..=10);
    let _ = n.filter(|v| *v > 0);

    let _ = Binding::mapping(&n, i64::from, |slot: &Binding<i32>, v: i64| {
        slot.set(v as i32);
    });

    let b = Binding::bool(true);
    let _ = b.bidirectional_select(1_i32, 0_i32);
    let _ = b.reverse();

    let opt = Binding::container(Some(1_i32));
    let _ = opt.unwrap_or(0);
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Transforming signals" (prose): `.select(a, b)` needs both
// arms as one concrete type — `let on: Color = Accent.into();`.
// ---------------------------------------------------------------------------
pub fn reactivity_select_token_conversion() {
    use waterui::prelude::theme_color::Accent;

    let on: Color = Accent.into();
    let off = Color::transparent();
    let flag = Binding::bool(true);
    let _ = flag.select(on, off);
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Combining signals" — rust block 4/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_04() {
    let price = Binding::f64(2.0);
    let quantity = Binding::f64(3.0);
    let loaded = Binding::bool(true);
    let authorized = Binding::bool(true);
    let count = Binding::i32(3);
    let unit = Binding::container(Str::from("items"));

    let total = price.zip(&quantity).map(|(p, q)| p * q);
    let ready = loaded.and(&authorized);
    let label = count.zip(&unit).map(|(n, u)| format!("{n} {u}"));

    let _ = (total, ready, label);
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Combining signals" — rust block 5/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_05() {
    fn build_config(_a: i32, _b: i32, _c: i32, _d: i32) -> i32 {
        0
    }
    let a = Binding::i32(1);
    let b = Binding::i32(2);
    let c = Binding::i32(3);
    let d = Binding::i32(4);

    let config = a
        .zip(&b)
        .zip(&c)
        .zip(&d)
        .map(|(((a, b), c), d)| build_config(a, b, c, d))
        .computed();

    let _ = config;
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Constants as signals" — rust block 6/20
// ---------------------------------------------------------------------------
pub mod reactivity_block_06 {
    use waterui::prelude::*;

    /// Glue: the custom enum the snippet declares as a constant signal.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ChartMode {
        Bar,
        Line,
    }

    fn decorated_dates() -> Vec<u32> {
        vec![1, 2, 3]
    }

    use waterui::reactive::impl_constant;

    impl_constant!(ChartMode); // your own Clone type as a constant signal

    pub fn body() {
        let dates = Computed::constant(decorated_dates()); // a nameable, shareable constant Computed<T>
        let _ = dates;
        let _ = ChartMode::Bar;
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Projecting struct fields" — rust block 7/20
// ---------------------------------------------------------------------------
pub mod reactivity_block_07 {
    use waterui::prelude::*;

    #[form]
    struct Settings {
        name: Str,
        volume: f64,
        dark: bool,
    } // text fields bind Str, not String

    pub fn body() -> impl View {
        let settings = Settings::binding();

        vstack((
            field("Name", &settings.project().name),
            slider("Volume", &settings.project().volume),
            toggle("Dark mode", &settings.project().dark),
            text!("Volume is {volume}", volume = settings.project().volume),
        ))
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Feeding signals to views" — rust block 8/20
// Listing: six independent one-line examples.
// ---------------------------------------------------------------------------
pub fn reactivity_block_08() {
    use waterui::media::Photo;

    let fade = Binding::f32(1.0);
    let has_items = Binding::bool(true);
    let is_loading = Binding::bool(false);
    let zoom = Binding::f32(1.0);
    let radius = Binding::f64(2.0);
    let sat = Binding::f64(1.0);
    let status = Binding::container(Str::from("idle"));
    let url = "https://waterui.dev/logo.png";

    let view = Divider;
    let _ = { view.opacity(fade.clone()) };
    let view = Divider;
    let _ = { view.visible(has_items.clone()) };
    let view = Divider;
    let _ = { view.disabled(is_loading.clone()) };
    let view = Divider;
    let _ = { view.scale(zoom.clone(), zoom.clone()) };
    let _ = { Photo::new(url).blur(radius.clone()).saturation(sat.clone()) };
    let _ = { text!("{status}") };
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Reading state in handlers" — rust block 9/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_09() -> impl View {
    use reactivity_settings::Settings;

    let form = Settings::binding();

    button("Reset")
        .action(|State(form): State<Binding<Settings>>| form.set(Settings::default()))
        .state(&form)
}

/// Glue: `Settings` for block 9, which reuses the type block 7 declares.
pub mod reactivity_settings {
    use waterui::prelude::*;

    #[form]
    pub struct Settings {
        pub name: Str,
        pub volume: f64,
        pub dark: bool,
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Reading state in handlers" — rust block 10/20
// ---------------------------------------------------------------------------
pub mod reactivity_block_10 {
    use super::Row;
    use waterui::component::list::ListDelete;
    use waterui::prelude::*;
    use waterui::reactive::collection::List as ReactiveList;

    #[derive(Clone)]
    pub struct Editor {
        pub rows: ReactiveList<Row>,
    }

    fn delete_row(ListDelete(index): ListDelete, State(state): State<Editor>) {
        let _ = state.rows.remove(index);
    }

    pub fn use_it() -> impl View {
        button("Delete").action(delete_row)
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Reading state in handlers" — rust block 11/20
// ---------------------------------------------------------------------------
pub mod reactivity_block_11 {
    use waterui::prelude::*;

    use waterui::Handler;

    fn drawer_item<F, Args>(title: &'static str, action: F) -> impl View
    where
        F: Handler<Args, ()> + 'static,
        Args: 'static,
    {
        text(title).padding().on_tap(action)
    }

    pub fn use_it() -> impl View {
        drawer_item("Inbox", || ())
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Animation" — rust block 12/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_12() {
    use core::time::Duration;
    use waterui::animation::Animation; // not in the prelude

    let scale = Binding::f32(1.0);
    let animated = scale.with(Animation::spring(300.0, 15.0));

    let view = Divider;
    let _ = { view.scale(animated.clone(), animated.clone()) };

    let _ = Duration::from_millis(1);
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Animation" (prose curve list, not a fenced block):
// `Animation::linear(d)`, `ease_in(d)`, `ease_out(d)`, `ease_in_out(d)`,
// `spring(stiffness, damping)`, `bezier(d, x1, y1, x2, y2)`,
// `Animation::default()`, `.with(animation)` as the one attachment spelling,
// and `.animated()` as shorthand for `.with(Animation::Default)` — one
// `AnimationExt`, reachable from the prelude and from `waterui::animation`.
// Not counted as one of the 20 rust blocks.
// ---------------------------------------------------------------------------
pub fn reactivity_animation_prose() {
    use core::time::Duration;
    use waterui::animation::Animation;

    let d = Duration::from_millis(250);
    let _ = Animation::linear(d);
    let _ = Animation::ease_in(d);
    let _ = Animation::ease_out(d);
    let _ = Animation::ease_in_out(d);
    let _ = Animation::spring(300.0, 15.0);
    let _ = Animation::bezier(d, 0.2, 0.0, 0.2, 1.0);
    let _ = Animation::default();

    let scale = Binding::f32(1.0);
    let _ = scale.with(Animation::spring(300.0, 15.0)); // the one attachment spelling
    let _ = scale.animated(); // shorthand for `.with(Animation::Default)`
}

/// The same `AnimationExt` reached by the other documented path — "one trait,
/// one meaning, whichever path you import it by".
pub fn reactivity_animation_ext_single_trait() {
    use waterui::animation::AnimationExt as _;

    let scale = Binding::f32(1.0);
    let _ = scale.animated();
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Animation" — rust block 13/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_13() {
    use waterui::animation::Animation;

    let hovered = Binding::bool(false);
    let bounce = Binding::f32(1.0);

    let hover_scale = hovered
        .select(1.05_f32, 1.0)
        .with(Animation::spring(400.0, 15.0));
    let drop_bounce = bounce.with(Animation::spring(500.0, 10.0));
    let combined = hover_scale.zip(&drop_bounce).map(|(a, b)| a * b); // still animated

    let _ = combined;
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Reactive collections" — rust block 14/20
// Prefix is a statement sequence; the last two lines are a listing.
// ---------------------------------------------------------------------------
pub fn reactivity_block_14() {
    use waterui::views::ForEach;

    let row_view = glue_row_view;
    let seed_vec = vec![Row {
        id: 1,
        title: Str::from("First"),
    }];
    let new_vec = vec![Row {
        id: 2,
        title: Str::from("Second"),
    }];

    use waterui::component::lazy::Lazy;
    use waterui::reactive::collection::List as ReactiveList;

    let rows = ReactiveList::from(seed_vec); // bulk-seed in one move — not a push loop
    rows.push(Row {
        id: 9,
        title: "Last".into(),
    });
    rows.insert(
        0,
        Row {
            id: 0,
            title: "First".into(),
        },
    ); // positional splice, id-diffed
    let _ = rows.remove(0); // #[must_use] — bind the removed value
    let snapshot = rows.snapshot(); // Vec<Row>, for read-only work
    let _ = rows.replace(new_vec); // wholesale swap, still diffed by id

    let _ = {
        Lazy::for_each(rows.clone(), row_view) // == Lazy::vstack(ForEach::new(..))
    };
    let _ = { Lazy::hstack(ForEach::new(rows.clone(), row_view)) };

    let _ = snapshot;
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Reactive collections" (prose): `VStack::for_each` /
// `HStack::for_each`. Not counted as one of the 20 rust blocks.
// ---------------------------------------------------------------------------
pub fn reactivity_stack_for_each_prose() {
    use waterui::layout::stack::{HStack, VStack};
    use waterui::reactive::collection::List as ReactiveList;

    let rows = ReactiveList::<Row>::new();
    let _ = VStack::for_each(rows.clone(), glue_row_view);
    let _ = HStack::for_each(rows, glue_row_view);
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Reactive collections" — rust block 15/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_15() {
    #[derive(Clone, Identifiable)]
    struct Message {
        #[id]
        id: u64,
        subject: Str,
    }
    fn message_row(m: Message) -> ListItem {
        ListItem::new(text(m.subject))
    }
    let messages = Binding::container(Vec::<Message>::new());
    let query = Binding::container(Str::from(""));

    use waterui::reactive::collection::SignalCollection;

    let visible = SignalCollection::new(messages.zip(&query).map(|(all, q)| {
        all.into_iter()
            .filter(|m| m.subject.contains(q.as_str()))
            .collect::<Vec<_>>()
    }));
    let _ = { List::for_each(visible, message_row) };
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Conditionals" — rust block 16/20
// Listing: independent conditional forms.
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn reactivity_block_16() {
    struct RowFlags {
        flagged: bool,
    }
    fn new_marker() -> impl View {
        text("new")
    }
    fn dashboard() -> impl View {
        text("dashboard")
    }
    fn login_form() -> impl View {
        text("login")
    }
    fn loading() -> impl View {
        text("loading")
    }
    fn ready() -> impl View {
        text("ready")
    }
    fn failed() -> impl View {
        text("failed")
    }
    fn unknown() -> impl View {
        text("unknown")
    }
    fn content() -> impl View {
        text("content")
    }

    let row = RowFlags { flagged: true };
    let is_new = Binding::bool(true);
    let logged_in = Binding::bool(true);
    let state = Binding::i32(0);
    let is_loading = Binding::bool(false);

    use waterui::widget::condition::when;

    // A plain bool: Option<impl View> is itself a View.
    let _ = { row.flagged.then(|| new_marker()) };

    // A reactive bool: a *signal* of Option<View> is NOT a view — use when(..) or .visible(..).
    let _ = { new_marker().visible(is_new.clone()) };

    // If / else.
    let _ = { when(logged_in.clone(), || dashboard()).otherwise(|| login_form()) };

    // If / else-if / else.
    let _ = {
        when(state.equal_to(0), || loading())
            .or(state.equal_to(1), || ready())
            .or(state.equal_to(2), || failed())
            .otherwise(|| unknown())
    };

    // Negation works through the Not impl on Binding.
    let _ = { when(!is_loading.clone(), || content()) };
}

// ---------------------------------------------------------------------------
// reactivity.md § "## `Dynamic` and `watch`" — rust block 17/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_17() -> impl View {
    use crate::ref_reactivity::ref_reactivity_charts::{ChartMode, bar_chart, line_chart};

    let mode = Binding::container(ChartMode::Bar);

    Dynamic::watch(mode.clone(), |mode| match mode {
        ChartMode::Bar => AnyView::new(bar_chart()),
        ChartMode::Line => AnyView::new(line_chart()),
    })
}

/// Glue: the chart-mode enum and the two view builders block 17 switches over.
pub mod ref_reactivity_charts {
    use waterui::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ChartMode {
        Bar,
        Line,
    }

    pub fn bar_chart() -> impl View {
        text("bar")
    }
    pub fn line_chart() -> impl View {
        text("line")
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## `Dynamic` and `watch`" — rust block 18/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_18() -> impl View {
    use waterui::media::{Photo, Url};

    let url = Binding::container(Str::from("https://waterui.dev/logo.png"));
    let blur = Binding::f64(0.0);

    let (handler, slot) = Dynamic::new();

    button("Load")
        .action(
            |State(url): State<Binding<Str>>,
             State(blur): State<Binding<f64>>,
             State(h): State<DynamicHandler>| {
                let Ok(parsed) = url.get().as_str().parse::<Url>() else {
                    return;
                };
                h.set(Photo::new(parsed).blur(blur.clone()));
            },
        )
        .state(&url)
        .state(&blur)
        .state(&handler);

    vstack((slot, ())) // [ellipsis filled]
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Async, tasks, and lifecycle" — rust block 19/20
// A statement, then four independent lifecycle one-liners.
// ---------------------------------------------------------------------------
#[expect(
    unused_must_use,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn reactivity_block_19() {
    async fn fetch() -> Str {
        Str::from("done")
    }
    async fn warm_cache() {}

    let result = Binding::container(Str::from(""));
    let query = Binding::container(Str::from(""));

    button("Fetch")
        .action_async(|State(out): State<Binding<Str>>| async move {
            out.set(fetch().await);
        })
        .state(&result);

    // These four already end in `;`, so each keeps its own statement in its own
    // scope rather than being wrapped in `let _ = { … }`.
    {
        let view = Divider;
        view.task(async { warm_cache().await }); // runs while the view is alive; dropped with it
    }
    {
        let view = Divider;
        view.on_appear(|| waterui::log::debug!("shown"));
    }
    {
        let view = Divider;
        view.on_disappear(|| ());
    }
    {
        let view = Divider;
        view.on_change(&query, |new_value| waterui::log::debug!(?new_value));
    }
}

// ---------------------------------------------------------------------------
// reactivity.md § "## Async, tasks, and lifecycle" — rust block 20/20
// ---------------------------------------------------------------------------
pub fn reactivity_block_20() {
    use core::time::Duration;

    let bounce = Binding::f32(0.0);

    use waterui::task::{sleep, spawn_local};

    spawn_local(async move {
        sleep(Duration::from_millis(200)).await; // the async sleep — never std::thread::sleep
        bounce.set(1.0);
    })
    .detach();
}
