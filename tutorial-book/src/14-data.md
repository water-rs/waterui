# Data Management

Effective data management is crucial for building robust applications. WaterUI provides powerful reactive state management through bindings, computed values, and data flow patterns.

## Reactive State Management

### Basic Data Binding

```rust,ignore
use waterui::*;
use nami::*;

fn basic_data_demo() -> impl View {
    let counter = binding(0);
    let message = binding("Hello, World!".to_string());
    let items = binding(vec!["Apple", "Banana", "Cherry"));
    
    vstack((
        text!("Counter: {}", counter),
        text!("Message: {}", message),
        text!("Items: {}", s!(items.join(", "))),
        
        hstack((
            button("Increment")
                .action({
                    let counter = counter.clone();
                    move |_| counter.update(|c| c + 1)
                }),
            button("Add Item")
                .action({
                    let items = items.clone();
                    move |_| items.update(|mut items| {
                        items.push("New Item");
                        items
                    })
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
}
```

### Computed Values and Derived State

```rust,ignore
fn computed_values_demo() -> impl View {
    let first_name = binding("John".to_string());
    let last_name = binding("Doe".to_string());
    let birth_year = binding(1990);
    
    // Computed values using s! macro
    let full_name = s!(format!("{} {}", first_name, last_name));
    let age = s!(2024 - birth_year);
    let is_adult = s!(age >= 18);
    let name_length = s!(full_name.len());
    
    vstack((
        text("Personal Information").font_size(20.0),
        
        text_field(first_name.clone())
            .label("First Name"),
        text_field(last_name.clone())
            .label("Last Name"),
        number_field(birth_year.clone())
            .label("Birth Year"),
            
        // Display computed values
        text!("Full Name: {}", full_name),
        text!("Age: {}", age),
        text!("Adult Status: {}", s!(if is_adult { "Adult" } else { "Minor" })),
        text!("Name Length: {} characters", name_length),
        
        // Conditional rendering based on computed values
        s!(if name_length > 20 {
            Some(text("That's a long name!").color(Color::orange()))
        } else {
            None
        }),
    ))
    .spacing(15.0)
    .padding(20.0)
}
```

### Complex Data Structures

