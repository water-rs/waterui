# WaterUI Task

A lightweight, no-std compatible framework for spawning and managing asynchronous tasks.
This crate provides a simple yet powerful API for task execution with platform-specific
optimizations.

## Core Features

- **No-std compatible**: Works in environments without the standard library
- **Thread-safe task execution**: `Task<T>` handles for sending tasks between threads
- **Thread-local tasks**: `LocalTask<T>` for tasks that must remain on the same thread
- **Priority-based scheduling**: Control execution priority of your tasks
- **Main thread execution**: API for ensuring code runs on the main/UI thread
- **Timing utilities**: Delay execution with timers and sleep functions
- **Platform optimizations**: Uses platform-specific scheduling where available

## Task Types

### `Task<T>`

A handle to a spawned asynchronous task that can be shared between threads:

```rust
# use waterui_task::Task;
# async fn perform_work() {}
# async fn background_work() {}
# async fn update_ui() {}
# use waterui_task::Priority;
// Create a task with default priority
let task = Task::new(async { perform_work() });

// Create a task with background priority
let bg_task = Task::with_priority(async { background_work() }, Priority::Background);

// Ensure a task runs on the main thread
let main_task = Task::on_main(async { update_ui() });
```

### `LocalTask<T>`

Similar to `Task<T>` but for futures that are not `Send` and must run on the same thread:

```rust
# use waterui_task::LocalTask;
# async fn use_thread_local_data() {}
// Execute a non-Send future
let local_task = LocalTask::new(async { use_thread_local_data() });
```

## Main Thread Safety

The `MainValue<T>` wrapper provides safe handling of values that must be accessed on the main thread:

```rust
# use waterui_task::MainValue;
# fn create_ui_element() -> u32 { 42 }
// Create a value that will be accessed only on the main thread
let ui_element = MainValue::new(create_ui_element());

# async fn example(ui_element: MainValue<u32>) {
// All operations occur safely on the main thread
ui_element.handle(|elem| elem + 1).await;
# }
```

## Timing Utilities

```rust
# use std::time::Duration;
# use waterui_task::timer::{Timer, sleep};
# async fn example() {
// Wait for a specific duration
Timer::after(Duration::from_secs(1)).await;

// Sleep for a given number of seconds
sleep(5).await;
# }
```

## Platform-Specific Implementations

The crate provides optimized implementations for different platforms:

- **Apple platforms**: Uses Grand Central Dispatch (GCD) for efficient background execution

## Usage Example

```rust
# use waterui_task::{task, timer::sleep};
async fn example() {
    // Spawn a background task
    let task = task(async {
        // Do some work
        println!("Working in the background");
        42
    });

    // Wait for some time
    sleep(1).await;

    // Get the result
    let result = task.await;
    println!("Task completed with result: {}", result);
}
```

This crate is part of the WaterUI framework but can be used independently in any async Rust
application that needs lightweight task management capabilities.
