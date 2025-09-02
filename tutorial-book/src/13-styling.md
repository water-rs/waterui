# Styling and Theming

WaterUI provides comprehensive styling capabilities with support for themes, custom styles, and responsive design. This chapter covers how to create beautiful, consistent interfaces.

## Basic Styling

### Colors and Backgrounds

```rust,ignore
fn color_demo() -> impl View {
    vstack((
        text("Color Demonstrations").font_size(20.0),
        
        // Basic colors
        hstack((
            color_swatch(Color::primary(), "Primary"),
            color_swatch(Color::secondary(), "Secondary"),
            color_swatch(Color::accent(), "Accent"),
        ))
        .spacing(10.0),
        
        // Semantic colors
        hstack((
            color_swatch(Color::success(), "Success"),
            color_swatch(Color::warning(), "Warning"),
            color_swatch(Color::error(), "Error"),
        ))
        .spacing(10.0),
        
        // Custom colors
        hstack((
            color_swatch(Color::rgb(0.2, 0.6, 0.9), "Custom RGB"),
            color_swatch(Color::hsl(240.0, 0.8, 0.6), "Custom HSL"),
            color_swatch(Color::hex("#FF6B6B"), "Custom Hex"),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn color_swatch(color: Color, label: &str) -> impl View {
    vstack((
        rectangle()
            .width(80.0)
            .height(60.0)
            .color(color)
            .corner_radius(8.0),
        text(label).font_size(12.0),
    ))
    .spacing(5.0)
    .alignment(.center)
}
```

### Typography Styling

```rust,ignore
fn typography_demo() -> impl View {
    vstack((
        text("Typography Styles").font_size(24.0).font_weight(FontWeight::Bold),
        
        // Font families
        text("Default Font Family").font_family("system"),
        text("Serif Font").font_family("serif"),
        text("Monospace Font").font_family("monospace"),
        text("Custom Font").font_family("SF Pro Display, Arial, sans-serif"),
        
        // Font weights
        text("Thin Text").font_weight(FontWeight::Thin),
        text("Light Text").font_weight(FontWeight::Light),
        text("Regular Text").font_weight(FontWeight::Regular),
        text("Medium Text").font_weight(FontWeight::Medium),
        text("Semibold Text").font_weight(FontWeight::SemiBold),
        text("Bold Text").font_weight(FontWeight::Bold),
        text("Heavy Text").font_weight(FontWeight::Heavy),
        
        // Text decoration
        text("Underlined Text").underline(true),
        text("Strikethrough Text").strikethrough(true),
        text("Italic Text").font_style(FontStyle::Italic),
    ))
    .spacing(8.0)
    .padding(20.0)
}
```

## Theming System

### Theme Definition

```rust,ignore
#[derive(Clone)]
struct AppTheme {
    primary: Color,
    secondary: Color,
    background: Color,
    surface: Color,
    text: Color,
    text_secondary: Color,
    border: Color,
    shadow: Color,
}

impl AppTheme {
    fn light() -> Self {
        Self {
            primary: Color::rgb(0.0, 0.48, 1.0),
            secondary: Color::rgb(0.34, 0.34, 0.36),
            background: Color::rgb(1.0, 1.0, 1.0),
            surface: Color::rgb(0.98, 0.98, 0.98),
            text: Color::rgb(0.0, 0.0, 0.0),
            text_secondary: Color::rgb(0.34, 0.34, 0.36),
            border: Color::rgb(0.9, 0.9, 0.9),
            shadow: Color::rgb(0.0, 0.0, 0.0).opacity(0.1),
        }
    }
    
    fn dark() -> Self {
        Self {
            primary: Color::rgb(0.04, 0.52, 1.0),
            secondary: Color::rgb(0.64, 0.64, 0.67),
            background: Color::rgb(0.0, 0.0, 0.0),
            surface: Color::rgb(0.07, 0.07, 0.07),
            text: Color::rgb(1.0, 1.0, 1.0),
            text_secondary: Color::rgb(0.64, 0.64, 0.67),
            border: Color::rgb(0.2, 0.2, 0.2),
            shadow: Color::rgb(0.0, 0.0, 0.0).opacity(0.3),
        }
    }
}
```

### Theme Application

