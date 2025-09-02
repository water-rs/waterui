# Performance Optimization

Performance is critical for creating smooth, responsive user interfaces. This chapter covers strategies for optimizing WaterUI applications, from reactive state management to rendering efficiency.

## Performance Fundamentals

### Understanding WaterUI's Performance Model

WaterUI's reactive architecture provides several performance advantages:

- **Fine-grained Reactivity**: Only components that depend on changed data re-render
- **Computed Memoization**: Expensive calculations are cached automatically
- **Lazy Evaluation**: Views are only computed when needed
- **Memory Efficiency**: Rust's ownership system prevents memory leaks

### Performance Metrics

Key metrics to monitor:

- **Frame Rate**: Target 60 FPS for smooth animations
- **Memory Usage**: Track allocation patterns and leaks
- **CPU Usage**: Monitor reactive computation overhead
- **Startup Time**: Measure application initialization
- **Response Time**: User interaction to UI update latency

## Reactive State Optimization

### Efficient Signal Usage

```rust,ignore
use waterui::*;
use nami::*;

// ❌ Inefficient: Creating new signals unnecessarily
fn inefficient_component() -> impl View {
    let counter = binding(0); // Creates new binding on every render
    
    vstack((
        text!("Count: {}", counter),
        button("+1", move || counter.set(counter.get() + 1)),
    ))
}

// ✅ Efficient: Reuse signals properly
fn efficient_component(counter: Binding<i32>) -> impl View {
    vstack((
        text!("Count: {}", counter),
        button("+1", move || counter.update(|c| *c += 1)),
    ))
}

// ✅ Efficient: Use computed signals for derived state
fn optimized_counter(counter: Binding<i32>) -> impl View {
    let is_even = s!(counter % 2 == 0);
    let status_text = s!(if is_even { "Even" } else { "Odd" });
    let status_color = s!(if is_even { Color::green() } else { Color::red() });
    
    vstack((
        text!("Count: {}", counter),
        text!("Status: {}", status_text).color(status_color),
        button("+1", move || counter.update(|c| *c += 1)),
    ))
}
```

### Batching State Updates

```rust,ignore
use nami::*;

// ❌ Inefficient: Multiple individual updates
fn update_user_inefficient(user: Binding<User>) {
    user.update(|u| u.name = "New Name".to_string());
    user.update(|u| u.email = "new@example.com".to_string());
    user.update(|u| u.age = 30);
    // Each update triggers dependent computations
}

// ✅ Efficient: Batch updates together
fn update_user_efficient(user: Binding<User>) {
    user.update(|u| {
        u.name = "New Name".to_string();
        u.email = "new@example.com".to_string();
        u.age = 30;
    }); // Only one update, one re-computation
}

// ✅ Advanced: Transaction-like updates
fn bulk_update_users(users: Binding<Vec<User>>) {
    users.update(|users_vec| {
        for user in users_vec.iter_mut() {
            user.last_active = chrono::Utc::now();
            user.status = UserStatus::Active;
        }
    });
}
```

### Avoiding Unnecessary Computations

```rust,ignore
use nami::*;

// ❌ Inefficient: Expensive computation in signal
fn inefficient_expensive_calc(data: Binding<Vec<f64>>) -> impl View {
    let result = s!({
        // This runs on every data change, even for small changes
        expensive_calculation(data.as_slice())
    });
    
    text!("Result: {:.2}", result)
}

// ✅ Efficient: Memoized computation with dependencies
fn efficient_expensive_calc(data: Binding<Vec<f64>>) -> impl View {
    let data_hash = s!(calculate_hash(data.as_slice())); // Fast hash
    let result = s!({
        // Only recalculate if hash changes
        if data_hash != previous_hash {
            expensive_calculation(data.as_slice())
        } else {
            cached_result
        }
    });
    
    text!("Result: {:.2}", result)
}

// ✅ Better: Use proper memoization
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

lazy_static! {
    static ref CALCULATION_CACHE: Arc<Mutex<HashMap<u64, f64>>> = 
        Arc::new(Mutex::new(HashMap::new()));
}

fn memoized_expensive_calc(data: Binding<Vec<f64>>) -> impl View {
    let result = s!({
        let hash = calculate_hash(data.as_slice());
        
        if let Some(&cached) = CALCULATION_CACHE.lock().unwrap().get(&hash) {
            cached
        } else {
            let result = expensive_calculation(data.as_slice());
            CALCULATION_CACHE.lock().unwrap().insert(hash, result);
            result
        }
    });
    
    text!("Result: {:.2}", result)
}
```

