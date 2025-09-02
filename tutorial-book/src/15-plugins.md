# Plugin Development

WaterUI's plugin system allows you to extend the framework with custom functionality, integrate third-party libraries, and create reusable components that can be shared across projects. The plugin architecture is built on Rust's powerful type system and trait-based approach.

## Understanding WaterUI Plugins

WaterUI plugins are packages that extend the framework's capabilities by:
- Adding new view types
- Providing backend-specific implementations
- Integrating with external systems
- Offering utility functions and helpers

### Plugin Architecture

```rust,ignore
use waterui::*;

// Base trait for all WaterUI plugins
pub trait WaterUIPlugin {
    /// Plugin name for identification
    fn name(&self) -> &'static str;
    
    /// Plugin version
    fn version(&self) -> &'static str;
    
    /// Initialize the plugin
    fn initialize(&self, app: &mut Application) -> Result<(), PluginError>;
    
    /// Clean up resources when the plugin is unloaded
    fn cleanup(&self) -> Result<(), PluginError>;
    
    /// Get plugin dependencies
    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
    }
}

#[derive(Debug)]
pub enum PluginError {
    InitializationFailed(String),
    DependencyMissing(String),
    ConfigurationError(String),
    RuntimeError(String),
}
```

## Creating Your First Plugin

Let's create a simple plugin that adds a custom gradient button:

### Basic Plugin Structure

```rust,ignore
// src/gradient_button_plugin.rs
use waterui::*;
use std::collections::HashMap;

pub struct GradientButtonPlugin {
    config: GradientButtonConfig,
}

#[derive(Debug, Clone)]
pub struct GradientButtonConfig {
    pub default_colors: Vec<Color>,
    pub animation_duration: Duration,
    pub corner_radius: f32,
}

impl Default for GradientButtonConfig {
    fn default() -> Self {
        Self {
            default_colors: vec![Color::blue(), Color::purple()],
            animation_duration: Duration::from_millis(200),
            corner_radius: 8.0,
        }
    }
}

impl GradientButtonPlugin {
    pub fn new() -> Self {
        Self {
            config: GradientButtonConfig::default(),
        }
    }
    
    pub fn with_config(config: GradientButtonConfig) -> Self {
        Self { config }
    }
}

impl WaterUIPlugin for GradientButtonPlugin {
    fn name(&self) -> &'static str {
        "gradient_button"
    }
    
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    
    fn initialize(&self, app: &mut Application) -> Result<(), PluginError> {
        // Register the gradient button component
        app.register_component::<GradientButton>();
        
        // Add plugin-specific environment values
        app.environment_mut().insert(self.config.clone());
        
        println!("GradientButton plugin initialized");
        Ok(())
    }
    
    fn cleanup(&self) -> Result<(), PluginError> {
        println!("GradientButton plugin cleaned up");
        Ok(())
    }
}
```

### Custom View Component