```rust,ignore
#[derive(Clone, Debug, Default)]
struct User {
    id: u32,
    name: String,
    email: String,
    age: u32,
    active: bool,
}

#[derive(Clone, Debug, Default)]
struct UserList {
    users: Vec<User>,
    selected_user: Option<usize>,
    filter: String,
}

fn user_management_demo() -> impl View {
    let user_list = binding(UserList::default());
    
    // Computed filtered users
    let filtered_users = s!({
        let filter = user_list.filter.to_lowercase();
        user_list.users.iter()
            .enumerate()
            .filter(|(_, user)| {
                filter.is_empty() || 
                user.name.to_lowercase().contains(&filter) ||
                user.email.to_lowercase().contains(&filter)
            })
            .collect::<Vec<_>>()
    });
    
    vstack((
        text("User Management").font_size(24.0),
        
        // Filter input
        text_field(s!(user_list.filter.clone()))
            .placeholder("Filter users...")
            .on_change({
                let user_list = user_list.clone();
                move |filter| {
                    user_list.update(|mut list| {
                        list.filter = filter;
                        list
                    });
                }
            }),
            
        // Add user section
        add_user_section(user_list.clone()),
        
        // User list
        scroll(
            vstack(
                filtered_users.map(|users| {
                    users.into_iter().map(|(index, user)| {
                        user_item(user.clone(), index, user_list.clone())
                    })
                })
            )
            .spacing(5.0)
        )
        .max_height(400.0),
        
        // Statistics
        user_statistics(user_list.clone()),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn add_user_section(user_list: Binding<UserList>) -> impl View {
    let new_user = binding(User::default());
    
    vstack((
        text("Add New User").font_size(18.0),
        
        hstack((
            text_field(s!(new_user.name.clone()))
                .placeholder("Name")
                .on_change({
                    let new_user = new_user.clone();
                    move |name| {
                        new_user.update(|mut user| {
                            user.name = name;
                            user
                        });
                    }
                }),
                
            text_field(s!(new_user.email.clone()))
                .placeholder("Email")
                .on_change({
                    let new_user = new_user.clone();
                    move |email| {
                        new_user.update(|mut user| {
                            user.email = email;
                            user
                        });
                    }
                }),
        ))
        .spacing(10.0),
        
        hstack((
            number_field(s!(new_user.age))
                .placeholder("Age")
                .on_change({
                    let new_user = new_user.clone();
                    move |age| {
                        new_user.update(|mut user| {
                            user.age = age as u32;
                            user
                        });
                    }
                }),
                
            button("Add User")
                .action({
                    let user_list = user_list.clone();
                    let new_user = new_user.clone();
                    move |_| {
                        let user = new_user.get();
                        if !user.name.is_empty() && !user.email.is_empty() {
                            user_list.update(|mut list| {
                                list.users.push(User {
                                    id: (list.users.len() + 1) as u32,
                                    ..user
                                });
                                list
                            });
                            new_user.set(User::default());
                        }
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
}

fn user_item(user: User, index: usize, user_list: Binding<UserList>) -> impl View {
    let is_selected = s!(user_list.selected_user == Some(index));
    
    hstack((
        vstack((
            text!("{}", user.name)
                .font_weight(FontWeight::Medium),
            text!("{}", user.email)
                .font_size(14.0)
                .color(Color::secondary()),
        ))
        .flex(1)
        .spacing(5.0),
        
        text!("Age: {}", user.age)
            .font_size(14.0),
            
        toggle(binding(user.active))
            .on_change({
                let user_list = user_list.clone();
                move |active| {
                    user_list.update(|mut list| {
                        if let Some(user) = list.users.get_mut(index) {
                            user.active = active;
                        }
                        list
                    });
                }
            }),
            
        button("Delete")
            .style(ButtonStyle::Destructive)
            .action({
                let user_list = user_list.clone();
                move |_| {
                    user_list.update(|mut list| {
                        list.users.remove(index);
                        list.selected_user = None;
                        list
                    });
                }
            }),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(s!(if is_selected { 
        Color::primary().opacity(0.1) 
    } else { 
        Color::surface() 
    }))
    .corner_radius(8.0)
    .on_tap({
        let user_list = user_list.clone();
        move |_| {
            user_list.update(|mut list| {
                list.selected_user = Some(index);
                list
            });
        }
    })
}

fn user_statistics(user_list: Binding<UserList>) -> impl View {
    let total_users = s!(user_list.users.len());
    let active_users = s!(user_list.users.iter().filter(|u| u.active).count());
    let average_age = s!({
        if user_list.users.is_empty() {
            0.0
        } else {
            user_list.users.iter().map(|u| u.age as f64).sum::<f64>() / user_list.users.len() as f64
        }
    });
    
    hstack((
        stat_card("Total Users", s!(total_users.to_string())),
        stat_card("Active Users", s!(active_users.to_string())),
        stat_card("Average Age", s!(format!("{:.1}", average_age))),
    ))
    .spacing(15.0)
}

fn stat_card(title: &str, value: Signal<String>) -> impl View {
    vstack((
        text!(title)
            .font_size(12.0)
            .color(Color::secondary()),
        text!("{}", value)
            .font_size(20.0)
            .font_weight(FontWeight::Bold),
    ))
    .spacing(5.0)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(8.0)
    .alignment(.center)
}
```

## Data Flow Patterns

### Event-Driven Architecture

