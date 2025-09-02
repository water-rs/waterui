# API Reference

This chapter provides a comprehensive reference for all WaterUI APIs, from core traits and types to component-specific functions and utilities.

## Core APIs

### View Trait

The fundamental trait that all UI components implement.

```rust,ignore
pub trait View {
    type Body: View;
    
    /// Returns the body content of this view
    fn body(self) -> Self::Body;
    
    /// Converts this view into a type-erased AnyView
    fn into_any(self) -> AnyView 
    where 
        Self: Sized + 'static 
    {
        AnyView::new(self)
    }
}
```

#### Associated Functions

- `body(self) -> Self::Body` - Returns the view's content
- `into_any(self) -> AnyView` - Type erasure for heterogeneous collections

#### Implementation Examples

```rust,ignore
// Function-based view (automatic View implementation)
fn hello_world() -> impl View {
    text!("Hello, World!")
}

// Struct-based view (manual View implementation)
struct Counter {
    value: Binding<i32>,
}

impl View for Counter {
    fn body(self) -> impl View {
        vstack((
            text!("Count: {}", self.value),
            button("+1", {
                let value = self.value.clone();
                move || value.update(|v| *v += 1)
            }),
        ))
    }
}
```

### Environment System

```rust,ignore
/// Type-based dependency injection
pub struct Environment {
    // Internal storage for environment values
}

impl Environment {
    /// Creates a new empty environment
    pub fn new() -> Self { /* ... */ }
    
    /// Inserts a value into the environment
    pub fn insert<T: 'static>(self, value: T) -> Self { /* ... */ }
    
    /// Gets a value from the environment
    pub fn get<T: 'static>(&self) -> Option<&T> { /* ... */ }
    
    /// Gets a value or panics if not found
    pub fn expect<T: 'static>(&self) -> &T { /* ... */ }
}
```

#### Usage Examples

```rust,ignore
// Setting environment values
fn app_root() -> impl View {
    main_content()
        .environment(AppTheme::dark())
        .environment(ApiClient::new("https://api.example.com"))
        .environment(UserPreferences::load())
}

// Consuming environment values
fn themed_button(label: &str) -> impl View {
    button(label, || {})
        .environment_reader(|env: &Environment| {
            let theme = env.expect::<AppTheme>();
            button.background(theme.primary_color())
        })
}
```

### AnyView

Type erasure for heterogeneous view collections.

```rust,ignore
pub struct AnyView {
    // Internal type-erased storage
}

impl AnyView {
    /// Creates a new AnyView from any View
    pub fn new<V: View + 'static>(view: V) -> Self { /* ... */ }
    
    /// Creates an empty AnyView
    pub fn empty() -> Self { /* ... */ }
}

impl View for AnyView {
    fn body(self) -> impl View {
        // Returns the wrapped view
    }
}
```

#### Usage Examples

```rust,ignore
// Heterogeneous collections
fn dynamic_content(items: Vec<ContentType>) -> impl View {
    let views: Vec<AnyView> = items.into_iter().map(|item| {
        match item {
            ContentType::Text(text) => text!(text).into_any(),
            ContentType::Image(url) => image(url).into_any(),
            ContentType::Button(label) => button(label, || {}).into_any(),
        }
    }).collect();
    
    vstack(views)
}
```

## Layout Components

### VStack

Vertical stack layout container.

```rust,ignore
/// Creates a vertical stack of views
pub fn vstack<V: View>(views: impl IntoIterator<Item = V>) -> VStack<V> {
    VStack::new(views)
}

pub struct VStack<V> {
    views: Vec<V>,
    spacing: f64,
    alignment: HorizontalAlignment,
}

impl<V> VStack<V> {
    pub fn spacing(mut self, spacing: f64) -> Self {
        self.spacing = spacing;
        self
    }
    
    pub fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.alignment = alignment;
        self
    }
}
```

#### Usage Examples

```rust,ignore
// Basic vertical stack
vstack((
    text!("Title"),
    text!("Subtitle"),
    button("Action", || {}),
))

// With custom spacing and alignment
vstack((
    text!("Centered Title"),
    text!("Centered Subtitle"),
))
.spacing(20.0)
.alignment(HorizontalAlignment::Center)
```

### HStack

Horizontal stack layout container.

```rust,ignore
/// Creates a horizontal stack of views
pub fn hstack<V: View>(views: impl IntoIterator<Item = V>) -> HStack<V> {
    HStack::new(views)
}

pub struct HStack<V> {
    views: Vec<V>,
    spacing: f64,
    alignment: VerticalAlignment,
}

impl<V> HStack<V> {
    pub fn spacing(mut self, spacing: f64) -> Self { /* ... */ }
    pub fn alignment(mut self, alignment: VerticalAlignment) -> Self { /* ... */ }
}
```