## View Optimization

### Component Composition

```rust,ignore
use waterui::*;
use nami::*;

// ❌ Inefficient: Monolithic component
fn large_dashboard(state: AppState) -> impl View {
    vstack((
        // Header section
        hstack((
            text!("Dashboard"),
            text!("User: {}", state.user.name),
            text!("Last Update: {}", state.last_update),
        )),
        // Stats section  
        hstack((
            text!("Total: {}", state.stats.total),
            text!("Active: {}", state.stats.active),
            text!("Pending: {}", state.stats.pending),
        )),
        // Content section
        vstack(s!(state.items.iter().map(|item| {
            item_view(item.clone())
        }).collect::<Vec<_>>())),
        // Footer
        text!("© 2024 MyApp"),
    ))
}

// ✅ Efficient: Modular components
fn dashboard_header(user: Computed<User>, last_update: Computed<DateTime<Utc>>) -> impl View {
    hstack((
        text!("Dashboard"),
        text!("User: {}", user.name),
        text!("Last Update: {}", last_update.format("%H:%M")),
    ))
}

fn dashboard_stats(stats: Computed<Stats>) -> impl View {
    hstack((
        stat_card("Total", s!(stats.total)),
        stat_card("Active", s!(stats.active)),
        stat_card("Pending", s!(stats.pending)),
    ))
}

fn dashboard_content(items: Computed<Vec<Item>>) -> impl View {
    vstack(s!(items.iter().map(|item| {
        item_view(item.clone())
    }).collect::<Vec<_>>()))
}

fn optimized_dashboard(state: AppState) -> impl View {
    vstack((
        dashboard_header(s!(state.user), s!(state.last_update)),
        dashboard_stats(s!(state.stats)),
        dashboard_content(s!(state.items)),
        dashboard_footer(),
    ))
}
```

### List Virtualization

```rust,ignore
use waterui::*;
use nami::*;

// ❌ Inefficient: Rendering all items
fn inefficient_large_list(items: Binding<Vec<ListItem>>) -> impl View {
    scroll_view(
        vstack(s!(items.iter().map(|item| {
            list_item_view(item.clone())
        }).collect::<Vec<_>>()))
    )
}

// ✅ Efficient: Virtual scrolling
struct VirtualList {
    items: Binding<Vec<ListItem>>,
    visible_range: Binding<Range<usize>>,
    item_height: f64,
    container_height: f64,
}

impl View for VirtualList {
    fn body(self) -> impl View {
        let items_count = s!(self.items.len());
        let total_height = s!(items_count * self.item_height as usize);
        let visible_items = s!({
            let range = self.visible_range.get();
            self.items.get()[range.clone()].to_vec()
        });
        
        scroll_view(
            vstack((
                // Spacer for items above visible range
                spacer()
                    .frame_height(s!(self.visible_range.start * self.item_height as usize)),
                // Visible items
                vstack(s!(visible_items.iter().map(|item| {
                    list_item_view(item.clone())
                        .frame_height(self.item_height)
                }).collect::<Vec<_>>())),
                // Spacer for items below visible range
                spacer()
                    .frame_height(s!(total_height - (self.visible_range.end * self.item_height as usize))),
            ))
        )
        .on_scroll({
            let visible_range = self.visible_range.clone();
            let item_height = self.item_height;
            let container_height = self.container_height;
            
            move |scroll_offset: f64| {
                let start_index = (scroll_offset / item_height).floor() as usize;
                let visible_count = (container_height / item_height).ceil() as usize + 1;
                let end_index = start_index + visible_count;
                
                visible_range.set(start_index..end_index);
            }
        })
    }
}

fn efficient_large_list(items: Binding<Vec<ListItem>>) -> impl View {
    VirtualList {
        items,
        visible_range: binding(0..20), // Show first 20 items initially
        item_height: 60.0,
        container_height: 400.0,
    }
}
```

### Conditional Rendering Optimization