```rust,ignore
#[derive(Clone, Debug)]
enum AppEvent {
    UserAction(UserAction),
    DataLoaded(Vec<String>),
    Error(String),
    NetworkStateChanged(bool),
}

#[derive(Clone, Debug)]
enum UserAction {
    ButtonClicked(String),
    TextChanged(String),
    ItemSelected(usize),
}

fn event_driven_demo() -> impl View {
    let events = binding(Vec::<AppEvent>::new());
    let app_state = binding(AppState::default());
    
    vstack((
        text("Event-Driven Architecture").font_size(20.0),
        
        // Event generators
        event_generators(events.clone()),
        
        // Event processor
        event_processor(events.clone(), app_state.clone()),
        
        // State display
        state_display(app_state.clone()),
        
        // Event log
        event_log(events.clone()),
    ))
    .spacing(20.0)
    .padding(20.0)
}

#[derive(Clone, Debug, Default)]
struct AppState {
    counter: i32,
    message: String,
    is_online: bool,
    last_error: Option<String>,
}

fn event_generators(events: Binding<Vec<AppEvent>>) -> impl View {
    vstack((
        text("Event Generators").font_size(16.0),
        
        hstack((
            button("Click Me")
                .action({
                    let events = events.clone();
                    move |_| {
                        dispatch_event(events.clone(), AppEvent::UserAction(
                            UserAction::ButtonClicked("Click Me".to_string())
                        ));
                    }
                }),
                
            button("Load Data")
                .action({
                    let events = events.clone();
                    move |_| {
                        // Simulate data loading
                        let mock_data = vec!["Item 1".to_string(), "Item 2".to_string()];
                        dispatch_event(events.clone(), AppEvent::DataLoaded(mock_data));
                    }
                }),
                
            button("Simulate Error")
                .action({
                    let events = events.clone();
                    move |_| {
                        dispatch_event(events.clone(), AppEvent::Error("Test error".to_string()));
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
}

fn dispatch_event(events: Binding<Vec<AppEvent>>, event: AppEvent) {
    events.update(|mut events| {
        events.push(event);
        // Keep only last 10 events
        if events.len() > 10 {
            events.remove(0);
        }
        events
    });
}

fn event_processor(events: Binding<Vec<AppEvent>>, app_state: Binding<AppState>) -> impl View {
    // Process events reactively
    let _processor = s!({
        if let Some(last_event) = events.last() {
            match last_event {
                AppEvent::UserAction(UserAction::ButtonClicked(_)) => {
                    app_state.update(|mut state| {
                        state.counter += 1;
                        state
                    });
                }
                AppEvent::DataLoaded(data) => {
                    app_state.update(|mut state| {
                        state.message = format!("Loaded {} items", data.len());
                        state
                    });
                }
                AppEvent::Error(error) => {
                    app_state.update(|mut state| {
                        state.last_error = Some(error.clone());
                        state
                    });
                }
                AppEvent::NetworkStateChanged(is_online) => {
                    app_state.update(|mut state| {
                        state.is_online = *is_online;
                        state
                    });
                }
                _ => {}
            }
        }
    });
    
    text("Event processor running...")
        .font_size(12.0)
        .color(Color::secondary())
}

fn state_display(app_state: Binding<AppState>) -> impl View {
    vstack((
        text("Current State").font_size(16.0),
        
        text!("Counter: {}", s!(app_state.counter)),
        text!("Message: {}", s!(app_state.message.clone())),
        text!("Online: {}", s!(app_state.is_online)),
        
        s!(if let Some(error) = &app_state.last_error {
            Some(
                text!("Last Error: {}", error)
                    .color(Color::red())
            )
        } else {
            None
        }),
    ))
    .spacing(5.0)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(8.0)
}

fn event_log(events: Binding<Vec<AppEvent>>) -> impl View {
    vstack((
        text("Event Log").font_size(16.0),
        
        scroll(
            vstack(
                events.get().map(|events| {
                    events.into_iter().enumerate().map(|(index, event)| {
                        text!("{}: {:?}", index, event)
                            .font_size(12.0)
                            .font_family("monospace")
                    })
                })
            )
            .spacing(2.0)
        )
        .max_height(200.0),
    ))
    .spacing(10.0)
}
```

### State Management with Stores

