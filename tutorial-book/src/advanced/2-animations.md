# Animation and Transitions

WaterUI provides a revolutionary reactive animation system that leverages nami's metadata capabilities to create smooth, performant animations with zero boilerplate. Unlike traditional animation systems that require explicit animation setup, WaterUI's animations work through reactive value metadata that automatically flows to the renderer.

## The Reactive Animation System

At its core, WaterUI's animation system is built on **metadata propagation** through the reactive system. When you use `.animated()` on a reactive value, you're attaching animation metadata that travels with every value change. The upstream renderer receives this metadata and handles all the complex interpolation, timing, and rendering automatically.

```text
┌─────────────────┐     .animated()     ┌──────────────────────────┐
│ Reactive Value  │ ──────────────────> │ Value + Animation        │
│                 │                     │ Metadata Wrapper        │
│ s!(0.0)         │                     │                          │
└─────────────────┘                     └─────────────┬────────────┘
                                                      │
                          .set(100.0)                 │
                               │                      │
                               ▼                      │
┌─────────────────┐     ┌─────────────────┐          │
│   nami Context  │ ──> │    Renderer     │ <────────┘
│                 │     │                 │
│ value: 100.0    │     │ - Sees new      │
│ metadata:       │     │   target value  │
│ Animation::     │     │ - Gets animation│
│ ease_in_out     │     │   config        │
│ (250ms)         │     │ - Creates       │
└─────────────────┘     │   interpolator  │
                        │ - Animates      │
                        │   0.0 → 100.0   │
                        └─────────────────┘
```

### The Secret: Metadata Flow

The fundamental innovation is that animation metadata flows through nami's reactive system alongside value changes:

```rust,ignore
use waterui::*;
use nami::s;

fn reactive_animation_demo() -> impl View {
    let position = s!(0.0);
    
    // The .animated() method attaches Animation metadata to the binding
    let animated_position = position.animated();
    
    vstack((
        rectangle()
            .width(50.0)
            .height(50.0)
            .offset_x(animated_position), // Renderer receives value + animation metadata
            .fill(color::BLUE),
        
        button("Move")
            .on_press(move || {
                // When we change the value, the metadata flows to the renderer
                position.set(200.0); // Renderer sees: new_value=200.0 + Animation::default()
            })
    ))
}
```

## Using `.animated()` - The Core Method

The `.animated()` method is your primary interface to WaterUI's animation system. It creates a binding wrapper that attaches default animation metadata:

```rust,ignore
use waterui::*;
use nami::s;

fn animated_examples() -> impl View {
    let opacity = s!(1.0);
    let scale = s!(1.0);
    let color = s!(color::RED);
    
    // Each .animated() call attaches metadata that the renderer will use
    let animated_opacity = opacity.animated();     // Default ease-in-out animation
    let animated_scale = scale.animated();         // Default ease-in-out animation  
    let animated_color = color.animated();         // Default ease-in-out animation
    
    rectangle()
        .width(100.0)
        .height(100.0)
        .opacity(animated_opacity)
        .scale(animated_scale)
        .fill(animated_color)
        .on_tap(move || {
            // All three changes will be animated by the renderer
            opacity.set(0.5);
            scale.set(1.2);  
            color.set(color::BLUE);
        })
}
```

## Custom Animation Configurations

While `.animated()` provides sensible defaults, you can customize animations using the `.with_animation()` method:

### Animation Types Available

WaterUI supports several animation curves, each creating different visual effects:

```rust,ignore
use waterui::*;
use nami::s;
use std::time::Duration;

fn animation_types_demo() -> impl View {
    let position = s!(0.0);
    
    vstack((
        // Default ease-in-out (250ms) - most common
        rectangle()
            .width(40.0).height(40.0)
            .offset_x(position.animated())  // Uses default animation
            .fill(color::BLUE),
        
        // Linear - constant speed
        rectangle()
            .width(40.0).height(40.0)
            .offset_x(position.with_animation(Animation::linear(Duration::from_millis(500))))
            .fill(color::GREEN),
        
        // Ease-in - starts slow, accelerates
        rectangle()
            .width(40.0).height(40.0)  
            .offset_x(position.with_animation(Animation::ease_in(Duration::from_millis(400))))
            .fill(color::RED),
            
        // Ease-out - starts fast, decelerates
        rectangle()
            .width(40.0).height(40.0)
            .offset_x(position.with_animation(Animation::ease_out(Duration::from_millis(400))))
            .fill(color::ORANGE),
            
        // Spring - physics-based bouncing
        rectangle()
            .width(40.0).height(40.0)
            .offset_x(position.with_animation(Animation::spring(300.0, 20.0)))
            .fill(color::PURPLE),
        
        button("Move All")
            .on_press(move || {
                position.set(if position.get() > 100.0 { 0.0 } else { 200.0 });
            })
    ))
    .spacing(20.0)
}
```

