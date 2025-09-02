# Memory Management

Memory management is crucial for building performant and reliable WaterUI applications. This chapter covers WaterUI's approach to memory management, optimization techniques, and best practices for avoiding memory leaks and ensuring efficient resource usage.

## Understanding WaterUI Memory Model

WaterUI leverages Rust's ownership system and provides additional abstractions for safe memory management in UI contexts.

### Memory Safety Guarantees

```rust,ignore
use waterui::*;
use nami::s;

// WaterUI automatically manages memory for views
fn memory_safe_view() -> impl View {
    let data = s!(vec![1, 2, 3, 4, 5)); // Automatically cleaned up
    
    vstack(
        data.get().iter().map(|&item| {
            text(format!("Item: {}", item)) // Temporary strings handled safely
        }).collect::<Vec<_>>()
    )
    // No manual cleanup needed - Rust's RAII handles everything
}
```

### Reference Counting for Shared State

```rust,ignore
use std::rc::Rc;
use std::sync::Arc;

// Shared immutable data
fn shared_data_example() -> impl View {
    let shared_data = Rc::new(vec!["Apple", "Banana", "Cherry"));
    
    vstack((
        // Multiple views can safely share this data
        fruit_list(shared_data.clone()),
        fruit_count(shared_data.clone()),
    ))
}

fn fruit_list(data: Rc<Vec<&'static str>>) -> impl View {
    vstack(
        data.iter().map(|&fruit| {
            text(fruit)
        }).collect::<Vec<_>>()
    )
}

fn fruit_count(data: Rc<Vec<&'static str>>) -> impl View {
    text(format!("Total fruits: {}", data.len()))
}

// Thread-safe shared data for concurrent access
fn thread_safe_shared_data() -> impl View {
    let shared_data = Arc::new(vec![1, 2, 3, 4, 5));
    let processed = s!(Vec::new());
    
    vstack((
        button("Process Data")
            .on_press({
                let data = shared_data.clone();
                let result = processed.clone();
                move || {
                    let data = data.clone();
                    let result = result.clone();
                    
                    tokio::spawn(async move {
                        // Process data in background thread
                        let processed_data: Vec<i32> = data.iter()
                            .map(|&x| x * 2)
                            .collect();
                        
                        result.set(processed_data);
                    });
                }
            }),
        
        vstack(
            processed.get().iter().map(|&item| {
                text(format!("Processed: {}", item))
            }).collect::<Vec<_>>()
        ),
    ))
}
```

## Managing View Lifecycle

### View Creation and Cleanup

```rust,ignore
// Views are created and destroyed as needed
fn dynamic_view_example() -> impl View {
    let show_details = s!(false);
    let selected_item = s!(None::<String>);
    
    vstack((
        button("Toggle Details")
            .on_press({
                let show = show_details.clone();
                move || show.set(!show.get())
            }),
        
        // Conditional views are created/destroyed automatically
        if show_details.get() {
            // This expensive view is only created when needed
            expensive_detail_view(selected_item.clone())
        } else {
            // Lightweight placeholder
            text("Click to show details")
        },
    ))
}

fn expensive_detail_view(selected: Binding<Option<String>>) -> impl View {
    // Expensive computation only happens when view is created
    let computed_data = compute_expensive_data();
    
    vstack((
        text("Detailed Information"),
        text(format!("Computed: {}", computed_data)),
        
        // Cleanup happens automatically when view is destroyed
    ))
    // Any Drop implementations will be called automatically
}

fn compute_expensive_data() -> String {
    // Simulate expensive computation
    "Expensive Result".to_string()
}
```

### Resource Management with RAII

