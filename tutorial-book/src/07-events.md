# Event Handling

Event handling in WaterUI is straightforward and type-safe. This chapter covers how to respond to user interactions and manage event-driven behavior.

## Basic Event Handling

### Button Click Events

```rust,ignore
use waterui::*;
use nami::s;

fn click_counter() -> impl View {
    let count = binding(0);
    
    vstack((
        text!("Clicked {} times", count),
        button("Click me!")
            .action({
                let count = count.clone();
                move |_| count.update(|c| c + 1)
            }),
    ))
}
```

### Text Input Events

```rust,ignore
fn text_input_demo() -> impl View {
    let input = binding(String::new());
    let char_count = s!(input.len());
    
    vstack((
        text_field(input.clone())
            .placeholder("Type something...")
            .on_change({
                let input = input.clone();
                move |new_value| {
                    println!("Input changed: {}", new_value);
                    input.set(new_value);
                }
            }),
        text!("Characters: {}", char_count),
        text!("You typed: '{}'", input),
    ))
    .spacing(10.0)
}
```

### Toggle and Selection Events

```rust,ignore
fn settings_panel() -> impl View {
    let notifications = binding(true);
    let theme = binding(Theme::Light);
    let volume = binding(50);
    
    vstack((
        // Toggle switch
        toggle(notifications.clone())
            .label("Enable notifications")
            .on_change(move |enabled| {
                println!("Notifications: {}", if enabled { "ON" } else { "OFF" });
            }),
            
        // Picker/dropdown
        picker(theme.clone(), vec![
            (Theme::Light, "Light Theme"),
            (Theme::Dark, "Dark Theme"),
            (Theme::Auto, "Auto Theme"),
        ))
        .on_change(move |new_theme| {
            println!("Theme changed to: {:?}", new_theme);
        }),
        
        // Slider
        slider(volume.clone())
            .range(0..=100)
            .label("Volume")
            .on_change(move |new_volume| {
                println!("Volume: {}%", new_volume);
            }),
        
        // Display current values
        text!("Settings: Notifications={}, Theme={:?}, Volume={}%", 
              notifications, theme, volume),
    ))
    .spacing(15.0)
    .padding(20.0)
}

#[derive(Debug, Clone, PartialEq)]
enum Theme {
    Light,
    Dark,
    Auto,
}
```

## Advanced Event Patterns

### Event Delegation and Bubbling

```rust,ignore
#[derive(Debug, Clone)]
enum TodoAction {
    Toggle(u32),
    Delete(u32),
    Edit(u32, String),
}

fn todo_list() -> impl View {
    let todos = binding(vec![
        Todo { id: 1, text: "Learn WaterUI".to_string(), completed: false },
        Todo { id: 2, text: "Build amazing app".to_string(), completed: true },
    ));
    
    vstack((
        text("My Todo List").size(24.0),
        
        // Event handler at the list level
        vstack(
            todos.get().map(|todos| {
                todos.into_iter().map(|todo| {
                    todo_item(todo, {
                        let todos = todos.clone();
                        move |action| handle_todo_action(todos.clone(), action)
                    })
                })
            })
        ),
        
        // Add new todo
        add_todo_button(todos.clone()),
    ))
    .spacing(10.0)
}

fn handle_todo_action(todos: Binding<Vec<Todo>>, action: TodoAction) {
    match action {
        TodoAction::Toggle(id) => {
            todos.update(|todos| {
                if let Some(todo) = todos.iter_mut().find(|t| t.id == id) {
                    todo.completed = !todo.completed;
                    println!("Toggled todo {}: {}", id, todo.completed);
                }
                todos
            });
        }
        TodoAction::Delete(id) => {
            todos.update(|todos| {
                todos.retain(|t| t.id != id);
                println!("Deleted todo {}", id);
                todos
            });
        }
        TodoAction::Edit(id, new_text) => {
            todos.update(|todos| {
                if let Some(todo) = todos.iter_mut().find(|t| t.id == id) {
                    todo.text = new_text;
                    println!("Edited todo {}", id);
                }
                todos
            });
        }
    }
}

fn todo_item<F>(todo: Todo, on_action: F) -> impl View
where
    F: Fn(TodoAction) + Clone + 'static,
{
    hstack((
        toggle(binding(todo.completed))
            .action({
                let on_action = on_action.clone();
                let id = todo.id;
                move |_| on_action(TodoAction::Toggle(id))
            }),
            
        text(&todo.text)
            .strikethrough(todo.completed)
            .flex(1),
            
        button("Edit")
            .action({
                let on_action = on_action.clone();
                let id = todo.id;
                move |_| {
                    // In a real app, this would open an edit dialog
                    on_action(TodoAction::Edit(id, "Edited text".to_string()));
                }
            }),
            
        button("×")
            .style(ButtonStyle::Destructive)
            .action({
                let on_action = on_action.clone();
                let id = todo.id;
                move |_| on_action(TodoAction::Delete(id))
            }),
    ))
    .padding(8.0)
    .background(Color::item_background())
}

#[derive(Debug, Clone)]
struct Todo {
    id: u32,
    text: String,
    completed: bool,
}
```

