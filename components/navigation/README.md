# WaterUI Navigation Components

Navigation components and utilities for building hierarchical user interfaces with navigation bars, tabs, and navigation flows.

## Overview

`waterui-navigation` provides components for creating navigation-based user interfaces including navigation views, navigation bars, tab views, and search functionality. It handles the hierarchical organization and flow of screens in your application.

## Components

### Navigation View

Container that provides navigation bar and content:

```rust
use waterui_navigation::{NavigationView, Bar};

let nav_view = NavigationView {
    bar: Bar {
        title: Text::new("My App"),
        color: constant(Color::BLUE),
        hidden: constant(false),
    },
    content: AnyView::new(main_content()),
};
```

### Navigation Bar

Configurable bar with title and controls:

```rust
use waterui_navigation::Bar;

let nav_bar = Bar {
    title: Text::new("Settings")
        .typography(Typography::HEADING_2),
    color: constant(Color::PRIMARY),
    hidden: constant(false),
}
.leading_button(
    Button::new("Back")
        .on_click(|| navigation.pop())
)
.trailing_button(
    Button::new("Done")
        .on_click(|| save_and_close())
);
```

### Tab View

Tab-based navigation interface:

```rust
use waterui_navigation::tab::{TabView, Tab};

let tab_view = TabView::new([
    Tab::new("Home", home_icon(), home_view()),
    Tab::new("Search", search_icon(), search_view()),
    Tab::new("Profile", profile_icon(), profile_view()),
])
.selected_index(selected_tab_binding)
.tab_bar_style(TabBarStyle::Prominent);
```

### Navigation Link

Declarative navigation between views:

```rust
use waterui_navigation::NavigationLink;

let nav_link = NavigationLink::new(
    destination: detail_view(item),
    label: Text::new("View Details")
);

// Button-style navigation
let nav_button = Button::new("Go to Settings")
    .navigation_destination(settings_view());
```

## Navigation State Management

### Navigation Stack

Manage navigation history:

```rust
use waterui_navigation::{NavigationStack, NavigationPath};

let nav_stack = NavigationStack::new(root_view())
    .path(navigation_path_binding)
    .on_path_change(|new_path| {
        // Handle navigation changes
        update_breadcrumbs(new_path);
    });
```

### Programmatic Navigation

Control navigation through code:

```rust
// Push new view
navigation.push(detail_view(item));

// Pop current view
navigation.pop();

// Pop to root
navigation.pop_to_root();

// Replace current view
navigation.replace(new_view());

// Set entire navigation stack
navigation.set_path(new_path);
```

## Search Integration

### Search Bar

Built-in search functionality:

```rust
use waterui_navigation::search::{SearchBar, SearchScope};

let search_bar = SearchBar::new(search_query_binding)
    .placeholder("Search items...")
    .scopes([
        SearchScope::new("All", "all"),
        SearchScope::new("Recent", "recent"),
        SearchScope::new("Favorites", "favorites"),
    ])
    .selected_scope(selected_scope_binding)
    .on_search_change(|query| {
        perform_search(query);
    });
```

### Searchable Modifier

Add search to any view:

```rust
let searchable_list = item_list()
    .searchable(
        search_binding,
        placeholder: "Search items",
        scopes: search_scopes,
    )
    .search_suggestions(
        suggestions_for_query(search_binding.get())
    );
```

## Advanced Navigation

### Modal Presentation

Present views modally:

```rust
let modal_button = Button::new("Show Modal")
    .sheet(modal_content(), is_presented: show_modal_binding)
    .presentation_style(PresentationStyle::FormSheet);

let alert_button = Button::new("Show Alert")
    .alert(
        title: "Confirm Action",
        message: "Are you sure?",
        is_presented: show_alert_binding,
        actions: [
            AlertAction::cancel("Cancel"),
            AlertAction::destructive("Delete", delete_action),
        ]
    );
```

### Navigation Transitions

Custom transitions between views:

```rust
let animated_nav = NavigationView::new(content)
    .transition(.slide)
    .transition_duration(0.3)
    .interactive_pop_gesture(true);
```

### Deep Linking

Handle URL-based navigation:

```rust
use waterui_navigation::DeepLink;

let deep_link_handler = DeepLink::new()
    .route("/home", || home_view())
    .route("/profile/:id", |id| profile_view(id))
    .route("/settings/*", || settings_flow())
    .fallback(|| not_found_view());
```

## Navigation Patterns

### Master-Detail

Split view navigation pattern:

```rust
let master_detail = MasterDetailView::new(
    master: item_list(),
    detail: |selected_item| detail_view(selected_item),
    selection: selected_item_binding,
)
.master_width(300.0)
.show_master_in_compact(false);
```

### Tab Navigation with Stacks

Combine tabs with navigation stacks:

```rust
let tabbed_nav = TabView::new([
    Tab::new("Home", home_icon(), NavigationStack::new(home_view())),
    Tab::new("Browse", browse_icon(), NavigationStack::new(browse_view())),
    Tab::new("Account", account_icon(), NavigationStack::new(account_view())),
]);
```

## Customization

### Navigation Bar Styling

```rust
let styled_bar = Bar::new()
    .title(title_text)
    .background_color(Color::BLUE)
    .foreground_color(Color::WHITE)
    .shadow_radius(4.0)
    .blur_effect(BlurEffect::SystemMaterial);
```

### Tab Bar Styling

```rust
let styled_tabs = TabView::new(tabs)
    .tab_bar_background(Color::WHITE)
    .selected_tab_color(Color::BLUE)
    .unselected_tab_color(Color::GRAY)
    .tab_bar_style(TabBarStyle::Compact);
```

## Accessibility

Navigation accessibility features:

```rust
let accessible_nav = NavigationView::new(content)
    .accessibility_navigation_style(.combined)
    .accessibility_label("Main navigation")
    .voice_over_navigation(true);
```

## Dependencies

- `waterui-core`: Core framework functionality
- `waterui-text`: Text display components

## Example

```rust
use waterui_navigation::*;

struct AppView {
    current_tab: Binding<usize>,
    navigation_path: Binding<NavigationPath>,
}

impl View for AppView {
    fn body(self, env: &Environment) -> impl View {
        TabView::new([
            Tab::new(
                "Home",
                icon("house"),
                NavigationStack::new(HomeView::new())
                    .path(self.navigation_path.clone())
            ),
            Tab::new(
                "Search",
                icon("magnifyingglass"),
                SearchView::new()
                    .searchable(search_binding, "Search everything...")
            ),
            Tab::new(
                "Profile",
                icon("person"),
                NavigationView::new(ProfileView::new())
                    .navigation_bar(Bar {
                        title: Text::new("Profile"),
                        color: constant(Color::BLUE),
                        hidden: constant(false),
                    })
            ),
        ])
        .selected_index(self.current_tab)
        .tab_bar_style(TabBarStyle::Prominent)
    }
}
```