```rust,ignore
use std::sync::Arc;
use std::sync::Mutex;

// Custom resource that needs cleanup
struct DatabaseConnection {
    id: u32,
    connected: bool,
}

impl DatabaseConnection {
    fn new(id: u32) -> Self {
        println!("Opening database connection {}", id);
        Self {
            id,
            connected: true,
        }
    }
    
    fn query(&self, sql: &str) -> Vec<String> {
        if self.connected {
            vec![format!("Result for: {}", sql)]
        } else {
            vec![]
        }
    }
}

impl Drop for DatabaseConnection {
    fn drop(&mut self) {
        if self.connected {
            println!("Closing database connection {}", self.id);
            self.connected = false;
        }
    }
}

// Resource is automatically cleaned up when view is destroyed
fn database_view() -> impl View {
    // Connection is created when view is built
    let connection = Arc::new(Mutex::new(DatabaseConnection::new(1)));
    let query_results = s!(Vec::<String>::new());
    
    vstack((
        button("Query Database")
            .on_press({
                let conn = connection.clone();
                let results = query_results.clone();
                move || {
                    if let Ok(conn) = conn.lock() {
                        let data = conn.query("SELECT * FROM users");
                        results.set(data);
                    }
                }
            }),
        
        vstack(
            query_results.get().iter().map(|result| {
                text(result.clone())
            }).collect::<Vec<_>>()
        ),
    ))
    // Connection is automatically dropped when view is destroyed
}
```

## State Management and Memory

### Efficient State Updates

```rust,ignore
// Avoid unnecessary allocations
fn efficient_state_updates() -> impl View {
    let items = s!(Vec::<String>::new());
    let filter = s!(String::new());
    
    // Efficient: Modify existing vector instead of creating new one
    let add_item = {
        let items = items.clone();
        move |new_item: String| {
            items.update(|items| {
                items.push(new_item); // Modify in place
            });
        }
    };
    
    // Efficient: Use filter without creating intermediate collections
    let filtered_items = items.map({
        let filter = filter.clone();
        move |items| {
            items.iter()
                .filter(|item| {
                    let filter_text = filter.get();
                    filter_text.is_empty() || item.contains(&filter_text)
                })
                .cloned()
                .collect::<Vec<_>>()
        }
    });
    
    vstack((
        hstack((
            text_field("Add item")
                .on_submit({
                    let add_item = add_item.clone();
                    move |text| add_item(text)
                }),
            
            text_field("Filter")
                .bind_text(filter.clone()),
        )),
        
        scroll_view(
            vstack(
                filtered_items.get().iter().map(|item| {
                    text(item.clone())
                }).collect::<Vec<_>>()
            )
        ),
    ))
}

// Memory-efficient large list handling
fn large_list_example() -> impl View {
    let items = s!((0..10000).map(|i| format!("Item {}", i)).collect::<Vec<_>>());
    let visible_range = s!((0, 50)); // Only render visible items
    
    let (start, end) = visible_range.get();
    
    vstack((
        text(format!("Showing items {} to {} of {}", start, end, items.get().len())),
        
        // Only render visible items to save memory
        scroll_view(
            vstack(
                items.get()[start..end.min(items.get().len())].iter().map(|item| {
                    text(item.clone())
                }).collect::<Vec<_>>()
            )
        )
        .on_scroll({
            let visible_range = visible_range.clone();
            move |scroll_info| {
                // Update visible range based on scroll position
                let items_per_screen = 50;
                let new_start = (scroll_info.offset.y / 30.0) as usize; // Assuming 30px per item
                let new_end = new_start + items_per_screen;
                visible_range.set((new_start, new_end));
            }
        }),
        
        hstack((
            button("Previous")
                .on_press({
                    let visible_range = visible_range.clone();
                    move || {
                        let (start, end) = visible_range.get();
                        let new_start = start.saturating_sub(50);
                        let new_end = new_start + 50;
                        visible_range.set((new_start, new_end));
                    }
                }),
            
            button("Next")
                .on_press({
                    let visible_range = visible_range.clone();
                    let items = items.clone();
                    move || {
                        let (start, end) = visible_range.get();
                        let max_items = items.get().len();
                        let new_start = (end).min(max_items.saturating_sub(50));
                        let new_end = (new_start + 50).min(max_items);
                        visible_range.set((new_start, new_end));
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
}
```

### Memory Pooling for Frequent Allocations

