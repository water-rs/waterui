# The Environment System

The Environment system is WaterUI's approach to dependency injection and configuration management. It provides a type-safe way to pass data, themes, services, and other dependencies through your view hierarchy without explicit parameter passing. Think of it as a context that flows down through your UI tree.

## Understanding Environment

The Environment is essentially a type-indexed map where each type serves as a unique key:

```rust,ignore
#[derive(Debug, Clone, Default)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
}
```

This design provides several benefits:
- **Type Safety**: Compile-time verification of what's available
- **Automatic Inheritance**: Child views inherit parent environments
- **No Prop Drilling**: Pass dependencies without explicit parameters
- **Plugin System**: Easy integration of cross-cutting concerns

## Basic Environment Usage

### Storing Values

The most common way to add values to an environment:

```rust,ignore
use waterui::{Environment, View};

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

// Create environment with values
let env = Environment::new()
    .with(AppConfig {
        api_url: "https://api.example.com".to_string(),
        timeout_seconds: 30,
    })
    .with(Theme {
        primary_color: Color::blue(),
        background_color: Color::white(),
    });
```

### Accessing Values in Views

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

### Providing Defaults

Always provide sensible defaults for optional environment values:

```rust,ignore
impl View for ThemedView {
    fn body(self, env: &Environment) -> impl View {
        // Provide a default theme if none exists
        let theme = env.get::<Theme>()
            .cloned()
            .unwrap_or_else(|| Theme::default());
            
        text("Themed content")
            .color(theme.primary_color)
            .background(theme.background_color)
    }
}
```

## Common Environment Patterns

### 1. Application Theming

One of the most common uses for Environment is theming:

```rust,ignore
#[derive(Debug, Clone)]
pub struct AppTheme {
    pub primary_color: Color,
    pub secondary_color: Color,
    pub background_color: Color,
    pub text_color: Color,
    pub accent_color: Color,
}

impl AppTheme {
    pub fn light() -> Self {
        Self {
            primary_color: Color::rgb(0.0, 0.3, 0.8),
            secondary_color: Color::rgb(0.5, 0.5, 0.5),
            background_color: Color::white(),
            text_color: Color::black(),
            accent_color: Color::rgb(1.0, 0.3, 0.3),
        }
    }
    
    pub fn dark() -> Self {
        Self {
            primary_color: Color::rgb(0.3, 0.6, 1.0),
            secondary_color: Color::rgb(0.7, 0.7, 0.7),
            background_color: Color::rgb(0.1, 0.1, 0.1),
            text_color: Color::white(),
            accent_color: Color::rgb(1.0, 0.5, 0.5),
        }
    }
}

// Themed button component
struct ThemedButton {
    text: String,
    style: ButtonTheme,
}

#[derive(Debug, Clone)]
enum ButtonTheme {
    Primary,
    Secondary,
    Accent,
}

impl View for ThemedButton {
    fn body(self, env: &Environment) -> impl View {
        let theme = env.get::<AppTheme>()
            .unwrap_or(&AppTheme::light());
            
        let (bg_color, text_color) = match self.style {
            ButtonTheme::Primary => (theme.primary_color, Color::white()),
            ButtonTheme::Secondary => (theme.secondary_color, theme.text_color),
            ButtonTheme::Accent => (theme.accent_color, Color::white()),
        };
        
        button(self.text)
            .background(bg_color)
            .color(text_color)
            .corner_radius(8.0)
            .padding_horizontal(16.0)
            .padding_vertical(8.0)
    }
}

// Usage with theme switching
struct App;

impl View for App {
    fn body(self, _env: &Environment) -> impl View {
        let is_dark_mode = binding(false);
        
        let theme = is_dark_mode.get().map(|&dark| {
            if dark { AppTheme::dark() } else { AppTheme::light() }
        });
        
        theme.map(|theme| {
            // Create new environment with current theme
            let env = Environment::new().with(theme);
            
            // All child views will use this theme
            vstack((
                toggle(is_dark_mode.clone())
                    .label("Dark Mode"),
                
                ThemedButton {
                    text: "Primary".to_string(),
                    style: ButtonTheme::Primary,
                },
                
                ThemedButton {
                    text: "Secondary".to_string(),
                    style: ButtonTheme::Secondary,
                },
            ))
            .environment(env)  // Apply environment to children
        })
    }
}
```