```rust,ignore
use waterui::*;
use nami::*;

// ❌ Inefficient: Always creating both views
fn inefficient_conditional(show_detail: Binding<bool>, data: Data) -> impl View {
    vstack((
        if show_detail.get() {
            Some(detailed_view(data.clone())) // Always created
        } else {
            Some(summary_view(data.clone()))  // Always created
        },
    ))
}

// ✅ Efficient: Lazy conditional rendering
fn efficient_conditional(show_detail: Binding<bool>, data: Data) -> impl View {
    s!(if show_detail {
        Some(detailed_view(data.clone()))
    } else {
        Some(summary_view(data.clone()))
    })
}

// ✅ Advanced: Cached conditional views
struct ConditionalView {
    show_detail: Binding<bool>,
    data: Data,
    detailed_cache: Option<Box<dyn View>>,
    summary_cache: Option<Box<dyn View>>,
}

impl View for ConditionalView {
    fn body(mut self) -> impl View {
        s!(if self.show_detail {
            if self.detailed_cache.is_none() {
                self.detailed_cache = Some(Box::new(detailed_view(self.data.clone())));
            }
            self.detailed_cache.as_ref().unwrap()
        } else {
            if self.summary_cache.is_none() {
                self.summary_cache = Some(Box::new(summary_view(self.data.clone())));
            }
            self.summary_cache.as_ref().unwrap()
        })
    }
}
```

## Memory Optimization

### String Handling

```rust,ignore
use waterui::*;
use nami::*;
use std::borrow::Cow;

// ❌ Inefficient: String allocations
fn inefficient_text_formatting(count: Binding<i32>) -> impl View {
    text!(format!("Count: {}", count)) // New string allocation each time
}

// ✅ Efficient: Use text! macro with interpolation
fn efficient_text_formatting(count: Binding<i32>) -> impl View {
    text!("Count: {}", count) // Optimized by text! macro
}

// ✅ Advanced: Cow for conditional string allocation
fn smart_text_formatting(count: Binding<i32>) -> impl View {
    let formatted_text = s!({
        let count_val = count.get();
        if count_val == 0 {
            Cow::Borrowed("No items")
        } else if count_val == 1 {
            Cow::Borrowed("1 item")
        } else {
            Cow::Owned(format!("{} items", count_val))
        }
    });
    
    text!(formatted_text)
}
```

### Resource Cleanup

```rust,ignore
use waterui::*;
use nami::*;
use std::sync::{Arc, Weak};

// Automatic cleanup with weak references
struct ResourceManager {
    active_resources: Arc<Mutex<Vec<Weak<Resource>>>>,
}

impl ResourceManager {
    fn cleanup_expired(&self) {
        let mut resources = self.active_resources.lock().unwrap();
        resources.retain(|weak_ref| weak_ref.upgrade().is_some());
    }
    
    fn register_resource(&self, resource: Arc<Resource>) {
        let mut resources = self.active_resources.lock().unwrap();
        resources.push(Arc::downgrade(&resource));
    }
}

// Component with proper cleanup
struct MediaViewer {
    image_url: Binding<String>,
    resource_manager: Arc<ResourceManager>,
}

impl Drop for MediaViewer {
    fn drop(&mut self) {
        self.resource_manager.cleanup_expired();
    }
}

impl View for MediaViewer {
    fn body(self) -> impl View {
        image(self.image_url)
            .on_load({
                let resource_manager = self.resource_manager.clone();
                move |resource| {
                    resource_manager.register_resource(resource);
                }
            })
    }
}
```

## Animation Performance

### Efficient Animations

```rust,ignore
use waterui::*;
use nami::*;
use std::time::{Duration, Instant};

// ❌ Inefficient: Frequent state updates
fn inefficient_animation() -> impl View {
    let position = binding(0.0);
    
    // Bad: Updates every millisecond
    std::thread::spawn({
        let position = position.clone();
        move || {
            loop {
                std::thread::sleep(Duration::from_millis(1));
                position.update(|p| *p += 0.1);
            }
        }
    });
    
    circle()
        .offset(s!(position), 0.0)
}

// ✅ Efficient: Frame-based animation
fn efficient_animation() -> impl View {
    let start_time = binding(Instant::now());
    let position = s!({
        let elapsed = start_time.elapsed().as_secs_f64();
        (elapsed * 100.0).sin() * 50.0
    });
    
    circle()
        .offset(position, 0.0)
        .animation_frame(move || {
            start_time.set(Instant::now());
        })
}

// ✅ Advanced: Physics-based animation
struct SpringAnimation {
    target: Binding<f64>,
    current: Binding<f64>,
    velocity: Binding<f64>,
    spring_constant: f64,
    damping: f64,
}

impl SpringAnimation {
    fn update(&self, dt: f64) {
        let target = self.target.get();
        let current = self.current.get();
        let velocity = self.velocity.get();
        
        let force = (target - current) * self.spring_constant;
        let damping_force = -velocity * self.damping;
        let acceleration = force + damping_force;
        
        let new_velocity = velocity + acceleration * dt;
        let new_position = current + new_velocity * dt;
        
        self.velocity.set(new_velocity);
        self.current.set(new_position);
    }
}

fn spring_animated_view() -> impl View {
    let animation = SpringAnimation {
        target: binding(100.0),
        current: binding(0.0),
        velocity: binding(0.0),
        spring_constant: 300.0,
        damping: 20.0,
    };
    
    rectangle()
        .offset(s!(animation.current), 0.0)
        .on_tap({
            let target = animation.target.clone();
            move || {
                target.update(|t| *t = if *t == 0.0 { 100.0 } else { 0.0 });
            }
        })
        .animation_frame({
            let animation = animation.clone();
            move || {
                animation.update(1.0 / 60.0); // 60 FPS
            }
        })
}
```