### Keyboard Events

```rust,ignore
fn keyboard_demo() -> impl View {
    let key_pressed = binding(String::new());
    let key_count = binding(0);
    
    vstack((
        text("Keyboard Event Demo"),
        text!("Last key pressed: {}", key_pressed),
        text!("Total keys pressed: {}", key_count),
        
        text_field(binding(String::new()))
            .placeholder("Type here to see keyboard events")
            .on_key_down({
                let key_pressed = key_pressed.clone();
                let key_count = key_count.clone();
                move |key| {
                    key_pressed.set(format!("{:?}", key));
                    key_count.update(|c| c + 1);
                }
            }),
    ))
    .spacing(10.0)
    .padding(20.0)
}
```

### Mouse and Touch Events

```rust,ignore
fn interactive_canvas() -> impl View {
    let mouse_pos = binding((0.0, 0.0));
    let is_dragging = binding(false);
    let points = binding(Vec::<(f64, f64)>::new());
    
    vstack((
        text!("Mouse: ({:.0}, {:.0}) - Dragging: {}", 
              s!(mouse_pos.0), s!(mouse_pos.1), is_dragging),
        
        canvas(400.0, 300.0)
            .background(Color::light_gray())
            .on_mouse_move({
                let mouse_pos = mouse_pos.clone();
                let points = points.clone();
                let is_dragging = is_dragging.clone();
                move |pos| {
                    mouse_pos.set(pos);
                    if is_dragging.get() {
                        points.update(|pts| {
                            pts.push(pos);
                            pts
                        });
                    }
                }
            })
            .on_mouse_down({
                let is_dragging = is_dragging.clone();
                let points = points.clone();
                move |pos| {
                    is_dragging.set(true);
                    points.update(|pts| {
                        pts.push(pos);
                        pts
                    });
                }
            })
            .on_mouse_up({
                let is_dragging = is_dragging.clone();
                move |_| is_dragging.set(false)
            }),
            
        hstack((
            button("Clear Canvas")
                .action({
                    let points = points.clone();
                    move |_| points.set(Vec::new())
                }),
            text!("Points drawn: {}", s!(points.len())),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
    .padding(20.0)
}
```

## Event Best Practices

### 1. Use Closures for Event Handlers

```rust,ignore
// Good: Clean closure syntax
button("Save")
    .action({
        let data = data.clone();
        move |_| save_data(data.get())
    })

// Avoid: Complex inline logic
button("Save")
    .action(move |_| {
        // Lots of complex logic here makes the view hard to read
        let validated_data = validate(data.get());
        if validated_data.is_ok() {
            match save_to_database(validated_data.unwrap()) {
                Ok(_) => show_success_message(),
                Err(e) => show_error_message(e),
            }
        }
    })
```

### 2. Extract Complex Event Logic