### 2. Dependency Injection

Environment is perfect for injecting services and dependencies:

```rust,ignore
// Define service trait
trait UserService: Send + Sync + 'static {
    fn get_user(&self, id: u64) -> Result<User, Error>;
    fn update_user(&self, user: &User) -> Result<(), Error>;
}

// HTTP implementation
struct HttpUserService {
    base_url: String,
    client: HttpClient,
}

impl UserService for HttpUserService {
    fn get_user(&self, id: u64) -> Result<User, Error> {
        // HTTP implementation
        todo!()
    }
    
    fn update_user(&self, user: &User) -> Result<(), Error> {
        // HTTP implementation
        todo!()
    }
}

// Mock implementation for testing
struct MockUserService {
    users: Vec<User>,
}

impl UserService for MockUserService {
    fn get_user(&self, id: u64) -> Result<User, Error> {
        self.users.iter()
            .find(|u| u.id == id)
            .cloned()
            .ok_or_else(|| Error::new("User not found"))
    }
    
    fn update_user(&self, user: &User) -> Result<(), Error> {
        // Mock implementation
        Ok(())
    }
}

// View that uses the service
struct UserProfile {
    user_id: u64,
}

impl View for UserProfile {
    fn body(self, env: &Environment) -> impl View {
        // Get service from environment
        let user_service = env.get::<Arc<dyn UserService>>()
            .expect("UserService must be provided");
            
        let user_state = binding(None::<User>);
        let loading = binding(true);
        
        // Load user data
        let user_service_clone = user_service.clone();
        let user_state_clone = user_state.clone();
        let loading_clone = loading.clone();
        
        task::spawn(async move {
            match user_service_clone.get_user(self.user_id).await {
                Ok(user) => {
                    user_state_clone.set(Some(user));
                    loading_clone.set(false);
                }
                Err(_) => {
                    loading_clone.set(false);
                    // Handle error
                }
            }
        });
        
        // UI based on loading state
        loading.get().map(move |&loading| {
            if loading {
                progress_indicator()
            } else {
                user_state.get().map(|user_opt| {
                    match user_opt {
                        Some(user) => user_details_view(user.clone()),
                        None => error_view("Failed to load user"),
                    }
                })
            }
        })
    }
}

// App setup with dependency injection
struct App;

impl View for App {
    fn body(self, _env: &Environment) -> impl View {
        // In production
        let user_service = Arc::new(HttpUserService {
            base_url: "https://api.example.com".to_string(),
            client: HttpClient::new(),
        }) as Arc<dyn UserService>;
        
        // In tests, use mock
        // let user_service = Arc::new(MockUserService { users: test_users });
        
        let env = Environment::new()
            .with(user_service)
            .with(AppTheme::light());
            
        UserProfile { user_id: 1 }
            .environment(env)
    }
}
```

### 3. Localization and Internationalization

Environment is excellent for managing localization:

