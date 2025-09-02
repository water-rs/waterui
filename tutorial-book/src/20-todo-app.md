# Building a Todo Application

Let's build a complete todo application to demonstrate WaterUI concepts in a real project.

## Application Structure

```rust,ignore
use waterui::*;
use nami::s;

#[derive(Clone, Debug)]
struct Todo {
    id: u32,
    text: String,
    completed: bool,
}

fn todo_app() -> impl View {
    let todos = binding(Vec::<Todo>::new());
    let new_todo = binding(String::new());
    let filter = binding(Filter::All);
    
    vstack((
        header(),
        add_todo_section(new_todo.clone(), todos.clone()),
        todo_list(todos.clone(), filter.clone()),
        footer(todos.clone(), filter.clone()),
    ))
    .padding(20.0)
    .max_width(600.0)
}
```

## Add Todo Section

```rust,ignore
fn add_todo_section(
    new_todo: Binding<String>,
    todos: Binding<Vec<Todo>>,
) -> impl View {
    let add_todo = {
        let new_todo = new_todo.clone();
        let todos = todos.clone();
        move || {
            let text = new_todo.get().trim().to_string();
            if !text.is_empty() {
                todos.update(|todos| {
                    let id = todos.len() as u32 + 1;
                    todos.push(Todo {
                        id,
                        text,
                        completed: false,
                    });
                    todos
                });
                new_todo.set(String::new());
            }
        }
    };
    
    hstack((
        text_field(new_todo)
            .placeholder("What needs to be done?")
            .on_submit(add_todo.clone()),
        button("Add")
            .action(move |_| add_todo()),
    ))
    .spacing(10.0)
}
```

## Todo List

```rust,ignore
fn todo_list(
    todos: Binding<Vec<Todo>>,
    filter: Binding<Filter>,
) -> impl View {
    let filtered_todos = s!(filter_todos(&todos, filter));
    
    scroll(
        vstack(
            filtered_todos.map(|todos| {
                todos.into_iter().map(|todo| {
                    todo_item(todo, todos.clone())
                })
            })
        )
    )
    .max_height(400.0)
}

fn todo_item(todo: Todo, todos: Binding<Vec<Todo>>) -> impl View {
    hstack((
        toggle(binding(todo.completed))
            .action({
                let todos = todos.clone();
                let todo_id = todo.id;
                move |completed| {
                    todos.update(|todos| {
                        if let Some(t) = todos.iter_mut().find(|t| t.id == todo_id) {
                            t.completed = completed;
                        }
                        todos
                    });
                }
            }),
            
        text(&todo.text)
            .strikethrough(todo.completed)
            .color(if todo.completed { 
                Color::secondary() 
            } else { 
                Color::primary() 
            })
            .flex(1),
            
        button("×")
            .style(ButtonStyle::Destructive)
            .action({
                let todos = todos.clone();
                let todo_id = todo.id;
                move |_| {
                    todos.update(|todos| {
                        todos.retain(|t| t.id != todo_id);
                        todos
                    });
                }
            }),
    ))
    .padding(8.0)
    .background(Color::item_background())
}
```

## Filtering and Footer

```rust,ignore
#[derive(Clone, PartialEq)]
enum Filter {
    All,
    Active,
    Completed,
}

fn footer(
    todos: Binding<Vec<Todo>>,
    filter: Binding<Filter>,
) -> impl View {
    let active_count = s!(todos.iter().filter(|t| !t.completed).count());
    let completed_count = s!(todos.iter().filter(|t| t.completed).count());
    
    hstack((
        text!(
            "{} item{} left", 
            active_count,
            s!(if active_count == 1 { "" } else { "s" })
        ),
        
        spacer(),
        
        hstack((
            filter_button("All", Filter::All, filter.clone()),
            filter_button("Active", Filter::Active, filter.clone()),
            filter_button("Completed", Filter::Completed, filter.clone()),
        ))
        .spacing(5.0),
        
        spacer(),
        
        button("Clear Completed")
            .disabled(s!(completed_count == 0))
            .action({
                let todos = todos.clone();
                move |_| {
                    todos.update(|todos| {
                        todos.retain(|t| !t.completed);
                        todos
                    });
                }
            }),
    ))
    .padding(10.0)
}

fn filter_button(
    label: &str, 
    filter_type: Filter,
    current_filter: Binding<Filter>,
) -> impl View {
    button(label)
        .style(s!(if current_filter == filter_type {
            ButtonStyle::Primary
        } else {
            ButtonStyle::Secondary
        }))
        .action({
            let current_filter = current_filter.clone();
            move |_| current_filter.set(filter_type.clone())
        })
}

fn filter_todos(todos: &[Todo], filter: &Filter) -> Vec<Todo> {
    todos.iter().filter(|todo| {
        match filter {
            Filter::All => true,
            Filter::Active => !todo.completed,
            Filter::Completed => todo.completed,
        }
    }).cloned().collect()
}
```

## Complete Application

```rust,ignore
fn main() -> Result<(), Box<dyn std::error::Error>> {
    waterui_gtk4::init()?;
    let app = waterui_gtk4::Gtk4App::new("com.example.todo-app");
    app.run(todo_app)
}
```

This todo app demonstrates:
- State management with bindings
- Event handling and user interactions
- List rendering and updates
- Filtering and computed values
- Component composition

Next: [Media Player](21-media-player.md)