```rust,ignore
// Good: Extract complex logic to functions
fn save_button(data: Binding<FormData>) -> impl View {
    button("Save")
        .action({
            let data = data.clone();
            move |_| handle_save(data.clone())
        })
}

fn handle_save(data: Binding<FormData>) {
    let form_data = data.get();
    match validate_form(&form_data) {
        Ok(validated) => {
            match save_to_database(&validated) {
                Ok(_) => show_success_notification("Data saved!"),
                Err(e) => show_error_notification(&format!("Save failed: {}", e)),
            }
        }
        Err(validation_errors) => {
            show_validation_errors(validation_errors);
        }
    }
}
```

### 3. Prevent Event Conflicts

```rust,ignore
fn form_with_validation() -> impl View {
    let email = binding(String::new());
    let is_valid = s!(email.contains("@") && !email.is_empty());
    
    vstack((
        text_field(email.clone())
            .placeholder("Enter email")
            .on_change({
                let email = email.clone();
                move |new_email| {
                    // Validate on every change
                    email.set(new_email);
                }
            })
            .on_submit({
                let email = email.clone();
                let is_valid = is_valid.clone();
                move || {
                    // Only submit if valid
                    if is_valid.get() {
                        submit_email(email.get());
                    }
                }
            }),
            
        text!("Email: {} ({})", 
              email, 
              s!(if is_valid { "Valid" } else { "Invalid" }))
              .color(s!(if is_valid { Color::green() } else { Color::red() })),
              
        button("Submit")
            .disabled(s!(!is_valid))
            .action({
                let email = email.clone();
                move |_| submit_email(email.get())
            }),
    ))
    .spacing(10.0)
}
```

### 4. Handle Async Events

```rust,ignore
fn async_button_demo() -> impl View {
    let is_loading = binding(false);
    let result = binding(None::<String>);
    
    vstack((
        button("Fetch Data")
            .disabled(is_loading.get())
            .action({
                let is_loading = is_loading.clone();
                let result = result.clone();
                move |_| {
                    is_loading.set(true);
                    result.set(None);
                    
                    let is_loading = is_loading.clone();
                    let result = result.clone();
                    
                    task::spawn(async move {
                        // Simulate async operation
                        task::sleep(Duration::from_secs(2)).await;
                        
                        match fetch_remote_data().await {
                            Ok(data) => {
                                result.set(Some(format!("Success: {}", data)));
                            }
                            Err(e) => {
                                result.set(Some(format!("Error: {}", e)));
                            }
                        }
                        
                        is_loading.set(false);
                    });
                }
            }),
            
        is_loading.get().map(|loading| {
            if loading {
                Some(progress_indicator())
            } else {
                None
            }
        }),
        
        result.get().map(|result| {
            result.map(|msg| text(msg))
        }),
    ))
    .spacing(10.0)
}

async fn fetch_remote_data() -> Result<String, Box<dyn std::error::Error>> {
    // Simulate API call
    Ok("Remote data loaded successfully".to_string())
}
```

## Testing Event Handlers

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_click_counter() {
        let counter = binding(0);
        
        // Simulate button click
        let action = {
            let counter = counter.clone();
            move |_| counter.update(|c| c + 1)
        };
        
        // Test initial state
        assert_eq!(counter.get(), 0);
        
        // Simulate clicks
        action(());
        assert_eq!(counter.get(), 1);
        
        action(());
        assert_eq!(counter.get(), 2);
    }
    
    #[test]
    fn test_todo_actions() {
        let todos = binding(vec![
            Todo { id: 1, text: "Test".to_string(), completed: false },
        ));
        
        // Test toggle action
        handle_todo_action(todos.clone(), TodoAction::Toggle(1));
        assert!(todos.get()[0].completed);
        
        // Test delete action
        handle_todo_action(todos.clone(), TodoAction::Delete(1));
        assert!(todos.get().is_empty());
    }
}
```

## Summary

Event handling in WaterUI provides:
- Type-safe event callbacks with compile-time checking
- Clean closure syntax for event handlers
- Reactive updates through binding mutations
- Support for keyboard, mouse, and custom events
- Async event handling with task spawning
- Event delegation patterns for complex UIs

Key patterns:
- Use closures for simple event handlers
- Extract complex logic to separate functions
- Handle async operations with task spawning
- Test event logic independently of UI
- Use reactive bindings to update UI automatically

Next: [Layout Components](08-layout.md)