### ZStack

Depth-based (z-axis) stack layout.

```rust,ignore
/// Creates a depth stack of overlapping views
pub fn zstack<V: View>(views: impl IntoIterator<Item = V>) -> ZStack<V> {
    ZStack::new(views)
}

pub struct ZStack<V> {
    views: Vec<V>,
    alignment: Alignment,
}

impl<V> ZStack<V> {
    pub fn alignment(mut self, alignment: Alignment) -> Self { /* ... */ }
}
```

### Grid

Grid layout with rows and columns.

```rust,ignore
/// Creates a grid layout
pub fn grid<V: View>(
    views: impl IntoIterator<Item = V>,
    columns: usize,
) -> Grid<V> {
    Grid::new(views, columns)
}

pub struct Grid<V> {
    views: Vec<V>,
    columns: usize,
    spacing: (f64, f64), // (horizontal, vertical)
    alignment: Alignment,
}

impl<V> Grid<V> {
    pub fn spacing(mut self, horizontal: f64, vertical: f64) -> Self { /* ... */ }
    pub fn alignment(mut self, alignment: Alignment) -> Self { /* ... */ }
}
```

#### Usage Examples

```rust,ignore
// Photo grid
let photos = vec!["photo1.jpg", "photo2.jpg", "photo3.jpg", "photo4.jpg"];
grid(photos.into_iter().map(|url| image(url)), 2)
    .spacing(10.0, 10.0)
    .alignment(Alignment::center())
```

## Text Components

### Text

Basic text display component.

```rust,ignore
/// Creates a text view with static content
pub fn text(content: impl Into<String>) -> Text {
    Text::new(content.into())
}

/// Creates a reactive text view (macro)
macro_rules! text {
    ($fmt:literal $(, $arg:expr)*) => {
        Text::reactive(s!(format!($fmt, $($arg),*)))
    };
}

pub struct Text {
    content: String,
    font_size: Option<f64>,
    font_weight: Option<FontWeight>,
    color: Option<Color>,
    alignment: Option<TextAlignment>,
}

impl Text {
    pub fn font_size(mut self, size: f64) -> Self { /* ... */ }
    pub fn font_weight(mut self, weight: FontWeight) -> Self { /* ... */ }
    pub fn color(mut self, color: Color) -> Self { /* ... */ }
    pub fn alignment(mut self, alignment: TextAlignment) -> Self { /* ... */ }
    pub fn line_limit(mut self, limit: Option<usize>) -> Self { /* ... */ }
}
```

#### Enums

```rust,ignore
#[derive(Clone, Copy, Debug)]
pub enum FontWeight {
    Thin,
    ExtraLight,
    Light,
    Regular,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Heavy,
}

#[derive(Clone, Copy, Debug)]
pub enum TextAlignment {
    Leading,
    Center,
    Trailing,
    Justified,
}
```

#### Usage Examples

```rust,ignore
// Static text
text("Hello, World!")
    .font_size(24.0)
    .font_weight(FontWeight::Bold)
    .color(Color::blue())

// Reactive text
let name = binding("Alice".to_string());
text!("Welcome, {}!", name)
    .font_size(18.0)
    .color(Color::green())
```

### TextField

Text input component.

```rust,ignore
/// Creates a text input field
pub fn text_field(
    placeholder: impl Into<String>,
    binding: Binding<String>,
) -> TextField {
    TextField::new(placeholder.into(), binding)
}

pub struct TextField {
    placeholder: String,
    binding: Binding<String>,
    is_secure: bool,
    is_disabled: bool,
    validation: Option<Box<dyn Fn(&str) -> Option<String>>>,
}

impl TextField {
    pub fn secure(mut self, secure: bool) -> Self { /* ... */ }
    pub fn disabled(mut self, disabled: bool) -> Self { /* ... */ }
    pub fn validation<F>(mut self, validator: F) -> Self 
    where
        F: Fn(&str) -> Option<String> + 'static
    { /* ... */ }
    pub fn on_submit<F>(mut self, handler: F) -> Self 
    where
        F: Fn(String) + 'static
    { /* ... */ }
}
```

#### Usage Examples

```rust,ignore
// Basic text field
let username = binding(String::new());
text_field("Enter username", username.clone())

// Password field with validation
let password = binding(String::new());
text_field("Enter password", password.clone())
    .secure(true)
    .validation(|text| {
        if text.len() < 8 {
            Some("Password must be at least 8 characters".to_string())
        } else {
            None
        }
    })
```