### The Metadata Attachment Process

Here's what happens under the hood when you use `.animated()`:

```rust,ignore
use waterui::*;
use nami::s;

fn metadata_flow_explanation() -> impl View {
    let opacity = s!(0.5);
    
    // Step 1: Raw reactive value
    let raw_value = opacity.clone();  // Just the binding
    
    // Step 2: Attach animation metadata
    let animated_value = opacity.animated();  // Now wrapped with Animation metadata
    
    // Step 3: When used in a view property, both value + metadata flow to renderer
    text!("Fade me")
        .opacity(animated_value)  // Renderer receives: value=0.5 + metadata=Animation::default()
        .on_tap(move || {
            raw_value.set(1.0);  // Change triggers: value=1.0 + metadata flows through
        })
}
```

### Understanding Metadata Propagation

Animation metadata propagates through reactive computations automatically:

```rust,ignore
use waterui::*;
use nami::s;

fn metadata_propagation() -> impl View {
    let count = s!(0);
    let animated_count = count.animated();
    
    // Metadata flows through map operations
    let opacity = animated_count.map(|n| (n as f32 / 10.0).clamp(0.0, 1.0));
    let scale = animated_count.map(|n| 1.0 + (n as f32 * 0.1));
    
    // Both opacity and scale inherit the animation metadata from count
    rectangle()
        .width(100.0).height(100.0)
        .opacity(opacity)    // Animated because count is animated
        .scale(scale)        // Also animated because count is animated
        .fill(color::BLUE)
        .on_tap(move || {
            count.set((count.get() + 1) % 11);  // 0-10 range
        })
}
```

## How Renderers Consume Animation Metadata

When a renderer (like GTK4 or Web backend) receives a reactive value change with animation metadata, it automatically handles the interpolation process:

### The Rendering Pipeline

```text
User Code                nami System              Renderer                 UI Output
─────────                ───────────              ────────                 ─────────

position.set(100.0) ──> Context {        ──────> ┌─────────────────┐    ┌─────────────┐
                         value: 100.0             │ Current: 0.0    │    │ ▒           │
                         metadata:                │ Target:  100.0  │    │ ▒           │
                         Animation::              │                 │    │ ▒           │
                         spring(300,20)           │ Creates Spring  │    │ ▒           │
                        }                         │ Interpolator    │    │ ▒           │
                                                  └─────────────────┘    └─────────────┘
                                                           │               frame 1 (t=0ms)
                                                           │
                                                    Timer triggers 60fps
                                                           │
                                                  ┌─────────────────┐    ┌─────────────┐
                                                  │ Spring calc:    │    │  ▒          │
                                                  │ t=0.1 → 15.0   │    │  ▒          │
                                                  │ Apply to UI     │    │  ▒          │
                                                  └─────────────────┘    └─────────────┘
                                                           │               frame 6 (t=100ms)
                                                           │
                                                    Continue until
                                                    target reached
                                                           │
                                                  ┌─────────────────┐    ┌─────────────┐
                                                  │ Final position  │    │            ▒│
                                                  │ reached: 100.0  │    │            ▒│
                                                  │ Remove timer    │    │            ▒│
                                                  └─────────────────┘    └─────────────┘
                                                                         final position
```

### Renderer Implementation Details

Here's a simplified view of how renderers handle animation metadata:

```rust,ignore
// Pseudocode showing renderer animation handling
impl Renderer {
    fn handle_property_change<T>(&mut self, context: Context<T>) 
    where 
        T: Interpolatable + Clone 
    {
        let Context { value: new_value, metadata } = context;
        
        // Check if animation metadata is present
        if let Some(animation) = metadata.get::<Animation>() {
            // Create interpolator based on animation type
            let interpolator = match animation {
                Animation::Linear(duration) => LinearInterpolator::new(duration),
                Animation::EaseInOut(duration) => EaseInterpolator::new(duration),
                Animation::Spring { stiffness, damping } => SpringInterpolator::new(stiffness, damping),
                Animation::Default => EaseInterpolator::new(Duration::from_millis(250)),
            };
            
            // Start animation from current_value to new_value
            self.start_animation(interpolator, self.current_value, new_value);
        } else {
            // No animation metadata - update immediately
            self.current_value = new_value;
            self.render_immediately();
        }
    }
}
```