```rust,ignore
use std::collections::VecDeque;

// Object pool for reusing expensive objects
struct ObjectPool<T> {
    objects: Arc<Mutex<VecDeque<T>>>,
    factory: Arc<dyn Fn() -> T + Send + Sync>,
}

impl<T> ObjectPool<T> {
    fn new<F>(factory: F) -> Self 
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self {
            objects: Arc::new(Mutex::new(VecDeque::new())),
            factory: Arc::new(factory),
        }
    }
    
    fn acquire(&self) -> PooledObject<T> {
        let obj = {
            let mut objects = self.objects.lock().unwrap();
            objects.pop_front().unwrap_or_else(|| (self.factory)())
        };
        
        PooledObject {
            object: Some(obj),
            pool: self.objects.clone(),
        }
    }
}

struct PooledObject<T> {
    object: Option<T>,
    pool: Arc<Mutex<VecDeque<T>>>,
}

impl<T> std::ops::Deref for PooledObject<T> {
    type Target = T;
    
    fn deref(&self) -> &Self::Target {
        self.object.as_ref().unwrap()
    }
}

impl<T> std::ops::DerefMut for PooledObject<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.object.as_mut().unwrap()
    }
}

impl<T> Drop for PooledObject<T> {
    fn drop(&mut self) {
        if let Some(obj) = self.object.take() {
            let mut pool = self.pool.lock().unwrap();
            pool.push_back(obj);
        }
    }
}

// Example: Pooled string buffers for text processing
fn text_processing_example() -> impl View {
    lazy_static::lazy_static! {
        static ref STRING_POOL: ObjectPool<String> = ObjectPool::new(|| String::with_capacity(1024));
    }
    
    let processed_text = s!(String::new());
    let input_text = s!(String::new());
    
    vstack((
        text_field("Enter text to process")
            .bind_text(input_text.clone()),
        
        button("Process Text")
            .on_press({
                let input = input_text.clone();
                let output = processed_text.clone();
                move || {
                    // Acquire a pooled string buffer
                    let mut buffer = STRING_POOL.acquire();
                    buffer.clear();
                    
                    // Process text using the buffer
                    let input_text = input.get();
                    for word in input_text.split_whitespace() {
                        buffer.push_str(&word.to_uppercase());
                        buffer.push(' ');
                    }
                    
                    output.set(buffer.clone());
                    // Buffer is automatically returned to pool when dropped
                }
            }),
        
        text(processed_text.get())
            .color(Color::blue()),
    ))
    .spacing(10.0)
}
```

## Async Memory Management

### Managing Async Tasks and Cancellation