```rust,ignore
fn themed_app() -> impl View {
    let theme_mode = binding(ThemeMode::Light);
    let theme = s!(match theme_mode {
        ThemeMode::Light => AppTheme::light(),
        ThemeMode::Dark => AppTheme::dark(),
        ThemeMode::System => system_theme(),
    });
    
    theme_provider(theme.clone(), 
        vstack((
            theme_selector(theme_mode.clone()),
            themed_content(),
        ))
        .spacing(20.0)
        .padding(20.0)
        .background(s!(theme.background))
    )
}

#[derive(Clone, PartialEq)]
enum ThemeMode {
    Light,
    Dark,
    System,
}

fn theme_selector(theme_mode: Binding<ThemeMode>) -> impl View {
    let theme = use_theme::<AppTheme>();
    
    hstack((
        text("Theme:")
            .color(s!(theme.text)),
            
        picker(theme_mode.clone(), vec![
            (ThemeMode::Light, "Light"),
            (ThemeMode::Dark, "Dark"),
            (ThemeMode::System, "System"),
        ))
        .on_change({
            let theme_mode = theme_mode.clone();
            move |mode| theme_mode.set(mode)
        }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(s!(theme.surface))
    .corner_radius(10.0)
}

fn themed_content() -> impl View {
    let theme = use_theme::<AppTheme>();
    
    vstack((
        themed_card("Primary Card", s!(theme.primary)),
        themed_card("Secondary Card", s!(theme.secondary)),
        themed_button_group(),
    ))
    .spacing(15.0)
}

fn themed_card(title: &str, accent_color: Signal<Color>) -> impl View {
    let theme = use_theme::<AppTheme>();
    
    vstack((
        text(title)
            .font_size(18.0)
            .font_weight(FontWeight::Medium)
            .color(s!(theme.text)),
            
        text("This is a themed card that adapts to the current theme.")
            .color(s!(theme.text_secondary))
            .line_height(1.4),
            
        rectangle()
            .height(4.0)
            .color(accent_color)
            .corner_radius(2.0),
    ))
    .spacing(10.0)
    .padding(20.0)
    .background(s!(theme.surface))
    .border_color(s!(theme.border))
    .border_width(1.0)
    .corner_radius(12.0)
    .shadow(s!(theme.shadow), 2.0)
}
```

### Custom Style System

```rust,ignore
trait Styleable {
    fn styled(self, style: Style) -> StyledView<Self>
    where
        Self: Sized,
    {
        StyledView::new(self, style)
    }
}

impl<V: View> Styleable for V {}

struct Style {
    background: Option<Color>,
    foreground: Option<Color>,
    padding: Option<EdgeInsets>,
    margin: Option<EdgeInsets>,
    border: Option<Border>,
    corner_radius: Option<f64>,
    shadow: Option<Shadow>,
}

impl Style {
    fn new() -> Self {
        Self {
            background: None,
            foreground: None,
            padding: None,
            margin: None,
            border: None,
            corner_radius: None,
            shadow: None,
        }
    }
    
    fn card() -> Self {
        let theme = current_theme::<AppTheme>();
        Self::new()
            .background(theme.surface)
            .padding(EdgeInsets::all(16.0))
            .corner_radius(12.0)
            .shadow(Shadow::new(theme.shadow, 2.0))
    }
    
    fn button_primary() -> Self {
        let theme = current_theme::<AppTheme>();
        Self::new()
            .background(theme.primary)
            .foreground(Color::white())
            .padding(EdgeInsets::symmetric(16.0, 12.0))
            .corner_radius(8.0)
    }
}

fn styled_components_demo() -> impl View {
    vstack((
        text("Styled Components").font_size(20.0),
        
        text("This is a card-styled container")
            .styled(Style::card()),
            
        button("Primary Button")
            .styled(Style::button_primary()),
            
        custom_styled_view(),
    ))
    .spacing(15.0)
    .padding(20.0)
}

fn custom_styled_view() -> impl View {
    let theme = use_theme::<AppTheme>();
    
    text("Custom styled view with theme")
        .styled(Style::new()
            .background(s!(theme.primary.opacity(0.1)))
            .foreground(s!(theme.primary))
            .padding(EdgeInsets::all(12.0))
            .corner_radius(6.0)
            .border(Border::all(1.0, s!(theme.primary)))
        )
}
```

## Responsive Design

### Breakpoint System

