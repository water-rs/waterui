# Suspense and Asynchronous Loading

Modern applications often need to load data asynchronously from APIs, databases, or other sources. The `Suspense` component in WaterUI provides an elegant way to handle async content loading while maintaining a responsive user interface. This chapter covers everything you need to know about implementing suspense in your applications.

## Understanding Suspense

Suspense is a pattern that allows you to show loading states while asynchronous content is being prepared. It automatically handles the transition from loading to loaded states, providing a smooth user experience during async operations.

### Key Benefits

- **Non-blocking UI**: Interface remains responsive during async operations
- **Automatic State Management**: Handles loading-to-loaded transitions seamlessly
- **Flexible Loading States**: Support for custom or default loading views
- **Performance Optimized**: Lazy loading and efficient state updates
- **Clean Code**: Declarative approach to async UI patterns

## Basic Suspense Usage

The simplest way to use Suspense is with async functions that return views:

```rust,no_run
use waterui::{Suspense, text, vstack};

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
use waterui::{Suspense, DefaultLoadingView, Environment};
use waterui_core::view::ViewBuilder;

// Set up default loading view in your app
fn setup_app_environment() -> Environment {
    let loading_view = ViewBuilder::new(|| {
        vstack((
            spinner(),
            text!("Loading..."),
        ))
        .padding(20)
    });
    
    Environment::new()
        .with(DefaultLoadingView(loading_view.anybuilder()))
}

// Components can now use the default loading view
fn simple_async_view() -> impl View {
    Suspense::new(load_data) // Uses default loading view from environment
}
```

## The SuspendedView Trait

Any type can be used with Suspense by implementing the `SuspendedView` trait. The trait is automatically implemented for any `Future` that resolves to a `View`.

### Automatic Implementation for Futures

```rust,no_run
use waterui::{Suspense, SuspendedView, text};

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
use waterui::{SuspendedView, Environment, View, text, vstack};

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

## Advanced Patterns

### Conditional Suspense

You can conditionally apply suspense based on application state:

```rust,no_run
use waterui::{Suspense, text, vstack};
use nami::*;

fn conditional_async_view(should_load_async: Binding<bool>) -> impl View {
    s!(if should_load_async {
        // Show suspense for async loading
        Some(Suspense::new(load_heavy_content)
            .loading(text!("Loading heavy content...")))
    } else {
        // Show static content immediately
        Some(static_content_view())
    })
}
```

### Nested Suspense

Suspense components can be nested for complex loading scenarios:

```rust,no_run
async fn load_dashboard() -> impl View {
    let user = fetch_current_user().await;
    
    vstack((
        user_header_view(user),
        // Nested suspense for additional async content
        Suspense::new(load_user_activity(user.id))
            .loading(text!("Loading activity...")),
        Suspense::new(load_user_settings(user.id))
            .loading(text!("Loading settings...")),
    ))
}

fn main_dashboard() -> impl View {
    Suspense::new(load_dashboard)
        .loading(text!("Loading dashboard..."))
}
```

### Error Handling in Suspense

While Suspense handles the loading state, you should handle errors within your async functions:

```rust,no_run
use waterui::{text, vstack};

async fn resilient_data_loader() -> impl View {
    match fetch_data().await {
        Ok(data) => data_view(data),
        Err(error) => error_view(error),
    }
}

fn error_view(error: AppError) -> impl View {
    vstack((
        text!("❌ Error: {}", error.message),
        button("Retry", || {
            // Retry logic
        }),
    ))
    .padding(20)
    .background(Color::red().opacity(0.1))
}

fn robust_async_view() -> impl View {
    Suspense::new(resilient_data_loader)
        .loading(text!("Loading..."))
}
```

## Performance Considerations

### Lazy Loading with Suspense

Suspense is excellent for implementing lazy loading patterns:

```rust,no_run
use waterui::{Suspense, scroll_view, vstack};

struct LazyImageLoader {
    image_urls: Vec<String>,
}

