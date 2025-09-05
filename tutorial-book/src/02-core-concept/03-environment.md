# The Environment System

The Environment system is WaterUI's approach to dependency injection and configuration management. It provides a type-safe way to pass data, themes, services, and other dependencies through your view hierarchy without explicit parameter passing. Think of it as a context that flows down through your UI tree.

## Basic Environment Usage

### Storing Values

The most common way to add values to an environment:

```rust,ignore
use waterui::{Environment, View, ViewExt};

#[derive(Debug, Clone)]
struct AppConfig {
    api_url: String,
    timeout_seconds: u64,
}

#[derive(Debug, Clone)]
struct Theme {
    primary_color: Color,
    background_color: Color,
}

pub fn entry() -> impl View{
	home
	.with(AppConfig {
        api_url: "https://api.example.com".to_string(),
        timeout_seconds: 30,
    })
    .with(Theme {
        primary_color: Color::blue(),
        background_color: Color::white(),
    });
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
            .unwrap_or(&Theme::default());
        
        vstack((
            text(format!("API: {}", config.api_url))
                .color(theme.primary_color),
            text(format!("Timeout: {}s", config.timeout_seconds))
                .size(14.0),
        ))
        .background(theme.background_color)
    }
}
```

#### For Function Views (Recommended)
`use_env` function could extract value from environment

```rust,ignore
pub fn api_status() -> impl View{
	use_env(|config:AppConfig, theme: Option<Theme>|{
		let theme = theme.unwrap_or_default();
		    vstack((
            text(format!("API: {}", config.api_url))
                .color(theme.primary_color),
            text(format!("Timeout: {}s", config.timeout_seconds))
                .size(14.0),
        ))
        .background(theme.background_color)
	})
}
```

#### In `action`
```rust,ignore
#[derive(Debug,Clone)]
pub struct Message(&'static str);

pub fn click_me() -> impl View{
	let binding = Binding::container("");
	vstack((
		button("Show environment value")
			.action_with(&binding, |binding, message:Message| binding.set(message.0)),
		text(binding)
	)).with(Message("I'm Lexo"))
}
```