```rust,ignore
use tokio::sync::watch;
use tokio::task::JoinHandle;
use std::sync::atomic::{AtomicBool, Ordering};

struct TaskManager {
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    shutdown_signal: Arc<AtomicBool>,
}

impl TaskManager {
    fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(Vec::new())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
        }
    }
    
    fn spawn_task<F, Fut>(&self, task: F) -> JoinHandle<()>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let shutdown = self.shutdown_signal.clone();
        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = task() => {},
                _ = tokio::time::sleep(Duration::from_secs(3600)) => {
                    // Task timeout
                }
            }
        });
        
        self.tasks.lock().unwrap().push(handle.clone());
        handle
    }
    
    fn shutdown(&self) {
        self.shutdown_signal.store(true, Ordering::Relaxed);
        
        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.drain(..) {
            task.abort();
        }
    }
}

impl Drop for TaskManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// Proper async task lifecycle management
fn async_data_loader() -> impl View {
    let task_manager = Arc::new(TaskManager::new());
    let loading_state = s!(false);
    let data = s!(Vec::<String>::new());
    let error = s!(None::<String>);
    
    vstack((
        button("Load Data")
            .disabled(loading_state.get())
            .on_press({
                let manager = task_manager.clone();
                let loading = loading_state.clone();
                let data = data.clone();
                let error = error.clone();
                
                move || {
                    loading.set(true);
                    error.set(None);
                    
                    manager.spawn_task({
                        let loading = loading.clone();
                        let data = data.clone();
                        let error = error.clone();
                        
                        move || async move {
                            match load_data_from_api().await {
                                Ok(result) => {
                                    data.set(result);
                                }
                                Err(e) => {
                                    error.set(Some(e.to_string()));
                                }
                            }
                            loading.set(false);
                        }
                    });
                }
            }),
        
        if loading_state.get() {
            hstack((
                spinner(),
                text("Loading..."),
            ))
            .spacing(10.0)
        } else {
            empty()
        },
        
        if let Some(err) = error.get() {
            text(format!("Error: {}", err))
                .color(Color::red())
        } else {
            empty()
        },
        
        scroll_view(
            vstack(
                data.get().iter().map(|item| {
                    text(item.clone())
                }).collect::<Vec<_>>()
            )
        ),
    ))
    .spacing(10.0)
    // task_manager is automatically cleaned up, cancelling all tasks
}

async fn load_data_from_api() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Simulate API call
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(vec!["Item 1".to_string(), "Item 2".to_string(), "Item 3".to_string()))
}

fn spinner() -> impl View {
    let rotation = s!(0.0);
    
    // Start rotation animation
    {
        let rotation = rotation.clone();
        tokio::spawn(async move {
            loop {
                rotation.set((rotation.get() + 5.0) % 360.0);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
    }
    
    circle()
        .size(20.0)
        .stroke(Color::blue(), 2.0)
        .stroke_dash([5.0, 5.0))
        .rotation(rotation.get())
        .animation(Animation::linear(Duration::from_millis(50)))
}
```

### Preventing Memory Leaks in Event Handlers

```rust,ignore
use std::sync::Weak;

// Use weak references to prevent retain cycles
fn event_handler_example() -> impl View {
    let data = Arc::new(Mutex::new(vec![1, 2, 3, 4, 5)));
    let display_data = s!(Vec::<i32>::new());
    
    // Create weak reference to avoid retain cycle
    let weak_data = Arc::downgrade(&data);
    
    vstack((
        button("Update Data")
            .on_press(move || {
                // Use weak reference in closure
                if let Some(data) = weak_data.upgrade() {
                    let mut data = data.lock().unwrap();
                    data.push(data.len() as i32 + 1);
                    display_data.set(data.clone());
                }
                // If data has been dropped, this becomes a no-op
            }),
        
        vstack(
            display_data.get().iter().map(|&item| {
                text(format!("Item: {}", item))
            }).collect::<Vec<_>>()
        ),
    ))
    .spacing(10.0)
}

// Proper cleanup of event listeners
struct EventListener {
    id: u32,
    active: Arc<AtomicBool>,
}

impl EventListener {
    fn new(id: u32) -> Self {
        Self {
            id,
            active: Arc::new(AtomicBool::new(true)),
        }
    }
    
    fn listen<F>(&self, mut callback: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        let active = self.active.clone();
        let id = self.id;
        
        tokio::spawn(async move {
            while active.load(Ordering::Relaxed) {
                // Simulate receiving events
                tokio::time::sleep(Duration::from_millis(1000)).await;
                
                if active.load(Ordering::Relaxed) {
                    callback(format!("Event from listener {}", id));
                }
            }
        });
    }
    
    fn stop(&self) {
        self.active.store(false, Ordering::Relaxed);
    }
}

impl Drop for EventListener {
    fn drop(&mut self) {
        self.stop();
        println!("Event listener {} stopped", self.id);
    }
}

fn event_listener_example() -> impl View {
    let listener = Arc::new(EventListener::new(1));
    let events = s!(Vec::<String>::new());
    
    // Start listening
    {
        let listener = listener.clone();
        let events = events.clone();
        listener.listen(move |event| {
            events.update(|events| {
                events.push(event);
                // Limit to last 10 events to prevent unbounded growth
                if events.len() > 10 {
                    events.remove(0);
                }
            });
        });
    }
    
    vstack((
        text("Event Listener Demo"),
        
        button("Clear Events")
            .on_press({
                let events = events.clone();
                move || events.set(Vec::new())
            }),
        
        scroll_view(
            vstack(
                events.get().iter().rev().map(|event| {
                    text(event.clone())
                        .font_size(14.0)
                }).collect::<Vec<_>>()
            )
        )
        .max_height(200.0),
    ))
    .spacing(10.0)
    // listener is automatically stopped when view is destroyed
}
```