### Animation Interpolation Types

Different animation types use different mathematical functions for interpolation:

```rust,ignore
// How different animation curves work internally
fn interpolate_value(animation: &Animation, progress: f32) -> f32 {
    match animation {
        Animation::Linear(_) => progress,  // t
        
        Animation::EaseIn(_) => progress * progress,  // t²
        
        Animation::EaseOut(_) => 1.0 - (1.0 - progress).powi(2),  // 1-(1-t)²
        
        Animation::EaseInOut(_) => {
            if progress < 0.5 {
                2.0 * progress * progress  // 2t² for first half
            } else {
                1.0 - (-2.0 * progress + 2.0).powi(2) / 2.0  // Smooth transition for second half
            }
        },
        
        Animation::Spring { stiffness, damping } => {
            // Complex physics simulation using spring equations
            spring_interpolation(progress, stiffness, damping)
        },
    }
}
```

### Zero-Cost Abstractions

The beauty of WaterUI's animation system is that it provides zero-cost abstractions:

- **No Animation Metadata**: If a value has no animation metadata, it updates immediately with no overhead
- **With Animation Metadata**: The renderer automatically creates the appropriate interpolator
- **Type Safety**: Animation metadata is type-erased but type-safe through nami's metadata system
- **Composability**: Multiple animated properties work independently without interference

```rust,ignore
use waterui::*;
use nami::s;

fn zero_cost_demo() -> impl View {
    let animated_opacity = s!(1.0).animated();      // Gets interpolator 
    let instant_opacity = s!(1.0);                  // Updates immediately
    let animated_position = s!(0.0).with_animation( // Gets custom interpolator
        Animation::spring(200.0, 15.0)
    );
    
    rectangle()
        .opacity(animated_opacity)     // Renderer creates opacity interpolator
        .width(instant_opacity)        // Renderer updates width immediately  
        .offset_x(animated_position)   // Renderer creates position interpolator
        .height(100.0)                 // Static value - no computation
        .fill(color::BLUE)
}
```

## Complete Animation Flow Example

Here's a practical example showing the complete metadata flow in a button component:

```rust,ignore
use waterui::*;
use nami::s;

fn animated_button_demo() -> impl View {
    let is_pressed = s!(false);
    let is_hovered = s!(false);
    
    // Create animated reactive values
    let scale = is_pressed.map(|pressed| if *pressed { 0.95 } else { 1.0 }).animated();
    let bg_color = is_hovered.map(|hovered| {
        if *hovered { color::BLUE } else { color::GRAY }  
    }).with_animation(Animation::ease_out(Duration::from_millis(150)));
    
    /*
    Metadata Flow Visualization:
    
    User hovers ──> is_hovered.set(true) ──> map() ──> bg_color gets new value + Animation metadata
                                             │
                                             ▼
                                     Renderer receives:
                                     Context {
                                         value: BLUE,
                                         metadata: Animation::ease_out(150ms)
                                     }
                                             │  
                                             ▼
                                     Creates color interpolator GRAY → BLUE over 150ms
    */
    
    rectangle()
        .width(120.0)
        .height(40.0)
        .scale(scale)              // Gets scale animation metadata 
        .fill(bg_color)            // Gets color animation metadata
        .corner_radius(8.0)
        .overlay(
            text!("Animated Button")
                .color(color::WHITE)
        )
        .on_press(move || {
            is_pressed.set(true);
            // Animation automatically triggered by metadata flow
        })
        .on_release(move || {
            is_pressed.set(false); 
        })
        .on_hover(move || {
            is_hovered.set(true);
        })
        .on_hover_end(move || {
            is_hovered.set(false);
        })
}
```

### Metadata Flow Through Complex Computations

Animation metadata propagates through reactive computations, creating sophisticated animations with simple code:

```text
                    ┌─────────────────┐
                    │ Base Signal     │
                    │ count.animated()│
                    │                 │
                    └─────┬───────────┘
                          │ Animation metadata attached
                          │
    ┌─────────────────────┼─────────────────────┐
    │                     │                     │
    ▼                     ▼                     ▼
┌─────────┐         ┌─────────┐         ┌─────────┐
│ .map()  │         │ .map()  │         │ .map()  │
│ opacity │         │ scale   │         │ rotation│
└─────────┘         └─────────┘         └─────────┘
    │                     │                     │
    │ metadata flows      │ metadata flows      │ metadata flows
    │                     │                     │
    ▼                     ▼                     ▼
┌─────────┐         ┌─────────┐         ┌─────────┐
│Renderer │         │Renderer │         │Renderer │
│creates  │         │creates  │         │creates  │
│opacity  │         │scale    │         │rotation │
│animator │         │animator │         │animator │
└─────────┘         └─────────┘         └─────────┘
```

```rust,ignore
fn complex_metadata_flow() -> impl View {
    let count = s!(0).animated();  // Single source of animation metadata
    
    // All these computed values inherit the animation metadata from count
    let opacity = count.map(|n| (*n as f32 / 10.0).clamp(0.0, 1.0));
    let scale = count.map(|n| 1.0 + (*n as f32 * 0.05));  
    let rotation = count.map(|n| *n as f32 * 36.0); // 36° per increment
    let bg_color = count.map(|n| {
        let hue = (*n as f32 * 30.0) % 360.0;
        Color::from_hsv(hue, 1.0, 1.0)
    });
    
    rectangle()
        .width(100.0).height(100.0)
        .opacity(opacity)     // Animated - renderer gets metadata from count
        .scale(scale)         // Animated - renderer gets metadata from count  
        .rotation(rotation)   // Animated - renderer gets metadata from count
        .fill(bg_color)       // Animated - renderer gets metadata from count
        .on_tap(move || {
            count.set((count.get() + 1) % 11);  // Single change animates everything
        })
}
```

## Animatable Properties

WaterUI can animate most visual properties smoothly:

### Transform Properties

Transform properties are ideal for animations as they're GPU-accelerated and don't trigger layout recalculations:

```rust,ignore
use waterui::*;
use nami::s;

fn transform_animations() -> impl View {
    let offset_x = s!(0.0);
    let offset_y = s!(0.0); 
    let scale = s!(1.0);
    let rotation = s!(0.0);
    
    // Apply animations to transform properties using reactive values
    rectangle()
        .width(100.0).height(100.0)
        .offset_x(offset_x.animated())           // Spring animation for smooth movement
        .offset_y(offset_y.with_animation(Animation::ease_out(Duration::from_millis(400))))
        .scale(scale.with_animation(Animation::spring(200.0, 15.0)))  // Bouncy scaling
        .rotation(rotation.animated())           // Default ease for rotation
        .fill(color::RED)
        .overlay(
            vstack((
                hstack((
                    button("→")
                        .on_press({
                            let x = offset_x.clone();
                            move || x.set(if x.get() > 50.0 { 0.0 } else { 100.0 })
                        }),
                    button("↓")  
                        .on_press({
                            let y = offset_y.clone();
                            move || y.set(if y.get() > 30.0 { 0.0 } else { 60.0 })
                        }),
                    button("⚡")
                        .on_press({
                            let s = scale.clone();
                            move || s.set(if s.get() > 1.1 { 1.0 } else { 1.5 })
                        }),
                    button("↻")
                        .on_press(move || {
                            rotation.update(|r| r + 90.0);
                        }),
                ))
                .spacing(5.0),
                
                button("Reset All")
                    .on_press({
                        let x = offset_x.clone();
                        let y = offset_y.clone();
                        let s = scale.clone();
                        let r = rotation.clone();
                        move || {
                            x.set(0.0);
                            y.set(0.0);
                            s.set(1.0);
                            r.set(0.0);
                        }
                    })
            ))
            .spacing(10.0)
            .padding(10.0)
        )
}
```

### Color Animations

Color transitions create smooth theme changes and visual feedback:

```rust,ignore
use waterui::*;
use nami::s;

fn color_animations() -> impl View {
    let theme_index = s!(0);
    
    // Define color themes
    let themes = [
        (color::BLUE, color::WHITE, "Ocean"),
        (color::RED, color::YELLOW, "Sunset"), 
        (color::GREEN, color::BLACK, "Forest"),
        (color::PURPLE, color::WHITE, "Royal"),
    ];
    
    // Map theme index to colors with animation
    let bg_color = theme_index.map(|&idx| themes[idx % 4].0).animated();
    let text_color = theme_index.map(|&idx| themes[idx % 4].1).with_animation(
        Animation::ease_in_out(Duration::from_millis(300))
    );
    let theme_name = theme_index.map(|&idx| themes[idx % 4].2);
    
    vstack((
        // Animated color display
        rectangle()
            .width(200.0).height(100.0)
            .fill(bg_color)
            .corner_radius(12.0)
            .overlay(
                text!(theme_name)
                    .font_size(24.0)
                    .color(text_color)
            ),
        
        // Theme selection buttons  
        hstack((
            button("← Prev")
                .on_press({
                    let idx = theme_index.clone();
                    move || idx.update(|i| (*i + 3) % 4)  // Wrap around backwards
                }),
            button("Next →")
                .on_press(move || {
                    theme_index.update(|i| (*i + 1) % 4)  // Cycle forward
                }),
        ))
        .spacing(20.0),
        
        // Random theme button
        button("🎲 Random Theme")
            .on_press({
                let idx = theme_index.clone();
                move || {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    std::time::SystemTime::now().hash(&mut hasher);
                    idx.set((hasher.finish() as usize) % 4);
                }
            })
    ))
    .spacing(30.0)
}
```

### Size and Layout Animations

Size animations can create expand/collapse effects and responsive layouts:

```rust,ignore
use waterui::*;
use nami::s;

fn size_animations() -> impl View {
    let is_expanded = s!(false);
    
    // Map boolean state to size values with different animations
    let width = is_expanded.map(|&expanded| if expanded { 250.0 } else { 100.0 })
        .with_animation(Animation::spring(180.0, 12.0));  // Bouncy width
    let height = is_expanded.map(|&expanded| if expanded { 180.0 } else { 100.0 })
        .with_animation(Animation::ease_out(Duration::from_millis(400)));  // Smooth height
    let corner_radius = is_expanded.map(|&expanded| if expanded { 20.0 } else { 8.0 }).animated();
    let padding = is_expanded.map(|&expanded| if expanded { 20.0 } else { 10.0 }).animated();
    
    vstack((
        // Animated container
        rectangle()
            .width(width)
            .height(height) 
            .corner_radius(corner_radius)
            .fill(color::PURPLE)
            .overlay(
                vstack((
                    text!("📦")
                        .font_size(32.0),
                    text!(is_expanded.map(|&exp| if exp { "Expanded!" } else { "Compact" }))
                        .color(color::WHITE)
                        .font_size(16.0),
                ))
                .spacing(8.0)
                .padding(padding)
            ),
        
        // Control buttons
        hstack((
            button(is_expanded.map(|&exp| if exp { "📦 Collapse" } else { "📂 Expand" }))
                .on_press({
                    let expanded = is_expanded.clone();
                    move || expanded.update(|e| !e)
                }),
                
            button("🔄 Quick Toggle")
                .on_press(move || {
                    let exp = is_expanded.clone();
                    // Rapid toggle demonstration
                    tokio::spawn(async move {
                        for _ in 0..3 {
                            exp.update(|e| !e);
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                    });
                })
        ))
        .spacing(15.0),
        
        // Status indicator
        text!(is_expanded.map(|&exp| {
            format!("State: {} | Size: {}x{}", 
                if exp { "EXPANDED" } else { "COMPACT" },
                if exp { "250" } else { "100" },
                if exp { "180" } else { "100" }
            )
        }))
        .font_size(12.0)
        .color(color::GRAY)
    ))
    .spacing(25.0)
}
```

## Complex Animation Sequences

For more complex animations, you can chain and combine multiple animations:

### Sequential Animations

```rust,ignore
use std::time::Duration;
use tokio::time::sleep;

fn sequential_animation() -> impl View {
    let position = s!(Offset::zero());
    let scale = s!(1.0);
    let opacity = s!(1.0);
    
    let animated_circle = circle()
        .size(50.0)
        .offset(position.clone())
        .scale(scale.clone())
        .opacity(opacity.clone())
        .animation(Animation::ease_in_out(Duration::from_millis(500)))
        .fill(Color::orange());
    
    vstack((
        animated_circle,
        
        button("Animate Sequence")
            .on_press({
                let position = position.clone();
                let scale = scale.clone();
                let opacity = opacity.clone();
                move || {
                    let pos = position.clone();
                    let sc = scale.clone();
                    let op = opacity.clone();
                    
                    tokio::spawn(async move {
                        // Step 1: Move right
                        pos.set(Offset::new(100.0, 0.0));
                        sleep(Duration::from_millis(500)).await;
                        
                        // Step 2: Scale up
                        sc.set(1.5);
                        sleep(Duration::from_millis(500)).await;
                        
                        // Step 3: Fade out
                        op.set(0.3);
                        sleep(Duration::from_millis(500)).await;
                        
                        // Step 4: Return to original state
                        pos.set(Offset::zero());
                        sc.set(1.0);
                        op.set(1.0);
                    });
                }
            }),
    ))
    .spacing(30.0)
}
```