```rust,ignore
// Custom gradient button view
pub struct GradientButton {
    title: String,
    colors: Vec<Color>,
    on_press: Option<Box<dyn Fn() + Send + Sync>>,
    disabled: bool,
    corner_radius: f32,
}

impl GradientButton {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            colors: vec![Color::blue(), Color::purple()],
            on_press: None,
            disabled: false,
            corner_radius: 8.0,
        }
    }
    
    pub fn colors(mut self, colors: Vec<Color>) -> Self {
        self.colors = colors;
        self
    }
    
    pub fn on_press<F>(mut self, action: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.on_press = Some(Box::new(action));
        self
    }
    
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }
}

impl View for GradientButton {
    fn body(self, env: &Environment) -> impl View {
        let config = env.get::<GradientButtonConfig>()
            .unwrap_or_default();
        
        let colors = if self.colors.len() >= 2 {
            self.colors
        } else {
            config.default_colors
        };
        
        let is_pressed = s!(false);
        let scale = is_pressed.map(|pressed| if *pressed { 0.95 } else { 1.0 });
        
        rectangle()
            .corner_radius(self.corner_radius)
            .gradient(LinearGradient::new()
                .colors(colors.clone())
                .start_point(Point::new(0.0, 0.0))
                .end_point(Point::new(1.0, 1.0)))
            .scale(scale.get())
            .animation(Animation::spring())
            .overlay(
                text(self.title)
                    .color(Color::white())
                    .font_weight(FontWeight::Semibold)
                    .padding(EdgeInsets::symmetric(16.0, 12.0))
            )
            .opacity(if self.disabled { 0.6 } else { 1.0 })
            .gesture(
                TapGesture::new()
                    .on_press_started(move |_| {
                        if !self.disabled {
                            is_pressed.set(true);
                        }
                    })
                    .on_press_ended(move |_| {
                        is_pressed.set(false);
                        if !self.disabled {
                            if let Some(action) = &self.on_press {
                                action();
                            }
                        }
                    })
            )
    }
}

// Helper function for easier usage
pub fn gradient_button(title: impl Into<String>) -> GradientButton {
    GradientButton::new(title)
}
```

## Advanced Plugin Features

### Plugin Configuration System

```rust,ignore
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub enabled: bool,
    pub auto_load: bool,
    pub settings: HashMap<String, serde_json::Value>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_load: true,
            settings: HashMap::new(),
        }
    }
}

pub struct ConfigurablePlugin {
    config: PluginConfig,
    config_path: String,
}

impl ConfigurablePlugin {
    pub fn new(config_path: impl Into<String>) -> Result<Self, PluginError> {
        let config_path = config_path.into();
        let config = Self::load_config(&config_path)?;
        
        Ok(Self {
            config,
            config_path,
        })
    }
    
    fn load_config(path: &str) -> Result<PluginConfig, PluginError> {
        match fs::read_to_string(path) {
            Ok(content) => {
                serde_json::from_str(&content)
                    .map_err(|e| PluginError::ConfigurationError(e.to_string()))
            }
            Err(_) => {
                // Create default config if file doesn't exist
                let config = PluginConfig::default();
                let _ = fs::write(path, serde_json::to_string_pretty(&config).unwrap());
                Ok(config)
            }
        }
    }
    
    pub fn save_config(&self) -> Result<(), PluginError> {
        let content = serde_json::to_string_pretty(&self.config)
            .map_err(|e| PluginError::ConfigurationError(e.to_string()))?;
        
        fs::write(&self.config_path, content)
            .map_err(|e| PluginError::ConfigurationError(e.to_string()))
    }
    
    pub fn update_setting<T>(&mut self, key: &str, value: T) -> Result<(), PluginError>
    where
        T: Serialize,
    {
        let json_value = serde_json::to_value(value)
            .map_err(|e| PluginError::ConfigurationError(e.to_string()))?;
        
        self.config.settings.insert(key.to_string(), json_value);
        self.save_config()
    }
    
    pub fn get_setting<T>(&self, key: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.config.settings.get(key)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}
```

### Plugin Manager