```rust,ignore
#[derive(Clone, PartialEq)]
enum Breakpoint {
    XSmall,  // < 576px
    Small,   // >= 576px
    Medium,  // >= 768px
    Large,   // >= 992px
    XLarge,  // >= 1200px
}

fn responsive_layout_demo() -> impl View {
    let window_size = use_window_size();
    let breakpoint = s!(get_breakpoint(window_size));
    
    s!(match breakpoint {
        Breakpoint::XSmall => mobile_layout(),
        Breakpoint::Small => tablet_portrait_layout(),
        Breakpoint::Medium => tablet_landscape_layout(),
        Breakpoint::Large | Breakpoint::XLarge => desktop_layout(),
    })
}

fn get_breakpoint(size: WindowSize) -> Breakpoint {
    match size.width {
        w if w < 576.0 => Breakpoint::XSmall,
        w if w < 768.0 => Breakpoint::Small,
        w if w < 992.0 => Breakpoint::Medium,
        w if w < 1200.0 => Breakpoint::Large,
        _ => Breakpoint::XLarge,
    }
}

fn mobile_layout() -> impl View {
    vstack((
        mobile_header(),
        scroll(mobile_content()),
        mobile_navigation(),
    ))
}

fn tablet_portrait_layout() -> impl View {
    vstack((
        tablet_header(),
        hstack((
            sidebar().width(200.0),
            scroll(main_content()).flex(1),
        )),
    ))
}

fn desktop_layout() -> impl View {
    hstack((
        desktop_sidebar().width(250.0),
        vstack((
            desktop_header(),
            scroll(desktop_content()).flex(1),
        ))
        .flex(1),
    ))
}
```

### Adaptive Components

```rust,ignore
fn adaptive_card(content: impl View) -> impl View {
    let window_size = use_window_size();
    let is_mobile = s!(window_size.width < 768.0);
    
    content
        .padding(s!(if is_mobile { EdgeInsets::all(12.0) } else { EdgeInsets::all(20.0) }))
        .corner_radius(s!(if is_mobile { 8.0 } else { 12.0 }))
        .background(Color::surface())
        .shadow(s!(if is_mobile { Shadow::light() } else { Shadow::medium() }))
}

fn responsive_grid<I>(items: I) -> impl View
where
    I: IntoIterator<Item = impl View>,
{
    let window_size = use_window_size();
    let columns = s!(match window_size.width {
        w if w < 576.0 => 1,
        w if w < 768.0 => 2,
        w if w < 992.0 => 3,
        _ => 4,
    });
    
    grid(items)
        .columns(columns)
        .spacing(s!(if window_size.width < 768.0 { 10.0 } else { 15.0 }))
}
```

## Animations and Transitions

### Basic Animations

```rust,ignore
fn animation_demo() -> impl View {
    let is_expanded = binding(false);
    let rotation = binding(0.0);
    let scale = binding(1.0);
    
    vstack((
        text("Animation Examples").font_size(20.0),
        
        // Expand/collapse animation
        button(s!(if is_expanded { "Collapse" } else { "Expand" }))
            .action({
                let is_expanded = is_expanded.clone();
                move |_| is_expanded.update(|expanded| !expanded)
            }),
            
        animated_container(
            s!(if is_expanded { 
                Some("This content expands and collapses with animation") 
            } else { 
                None 
            }),
            Duration::from_millis(300)
        ),
        
        // Rotation animation
        button("Rotate")
            .action({
                let rotation = rotation.clone();
                move |_| rotation.update(|r| r + 90.0)
            }),
            
        rectangle()
            .width(50.0)
            .height(50.0)
            .color(Color::primary())
            .rotation(rotation.get())
            .animated(Duration::from_millis(500), AnimationCurve::EaseInOut),
            
        // Scale animation
        button("Scale")
            .on_press({
                let scale = scale.clone();
                move || {
                    scale.set(1.2);
                    
                    let scale = scale.clone();
                    task::spawn(async move {
                        task::sleep(Duration::from_millis(150)).await;
                        scale.set(1.0);
                    });
                }
            }),
            
        circle()
            .width(60.0)
            .height(60.0)
            .color(Color::accent())
            .scale(scale.get())
            .animated(Duration::from_millis(300), AnimationCurve::EaseOut),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn animated_container(content: Signal<Option<&str>>, duration: Duration) -> impl View {
    content
        .map(|text| text.map(|t| 
            text(t)
                .padding(15.0)
                .background(Color::light_gray())
                .corner_radius(8.0)
        ))
        .animated_presence(duration, AnimationCurve::EaseInOut)
}
```

### Complex Animations