## Form Components

### Button

Clickable button component.

```rust,ignore
/// Creates a button with label and action
pub fn button<F>(label: impl Into<String>, action: F) -> Button 
where
    F: Fn() + 'static,
{
    Button::new(label.into(), Box::new(action))
}

pub struct Button {
    label: String,
    action: Box<dyn Fn()>,
    style: ButtonStyle,
    is_disabled: bool,
    background_color: Option<Color>,
    foreground_color: Option<Color>,
}

impl Button {
    pub fn style(mut self, style: ButtonStyle) -> Self { /* ... */ }
    pub fn disabled(mut self, disabled: bool) -> Self { /* ... */ }
    pub fn background(mut self, color: Color) -> Self { /* ... */ }
    pub fn foreground(mut self, color: Color) -> Self { /* ... */ }
}
```

#### Enums

```rust,ignore
#[derive(Clone, Copy, Debug)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Destructive,
    Borderless,
}
```

### Toggle

Toggle/checkbox component.

```rust,ignore
/// Creates a toggle switch
pub fn toggle(
    label: impl Into<String>,
    binding: Binding<bool>,
) -> Toggle {
    Toggle::new(label.into(), binding)
}

pub struct Toggle {
    label: String,
    binding: Binding<bool>,
    is_disabled: bool,
}

impl Toggle {
    pub fn disabled(mut self, disabled: bool) -> Self { /* ... */ }
}
```

### Slider

Numeric slider component.

```rust,ignore
/// Creates a numeric slider
pub fn slider(
    binding: Binding<f64>,
    range: std::ops::RangeInclusive<f64>,
) -> Slider {
    Slider::new(binding, range)
}

pub struct Slider {
    binding: Binding<f64>,
    range: std::ops::RangeInclusive<f64>,
    step: Option<f64>,
    is_disabled: bool,
}

impl Slider {
    pub fn step(mut self, step: f64) -> Self { /* ... */ }
    pub fn disabled(mut self, disabled: bool) -> Self { /* ... */ }
}
```

### Picker

Selection picker component.

```rust,ignore
/// Creates a selection picker
pub fn picker<T>(
    label: impl Into<String>,
    selection: Binding<T>,
    options: Vec<T>,
) -> Picker<T> 
where
    T: Clone + PartialEq + Display,
{
    Picker::new(label.into(), selection, options)
}

pub struct Picker<T> {
    label: String,
    selection: Binding<T>,
    options: Vec<T>,
    is_disabled: bool,
}

impl<T> Picker<T> {
    pub fn disabled(mut self, disabled: bool) -> Self { /* ... */ }
}
```

## Media Components

### Image

Image display component.

```rust,ignore
/// Creates an image view
pub fn image(source: impl Into<ImageSource>) -> Image {
    Image::new(source.into())
}

pub struct Image {
    source: ImageSource,
    content_mode: ContentMode,
    placeholder: Option<Box<dyn View>>,
    error_view: Option<Box<dyn View>>,
}

impl Image {
    pub fn content_mode(mut self, mode: ContentMode) -> Self { /* ... */ }
    pub fn placeholder<V: View + 'static>(mut self, view: V) -> Self { /* ... */ }
    pub fn on_error<V: View + 'static>(mut self, view: V) -> Self { /* ... */ }
    pub fn on_load<F>(mut self, handler: F) -> Self 
    where
        F: Fn() + 'static
    { /* ... */ }
}
```

#### Enums and Types

```rust,ignore
#[derive(Clone, Debug)]
pub enum ImageSource {
    Url(String),
    Path(PathBuf),
    Data(Vec<u8>),
    Asset(String),
}

#[derive(Clone, Copy, Debug)]
pub enum ContentMode {
    Fill,
    Fit,
    Stretch,
    Center,
}

impl From<&str> for ImageSource {
    fn from(s: &str) -> Self {
        if s.starts_with("http") {
            ImageSource::Url(s.to_string())
        } else {
            ImageSource::Path(PathBuf::from(s))
        }
    }
}
```

## Navigation Components

### NavigationView

Navigation container with stack-based navigation.

```rust,ignore
/// Creates a navigation view
pub fn navigation_view<V: View>(root: V) -> NavigationView<V> {
    NavigationView::new(root)
}

pub struct NavigationView<V> {
    root: V,
    navigation_stack: Binding<Vec<AnyView>>,
    title: Option<String>,
}

impl<V> NavigationView<V> {
    pub fn title(mut self, title: impl Into<String>) -> Self { /* ... */ }
}
```