```rust,ignore
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct PluginManager {
    plugins: Arc<RwLock<HashMap<String, Box<dyn WaterUIPlugin + Send + Sync>>>>,
    load_order: Vec<String>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            load_order: Vec::new(),
        }
    }
    
    pub fn register_plugin(&mut self, plugin: Box<dyn WaterUIPlugin + Send + Sync>) -> Result<(), PluginError> {
        let name = plugin.name().to_string();
        
        // Check dependencies
        for dep in plugin.dependencies() {
            if !self.is_plugin_loaded(dep) {
                return Err(PluginError::DependencyMissing(dep.to_string()));
            }
        }
        
        // Add to plugins map
        {
            let mut plugins = self.plugins.write().unwrap();
            plugins.insert(name.clone(), plugin);
        }
        
        // Add to load order
        self.load_order.push(name);
        
        Ok(())
    }
    
    pub fn initialize_all(&self, app: &mut Application) -> Result<(), PluginError> {
        for plugin_name in &self.load_order {
            let plugins = self.plugins.read().unwrap();
            if let Some(plugin) = plugins.get(plugin_name) {
                plugin.initialize(app)?;
            }
        }
        Ok(())
    }
    
    pub fn is_plugin_loaded(&self, name: &str) -> bool {
        self.plugins.read().unwrap().contains_key(name)
    }
    
    pub fn unload_plugin(&mut self, name: &str) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.remove(name) {
            plugin.cleanup()?;
            self.load_order.retain(|n| n != name);
        }
        Ok(())
    }
    
    pub fn cleanup_all(&self) -> Result<(), PluginError> {
        let plugins = self.plugins.read().unwrap();
        for plugin in plugins.values() {
            plugin.cleanup()?;
        }
        Ok(())
    }
    
    pub fn list_plugins(&self) -> Vec<(String, String)> {
        let plugins = self.plugins.read().unwrap();
        plugins.values()
            .map(|plugin| (plugin.name().to_string(), plugin.version().to_string()))
            .collect()
    }
}

// Global plugin manager instance
lazy_static::lazy_static! {
    static ref PLUGIN_MANAGER: RwLock<PluginManager> = RwLock::new(PluginManager::new());
}

// Convenience functions
pub fn register_plugin(plugin: Box<dyn WaterUIPlugin + Send + Sync>) -> Result<(), PluginError> {
    PLUGIN_MANAGER.write().unwrap().register_plugin(plugin)
}

pub fn initialize_plugins(app: &mut Application) -> Result<(), PluginError> {
    PLUGIN_MANAGER.read().unwrap().initialize_all(app)
}
```

## Specialized Plugin Types

### Widget Library Plugin

```rust,ignore
// Example: Chart plugin providing various chart types
pub struct ChartPlugin;

impl WaterUIPlugin for ChartPlugin {
    fn name(&self) -> &'static str { "charts" }
    fn version(&self) -> &'static str { "1.0.0" }
    
    fn initialize(&self, app: &mut Application) -> Result<(), PluginError> {
        // Register chart components
        app.register_component::<LineChart>();
        app.register_component::<BarChart>();
        app.register_component::<PieChart>();
        Ok(())
    }
    
    fn cleanup(&self) -> Result<(), PluginError> { Ok(()) }
}

// Line chart component
pub struct LineChart {
    data: Vec<Point>,
    color: Color,
    stroke_width: f32,
    animated: bool,
}

impl LineChart {
    pub fn new(data: Vec<Point>) -> Self {
        Self {
            data,
            color: Color::blue(),
            stroke_width: 2.0,
            animated: true,
        }
    }
    
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
    
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = width;
        self
    }
    
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }
}

impl View for LineChart {
    fn body(self, _env: &Environment) -> impl View {
        let animation_progress = s!(if self.animated { 0.0 } else { 1.0 });
        
        // Animate chart drawing if enabled
        if self.animated && animation_progress.get() < 1.0 {
            let progress = animation_progress.clone();
            tokio::spawn(async move {
                while progress.get() < 1.0 {
                    progress.set((progress.get() + 0.02).min(1.0));
                    tokio::time::sleep(Duration::from_millis(16)).await;
                }
            });
        }
        
        canvas(move |ctx, size| {
            if self.data.is_empty() { return; }
            
            let progress = animation_progress.get();
            let visible_points = ((self.data.len() as f32) * progress) as usize;
            
            // Draw axes
            ctx.stroke_line(
                Point::new(0.0, size.height),
                Point::new(size.width, size.height),
                self.color.opacity(0.3),
                1.0
            );
            ctx.stroke_line(
                Point::new(0.0, 0.0),
                Point::new(0.0, size.height),
                self.color.opacity(0.3),
                1.0
            );
            
            // Draw data line
            if visible_points >= 2 {
                let mut path = Path::new();
                let first_point = scale_point(&self.data[0], size);
                path.move_to(first_point);
                
                for i in 1..visible_points.min(self.data.len()) {
                    let point = scale_point(&self.data[i], size);
                    path.line_to(point);
                }
                
                ctx.stroke_path(&path, self.color, self.stroke_width);
            }
        })
        .frame_size(Size::new(300.0, 200.0))
    }
}

fn scale_point(point: &Point, size: Size) -> Point {
    Point::new(
        point.x * size.width,
        size.height - (point.y * size.height)
    )
}

// Helper function
pub fn line_chart(data: Vec<Point>) -> LineChart {
    LineChart::new(data)
}
```