```rust,ignore
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Localizer {
    current_language: Language,
    translations: HashMap<String, HashMap<Language, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    Spanish,
    French,
    German,
}

impl Localizer {
    pub fn new(language: Language) -> Self {
        let mut translations = HashMap::new();
        
        // Add translations
        let mut welcome = HashMap::new();
        welcome.insert(Language::English, "Welcome".to_string());
        welcome.insert(Language::Spanish, "Bienvenido".to_string());
        welcome.insert(Language::French, "Bienvenue".to_string());
        welcome.insert(Language::German, "Willkommen".to_string());
        translations.insert("welcome".to_string(), welcome);
        
        let mut goodbye = HashMap::new();
        goodbye.insert(Language::English, "Goodbye".to_string());
        goodbye.insert(Language::Spanish, "Adiós".to_string());
        goodbye.insert(Language::French, "Au revoir".to_string());
        goodbye.insert(Language::German, "Auf Wiedersehen".to_string());
        translations.insert("goodbye".to_string(), goodbye);
        
        Self {
            current_language: language,
            translations,
        }
    }
    
    pub fn get(&self, key: &str) -> String {
        self.translations
            .get(key)
            .and_then(|translations| translations.get(&self.current_language))
            .cloned()
            .unwrap_or_else(|| format!("[{}]", key))
    }
    
    pub fn set_language(&mut self, language: Language) {
        self.current_language = language;
    }
}

// Localized text component
struct LocalizedText {
    key: String,
}

impl View for LocalizedText {
    fn body(self, env: &Environment) -> impl View {
        let localizer = env.get::<Localizer>()
            .unwrap_or(&Localizer::new(Language::English));
            
        text(localizer.get(&self.key))
    }
}

// Convenience function
fn localized(key: impl Into<String>) -> LocalizedText {
    LocalizedText { key: key.into() }
}

// Multi-language app
struct MultiLanguageApp;

impl View for MultiLanguageApp {
    fn body(self, _env: &Environment) -> impl View {
        let current_language = binding(Language::English);
        
        let localizer = current_language.get()
            .map(|&lang| Localizer::new(lang));
            
        localizer.map(|localizer| {
            let env = Environment::new().with(localizer);
            
            vstack((
                // Language selector
                hstack((
                    "Language:",
                    picker(current_language.clone(), vec![
                        (Language::English, "English"),
                        (Language::Spanish, "Español"),
                        (Language::French, "Français"),
                        (Language::German, "Deutsch"),
                    )),
                ))
                .spacing(10.0),
                
                // Localized content
                vstack((
                    localized("welcome")
                        .size(24.0)
                        .weight(.bold),
                    localized("goodbye")
                        .size(18.0),
                ))
                .spacing(10.0),
            ))
            .environment(env)
        })
    }
}
```

## Advanced Environment Techniques

### Environment Modifiers

Create views that modify the environment for their children:

```rust,ignore
struct WithTheme<V> {
    theme: AppTheme,
    content: V,
}

impl<V: View> View for WithTheme<V> {
    fn body(self, env: &Environment) -> impl View {
        let new_env = env.clone().with(self.theme);
        self.content.environment(new_env)
    }
}

// Helper function
fn with_theme<V: View>(theme: AppTheme, content: V) -> WithTheme<V> {
    WithTheme { theme, content }
}

// Usage
with_theme(
    AppTheme::dark(),
    my_content_view()
)
```

### Environment Observers

Views that react to environment changes:

```rust,ignore
struct EnvironmentWatcher<T, V> {
    content: V,
    on_change: Box<dyn Fn(&T) + 'static>,
    _marker: PhantomData<T>,
}

impl<T: Clone + PartialEq + 'static, V: View> View for EnvironmentWatcher<T, V> {
    fn body(self, env: &Environment) -> impl View {
        if let Some(value) = env.get::<T>() {
            (self.on_change)(value);
        }
        self.content
    }
}

// Usage
EnvironmentWatcher {
    content: my_view(),
    on_change: Box::new(|theme: &AppTheme| {
        println!("Theme changed: {:?}", theme);
    }),
    _marker: PhantomData::<AppTheme>,
}
```

### Conditional Environment Values

Provide different values based on conditions:

```rust,ignore
struct ConditionalEnvironment<V> {
    condition: bool,
    true_env: Environment,
    false_env: Environment,
    content: V,
}

impl<V: View> View for ConditionalEnvironment<V> {
    fn body(self, base_env: &Environment) -> impl View {
        let env = if self.condition {
            base_env.clone().merge(self.true_env)
        } else {
            base_env.clone().merge(self.false_env)
        };
        
        self.content.environment(env)
    }
}

// Usage in development vs production
ConditionalEnvironment {
    condition: cfg!(debug_assertions),
    true_env: Environment::new()
        .with(LogLevel::Debug)
        .with(ApiEndpoint::Development),
    false_env: Environment::new()
        .with(LogLevel::Info)
        .with(ApiEndpoint::Production),
    content: app_content(),
}
```

## Environment Best Practices

### 1. Use Specific Types

Don't store generic types directly. Create wrapper types:

```rust,ignore
// Good: Specific, meaningful types
#[derive(Debug, Clone)]
pub struct ApiEndpoint(pub String);

#[derive(Debug, Clone)]
pub struct DatabaseUrl(pub String);

#[derive(Debug, Clone)]
pub struct MaxRetries(pub u32);

// Avoid: Generic types that could conflict
// env.with("https://api.example.com".to_string());  // Which string?
// env.with(5u32);  // What does 5 represent?
```