## Profiling and Debugging

### Performance Monitoring

```rust,ignore
use waterui::*;
use nami::*;
use std::time::Instant;

// Performance measurement utilities
struct PerformanceMonitor {
    render_times: Binding<Vec<Duration>>,
    average_frame_time: Computed<Duration>,
    fps: Computed<f64>,
}

impl PerformanceMonitor {
    fn new() -> Self {
        let render_times = binding(Vec::new());
        let average_frame_time = s!({
            let times = render_times.as_slice();
            if times.is_empty() {
                Duration::from_millis(16) // Default to 60 FPS
            } else {
                let sum: Duration = times.iter().sum();
                sum / times.len() as u32
            }
        });
        let fps = s!(1.0 / average_frame_time.as_secs_f64());
        
        Self {
            render_times,
            average_frame_time,
            fps,
        }
    }
    
    fn record_frame_time(&self, duration: Duration) {
        self.render_times.update(|times| {
            times.push(duration);
            if times.len() > 60 {
                times.remove(0);
            }
        });
    }
}

// Debug overlay component
fn debug_overlay(monitor: PerformanceMonitor) -> impl View {
    vstack((
        text!("FPS: {:.1}", monitor.fps),
        text!("Frame Time: {:.2}ms", monitor.average_frame_time.as_millis()),
        text!("Memory Usage: {}MB", get_memory_usage_mb()),
    ))
    .background(Color::black().opacity(0.7))
    .color(Color::white())
    .padding(10)
    .position(.top_trailing)
}

// Timed component wrapper
fn timed_component<V: View>(name: &str, view: V) -> impl View {
    let start_time = Instant::now();
    let result = view;
    let elapsed = start_time.elapsed();
    
    if elapsed > Duration::from_millis(16) {
        println!("Slow component '{}': {:.2}ms", name, elapsed.as_millis());
    }
    
    result
}

// Usage in app
fn app() -> impl View {
    let monitor = PerformanceMonitor::new();
    
    zstack((
        main_content(),
        debug_overlay(monitor.clone()),
    ))
    .on_frame({
        let monitor = monitor.clone();
        let mut last_frame = Instant::now();
        
        move || {
            let now = Instant::now();
            let frame_time = now - last_frame;
            monitor.record_frame_time(frame_time);
            last_frame = now;
        }
    })
}
```

### Memory Profiling

```rust,ignore
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

// Custom allocator for tracking memory usage
struct TrackingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

fn get_allocated_memory() -> usize {
    ALLOCATED.load(Ordering::Relaxed)
}

// Memory usage component
fn memory_monitor() -> impl View {
    let memory_usage = s!(get_allocated_memory());
    
    text!("Memory: {:.2}MB", memory_usage as f64 / 1024.0 / 1024.0)
        .color(s!(if memory_usage > 100 * 1024 * 1024 { // 100MB
            Color::red()
        } else if memory_usage > 50 * 1024 * 1024 { // 50MB
            Color::yellow()
        } else {
            Color::green()
        }))
}
```

## Best Practices Summary

### 1. Reactive State
- Use `s!` macro for computed values
- Batch state updates when possible
- Avoid unnecessary signal dependencies
- Cache expensive computations

### 2. View Composition
- Break large components into smaller ones
- Use lazy evaluation for conditional views
- Implement virtualization for large lists
- Minimize view hierarchy depth

### 3. Memory Management
- Use `text!` macro instead of string formatting
- Implement proper resource cleanup
- Monitor memory usage in development
- Use weak references to prevent cycles

### 4. Animation
- Target 60 FPS for smooth animations
- Use frame-based timing instead of timers
- Implement easing functions for natural motion
- Optimize animation calculations

### 5. Profiling
- Measure performance regularly
- Use debug overlays during development
- Profile memory allocation patterns
- Test on target hardware

By following these performance optimization strategies, your WaterUI applications will be fast, responsive, and efficient across all target platforms.