### Platform Integration Plugin

```rust,ignore
// Example: File system plugin for native file operations
pub struct FileSystemPlugin;

impl WaterUIPlugin for FileSystemPlugin {
    fn name(&self) -> &'static str { "filesystem" }
    fn version(&self) -> &'static str { "1.0.0" }
    
    fn initialize(&self, app: &mut Application) -> Result<(), PluginError> {
        // Register file system utilities in environment
        let fs_utils = FileSystemUtils::new();
        app.environment_mut().insert(fs_utils);
        Ok(())
    }
    
    fn cleanup(&self) -> Result<(), PluginError> { Ok(()) }
}

#[derive(Clone)]
pub struct FileSystemUtils;

impl FileSystemUtils {
    pub fn new() -> Self {
        Self
    }
    
    pub async fn pick_file(&self, extensions: Vec<&str>) -> Option<String> {
        // Platform-specific file picker implementation
        #[cfg(target_os = "windows")]
        {
            self.pick_file_windows(extensions).await
        }
        #[cfg(target_os = "macos")]
        {
            self.pick_file_macos(extensions).await
        }
        #[cfg(target_os = "linux")]
        {
            self.pick_file_linux(extensions).await
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.pick_file_web(extensions).await
        }
    }
    
    pub async fn save_file(&self, content: &str, default_name: &str) -> Result<(), String> {
        // Platform-specific file saving
        std::fs::write(default_name, content)
            .map_err(|e| e.to_string())
    }
    
    #[cfg(target_os = "windows")]
    async fn pick_file_windows(&self, _extensions: Vec<&str>) -> Option<String> {
        // Windows-specific implementation using win32 APIs
        None // Placeholder
    }
    
    #[cfg(target_os = "macos")]
    async fn pick_file_macos(&self, _extensions: Vec<&str>) -> Option<String> {
        // macOS-specific implementation using NSOpenPanel
        None // Placeholder
    }
    
    #[cfg(target_os = "linux")]
    async fn pick_file_linux(&self, _extensions: Vec<&str>) -> Option<String> {
        // Linux-specific implementation using GTK file chooser
        None // Placeholder
    }
    
    #[cfg(target_arch = "wasm32")]
    async fn pick_file_web(&self, _extensions: Vec<&str>) -> Option<String> {
        // Web-specific implementation using HTML5 file input
        None // Placeholder
    }
}

// File picker view component
pub fn file_picker(
    label: impl Into<String>,
    extensions: Vec<&'static str>,
    on_file_selected: impl Fn(String) + 'static,
) -> impl View {
    button(label)
        .on_press(move || {
            let extensions = extensions.clone();
            let callback = on_file_selected.clone();
            
            tokio::spawn(async move {
                if let Some(fs_utils) = Environment::global().get::<FileSystemUtils>() {
                    if let Some(file_path) = fs_utils.pick_file(extensions).await {
                        callback(file_path);
                    }
                }
            });
        })
}
```

### Backend-Specific Plugin

