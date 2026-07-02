//! Navigation example demonstrating typed paths, bar slots, and split-ready APIs.

use waterui::app::App;
use waterui::navigation::{
    NavigationLink, NavigationPath, NavigationPathController, NavigationStack,
};
use waterui::prelude::theme_color::{Foreground, MutedForeground, SurfaceVariant};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;

#[derive(Clone, PartialEq, Eq)]
struct Topic {
    title: &'static str,
    description: &'static str,
}

#[derive(Clone, PartialEq, Eq)]
enum Route {
    Topic(Topic),
    Settings,
    About,
    Privacy,
    Nested(Topic),
    Deepest,
}

fn sample_topics() -> Vec<Topic> {
    vec![
        Topic {
            title: "Getting Started",
            description: "Learn the basics of WaterUI navigation",
        },
        Topic {
            title: "Advanced Patterns",
            description: "Deep dive into route-driven navigation",
        },
        Topic {
            title: "Best Practices",
            description: "Keep navigation ergonomic and predictable",
        },
    ]
}

#[preview]
pub fn demo() -> impl View {
    let counter = binding(0);
    let notifications = binding(true);
    let dark_mode = binding(false);
    let path = NavigationPath::new();

    NavigationStack::with(
        path.clone(),
        home_view(counter.clone(), notifications.clone(), dark_mode.clone())
            .title("Navigation Demo")
            .navigation_bar_trailing(
                button("Reset").action(|path: NavigationPathController<Route>| path.clear()),
            ),
    )
    .destination(move |route| match route {
        Route::Topic(topic) => detail_view(topic, counter.clone()),
        Route::Settings => settings_view(notifications.clone(), dark_mode.clone()),
        Route::About => about_view(),
        Route::Privacy => privacy_view(),
        Route::Nested(topic) => nested_detail_view(topic),
        Route::Deepest => deepest_view(),
    })
}

