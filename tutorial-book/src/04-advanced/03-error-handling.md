# Error Handling

Error handling in WaterUI is designed to integrate seamlessly with the declarative view system, allowing you to convert standard Rust errors into renderable UI components while maintaining type safety and customization flexibility.

## Core Concepts

### The `Error` Type

The `Error` type is a type-erased wrapper that can hold any error implementing the standard `Error` trait and render it as a view:

```rust
use waterui::widget::error::Error;
use std::io;

// Convert any standard error to a renderable Error
let io_error = io::Error::new(io::ErrorKind::NotFound, "Config file not found");
let ui_error = Error::new(io_error);
```

### Environment-Based Error Styling

WaterUI uses the environment system to configure how errors are displayed throughout your application:

```rust
use waterui::Environment;
use waterui::widget::error::{DefaultErrorView, BoxedStdError};

let env = Environment::new()
    .with(DefaultErrorView::new(|error: BoxedStdError| {
        VStack((
            "⚠️ Application Error",
            format!("Details: {}", error),
            "Please contact support if this persists."
                .color(Color::SECONDARY)
        ))
    }));
```

## Basic Usage Patterns

### Converting Results to Views

The `ResultExt` trait provides convenient methods for converting `Result` types to views:

```rust
use waterui::widget::error::ResultExt;
use waterui::prelude::*;

fn load_user_data(user_id: u32) -> Result<String, DatabaseError> {
    // Simulate database operation
    if user_id == 0 {
        Err(DatabaseError::InvalidId)
    } else {
        Ok(format!("User {}", user_id))
    }
}

fn user_profile_view(user_id: u32) -> impl View {
    match load_user_data(user_id)
        .error_view(|err| text!("Failed to load user: {}", err))
    {
        Ok(user_data) => text!(user_data),
        Err(error_view) => error_view.any_view(),
    }
}
```

### Inline Error Handling

You can handle errors inline within view construction:

```rust
fn network_status_view() -> impl View {
    vstack((
        "Network Status",
        match check_connection() {
            Ok(status) => text!(status),
            Err(error) => Error::new(error).any_view(),
        }
    ))
}
```

## Advanced Features

### Type Downcasting

The error system preserves type information, allowing you to downcast to specific error types for specialized handling:

```rust
use waterui::widget::error::Error;
use std::io;

fn handle_file_error(error: Error) -> impl View {
    match error.downcast::<io::Error>() {
        Ok(io_error) => {
            match io_error.kind() {
                io::ErrorKind::NotFound => {
                    vstack((
                        "File Not Found",
                        "The requested file could not be located.",
                        button("Browse for File", browse_action)
                    )).any_view()
                }
                io::ErrorKind::PermissionDenied => {
                    vstack((
                        "Permission Denied",
                        "You don't have permission to access this file.",
                        button("Request Access", request_access_action)
                    )).any_view()
                }
                _ => text!("IO Error: {}", io_error).any_view()
            }
        }
        Err(original_error) => {
            // Handle as generic error
            text!("Error: {}", original_error).any_view()
        }
    }
}
```

### Custom Error Views

Create errors directly from custom views for complete control over presentation:

```rust
use waterui::widget::error::Error;

fn validation_error_view(field: &str, message: &str) -> Error {
    Error::from_view(
        hstack((
            Icon::warning().foreground(Color::WARNING),
            vstack((
                text!("Validation Error: {}", field),
                text!(message).foreground(Color::SECONDARY)
            ))
        ))
        .padding(16.0)
        .background(Color::WARNING.opacity(0.1))
    )
}

// Usage in form validation
fn validate_email(email: &str) -> Result<String, Error> {
    if email.contains('@') {
        Ok(email.to_string())
    } else {
        Err(validation_error_view("Email", "Must contain @ symbol"))
    }
}
```

## Error Handling Patterns

### Loading States with Error Handling

Combine error handling with loading states for better user experience:

```rust
enum LoadingState<T> {
    Loading,
    Loaded(T),
    Error(Error),
}

fn data_view(state: LoadingState<UserData>) -> impl View {
    match state {
        LoadingState::Loading => {
            hstack((
                ProgressIndicator::spinning(),
                "Loading user data..."
            )).any_view()
        }
        LoadingState::Loaded(data) => {
            user_profile_component(data).any_view()
        }
        LoadingState::Error(error) => {
            error.any_view()
        }
    }
}
```

### Contextual Error Information

Provide context-aware error messages based on the current view:

```rust
fn api_request_view(endpoint: &str) -> impl View {
    match make_api_request(endpoint)
        .error_view(|err| {
            vstack((
                "Network Request Failed",
                text!("Endpoint: {}", endpoint),
                text!("Error: {}", err),
                hstack((
                    button("Retry", retry_action),
                    button("Go Offline", offline_mode_action)
                ))
            ))
            .padding(20.0)
            .background(Color::ERROR.opacity(0.1))
        })
    {
        Ok(response) => response_view(response).any_view(),
        Err(error_view) => error_view.any_view(),
    }
}
```

## Best Practices

### 1. Configure Global Error Styling

Set up a consistent error presentation style at your app's root:

```rust
fn app_root() -> impl View {
    ContentView::new()
        .with(DefaultErrorView::new(|error| {
            vstack((
                Icon::error().size(24),
                text!("{}", error),
                text!("If this problem persists, please contact support.")
                    .foreground(Color::SECONDARY)
            ))
            .padding(16.0)
        }))
        .body(main_content_view())
}
```

### 2. Provide Actionable Error Messages

Include relevant actions users can take to resolve errors:

```rust
fn network_error_view(error: NetworkError) -> impl View {
    vstack((
        "Connection Problem",
        text!("{}", error),
        hstack((
            button("Check Connection", check_connection_action),
            button("Work Offline", enable_offline_mode),
            button("Retry", retry_last_action)
        ))
    ))
}
```

### 3. Use Appropriate Error Granularity

Handle different error types at appropriate levels in your view hierarchy:

```rust
fn user_dashboard() -> impl View {
    vstack((
        // Handle critical errors at component level
        match load_user_session() {
            Ok(session) => session_header(session).any_view(),
            Err(auth_error) => login_prompt(auth_error).any_view(),
        },
        
        // Handle non-critical errors inline
        hstack((
            user_avatar().unwrap_or_else(|_| default_avatar()),
            user_stats().unwrap_or_else(|err| {
                text!("Stats unavailable").foreground(Color::SECONDARY)
            })
        ))
    ))
}
```

## ErrorView

The `ErrorView` is an internal component that wraps views to be used as errors. It's primarily used internally by the error system but can be useful in advanced scenarios.

## DefaultErrorView

`DefaultErrorView` is the environment-based configuration mechanism that allows you to define how errors should be rendered throughout your application. It acts as a fallback when no specific error handling is provided, ensuring consistent error presentation across your entire UI.
