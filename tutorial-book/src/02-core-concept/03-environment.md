# The Environment System

The Environment system is WaterUI's approach to dependency injection and configuration management. It provides a type-safe way to pass data, themes, services, and other dependencies through your view hierarchy without explicit parameter passing. Think of it as a context that flows down through your UI tree.

## Basic Environment Usage

### Storing Values

The most common way to add values to an environment:

```rust,ignore
use waterui::{Environment, View, ViewExt};
use waterui::component::layout::{Edge, Frame};

#[derive(Debug, Clone)]
struct AppConfig {
    api_url: String,
    timeout_seconds: u64,
}

#[derive(Debug, Clone)]
struct Theme {
    primary_color: waterui::core::Color,
    background_color: waterui::core::Color,
}

pub fn entry() -> impl View {
    home
        .with(AppConfig {
        api_url: "https://api.example.com".to_string(),
        timeout_seconds: 30,
    })
    .with(Theme {
        primary_color: (0.0, 0.4, 1.0).into(),
        background_color: (1.0, 1.0, 1.0).into(),
    })
}

pub fn home() -> impl View{
	// Your home page
}
```

### Accessing Values in Views

#### For Struct Views

Views can access environment values in their `body` method:

```rust,ignore
struct ApiStatusView;

impl View for ApiStatusView {
    fn body(self, env: &Environment) -> impl View {
        // Get configuration from environment
        let config = env.get::<AppConfig>()
            .expect("AppConfig should be provided");
            
        let theme = env.get::<Theme>()
            .expect("Theme should be provided");
        
        vstack((
            waterui_text::Text::new(config.api_url.clone()).foreground(theme.primary_color.clone()),
            waterui_text::Text::new(format!("Timeout: {}s", config.timeout_seconds)).size(14.0),
        ))
        .background(waterui::background::Background::color(theme.background_color.clone()))
        .frame(Frame::new().margin(Edge::round(12.0)))
    }
}
```

#### For Function Views
You can pass values through the environment at the call site and read them inside struct views as shown above. Function views typically compose other views and don’t receive `env` directly.

#### In `action`
```rust,ignore
use waterui::{View};
use waterui::reactive::binding;
use waterui_text::text;
use waterui::component::{layout::stack::vstack, button::button};

#[derive(Debug, Clone)]
pub struct Message(&'static str);

pub fn click_me() -> impl View {
    let value = binding(String::new());
    vstack((
        button("Show environment value").action_with(&value, |value, msg: waterui::core::extract::Use<Message>| {
            value.set(msg.0 .0.to_string());
        }),
        text!("{}", value),
    ))
    .with(Message("I'm Lexo"))
}
```