```rust,ignore
// Example: GTK4-specific plugin
#[cfg(feature = "gtk4")]
pub struct GTK4ExtensionsPlugin;

#[cfg(feature = "gtk4")]
impl WaterUIPlugin for GTK4ExtensionsPlugin {
    fn name(&self) -> &'static str { "gtk4_extensions" }
    fn version(&self) -> &'static str { "1.0.0" }
    
    fn initialize(&self, app: &mut Application) -> Result<(), PluginError> {
        // Register GTK4-specific components
        app.register_component::<NativeMenuBar>();
        app.register_component::<SystemTrayIcon>();
        Ok(())
    }
    
    fn cleanup(&self) -> Result<(), PluginError> { Ok(()) }
}

#[cfg(feature = "gtk4")]
pub struct NativeMenuBar {
    items: Vec<MenuItem>,
}

#[cfg(feature = "gtk4")]
impl NativeMenuBar {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }
    
    pub fn item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }
}

#[cfg(feature = "gtk4")]
impl View for NativeMenuBar {
    fn body(self, _env: &Environment) -> impl View {
        // GTK4-specific implementation
        empty() // Placeholder - would integrate with GTK4 menu system
    }
}

#[cfg(feature = "gtk4")]
pub struct MenuItem {
    title: String,
    action: Option<Box<dyn Fn() + Send + Sync>>,
    submenu: Vec<MenuItem>,
    enabled: bool,
}

#[cfg(feature = "gtk4")]
impl MenuItem {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            action: None,
            submenu: Vec::new(),
            enabled: true,
        }
    }
    
    pub fn action<F>(mut self, action: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.action = Some(Box::new(action));
        self
    }
    
    pub fn submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = items;
        self
    }
}
```

## Plugin Communication

### Event System for Plugins

```rust,ignore
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender, Receiver};

pub struct PluginEventSystem {
    channels: HashMap<String, Sender<PluginEvent>>,
}

#[derive(Debug, Clone)]
pub enum PluginEvent {
    CustomEvent { plugin: String, data: serde_json::Value },
    StateChanged { key: String, value: serde_json::Value },
    UserAction { action: String, context: serde_json::Value },
}

impl PluginEventSystem {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }
    
    pub fn register_listener(&mut self, plugin_name: String) -> Receiver<PluginEvent> {
        let (sender, receiver) = channel();
        self.channels.insert(plugin_name, sender);
        receiver
    }
    
    pub fn emit_event(&self, event: PluginEvent) {
        for sender in self.channels.values() {
            let _ = sender.send(event.clone());
        }
    }
    
    pub fn emit_to_plugin(&self, plugin_name: &str, event: PluginEvent) {
        if let Some(sender) = self.channels.get(plugin_name) {
            let _ = sender.send(event);
        }
    }
}

// Example plugin that listens for events
pub struct EventListenerPlugin {
    event_receiver: Option<Receiver<PluginEvent>>,
}

impl EventListenerPlugin {
    pub fn new() -> Self {
        Self {
            event_receiver: None,
        }
    }
    
    fn handle_events(&self) {
        if let Some(receiver) = &self.event_receiver {
            while let Ok(event) = receiver.try_recv() {
                match event {
                    PluginEvent::CustomEvent { plugin, data } => {
                        println!("Received custom event from {}: {:?}", plugin, data);
                    }
                    PluginEvent::StateChanged { key, value } => {
                        println!("State changed: {} = {:?}", key, value);
                    }
                    PluginEvent::UserAction { action, context } => {
                        println!("User action: {} with context {:?}", action, context);
                    }
                }
            }
        }
    }
}

impl WaterUIPlugin for EventListenerPlugin {
    fn name(&self) -> &'static str { "event_listener" }
    fn version(&self) -> &'static str { "1.0.0" }
    
    fn initialize(&self, app: &mut Application) -> Result<(), PluginError> {
        // Register for events
        let mut event_system = app.get_plugin_event_system();
        let receiver = event_system.register_listener(self.name().to_string());
        
        // Start event handling loop
        let self_clone = self.clone();
        tokio::spawn(async move {
            loop {
                self_clone.handle_events();
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        });
        
        Ok(())
    }
    
    fn cleanup(&self) -> Result<(), PluginError> { Ok(()) }
}
```