### TabView

Tabbed interface component.

```rust,ignore
/// Creates a tab view
pub fn tab_view(tabs: Vec<Tab>) -> TabView {
    TabView::new(tabs)
}

pub struct Tab {
    pub title: String,
    pub content: AnyView,
    pub icon: Option<String>,
    pub badge: Option<String>,
}

pub struct TabView {
    tabs: Vec<Tab>,
    selected_tab: Binding<usize>,
    tab_placement: TabPlacement,
}

#[derive(Clone, Copy, Debug)]
pub enum TabPlacement {
    Top,
    Bottom,
    Leading,
    Trailing,
}
```

## Reactive State APIs

### Binding

Mutable reactive state container.

```rust,ignore
/// Creates a new binding with initial value
pub fn binding<T>(initial: T) -> Binding<T> {
    Binding::new(initial)
}

pub struct Binding<T> {
    // Internal reactive state
}

impl<T: Clone> Binding<T> {
    /// Gets the current value
    pub fn get(&self) -> T { /* ... */ }
    
    /// Sets a new value
    pub fn set(&self, value: T) { /* ... */ }
    
    /// Updates the value with a function
    pub fn update<F>(&self, updater: F) 
    where
        F: FnOnce(&mut T),
    { /* ... */ }
    
    /// Creates a computed value based on this binding
    pub fn map<U, F>(&self, mapper: F) -> Computed<U>
    where
        F: Fn(&T) -> U + 'static,
        U: Clone,
    { /* ... */ }
}
```

### Computed

Read-only computed reactive value.

```rust,ignore
pub struct Computed<T> {
    // Internal computed state
}

impl<T: Clone> Computed<T> {
    /// Gets the current computed value
    pub fn get(&self) -> T { /* ... */ }
    
    /// Creates a new computed value based on this one
    pub fn map<U, F>(&self, mapper: F) -> Computed<U>
    where
        F: Fn(&T) -> U + 'static,
        U: Clone,
    { /* ... */ }
}
```

### Signal Macro

The `s!` macro for creating computed signals.

```rust,ignore
/// Creates a computed signal
macro_rules! s {
    ($expr:expr) => {
        // Creates a Computed<T> from the expression
        // Automatically tracks dependencies
    };
}
```

## Color System

### Color

Color representation and manipulation.

```rust,ignore
pub struct Color {
    // Internal color representation
}

impl Color {
    /// Creates a color from RGB values (0.0-1.0)
    pub fn rgb(r: f64, g: f64, b: f64) -> Self { /* ... */ }
    
    /// Creates a color from RGBA values (0.0-1.0)
    pub fn rgba(r: f64, g: f64, b: f64, a: f64) -> Self { /* ... */ }
    
    /// Creates a color from HSL values
    pub fn hsl(h: f64, s: f64, l: f64) -> Self { /* ... */ }
    
    /// Creates a color from hex string
    pub fn hex(hex: &str) -> Result<Self, ColorError> { /* ... */ }
    
    /// Predefined colors
    pub fn black() -> Self { /* ... */ }
    pub fn white() -> Self { /* ... */ }
    pub fn red() -> Self { /* ... */ }
    pub fn green() -> Self { /* ... */ }
    pub fn blue() -> Self { /* ... */ }
    pub fn yellow() -> Self { /* ... */ }
    pub fn orange() -> Self { /* ... */ }
    pub fn purple() -> Self { /* ... */ }
    pub fn pink() -> Self { /* ... */ }
    pub fn gray() -> Self { /* ... */ }
    pub fn clear() -> Self { /* ... */ }
    
    /// Color manipulation
    pub fn opacity(self, opacity: f64) -> Self { /* ... */ }
    pub fn lighten(self, amount: f64) -> Self { /* ... */ }
    pub fn darken(self, amount: f64) -> Self { /* ... */ }
    pub fn saturate(self, amount: f64) -> Self { /* ... */ }
    pub fn desaturate(self, amount: f64) -> Self { /* ... */ }
}
```

## Styling APIs

### ViewModifier Trait

Trait for applying modifications to views.