impl SuspendedView for LazyImageLoader {
    async fn body(self, _env: Environment) -> impl View {
        let mut views = Vec::new();
        
        for url in self.image_urls {
            let image = load_image_async(url).await;
            views.push(image);
        }
        
        vstack(views)
    }
}

fn photo_gallery(urls: Vec<String>) -> impl View {
    scroll_view(
        Suspense::new(LazyImageLoader { image_urls: urls })
            .loading(text!("Loading images..."))
    )
}
```

### Caching with Suspense

Implement caching to avoid redundant async operations:

```rust,no_run
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

lazy_static! {
    static ref CONTENT_CACHE: Arc<Mutex<HashMap<String, CachedContent>>> = 
        Arc::new(Mutex::new(HashMap::new()));
}

async fn cached_content_loader(content_id: String) -> impl View {
    // Check cache first
    if let Some(cached) = CONTENT_CACHE.lock().unwrap().get(&content_id) {
        return cached.view.clone();
    }
    
    // Load and cache
    let content = fetch_content(&content_id).await;
    let view = content_view(content);
    
    CONTENT_CACHE.lock().unwrap().insert(
        content_id,
        CachedContent { view: view.clone() }
    );
    
    view
}
```

### FnOnce for Lazy Initialization

Use closures for lazy initialization of loading views:

```rust,no_run
use waterui::{Suspense, text};

fn optimized_loading_view() -> impl View {
    Suspense::new(expensive_async_operation)
        .loading(|| {
            // This closure is only called when needed
            // Perfect for expensive loading animations
            complex_loading_spinner()
        })
}
```

## Environment Integration

### Global Loading Configuration

Set up consistent loading experiences across your app:

```rust,no_run
use waterui::{DefaultLoadingView, Environment, text, vstack, circle, animation};

fn create_app_environment() -> Environment {
    // Create a sophisticated loading view
    let loading_view = ViewBuilder::new(|| {
        vstack((
            circle()
                .size(30, 30)
                .color(Color::blue())
                .rotation(animation::linear(Duration::from_secs(1)).repeat()),
            text!("Loading...")
                .color(Color::gray()),
        ))
        .spacing(10)
        .center()
    });
    
    Environment::new()
        .with(DefaultLoadingView(loading_view.anybuilder()))
        .with(NetworkConfig::default())
        .with(CacheConfig::default())
}

// All suspense components in your app will use this loading view by default
fn app() -> impl View {
    vstack((
        header(),
        Suspense::new(load_main_content), // Uses default loading view
        footer(),
    ))
    .environment(create_app_environment())
}
```

### Environment-Aware Loading

Access environment data in your async loaders:

```rust,no_run
use waterui::{SuspendedView, Environment};

struct UserDataLoader {
    user_id: u32,
}

impl SuspendedView for UserDataLoader {
    async fn body(self, env: Environment) -> impl View {
        // Access configuration from environment
        let api_config = env.get::<ApiConfig>().unwrap();
        let theme = env.get::<ThemeConfig>().unwrap();
        
        let user = fetch_user_with_config(self.user_id, &api_config).await;
        
        user_view(user)
            .color(theme.primary_color)
            .font(theme.body_font)
    }
}
```

## Testing Suspense Components

### Unit Testing Async Views

```rust,no_run
#[cfg(test)]
mod tests {
    use super::*;
    use tokio_test;

    #[tokio::test]
    async fn test_async_content_loading() {
        let content_loader = async {
            text!("Test content")
        };
        
        let result = content_loader.await;
        // Test the resolved content
        assert_eq!(get_text_content(&result), "Test content");
    }

