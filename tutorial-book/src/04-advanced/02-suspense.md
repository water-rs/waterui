# Suspense and Asynchronous Loading

Modern applications often need to load data asynchronously from APIs, databases, or other sources. The `Suspense` component in WaterUI provides an elegant way to handle async content loading while maintaining a responsive user interface. This chapter covers everything you need to know about implementing suspense in your applications.

## Basic Suspense Usage

The simplest way to use Suspense is with async functions that return views:

```rust,no_run
use waterui::{View};
use waterui_text::text;
use waterui::component::layout::stack::{vstack};
use waterui::widget::suspense::Suspense;

// Async function that loads data
async fn load_user_profile(user_id: u32) -> impl View {
    // Simulate API call
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    let user_data = fetch_user_data(user_id).await;
    
    vstack((
        text!("Name: {}", user_data.name),
        text!("Email: {}", user_data.email),
        text!("Joined: {}", user_data.joined_date),
    ))
}

fn user_profile_view(user_id: u32) -> impl View {
    // Basic suspense with custom loading view
    Suspense::new(load_user_profile(user_id))
        .loading(text!("Loading user profile..."))
}
```

### Using Default Loading Views

You can set up default loading views in your application's environment:

```rust,no_run
use waterui::{Environment};
use waterui::view::AnyViewBuilder;
use waterui::widget::suspense::{Suspense, DefaultLoadingView};
use waterui_text::text;
use waterui::component::layout::stack::vstack;
use waterui::component::layout::{Edge, Frame};

// Set up default loading view in your app
fn setup_app_environment() -> Environment {
    let loading_view = AnyViewBuilder::new(|_| {
        vstack((
            text!("Loading..."),
        ))
        .frame(Frame::new().margin(Edge::round(20.0)))
    });

    Environment::new().with(DefaultLoadingView(loading_view))
}

// Components can now use the default loading view
fn simple_async_view() -> impl View {
    Suspense::new(load_data) // Uses default loading view from environment
}
```

## The SuspendedView Trait

Any type can be used with Suspense by implementing the `SuspendedView` trait. The trait is automatically implemented for any `Future` that resolves to a `View`.

```rust,no_run
pub trait SuspendedView: 'static {
    fn body(self, _env: Environment) -> impl Future<Output = impl View>;
}
```

### Automatic Implementation for Futures

```rust,no_run
use waterui::widget::suspense::{Suspense, SuspendedView};
use waterui_text::text;

// These all work with Suspense automatically:

// 1. Async functions
async fn fetch_weather() -> impl View {
    let weather = get_weather_data().await;
    text!("Temperature: {}°F", weather.temperature)
}

// 2. Async closures
let load_news = async move || {
    let articles = fetch_news_articles().await;
    news_list_view(articles)
};

// 3. Future types
use std::future::Future;
use std::pin::Pin;

type BoxedFuture = Pin<Box<dyn Future<Output = impl View>>>;

fn get_async_content() -> BoxedFuture {
    Box::pin(async {
        text!("Async content loaded!")
    })
}

// All work with Suspense:
let weather_view = Suspense::new(fetch_weather);
let news_view = Suspense::new(load_news);
let content_view = Suspense::new(get_async_content());
```

### Custom SuspendedView Implementation

For more complex scenarios, you can implement `SuspendedView` manually:

```rust,no_run
use waterui::{Environment, View};
use waterui::widget::suspense::SuspendedView;
use waterui_text::text;
use waterui::component::layout::stack::vstack;

struct DataLoader {
    user_id: u32,
    include_posts: bool,
}

impl SuspendedView for DataLoader {
    async fn body(self, _env: Environment) -> impl View {
        // Custom loading logic with environment access
        let user = fetch_user(self.user_id).await;
        
        if self.include_posts {
            let posts = fetch_user_posts(self.user_id).await;
            vstack((
                user_profile_view(user),
                posts_list_view(posts),
            ))
        } else {
            user_profile_view(user)
        }
    }
}

// Usage
fn user_dashboard(user_id: u32, show_posts: bool) -> impl View {
    Suspense::new(DataLoader {
        user_id,
        include_posts: show_posts,
    })
    .loading(text!("Loading dashboard..."))
}
```