```rust,ignore
trait Store<T> {
    fn get(&self) -> T;
    fn set(&self, value: T);
    fn subscribe<F>(&self, callback: F) where F: Fn(&T) + 'static;
}

struct AppStore {
    user: Binding<Option<User>>,
    settings: Binding<AppSettings>,
    ui_state: Binding<UiState>,
}

#[derive(Clone, Debug, Default)]
struct AppSettings {
    theme: String,
    language: String,
    notifications_enabled: bool,
}

#[derive(Clone, Debug, Default)]
struct UiState {
    current_page: String,
    sidebar_open: bool,
    loading: bool,
}

impl AppStore {
    fn new() -> Self {
        Self {
            user: binding(None),
            settings: binding(AppSettings::default()),
            ui_state: binding(UiState::default()),
        }
    }
    
    fn login(&self, user: User) {
        self.user.set(Some(user));
    }
    
    fn logout(&self) {
        self.user.set(None);
        self.ui_state.update(|mut state| {
            state.current_page = "login".to_string();
            state
        });
    }
    
    fn navigate_to(&self, page: String) {
        self.ui_state.update(|mut state| {
            state.current_page = page;
            state
        });
    }
    
    fn toggle_sidebar(&self) {
        self.ui_state.update(|mut state| {
            state.sidebar_open = !state.sidebar_open;
            state
        });
    }
}

fn store_demo() -> impl View {
    let store = AppStore::new();
    
    vstack((
        text("Store Pattern Demo").font_size(20.0),
        
        // User section
        user_section(&store),
        
        // Settings section
        settings_section(&store),
        
        // UI state section
        ui_state_section(&store),
    ))
    .spacing(20.0)
    .padding(20.0)
}

fn user_section(store: &AppStore) -> impl View {
    let is_logged_in = s!(store.user.is_some());
    let user_name = s!(store.user.as_ref().map(|u| u.name.clone()).unwrap_or_default());
    
    vstack((
        text("User Management").font_size(16.0),
        
        s!(if is_logged_in {
            Some(
                vstack((
                    text!("Welcome, {}!", user_name),
                    button("Logout")
                        .action({
                            let store = store.clone();
                            move |_| store.logout()
                        }),
                ))
                .spacing(10.0)
            )
        } else {
            Some(
                button("Login as Demo User")
                    .action({
                        let store = store.clone();
                        move |_| {
                            store.login(User {
                                id: 1,
                                name: "Demo User".to_string(),
                                email: "demo@example.com".to_string(),
                                age: 25,
                                active: true,
                            });
                        }
                    })
            )
        }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(8.0)
}

fn settings_section(store: &AppStore) -> impl View {
    vstack((
        text("Settings").font_size(16.0),
        
        toggle(s!(store.settings.notifications_enabled))
            .label("Enable Notifications")
            .on_change({
                let store = store.clone();
                move |enabled| {
                    store.settings.update(|mut settings| {
                        settings.notifications_enabled = enabled;
                        settings
                    });
                }
            }),
            
        picker(s!(store.settings.theme.clone()), vec![
            ("light".to_string(), "Light Theme"),
            ("dark".to_string(), "Dark Theme"),
            ("auto".to_string(), "Auto"),
        ))
        .label("Theme")
        .on_change({
            let store = store.clone();
            move |theme| {
                store.settings.update(|mut settings| {
                    settings.theme = theme;
                    settings
                });
            }
        }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(8.0)
}

fn ui_state_section(store: &AppStore) -> impl View {
    vstack((
        text("UI State").font_size(16.0),
        
        text!("Current Page: {}", s!(store.ui_state.current_page.clone())),
        text!("Sidebar Open: {}", s!(store.ui_state.sidebar_open)),
        
        hstack((
            button("Home")
                .action({
                    let store = store.clone();
                    move |_| store.navigate_to("home".to_string())
                }),
            button("Profile")
                .action({
                    let store = store.clone();
                    move |_| store.navigate_to("profile".to_string())
                }),
            button("Settings")
                .action({
                    let store = store.clone();
                    move |_| store.navigate_to("settings".to_string())
                }),
        ))
        .spacing(10.0),
        
        button("Toggle Sidebar")
            .action({
                let store = store.clone();
                move |_| store.toggle_sidebar()
            }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::light_gray())
    .corner_radius(8.0)
}
```

