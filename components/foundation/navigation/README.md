# WaterUI Navigation

WaterUI navigation keeps route state in Rust and projects it into native platform containers. Apple uses `UINavigationController`, `UITabBarController` or sidebar presentation, and `UISplitViewController`; Android uses fragments, Material navigation chrome, adaptive tab bars or rails, `SlidingPaneLayout`, and predictive back. Hydrolysis provides the retained GPU realization.

## Typed navigation

`NavigationPath<R>` is itself a shared reactive value. Clone it to share the same path; do not wrap it in `Binding`.

```rust
use waterui::prelude::*;

#[derive(Clone, PartialEq, Eq)]
enum Route {
    Article(u64),
    Settings,
}

fn app() -> impl View {
    let path = NavigationPath::<Route>::new();

    NavigationStack::with_path(
        path,
        NavigationView::new(
            "Library",
            NavigationLink::value("Open settings", Route::Settings),
        ),
    )
    .destination(|route| match route {
        Route::Article(id) => NavigationView::new("Article", text!("Article {id}")),
        Route::Settings => NavigationView::new("Settings", text("Preferences")),
    })
}
```

Path mutations are atomic at the native boundary. `replace` computes one retained prefix and sends one transaction rather than replaying a sequence of pushes and pops.

```rust
path.push(Route::Article(42));
path.pop();
path.pop_n(2);
path.replace([Route::Settings, Route::Article(7)]);
path.clear();
```

Inside a typed stack, extract `Navigator<Route>` for ergonomic programmatic navigation:

```rust
button("Home").action(|navigator: Navigator<Route>| navigator.pop_to_root())
```

For local navigation that does not need route state, use the implicit stack and destination-building links:

```rust
NavigationStack::new(NavigationView::new(
    "Home",
    NavigationLink::new("Details", || {
        NavigationView::new("Details", text("Native destination"))
    }),
))
```

## Heterogeneous paths

Use the same public wrapper and opt into heterogeneous storage explicitly. Each `.destination::<T>` registration supplies runtime `TypeId` dispatch automatically.

```rust
#[derive(Clone, PartialEq, Eq)]
struct Project(u64);

#[derive(Clone, PartialEq, Eq)]
struct Preferences;

let path = NavigationPath::heterogeneous();
path.push(Project(9));
path.push(Preferences);

let stack = NavigationStack::with_path(
    path,
    NavigationView::new("Projects", text("Select a project")),
)
.destination::<Project, _>(|project| {
    NavigationView::new("Project", text!("Project {}", project.0))
})
.destination::<Preferences, _>(|_| {
    NavigationView::new("Preferences", text("Preferences"))
});
```

No route key or codec is required. Runtime dispatch uses `TypeId`; serde restoration uses the route's Rust type name plus its serde payload. URL routing remains a separate application-facing concern.

## Restoration

Enable `waterui-navigation/serde`, or the facade crate's `waterui/navigation-restoration` feature, to serialize typed paths through the normal serde ecosystem.

```rust
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Route {
    Article(u64),
    Settings,
}

let encoded = serde_json::to_string(&path)?;
let restored: NavigationPath<Route> = serde_json::from_str(&encoded)?;
```

For a heterogeneous path, construct the stack and register all destinations before deserializing. Destination registration automatically registers that concrete route type with the path; the value returned by `restoration()` is a serde `DeserializeSeed` that atomically replaces the existing shared path.

```rust
use serde::de::DeserializeSeed as _;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct RestoredProject(u64);

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct RestoredPreferences;

let path = NavigationPath::heterogeneous();
let stack = NavigationStack::with_path(
    path.clone(),
    NavigationView::new("Root", ()),
)
.destination::<RestoredProject, _>(|project| {
    NavigationView::new("Project", text!("Project {}", project.0))
})
.destination::<RestoredPreferences, _>(|_| NavigationView::new("Preferences", ()));

let mut deserializer = serde_json::Deserializer::from_str(saved_path);
path.restoration().deserialize(&mut deserializer)?;
```

Applications own when and where serialized state is persisted. On process recreation, the restored Rust path remains the source of truth and is atomically reprojected into the native controller stack.

## Deep links

`NavigationRouter` maps external `waterui_url::Url` values to a complete typed path. It intentionally does not reuse restoration type names as public URL syntax.

```rust
let router = NavigationRouter::new(path.clone()).route(|url| {
    (url.path() == "/settings").then_some(vec![Route::Settings])
});

router.open(&Url::new("waterui://app/settings"));
```

Exactly one resolver may claim a URL. A successful match performs one atomic path replacement.