```rust,ignore
fn complex_animation_demo() -> impl View {
    let animation_state = binding(AnimationPhase::Idle);
    
    vstack((
        text("Complex Animation Demo").font_size(20.0),
        
        button("Start Animation")
            .action({
                let animation_state = animation_state.clone();
                move |_| {
                    animation_state.set(AnimationPhase::Phase1);
                    
                    let animation_state = animation_state.clone();
                    task::spawn(async move {
                        task::sleep(Duration::from_millis(500)).await;
                        animation_state.set(AnimationPhase::Phase2);
                        
                        task::sleep(Duration::from_millis(500)).await;
                        animation_state.set(AnimationPhase::Phase3);
                        
                        task::sleep(Duration::from_millis(500)).await;
                        animation_state.set(AnimationPhase::Idle);
                    });
                }
            }),
            
        animated_shapes(animation_state.clone()),
    ))
    .spacing(20.0)
    .padding(20.0)
}

#[derive(Clone, PartialEq)]
enum AnimationPhase {
    Idle,
    Phase1,
    Phase2,
    Phase3,
}

fn animated_shapes(phase: Binding<AnimationPhase>) -> impl View {
    hstack((
        circle()
            .width(s!(match phase {
                AnimationPhase::Idle => 30.0,
                AnimationPhase::Phase1 => 50.0,
                AnimationPhase::Phase2 => 40.0,
                AnimationPhase::Phase3 => 60.0,
            }))
            .height(s!(match phase {
                AnimationPhase::Idle => 30.0,
                AnimationPhase::Phase1 => 50.0,
                AnimationPhase::Phase2 => 40.0,
                AnimationPhase::Phase3 => 60.0,
            }))
            .color(Color::primary())
            .animated(Duration::from_millis(300), AnimationCurve::EaseInOut),
            
        rectangle()
            .width(50.0)
            .height(50.0)
            .color(Color::secondary())
            .rotation(s!(match phase {
                AnimationPhase::Idle => 0.0,
                AnimationPhase::Phase1 => 90.0,
                AnimationPhase::Phase2 => 180.0,
                AnimationPhase::Phase3 => 270.0,
            }))
            .animated(Duration::from_millis(400), AnimationCurve::EaseOut),
            
        triangle()
            .width(40.0)
            .height(40.0)
            .color(Color::accent())
            .scale(s!(match phase {
                AnimationPhase::Idle => 1.0,
                AnimationPhase::Phase1 => 1.5,
                AnimationPhase::Phase2 => 0.8,
                AnimationPhase::Phase3 => 1.2,
            }))
            .animated(Duration::from_millis(350), AnimationCurve::Bounce),
    ))
    .spacing(30.0)
}
```

## Style Performance

### Style Optimization

```rust,ignore
fn optimized_styling_demo() -> impl View {
    let items = (0..1000).collect::<Vec<_>>();
    
    // Good: Pre-computed styles
    let item_style = Style::new()
        .background(Color::surface())
        .padding(EdgeInsets::all(10.0))
        .corner_radius(6.0);
    
    scroll(
        vstack(
            items.into_iter().map(|index| {
                optimized_list_item(index, item_style.clone())
            })
        )
        .spacing(5.0)
    )
}

fn optimized_list_item(index: usize, style: Style) -> impl View {
    // Good: Reuse computed styles instead of recalculating
    text!("Item {}", index)
        .styled(style)
}

// Avoid: Recalculating styles for every item
fn unoptimized_list_item(index: usize) -> impl View {
    text!("Item {}", index)
        .background(Color::surface())  // Recalculated for each item
        .padding(EdgeInsets::all(10.0))
        .corner_radius(6.0)
}
```

### Style Caching

```rust,ignore
struct StyleCache {
    cache: HashMap<String, Style>,
}

impl StyleCache {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }
    
    fn get_or_create<F>(&mut self, key: &str, create: F) -> Style
    where
        F: FnOnce() -> Style,
    {
        self.cache.entry(key.to_string())
            .or_insert_with(create)
            .clone()
    }
}

fn cached_styling_demo() -> impl View {
    let style_cache = use_memo(|| StyleCache::new());
    
    vstack((
        cached_button("Primary", "primary", &style_cache),
        cached_button("Secondary", "secondary", &style_cache),
        cached_button("Accent", "accent", &style_cache),
    ))
    .spacing(10.0)
}

fn cached_button(label: &str, style_key: &str, cache: &StyleCache) -> impl View {
    let style = cache.get_or_create(style_key, || {
        match style_key {
            "primary" => Style::button_primary(),
            "secondary" => Style::button_secondary(),
            "accent" => Style::button_accent(),
            _ => Style::new(),
        }
    });
    
    button(label).styled(style)
}
```

## Summary

WaterUI's styling system provides:

- **Comprehensive Color System**: RGB, HSL, hex, and semantic colors
- **Typography Control**: Font families, weights, sizes, and decorations
- **Theme Support**: Light/dark themes with custom theme creation
- **Responsive Design**: Breakpoint-based layouts and adaptive components
- **Animation System**: Transitions, transforms, and complex animations
- **Performance Optimization**: Style caching and efficient rendering

Key best practices:
- Use theme providers for consistent styling
- Implement responsive breakpoints for different screen sizes
- Cache and reuse styles for performance
- Use semantic colors for better theme support
- Test animations on different devices
- Consider accessibility in color choices and contrast ratios

Next: [Data Management](14-data.md)