## Performance Optimization

### Efficient Data Updates

```rust,ignore
fn performance_demo() -> impl View {
    let large_dataset = binding((0..10000).map(|i| DataItem {
        id: i,
        name: format!("Item {}", i),
        value: i * 2,
        active: i % 3 == 0,
    }).collect::<Vec<_>>());
    
    let filter = binding(String::new());
    let sort_by = binding(SortBy::Name);
    
    // Efficiently computed filtered and sorted data
    let processed_data = s!({
        let mut items = large_dataset.clone();
        
        // Filter
        if !filter.is_empty() {
            items.retain(|item| 
                item.name.to_lowercase().contains(&filter.to_lowercase())
            );
        }
        
        // Sort
        match sort_by {
            SortBy::Name => items.sort_by(|a, b| a.name.cmp(&b.name)),
            SortBy::Value => items.sort_by(|a, b| a.value.cmp(&b.value)),
            SortBy::Id => items.sort_by(|a, b| a.id.cmp(&b.id)),
        }
        
        // Take only first 100 for display
        items.truncate(100);
        items
    });
    
    vstack((
        text("Performance Optimization Demo").font_size(20.0),
        
        hstack((
            text_field(filter.clone())
                .placeholder("Filter items...")
                .on_change({
                    let filter = filter.clone();
                    move |value| filter.set(value)
                }),
                
            picker(sort_by.clone(), vec![
                (SortBy::Name, "Name"),
                (SortBy::Value, "Value"),
                (SortBy::Id, "ID"),
            ))
            .on_change({
                let sort_by = sort_by.clone();
                move |value| sort_by.set(value)
            }),
        ))
        .spacing(10.0),
        
        text!("Showing {} of {} items", 
              s!(processed_data.len()),
              s!(large_dataset.len())),
              
        // Virtualized list for performance
        virtual_scroll(
            processed_data.get().map(|items| {
                items.into_iter().map(|item| {
                    optimized_item_view(item)
                })
            })
        )
        .item_height(60.0)
        .max_height(400.0),
    ))
    .spacing(15.0)
    .padding(20.0)
}

#[derive(Clone, Debug)]
struct DataItem {
    id: usize,
    name: String,
    value: i32,
    active: bool,
}

#[derive(Clone, PartialEq)]
enum SortBy {
    Name,
    Value,
    Id,
}

fn optimized_item_view(item: DataItem) -> impl View {
    // Memoized view for performance
    memo(item.id, move || {
        hstack((
            text!("{}", item.name)
                .flex(1)
                .font_weight(FontWeight::Medium),
            text!("Value: {}", item.value)
                .font_size(14.0),
            circle()
                .width(12.0)
                .height(12.0)
                .color(if item.active { Color::green() } else { Color::gray() }),
        ))
        .spacing(15.0)
        .padding(15.0)
        .background(Color::surface())
        .corner_radius(8.0)
    })
}
```

## Summary

WaterUI's data management provides:

- **Reactive Bindings**: Automatic UI updates with `binding()` and `s!()` macro
- **Computed Values**: Derived state that updates automatically
- **Complex Data Structures**: Support for nested data and collections
- **Event-Driven Architecture**: Clean separation of concerns with events
- **Store Patterns**: Centralized state management
- **Performance Optimization**: Efficient updates and virtualization

Key best practices:
- Use `s!()` macro for computed values
- Structure data with clear ownership
- Implement event-driven patterns for complex flows
- Optimize large datasets with filtering and virtualization
- Use stores for global state management
- Test data flow and state transitions

Next: [Networking](15-networking.md)