## Memory Profiling and Debugging

### Built-in Memory Monitoring

```rust,ignore
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

// Custom allocator for memory tracking
struct TrackingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ret = System.alloc(layout);
        if !ret.is_null() {
            ALLOCATED.fetch_add(layout.size(), Ordering::SeqCst);
            ALLOCATIONS.fetch_add(1, Ordering::SeqCst);
        }
        ret
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        ALLOCATED.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

// Memory stats view
fn memory_monitor() -> impl View {
    let update_trigger = s!(0);
    
    // Update stats every second
    {
        let trigger = update_trigger.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                trigger.set(trigger.get() + 1);
            }
        });
    }
    
    // Reactive memory stats
    let stats = update_trigger.map(|_| {
        (
            ALLOCATED.load(Ordering::SeqCst),
            ALLOCATIONS.load(Ordering::SeqCst),
        )
    });
    
    let (allocated, allocations) = stats.get();
    
    vstack((
        text("Memory Monitor")
            .font_weight(FontWeight::Bold),
        
        hstack((
            text("Allocated:"),
            text(format!("{} bytes", allocated))
                .color(Color::blue()),
        ))
        .spacing(10.0),
        
        hstack((
            text("Allocations:"),
            text(format!("{}", allocations))
                .color(Color::green()),
        ))
        .spacing(10.0),
        
        hstack((
            text("Avg per allocation:"),
            text(format!("{} bytes", 
                if allocations > 0 { allocated / allocations } else { 0 }))
                .color(Color::orange()),
        ))
        .spacing(10.0),
    ))
    .spacing(5.0)
    .padding(10.0)
    .background(Color::gray().opacity(0.1))
    .corner_radius(5.0)
}
```

### Memory Leak Detection

```rust,ignore
use std::collections::HashMap;
use std::sync::Mutex;

// Reference tracking for leak detection
lazy_static::lazy_static! {
    static ref REFERENCE_TRACKER: Mutex<HashMap<String, usize>> = Mutex::new(HashMap::new());
}

struct LeakTracker {
    name: String,
}

impl LeakTracker {
    fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        {
            let mut tracker = REFERENCE_TRACKER.lock().unwrap();
            *tracker.entry(name.clone()).or_insert(0) += 1;
        }
        Self { name }
    }
}

impl Drop for LeakTracker {
    fn drop(&mut self) {
        let mut tracker = REFERENCE_TRACKER.lock().unwrap();
        if let Some(count) = tracker.get_mut(&self.name) {
            *count -= 1;
            if *count == 0 {
                tracker.remove(&self.name);
            }
        }
    }
}

fn leak_detection_demo() -> impl View {
    let _tracker = LeakTracker::new("leak_detection_demo");
    let show_leaks = s!(false);
    
    vstack((
        button("Toggle Leak Report")
            .on_press({
                let show = show_leaks.clone();
                move || show.set(!show.get())
            }),
        
        if show_leaks.get() {
            leak_report_view()
        } else {
            empty()
        },
        
        // Create some tracked objects
        button("Create Temporary Objects")
            .on_press(|| {
                for i in 0..5 {
                    let _tracker = LeakTracker::new(format!("temp_object_{}", i));
                    // Objects go out of scope and should be cleaned up
                }
            }),
        
        // Create long-lived object
        button("Create Long-lived Object")
            .on_press(|| {
                let _tracker = LeakTracker::new("long_lived_object");
                // This would cause a "leak" until view is destroyed
                std::mem::forget(_tracker); // Intentionally don't drop
            }),
    ))
    .spacing(10.0)
}

fn leak_report_view() -> impl View {
    let update_trigger = s!(0);
    
    // Update report every 2 seconds
    {
        let trigger = update_trigger.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                trigger.set(trigger.get() + 1);
            }
        });
    }
    
    let references = update_trigger.map(|_| {
        let tracker = REFERENCE_TRACKER.lock().unwrap();
        tracker.clone()
    });
    
    vstack((
        text("Active References")
            .font_weight(FontWeight::Bold)
            .color(Color::red()),
        
        if references.get().is_empty() {
            text("No active references (good!)")
                .color(Color::green())
        } else {
            vstack(
                references.get().iter().map(|(name, count)| {
                    hstack((
                        text(name.clone()),
                        text(format!(": {}", count))
                            .color(if *count > 1 { Color::red() } else { Color::orange() }),
                    ))
                    .spacing(10.0)
                }).collect::<Vec<_>>()
            )
        },
    ))
    .spacing(5.0)
    .padding(10.0)
    .background(Color::red().opacity(0.1))
    .corner_radius(5.0)
}
```