fn home_view(
    counter: Binding<i32>,
    notifications: Binding<bool>,
    dark_mode: Binding<bool>,
) -> impl View {
    scroll(
        vstack((
            header_section(),
            counter_section(&counter),
            topics_section(sample_topics()),
            settings_section(notifications, dark_mode),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

fn header_section() -> impl View {
    vstack((
        text("Welcome to Navigation Demo").title(),
        text("Typed route values drive every destination here.")
            .body()
            .foreground(MutedForeground),
    ))
}

fn counter_section(counter: &Binding<i32>) -> impl View {
    let counter_display = counter.clone();
    vstack((
        spacer_min(16.0),
        Divider,
        spacer_min(16.0),
        vstack((
            text("Shared Counter").sub_headline(),
            hstack((
                button("-")
                    .action(|State(value): State<Binding<i32>>| value.set(value.get() - 1))
                    .state(counter),
                spacer_min(16.0),
                text!("Count: {counter_display}").body(),
                spacer_min(16.0),
                button("+")
                    .action(|State(value): State<Binding<i32>>| value.set(value.get() + 1))
                    .state(counter),
            )),
            text("This value stays live across the whole path.")
                .caption()
                .foreground(MutedForeground),
        ))
        .padding_with(EdgeInsets::all(12.0))
        .background(SurfaceVariant),
    ))
}

fn topics_section(items: Vec<Topic>) -> impl View {
    vstack((
        spacer_min(16.0),
        Divider,
        spacer_min(16.0),
        text("Topics").sub_headline(),
        spacer_min(8.0),
        vstack(items.into_iter().map(topic_row).collect::<Vec<_>>()),
    ))
}

fn topic_row(topic: Topic) -> impl View {
    let route = Route::Topic(topic.clone());
    vstack((
        NavigationLink::value(label(topic.title).icon(text(">")).trailing(), route),
        text(topic.description)
            .caption()
            .foreground(MutedForeground),
    ))
    .padding_with(EdgeInsets::symmetric(8.0, 0.0))
}

fn settings_section(notifications: Binding<bool>, dark_mode: Binding<bool>) -> impl View {
    let _ = (notifications, dark_mode);
    vstack((
        spacer_min(16.0),
        Divider,
        spacer_min(16.0),
        NavigationLink::value(
            label("Settings").icon(text(">")).trailing(),
            Route::Settings,
        ),
    ))
}

fn detail_view(topic: Topic, counter: Binding<i32>) -> NavigationView {
    scroll(
        vstack((
            vstack((
                text(topic.title).headline(),
                spacer_min(8.0),
                text(topic.description).body().foreground(MutedForeground),
                spacer_min(24.0),
                Divider,
            )),
            spacer_min(24.0),
            vstack((
                text("Shared Counter").sub_headline(),
                text!("Current value: {counter}").body(),
            ))
            .padding_with(EdgeInsets::all(12.0))
            .background(SurfaceVariant),
            spacer_min(24.0),
            NavigationLink::value(
                label("Go Deeper").icon(text(">")).trailing(),
                Route::Nested(topic.clone()),
            ),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
    .title(topic.title)
}

fn nested_detail_view(topic: Topic) -> NavigationView {
    vstack((
        text("Nested Navigation").title(),
        spacer_min(16.0),
        text!("You navigated from: {title}", title = topic.title).body(),
        spacer_min(24.0),
        NavigationLink::value(
            label("Go Even Deeper").icon(text(">")).trailing(),
            Route::Deepest,
        ),
    ))
    .padding_with(EdgeInsets::all(16.0))
    .title("Nested View")
}

fn deepest_view() -> NavigationView {
    vstack((
        text("You're Deep!").headline(),
        spacer_min(24.0),
        text("This is route level three in the stack.").body(),
        spacer_min(16.0),
        text("Use the back button or Reset to unwind the path.")
            .body()
            .foreground(MutedForeground),
    ))
    .padding_with(EdgeInsets::all(16.0))
    .title("Level 3")
}

fn settings_view(notifications: Binding<bool>, dark_mode: Binding<bool>) -> NavigationView {
    scroll(
        vstack((
            text("Settings").title(),
            spacer_min(16.0),
            toggle_row(
                "Notifications",
                "Receive push notifications",
                &notifications,
            ),
            Divider,
            toggle_row("Dark Mode", "Use dark color scheme", &dark_mode),
            Divider,
            settings_links(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
    .title("Settings")
    .navigation_bar_leading(text("Routes").caption().foreground(MutedForeground))
}

fn toggle_row(title: &'static str, subtitle: &'static str, value: &Binding<bool>) -> impl View {
    hstack((
        vstack((
            text(title).sub_headline(),
            text(subtitle).caption().foreground(MutedForeground),
        )),
        spacer(),
        Toggle::new(value),
    ))
    .padding_with(EdgeInsets::symmetric(8.0, 0.0))
}

fn settings_links() -> impl View {
    vstack((
        NavigationLink::value(label("About").icon(text(">")).trailing(), Route::About),
        Divider,
        NavigationLink::value(
            label("Privacy Policy").icon(text(">")).trailing(),
            Route::Privacy,
        ),
    ))
}

fn about_view() -> NavigationView {
    vstack((
        text("Navigation Example").title(),
        spacer_min(8.0),
        text("Route-driven WaterUI navigation")
            .caption()
            .foreground(MutedForeground),
        spacer_min(24.0),
        text("This example exercises typed routes, path control, and bar slots.").body(),
        spacer_min(16.0),
        vstack((
            text("Features showcased:").foreground(Foreground),
            "- Path-backed NavigationStack",
            "- NavigationLink::value",
            "- NavigationPathController",
            "- Navigation bar leading/trailing slots",
        )),
    ))
    .padding_with(EdgeInsets::all(16.0))
    .title("About")
}

fn privacy_view() -> NavigationView {
    scroll(
        vstack((
            text("Privacy Policy").title(),
            spacer_min(16.0),
            text("This is a sample privacy page for the navigation example.").body(),
            spacer_min(16.0),
            text("Typed routes keep the navigation tree explicit and inspectable.")
                .body()
                .foreground(MutedForeground),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
    .title("Privacy")
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