## Navigation chrome and lifecycle

Chrome is semantic, so each backend maps placements into its platform conventions.

```rust
NavigationView::new("Editor", editor)
    .navigation_subtitle("Draft")
    .large_title()
    .navigation_toolbar(NavigationToolbar::new(vec![
        NavigationToolbarItem::action(
            NavigationToolbarPlacement::Cancellation,
            "Cancel",
            cancel,
        ),
        NavigationToolbarItem::action(
            NavigationToolbarPlacement::Confirmation,
            "Save",
            save,
        ),
    ]))
    .searchable(&query, "Search")
    .navigation_bar_visibility(show_bar)
```

Available placements are `Principal`, `PrimaryAction`, `SecondaryAction`, `Confirmation`, `Cancellation`, `BottomBar`, `Status`, `TopBarLeading`, and `TopBarTrailing`.

Destination policy and lifecycle are also semantic:

```rust
NavigationView::new("Checkout", checkout)
    .navigation_pop_enabled(can_leave)
    .on_navigation_pop_attempted(record_attempt)
    .on_navigation_appear(start_observing)
    .on_navigation_disappear(stop_observing)
    .on_navigation_pop(discard_draft)
```

The pop-attempt handler runs for user and system attempts, including denied attempts. `on_navigation_pop` runs only after a completed removal. Native interactive gestures and Android predictive back honor the same policy.

## Transitions

Built-in transitions are platform automatic, fade, matched zoom, and none.

```rust
NavigationStack::with_path(path, root)
    .destination(destination)
    .transition(navigation_transition::fade())
```

Matched zoom uses one stable `Id` on the source and destination:

```rust
let transition_id = Id::try_from(7).expect("transition id must be non-zero");

let source = thumbnail.navigation_transition_source(transition_id);

NavigationStack::with_path(path, root)
    .destination(move |_| {
        NavigationView::new(
            "Photo",
            hero().navigation_transition_destination(transition_id),
        )
    })
    .transition(navigation_transition::zoom(transition_id))
```

Apple and Android execute supported transitions through native transition systems. Hydrolysis executes the same built-ins, interactive edge-pop motion, matched geometry, and custom `NavigationTransition` implementations on the retained GPU scene. A custom transition without a native projection is intentionally applied without animation on native backends and logged as unsupported there.

## Tabs

Tabs are keyed by the application's own tab type and retain each tab root. Badge and enabled state are reactive.

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Home,
    Settings,
}

let selection = binding(Section::Home);

let tabs = Tabs::new(
    &selection,
    vec![
        Tab::new(Section::Home, "Home", || {
            NavigationView::new("Home", home())
        })
        .badge(unread_count),
        Tab::new(Section::Settings, "Settings", || {
            NavigationView::new("Settings", settings())
        })
        .enabled(settings_enabled),
    ],
)
.style(tab_style::automatic());
```

`automatic` selects native tab-bar, sidebar, or navigation-rail presentation from platform and window size. `tab_bar` and `sidebar` request explicit native styles.

`Tabs` erases its identifiers into `TabsLayout`, the `Id`-keyed container the C ABI carries; backends consume `TabsLayout`, application code never names an `Id`.

## Split navigation

Two-column and three-column splits use caller-owned selection bindings and native adaptive containers.

```rust
let split = NavigationSplitView::new(
    &selection,
    sidebar,
    |item| NavigationView::new("Detail", detail(item)),
)
.placeholder(empty_detail)
.column_visibility(visibility)
.sidebar_width(ColumnWidth::new(240.0, 320.0, 480.0))
.style(split_style::prominent_detail());
```

Use `NavigationSplitView::three_column` for independent sidebar and content selections. Apple maps this to `UISplitViewController`; Android uses adaptive `SlidingPaneLayout`, window size classes, folding features, and predictive back; compact layouts collapse to native stack-like navigation.

## Backend behavior

- Apple: native navigation controllers, interactive pop, native bars/search/toolbars, native tabs/sidebar, and native split controllers.
- Android: fragments, Material chrome/transitions, adaptive bottom navigation or rail, folding-aware sliding panes, and predictive back progress/cancel/commit.
- Hydrolysis: retained GPU pages, interruptible and interactive transitions, matched geometry, semantic chrome, tabs, and adaptive split layout.
- GTK: native GTK stack, header bars, tabs, and paned split layouts.

The Rust path is the sole navigation state. Backends receive atomic transactions shaped as `{ id, retained_prefix, removed, inserted }` and report completion or cancellation, preventing native stacks and explicit paths from drifting apart.