## Best Practices

### Memory-Efficient View Patterns

```rust,ignore
// 1. Use lazy evaluation for expensive computations
fn lazy_computation_example() -> impl View {
    let input = s!(10);
    
    // Compute only when input changes
    let expensive_result = input.map(|&value| {
        // This expensive computation only runs when input changes
        (0..value).map(|i| i * i).sum::<i32>()
    });
    
    vstack((
        hstack((
            text("Input:"),
            text_field("")
                .bind_text(input.map(|i| i.to_string())),
        )),
        
        hstack((
            text("Sum of squares:"),
            text(expensive_result.get().to_string())
                .color(Color::blue()),
        )),
    ))
    .spacing(10.0)
}

// 2. Prefer streaming over collecting large datasets
fn streaming_data_example() -> impl View {
    let current_batch = s!(Vec::<String>::new());
    let batch_size = 100;
    let total_processed = s!(0);
    
    vstack((
        hstack((
            text("Processed:"),
            text(total_processed.get().to_string()),
        )),
        
        button("Process Next Batch")
            .on_press({
                let batch = current_batch.clone();
                let total = total_processed.clone();
                move || {
                    // Process data in small batches instead of all at once
                    let new_batch: Vec<String> = (0..batch_size)
                        .map(|i| format!("Item {}", total.get() + i))
                        .collect();
                    
                    batch.set(new_batch);
                    total.set(total.get() + batch_size);
                }
            }),
        
        // Only show current batch, not all processed items
        scroll_view(
            vstack(
                current_batch.get().iter().map(|item| {
                    text(item.clone())
                }).collect::<Vec<_>>()
            )
        )
        .max_height(300.0),
    ))
    .spacing(10.0)
}

// 3. Use string interning for repeated strings
use std::collections::HashSet;

lazy_static::lazy_static! {
    static ref STRING_INTERNER: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

fn intern_string(s: String) -> String {
    let mut interner = STRING_INTERNER.lock().unwrap();
    if let Some(interned) = interner.get(&s) {
        interned.clone()
    } else {
        interner.insert(s.clone());
        s
    }
}

fn string_interning_example() -> impl View {
    let items = s!(Vec::<String>::new());
    
    vstack((
        button("Add Common Strings")
            .on_press({
                let items = items.clone();
                move || {
                    items.update(|items| {
                        // These strings will be interned, saving memory for duplicates
                        for _ in 0..10 {
                            items.push(intern_string("Common String".to_string()));
                            items.push(intern_string("Another Common String".to_string()));
                        }
                    });
                }
            }),
        
        text(format!("Total items: {}", items.get().len())),
        
        button("Clear")
            .on_press({
                let items = items.clone();
                move || items.set(Vec::new())
            }),
    ))
    .spacing(10.0)
}
```

### Memory-Safe Callbacks and Closures