### 2. Document Dependencies

Clearly document what environment values your views expect:

```rust,ignore
/// A user profile component.
/// 
/// ## Environment Dependencies
/// 
/// - `UserService`: Required for loading user data
/// - `AppTheme`: Optional, defaults to light theme
/// - `Localizer`: Optional, defaults to English
struct UserProfile {
    user_id: u64,
}
```

### 3. Provide Defaults

Always handle missing environment values gracefully:

```rust,ignore
impl View for MyView {
    fn body(self, env: &Environment) -> impl View {
        // Get with default
        let theme = env.get::<AppTheme>()
            .cloned()
            .unwrap_or_default();
            
        // Or use a more specific default
        let config = env.get::<AppConfig>()
            .cloned()
            .unwrap_or_else(|| AppConfig::development());
            
        // Use the values...
    }
}
```

### 4. Keep Environment Minimal

Only store values that multiple views need:

```rust,ignore
// Good: Shared configuration
env.with(AppTheme::default())
   .with(ApiConfig::production())
   .with(Localizer::new(Language::English))

// Avoid: Local component state
env.with(ButtonClickCount(0))  // This should be local binding
   .with(CurrentTab(0))        // This should be local state
```

### 5. Use Type Aliases for Complex Types

```rust,ignore
type UserServiceRef = Arc<dyn UserService>;
type ConfigServiceRef = Arc<dyn ConfigService>;

// Much cleaner than Arc<dyn UserService> everywhere
let env = Environment::new()
    .with(user_service as UserServiceRef)
    .with(config_service as ConfigServiceRef);
```

## Testing with Environment

Environment makes testing much easier:

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_user_profile_loading() {
        // Create mock service
        let mock_service = Arc::new(MockUserService {
            users: vec![
                User { id: 1, name: "Test User".to_string() },
            ],
        }) as Arc<dyn UserService>;
        
        // Create test environment
        let env = Environment::new()
            .with(mock_service)
            .with(AppTheme::light())
            .with(Localizer::new(Language::English));
            
        // Test the view
        let view = UserProfile { user_id: 1 };
        
        // In a real test framework, you'd render and verify
        let rendered = view.body(&env);
        
        // Assertions would go here
    }
    
    #[test]
    fn test_theming() {
        let dark_env = Environment::new()
            .with(AppTheme::dark());
            
        let light_env = Environment::new()
            .with(AppTheme::light());
            
        let view = ThemedButton {
            text: "Test".to_string(),
            style: ButtonTheme::Primary,
        };
        
        // Test with both themes
        let dark_rendered = view.clone().body(&dark_env);
        let light_rendered = view.body(&light_env);
        
        // Verify different styling
    }
}
```

## Plugin System Integration

Environment integrates with WaterUI's plugin system:

```rust,ignore
trait Plugin: Sized + 'static {
    fn install(self, env: &mut Environment);
}

// Theme plugin
struct ThemePlugin {
    theme: AppTheme,
}

impl Plugin for ThemePlugin {
    fn install(self, env: &mut Environment) {
        env.insert(self.theme);
    }
}

// Analytics plugin
struct AnalyticsPlugin {
    service: Arc<dyn AnalyticsService>,
}

impl Plugin for AnalyticsPlugin {
    fn install(self, env: &mut Environment) {
        env.insert(self.service);
    }
}

// App with plugins
let env = Environment::new()
    .install(ThemePlugin { theme: AppTheme::dark() })
    .install(AnalyticsPlugin { service: analytics })
    .install(LocalizationPlugin::new(Language::Spanish));
```

## Summary

The Environment system provides:

- ✅ **Type-safe dependency injection** without prop drilling
- ✅ **Automatic inheritance** through the view hierarchy
- ✅ **Plugin integration** for cross-cutting concerns
- ✅ **Easy testing** through dependency substitution
- ✅ **Performance** with efficient cloning and lookup

Key patterns:
- Use specific types as environment keys
- Always provide sensible defaults
- Document environment dependencies
- Keep the environment focused on shared concerns
- Leverage environment for testing with mocks

In the next chapter, we'll dive deep into reactive state management and learn how to build complex, interactive applications that respond automatically to data changes.

Ready to master reactivity? Let's explore [Reactive State Management](05-state.md)!