```rust,ignore
pub trait ViewModifier {
    type Output: View;
    
    fn modify<V: View>(self, view: V) -> Self::Output;
}

// Extension trait for all views
pub trait ViewExt: View {
    fn modifier<M: ViewModifier>(self, modifier: M) -> M::Output {
        modifier.modify(self)
    }
    
    // Common modifiers
    fn padding(self, padding: f64) -> PaddingModifier<Self> { /* ... */ }
    fn background(self, color: Color) -> BackgroundModifier<Self> { /* ... */ }
    fn foreground(self, color: Color) -> ForegroundModifier<Self> { /* ... */ }
    fn frame_width(self, width: f64) -> FrameModifier<Self> { /* ... */ }
    fn frame_height(self, height: f64) -> FrameModifier<Self> { /* ... */ }
    fn border(self, color: Color, width: f64) -> BorderModifier<Self> { /* ... */ }
    fn corner_radius(self, radius: f64) -> CornerRadiusModifier<Self> { /* ... */ }
    fn shadow(self, color: Color, radius: f64, offset: (f64, f64)) -> ShadowModifier<Self> { /* ... */ }
    fn opacity(self, opacity: f64) -> OpacityModifier<Self> { /* ... */ }
    fn rotation(self, angle: f64) -> RotationModifier<Self> { /* ... */ }
    fn scale(self, scale: f64) -> ScaleModifier<Self> { /* ... */ }
    fn offset(self, x: f64, y: f64) -> OffsetModifier<Self> { /* ... */ }
}
```

## Event Handling

### Event Types

```rust,ignore
#[derive(Clone, Debug)]
pub struct TapEvent {
    pub position: Point,
    pub timestamp: std::time::Instant,
}

#[derive(Clone, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: KeyModifiers,
    pub is_repeat: bool,
}

#[derive(Clone, Debug)]
pub struct ScrollEvent {
    pub delta: (f64, f64),
    pub position: Point,
}

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
```

### Event Modifiers

```rust,ignore
pub trait EventModifier: ViewExt {
    fn on_tap<F>(self, handler: F) -> TapModifier<Self>
    where
        F: Fn(TapEvent) + 'static;
    
    fn on_key<F>(self, handler: F) -> KeyModifier<Self>
    where
        F: Fn(KeyEvent) + 'static;
    
    fn on_scroll<F>(self, handler: F) -> ScrollModifier<Self>
    where
        F: Fn(ScrollEvent) + 'static;
    
    fn on_hover<F>(self, handler: F) -> HoverModifier<Self>
    where
        F: Fn(bool) + 'static; // true for enter, false for exit
}
```

## Animation APIs

### Animation

Animation configuration and timing.

```rust,ignore
pub struct Animation {
    duration: std::time::Duration,
    timing_function: TimingFunction,
    delay: std::time::Duration,
}

impl Animation {
    pub fn linear(duration: std::time::Duration) -> Self { /* ... */ }
    pub fn ease_in(duration: std::time::Duration) -> Self { /* ... */ }
    pub fn ease_out(duration: std::time::Duration) -> Self { /* ... */ }
    pub fn ease_in_out(duration: std::time::Duration) -> Self { /* ... */ }
    pub fn spring() -> Self { /* ... */ }
    
    pub fn delay(mut self, delay: std::time::Duration) -> Self { /* ... */ }
}

#[derive(Clone, Copy, Debug)]
pub enum TimingFunction {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Spring { damping: f64, stiffness: f64 },
    Custom(fn(f64) -> f64),
}
```

### Animatable Trait

Trait for values that can be animated.

```rust,ignore
pub trait Animatable {
    fn interpolate(&self, to: &Self, progress: f64) -> Self;
}

// Implementations for common types
impl Animatable for f64 { /* ... */ }
impl Animatable for Color { /* ... */ }
impl Animatable for Point { /* ... */ }
impl Animatable for (f64, f64) { /* ... */ }
```

## Utility Functions

### Conditional Helpers

```rust,ignore
/// Creates a conditional view
pub fn if_then<V: View>(
    condition: bool, 
    then_view: impl FnOnce() -> V
) -> Option<V> {
    if condition {
        Some(then_view())
    } else {
        None
    }
}

/// Creates a conditional view with else branch
pub fn if_then_else<V: View>(
    condition: bool,
    then_view: impl FnOnce() -> V,
    else_view: impl FnOnce() -> V,
) -> V {
    if condition {
        then_view()
    } else {
        else_view()
    }
}
```

### Collection Helpers

```rust,ignore
/// Converts an iterator of views into a vector
pub fn collect_views<V: View>(
    views: impl Iterator<Item = V>
) -> Vec<V> {
    views.collect()
}

/// Creates a view from an optional value
pub fn optional<V: View>(view: Option<V>) -> Option<V> {
    view
}
```

This API reference provides comprehensive documentation for all major WaterUI APIs. For the most up-to-date information, refer to the generated documentation using `cargo doc --open`.