### Parallel Animations

```rust,ignore
fn parallel_animations() -> impl View {
    let transform = s!(Transform::identity());
    let color = s!(Color::blue());
    
    rectangle()
        .size(100.0)
        .transform(transform.clone())
        .fill(color.clone())
        .animation(Animation::spring())
        .on_tap(move || {
            // Both animations happen simultaneously
            transform.set(Transform::identity()
                .scaled(1.5)
                .rotated(45.0)
                .translated(50.0, 25.0));
            color.set(Color::red());
        })
}
```

## Gesture-Driven Animations

WaterUI animations work seamlessly with gesture recognizers:

### Drag Animations

```rust,ignore
fn draggable_animation() -> impl View {
    let position = s!(Offset::zero());
    let is_dragging = s!(false);
    
    circle()
        .size(80.0)
        .offset(position.clone())
        .scale(s!(if is_dragging { 1.1 } else { 1.0 }))
        .animation(Animation::spring())
        .fill(Color::green())
        .gesture(
            DragGesture::new()
                .on_started(move |_| is_dragging.set(true))
                .on_changed({
                    let position = position.clone();
                    move |delta| position.update(|pos| pos + delta.translation)
                })
                .on_ended(move |_| is_dragging.set(false))
        )
}
```

### Swipe Animations

```rust,ignore
fn swipe_cards() -> impl View {
    let cards = s!(vec!["Card 1", "Card 2", "Card 3", "Card 4"));
    let current_offset = s!(0.0);
    
    zstack(
        cards.iter().enumerate().map(|(index, card)| {
            let offset = s!(current_offset + (index as f32 * 300.0));
            
            rectangle()
                .width(250.0)
                .height(150.0)
                .offset_x(offset)
                .corner_radius(10.0)
                .fill(Color::blue())
                .animation(Animation::spring())
                .overlay(
                    text(card)
                        .color(Color::white())
                        .font_size(18.0)
                )
                .gesture(
                    DragGesture::new()
                        .on_ended({
                            let current_offset = current_offset.clone();
                            move |details| {
                                if details.velocity.x.abs() > 500.0 {
                                    // Snap to next card
                                    current_offset.update(|offset| {
                                        let new_offset = if details.velocity.x < 0.0 {
                                            offset - 300.0
                                        } else {
                                            offset + 300.0
                                        };
                                        new_offset.max(-600.0).min(0.0)
                                    });
                                }
                            }
                        })
                )
        }).collect::<Vec<_>>()
    )
}
```

## Performance Optimization

### Animation Performance Tips

1. **Use Transform Properties**: Transform properties (translate, scale, rotate) are GPU-accelerated:

```rust,ignore
// ✅ Efficient - uses transforms
fn efficient_animation() -> impl View {
    let scale = s!(1.0);
    
    circle()
        .width(100.0).height(100.0)
        .scale(scale.animated())  // GPU-accelerated, metadata-driven animation
        .fill(color::BLUE)
        .on_tap(move || scale.update(|s| if *s > 1.1 { 1.0 } else { 1.3 }))
}

// ❌ Less efficient - changes layout
fn less_efficient_animation() -> impl View {
    let size = s!(100.0);
    
    circle()
        .width(size.animated())  // May trigger layout recalculation
        .height(size.clone())    // Better to use transform scale instead
        .fill(color::BLUE)
        .on_tap(move || size.update(|s| if *s > 110.0 { 100.0 } else { 150.0 }))
}
```

2. **Leverage Reactive Computations**: Use reactive patterns to batch logical updates:

```rust,ignore  
use waterui::*;
use nami::s;

fn efficient_batched_updates() -> impl View {
    let interaction_state = s!(0); // Single source of truth
    
    // All animations derive from one state change - efficient!
    let scale = interaction_state.map(|&state| match state {
        0 => 1.0,      // Normal
        1 => 1.1,      // Hover
        2 => 0.95,     // Pressed
        _ => 1.0,
    }).animated();
    
    let color = interaction_state.map(|&state| match state {
        0 => color::BLUE,
        1 => color::CYAN, 
        2 => color::NAVY,
        _ => color::BLUE,
    }).with_animation(Animation::ease_in_out(Duration::from_millis(150)));
    
    let rotation = interaction_state.map(|&state| (state as f32) * 15.0).animated();
    
    rectangle()
        .width(100.0).height(100.0)
        .scale(scale)       // All three properties animate from single state change
        .fill(color)        // Efficient: one reactive update → three animations
        .rotation(rotation)
        .on_hover(move || interaction_state.set(1))
        .on_press(move || interaction_state.set(2)) 
        .on_release(move || interaction_state.set(0))
}
```

3. **Use Appropriate Animation Curves**: Choose the right animation type for your use case:

```rust,ignore
fn optimized_animations() -> impl View {
    vstack((
        // For UI feedback - use spring for natural feel
        button("Spring Animation")
            .animation(Animation::spring()),
        
        // For loading indicators - use linear for consistency  
        progress_bar()
            .animation(Animation::linear(Duration::from_millis(1000))),
        
        // For page transitions - use ease curves
        page_transition()
            .animation(Animation::ease_in_out(Duration::from_millis(300))),
    ))
}
```

## Custom Animation Curves

You can create custom animation curves for unique effects:

```rust,ignore
fn custom_curve_animation() -> impl View {
    let progress = s!(0.0);
    
    // Custom bounce curve
    let bounce_curve = |t: f32| -> f32 {
        if t < 1.0 / 2.75 {
            7.5625 * t * t
        } else if t < 2.0 / 2.75 {
            let t = t - 1.5 / 2.75;
            7.5625 * t * t + 0.75
        } else if t < 2.5 / 2.75 {
            let t = t - 2.25 / 2.75;
            7.5625 * t * t + 0.9375
        } else {
            let t = t - 2.625 / 2.75;
            7.5625 * t * t + 0.984375
        }
    };
    
    circle()
        .size(60.0)
        .offset_y(s!(progress * 200.0))
        .animation(Animation::custom(
            Duration::from_millis(1000),
            bounce_curve
        ))
        .fill(Color::red())
        .on_tap(move || {
            progress.update(|p| if p > 0.5 { 0.0 } else { 1.0 });
        })
}
```

## Animation State Management

### Animation Controllers

For complex animation sequences, use animation controllers:

```rust,ignore
struct AnimationController {
    is_playing: Binding<bool>,
    progress: Binding<f32>,
    direction: Binding<i32>, // 1 for forward, -1 for reverse
}

impl AnimationController {
    fn new() -> Self {
        Self {
            is_playing: s!(false),
            progress: s!(0.0),
            direction: s!(1),
        }
    }
    
    fn play(&self) {
        self.is_playing.set(true);
        self.animate_to_end();
    }
    
    fn reverse(&self) {
        self.direction.set(-1);
        self.is_playing.set(true);
        self.animate_to_start();
    }
    
    fn animate_to_end(&self) {
        let progress = self.progress.clone();
        let is_playing = self.is_playing.clone();
        
        tokio::spawn(async move {
            while progress.with(|p| *p < 1.0) {
                progress.update(|p| (p + 0.02).min(1.0));
                sleep(Duration::from_millis(16)).await; // ~60 FPS
            }
            is_playing.set(false);
        });
    }
    
    fn animate_to_start(&self) {
        let progress = self.progress.clone();
        let is_playing = self.is_playing.clone();
        
        tokio::spawn(async move {
            while progress.with(|p| *p > 0.0) {
                progress.update(|p| (p - 0.02).max(0.0));
                sleep(Duration::from_millis(16)).await;
            }
            is_playing.set(false);
        });
    }
}

fn controlled_animation() -> impl View {
    let controller = AnimationController::new();
    let scale = controller.progress.map(|p| 1.0 + p * 0.5);
    let rotation = controller.progress.map(|p| p * 360.0);
    
    vstack((
        rectangle()
            .size(100.0)
            .scale(scale.clone())
            .rotation(rotation.clone())
            .fill(Color::blue()),
        
        hstack((
            button("Play")
                .on_press({
                    let controller = controller.clone();
                    move || controller.play()
                }),
            
            button("Reverse")
                .on_press(move || controller.reverse()),
        ))
        .spacing(10.0),
    ))
    .spacing(20.0)
}
```

## Real-World Animation Examples

### Loading Animations