    #[tokio::test] 
    async fn test_custom_suspended_view() {
        let loader = DataLoader {
            user_id: 123,
            include_posts: true,
        };
        
        let env = Environment::new();
        let result = loader.body(env).await;
        
        // Test the loaded view structure
        verify_view_structure(&result);
    }
}
```

### Integration Testing

```rust,no_run
#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[test]
    fn test_suspense_loading_state() {
        let mut app = TestApp::new();
        
        // Render suspense component
        let suspense_view = Suspense::new(slow_async_loader)
            .loading(text!("Loading..."));
        
        app.render(suspense_view);
        
        // Verify loading state is shown
        assert!(app.contains_text("Loading..."));
        
        // Wait for async operation to complete
        app.wait_for_completion();
        
        // Verify content is loaded
        assert!(app.contains_text("Loaded content"));
    }
}
```

## Best Practices

### 1. Loading State Design

- **Provide meaningful feedback**: Use descriptive loading messages
- **Match your brand**: Ensure loading states fit your app's design
- **Progressive disclosure**: Show partial content when possible
- **Skeleton screens**: Use placeholder content that matches final layout

```rust,no_run
fn good_loading_state() -> impl View {
    vstack((
        // Skeleton for the actual content structure
        rectangle()
            .size(200, 20)
            .background(Color::gray().opacity(0.3))
            .corner_radius(4),
        rectangle()
            .size(150, 20)
            .background(Color::gray().opacity(0.3))
            .corner_radius(4),
        rectangle()
            .size(300, 60)
            .background(Color::gray().opacity(0.3))
            .corner_radius(4),
    ))
    .spacing(8)
    .padding(16)
}
```

### 2. Error Handling

- **Always handle errors**: Don't let async operations fail silently
- **Provide retry mechanisms**: Allow users to retry failed operations
- **Meaningful error messages**: Explain what went wrong and how to fix it

### 3. Performance

- **Use caching**: Avoid redundant network requests
- **Lazy loading**: Only load content when needed
- **Minimize async work**: Keep async operations focused and efficient
- **Consider offline states**: Handle network unavailability gracefully

### 4. User Experience

- **Immediate feedback**: Show loading states instantly
- **Progressive enhancement**: Show content as it becomes available
- **Smooth transitions**: Use animations for state changes
- **Predictable timing**: Set user expectations for loading duration

## Common Patterns

### Data Fetching

```rust,no_run
async fn fetch_user_dashboard(user_id: u32) -> impl View {
    // Parallel data fetching for better performance
    let (user, posts, notifications) = tokio::join!(
        fetch_user(user_id),
        fetch_user_posts(user_id),
        fetch_user_notifications(user_id)
    );
    
    dashboard_view(user, posts, notifications)
}
```

### Route-Based Loading

```rust,no_run
use waterui_navigation::Route;

async fn route_loader(route: Route) -> impl View {
    match route {
        Route::Home => home_view().await,
        Route::Profile { user_id } => profile_view(user_id).await,
        Route::Settings => settings_view().await,
        _ => not_found_view(),
    }
}

fn app_router(current_route: Binding<Route>) -> impl View {
    Suspense::new(s!(route_loader(current_route)))
        .loading(navigation_loading_view())
}
```

### Form Submission

```rust,no_run
struct FormSubmitter {
    form_data: FormData,
}

impl SuspendedView for FormSubmitter {
    async fn body(self, _env: Environment) -> impl View {
        match submit_form(self.form_data).await {
            Ok(response) => success_view(response),
            Err(error) => form_error_view(error),
        }
    }
}

fn submit_form_view(form_data: Binding<FormData>) -> impl View {
    let is_submitting = binding(false);
    
    s!(if is_submitting {
        Some(Suspense::new(FormSubmitter {
            form_data: form_data.get()
        }).loading(text!("Submitting form...")))
    } else {
        Some(form_view(form_data, is_submitting))
    })
}
```

## Conclusion

Suspense provides a powerful and elegant way to handle asynchronous content in WaterUI applications. By understanding the patterns and best practices covered in this chapter, you can create responsive, user-friendly interfaces that handle async operations gracefully.

Key takeaways:
- Use Suspense for any async content loading
- Implement meaningful loading states
- Handle errors gracefully within async functions  
- Leverage environment configuration for consistent experiences
- Apply performance optimizations like caching and lazy loading
- Test both loading and loaded states thoroughly

With these techniques, your applications will provide smooth, professional user experiences even during complex async operations.