```rust,ignore
// Safe callback patterns that avoid memory leaks
fn safe_callback_patterns() -> impl View {
    let counter = s!(0);
    let timer_active = s!(false);
    
    vstack((
        hstack((
            text("Counter:"),
            text(counter.get().to_string()),
        )),
        
        button(if timer_active.get() { "Stop Timer" } else { "Start Timer" })
            .on_press({
                let counter = counter.clone();
                let active = timer_active.clone();
                move || {
                    if active.get() {
                        active.set(false);
                    } else {
                        active.set(true);
                        
                        // Use weak reference to prevent retain cycle
                        let weak_counter = counter.downgrade();
                        let weak_active = active.downgrade();
                        
                        tokio::spawn(async move {
                            while let (Some(counter), Some(active)) = 
                                (weak_counter.upgrade(), weak_active.upgrade()) {
                                
                                if !active.get() {
                                    break;
                                }
                                
                                counter.set(counter.get() + 1);
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        });
                    }
                }
            }),
    ))
    .spacing(10.0)
}
```

## Platform-Specific Memory Considerations

### Desktop Memory Management

```rust,ignore
// Desktop apps can use more memory but should still be efficient
fn desktop_memory_patterns() -> impl View {
    let image_cache = s!(HashMap::<String, Vec<u8>>::new());
    let cache_size_mb = image_cache.map(|cache| {
        cache.values().map(|data| data.len()).sum::<usize>() / 1_048_576
    });
    
    vstack((
        hstack((
            text("Image cache size:"),
            text(format!("{} MB", cache_size_mb.get())),
        )),
        
        button("Load Image")
            .on_press({
                let cache = image_cache.clone();
                move || {
                    cache.update(|cache| {
                        // Simulate loading image data
                        let image_data = vec![0u8; 1_048_576]; // 1MB image
                        let filename = format!("image_{}.png", cache.len());
                        cache.insert(filename, image_data);
                        
                        // Limit cache to 50MB
                        while cache.values().map(|data| data.len()).sum::<usize>() > 50 * 1_048_576 {
                            if let Some(key) = cache.keys().next().cloned() {
                                cache.remove(&key);
                            }
                        }
                    });
                }
            }),
        
        button("Clear Cache")
            .on_press({
                let cache = image_cache.clone();
                move || cache.set(HashMap::new())
            }),
    ))
    .spacing(10.0)
}
```

### Mobile Memory Management

```rust,ignore
// Mobile apps need to be very memory conscious
fn mobile_memory_patterns() -> impl View {
    let low_memory_mode = s!(false);
    let data = s!(Vec::<String>::new());
    
    // Simulate memory pressure detection
    {
        let low_memory = low_memory_mode.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                
                // Simulate random memory pressure
                if rand::random::<bool>() {
                    low_memory.set(true);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    low_memory.set(false);
                }
            }
        });
    }
    
    vstack((
        if low_memory_mode.get() {
            hstack((
                text("⚠️ Low Memory Mode Active")
                    .color(Color::red()),
            ))
        } else {
            empty()
        },
        
        button("Add Data")
            .on_press({
                let data = data.clone();
                let low_memory = low_memory_mode.clone();
                move || {
                    data.update(|data| {
                        let item_count = if low_memory.get() { 10 } else { 100 };
                        for i in 0..item_count {
                            data.push(format!("Item {}", data.len() + i));
                        }
                        
                        // Aggressively limit data in low memory mode
                        if low_memory.get() && data.len() > 500 {
                            data.truncate(500);
                        }
                    });
                }
            }),
        
        text(format!("Items: {}", data.get().len())),
        
        // Show fewer items in low memory mode
        scroll_view(
            vstack(
                data.get().iter()
                    .take(if low_memory_mode.get() { 20 } else { 100 })
                    .map(|item| text(item.clone()))
                    .collect::<Vec<_>>()
            )
        )
        .max_height(200.0),
    ))
    .spacing(10.0)
}
```

By following these memory management patterns and best practices, you can build WaterUI applications that are both performant and memory-efficient across all target platforms. The key is to leverage Rust's ownership system while being mindful of UI-specific memory patterns and lifecycle management.