## Plugin Distribution and Packaging

### Plugin Manifest

```toml
# plugin.toml
[package]
name = "waterui-gradient-buttons"
version = "1.0.0"
authors = ["Your Name <your.email@example.com>"]
description = "Beautiful gradient buttons for WaterUI"
license = "MIT OR Apache-2.0"
repository = "https://github.com/yourusername/waterui-gradient-buttons"

[plugin]
main_class = "GradientButtonPlugin"
min_waterui_version = "0.1.0"
max_waterui_version = "0.2.0"

[dependencies]
waterui-core = "0.1.0"
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["rt", "time"] }

[features]
default = ["animations"]
animations = []
gtk4 = ["waterui/gtk4"]
web = ["waterui/web"]

[[example]]
name = "gradient_button_demo"
path = "examples/demo.rs"
```

### Plugin Loading System

```rust,ignore
use std::path::Path;
use toml::Value;

pub struct PluginLoader {
    plugin_dirs: Vec<String>,
}

impl PluginLoader {
    pub fn new() -> Self {
        Self {
            plugin_dirs: vec![
                "plugins/".to_string(),
                "~/.waterui/plugins/".to_string(),
                "/usr/local/lib/waterui/plugins/".to_string(),
            ],
        }
    }
    
    pub fn discover_plugins(&self) -> Result<Vec<PluginManifest>, PluginError> {
        let mut plugins = Vec::new();
        
        for dir in &self.plugin_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let manifest_path = entry.path().join("plugin.toml");
                    if manifest_path.exists() {
                        match self.load_manifest(&manifest_path) {
                            Ok(manifest) => plugins.push(manifest),
                            Err(e) => eprintln!("Failed to load plugin manifest {:?}: {:?}", manifest_path, e),
                        }
                    }
                }
            }
        }
        
        Ok(plugins)
    }
    
    fn load_manifest(&self, path: &Path) -> Result<PluginManifest, PluginError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| PluginError::ConfigurationError(e.to_string()))?;
        
        let toml: Value = toml::from_str(&content)
            .map_err(|e| PluginError::ConfigurationError(e.to_string()))?;
        
        Ok(PluginManifest::from_toml(toml)?)
    }
    
    pub async fn load_plugin(&self, manifest: &PluginManifest) -> Result<Box<dyn WaterUIPlugin + Send + Sync>, PluginError> {
        // In a real implementation, this would use dynamic loading (libloading crate)
        // or compile-time registration of plugins
        
        match manifest.name.as_str() {
            "gradient-buttons" => Ok(Box::new(GradientButtonPlugin::new())),
            "charts" => Ok(Box::new(ChartPlugin)),
            "filesystem" => Ok(Box::new(FileSystemPlugin)),
            _ => Err(PluginError::InitializationFailed(
                format!("Unknown plugin: {}", manifest.name)
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub main_class: String,
    pub min_waterui_version: String,
    pub max_waterui_version: String,
    pub dependencies: Vec<String>,
    pub features: Vec<String>,
}

impl PluginManifest {
    pub fn from_toml(toml: Value) -> Result<Self, PluginError> {
        // Parse TOML manifest into PluginManifest struct
        let package = toml.get("package")
            .ok_or_else(|| PluginError::ConfigurationError("Missing [package] section".to_string()))?;
        
        let plugin = toml.get("plugin")
            .ok_or_else(|| PluginError::ConfigurationError("Missing [plugin] section".to_string()))?;
        
        Ok(Self {
            name: package.get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::ConfigurationError("Missing package name".to_string()))?
                .to_string(),
            version: package.get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.1.0")
                .to_string(),
            description: package.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            authors: package.get("authors")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            main_class: plugin.get("main_class")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::ConfigurationError("Missing main_class".to_string()))?
                .to_string(),
            min_waterui_version: plugin.get("min_waterui_version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.1.0")
                .to_string(),
            max_waterui_version: plugin.get("max_waterui_version")
                .and_then(|v| v.as_str())
                .unwrap_or("1.0.0")
                .to_string(),
            dependencies: Vec::new(), // Would parse from [dependencies] section
            features: Vec::new(),     // Would parse from [features] section
        })
    }
}
```