```rust,ignore
fn loading_animations() -> impl View {
    let rotation = s!(0.0);
    let pulse_scale = s!(1.0);
    
    // Start continuous animations
    let rotation_clone = rotation.clone();
    tokio::spawn(async move {
        loop {
            rotation_clone.update(|r| r + 2.0);
            sleep(Duration::from_millis(16)).await;
        }
    });
    
    let pulse_clone = pulse_scale.clone();
    tokio::spawn(async move {
        let growing = binding(true);
        loop {
            let current_val = pulse_clone.with(|p| *p);
            let is_growing = growing.with(|g| *g);
            
            if is_growing {
                let new_val = current_val + 0.01;
                pulse_clone.set(new_val);
                if new_val >= 1.2 { growing.set(false); }
            } else {
                let new_val = current_val - 0.01;
                pulse_clone.set(new_val);
                if new_val <= 0.8 { growing.set(true); }
            }
            sleep(Duration::from_millis(16)).await;
        }
    });
    
    vstack((
        // Spinning loader
        circle()
            .size(40.0)
            .stroke(Color::blue(), 4.0)
            .stroke_dash([10.0, 5.0))
            .rotation(rotation.clone())
            .animation(Animation::linear(Duration::from_millis(16))),
        
        // Pulsing dot
        circle()
            .size(20.0)
            .scale(pulse_scale.clone())
            .animation(Animation::ease_in_out(Duration::from_millis(16)))
            .fill(Color::green()),
        
        text("Loading...")
            .color(Color::gray()),
    ))
    .spacing(20.0)
}
```

### Page Transitions

```rust,ignore
fn page_transition_example() -> impl View {
    let current_page = s!(0);
    let transition_offset = s!(0.0);
    
    let pages = vec!["Home", "Profile", "Settings"];
    
    vstack((
        // Page content with slide transition
        zstack(
            pages.iter().enumerate().map(|(index, page)| {
                let offset = s!((index as f32 - current_page as f32) * 300.0 + transition_offset);
                
                rectangle()
                    .width(300.0)
                    .height(200.0)
                    .offset_x(offset)
                    .fill(Color::white())
                    .border(Color::gray(), 1.0)
                    .animation(Animation::ease_out(Duration::from_millis(300)))
                    .overlay(
                        text(page)
                            .font_size(24.0)
                            .color(Color::black())
                    )
            }).collect::<Vec<_>>()
        ),
        
        // Navigation
        hstack(
            pages.iter().enumerate().map(|(index, page)| {
                button(page)
                    .on_press({
                        let current_page = current_page.clone();
                        move || current_page.set(index)
                    })
                    .background(s!(if current_page == index { 
                        Color::blue() 
                    } else { 
                        Color::gray() 
                    }))
            }).collect::<Vec<_>>()
        )
        .spacing(10.0),
    ))
    .spacing(20.0)
}
```

## Animation Testing and Debugging

### Animation Inspector

```rust,ignore
fn animation_debug_view(animated_view: impl View) -> impl View {
    let show_debug = s!(false);
    
    vstack((
        animated_view,
        
        when(show_debug.clone(), |debug| if debug {
            vstack((
                text("Animation Debug Info"),
                text("Frame Rate: 60 FPS"),
                text("GPU Accelerated: Yes"),
                text("Active Animations: 2"),
            ))
            .background(Color::black().opacity(0.8))
            .color(Color::white())
            .padding(10.0)
            .into_view()
        } else {
            empty().into_view()
        }),
        
        button("Toggle Debug")
            .on_press(move || show_debug.update(|d| !d)),
    ))
    .spacing(10.0)
}
```

## Best Practices

1. **Keep Animations Purposeful**: Every animation should serve a purpose - providing feedback, guiding attention, or enhancing understanding.

2. **Follow Platform Conventions**: Respect platform-specific animation durations and curves.

3. **Test Performance**: Profile your animations on target devices to ensure smooth performance.

4. **Provide Accessibility Options**: Allow users to disable animations if needed for accessibility.

5. **Use Appropriate Durations**: 
   - Micro-interactions: 100-200ms
   - Page transitions: 300-500ms
   - Loading animations: Continuous
   - Attention-seeking: 500-800ms

6. **Optimize for Battery Life**: Avoid unnecessary continuous animations on mobile devices.

By following these patterns and best practices, you can create smooth, performant animations that enhance your WaterUI applications' user experience while maintaining good performance across all target platforms.