## Using Plugins in Applications

### Application with Plugin Support

```rust,ignore
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = Application::new("Plugin Demo");
    
    // Initialize plugin system
    let mut plugin_manager = PluginManager::new();
    
    // Register plugins
    plugin_manager.register_plugin(Box::new(GradientButtonPlugin::new()))?;
    plugin_manager.register_plugin(Box::new(ChartPlugin))?;
    plugin_manager.register_plugin(Box::new(FileSystemPlugin))?;
    
    // Initialize all plugins
    plugin_manager.initialize_all(&mut app)?;
    
    // Set the root view
    app.set_root_view(plugin_demo_view());
    
    // Run the application
    app.run()
}

fn plugin_demo_view() -> impl View {
    vstack((
        text("WaterUI Plugin Demo")
            .font_size(24.0)
            .font_weight(FontWeight::Bold),
        
        // Using the gradient button from the plugin
        gradient_button("Gradient Button")
            .colors(vec![Color::red(), Color::orange(), Color::yellow()))
            .on_press(|| println!("Gradient button pressed!")),
        
        // Using the chart from the chart plugin
        line_chart(vec![
            Point::new(0.0, 0.2),
            Point::new(0.2, 0.5),
            Point::new(0.4, 0.3),
            Point::new(0.6, 0.8),
            Point::new(0.8, 0.6),
            Point::new(1.0, 0.9),
        ))
        .color(Color::blue()),
        
        // Using the file picker from the filesystem plugin
        file_picker(
            "Choose File",
            vec!["txt", "md", "rs"],
            |file_path| println!("Selected file: {}", file_path)
        ),
    ))
    .spacing(20.0)
    .padding(20.0)
}
```

## Best Practices

### Plugin Development Guidelines

1. **Follow Naming Conventions**: Use descriptive, unique names for plugins
2. **Version Compatibility**: Clearly specify supported WaterUI versions
3. **Error Handling**: Provide meaningful error messages and graceful failures
4. **Documentation**: Include comprehensive documentation and examples
5. **Testing**: Write unit tests and integration tests for your plugins
6. **Performance**: Optimize plugin initialization and runtime performance

### Security Considerations

```rust,ignore
pub struct PluginSecurity {
    allowed_operations: HashSet<String>,
    sandbox_enabled: bool,
}

impl PluginSecurity {
    pub fn new() -> Self {
        Self {
            allowed_operations: HashSet::new(),
            sandbox_enabled: true,
        }
    }
    
    pub fn allow_operation(&mut self, operation: &str) {
        self.allowed_operations.insert(operation.to_string());
    }
    
    pub fn check_permission(&self, operation: &str) -> bool {
        !self.sandbox_enabled || self.allowed_operations.contains(operation)
    }
}
```

### Plugin Testing Framework

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_gradient_button_plugin() {
        let plugin = GradientButtonPlugin::new();
        let mut app = Application::test_instance();
        
        // Test plugin initialization
        assert!(plugin.initialize(&mut app).is_ok());
        
        // Test component registration
        assert!(app.is_component_registered::<GradientButton>());
        
        // Test plugin cleanup
        assert!(plugin.cleanup().is_ok());
    }
    
    #[test]
    fn test_gradient_button_component() {
        let button = gradient_button("Test")
            .colors(vec![Color::red(), Color::blue()));
        
        // Test that the component renders without panicking
        let rendered = button.render_for_testing();
        assert!(!rendered.is_empty());
    }
}
```

The WaterUI plugin system provides a powerful foundation for extending the framework with custom functionality while maintaining type safety and performance. By following these patterns and best practices, you can create robust, reusable plugins that enhance the WaterUI ecosystem.