# Testing

Testing is crucial for maintaining reliable WaterUI applications. This chapter covers various testing strategies, from unit testing individual components to integration testing of complete user interfaces.

## Testing Philosophy

WaterUI's reactive architecture and functional design make testing straightforward:

- **Pure Functions**: Most components are pure functions that return views
- **Deterministic State**: Reactive state changes are predictable and testable
- **Isolated Components**: Components can be tested in isolation
- **Snapshot Testing**: UI structures can be verified through snapshot comparisons

## Unit Testing Components

### Testing Pure View Functions

```rust,ignore
use waterui::*;
use nami::*;

// Component to test
fn counter_display(count: Computed<i32>) -> impl View {
    vstack((
        text!("Count: {}", count),
        text!("Status: {}", s!(if count > 0 { "Positive" } else { "Zero or Negative" })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_display_positive() {
        let count = binding(5);
        let computed_count = s!(count);
        let view = counter_display(computed_count);
        
        // Test view structure
        // In a real implementation, you'd have testing utilities
        // to inspect the view hierarchy
    }

    #[test]
    fn test_counter_display_zero() {
        let count = binding(0);
        let computed_count = s!(count);
        let view = counter_display(computed_count);
        
        // Verify zero case behavior
    }
}
```

### Testing Stateful Components

```rust,ignore
use waterui::*;
use nami::*;

#[derive(Clone)]
struct TodoList {
    items: Binding<Vec<TodoItem>>,
    filter: Binding<TodoFilter>,
}

#[derive(Clone, Debug, PartialEq)]
struct TodoItem {
    id: u32,
    text: String,
    completed: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum TodoFilter {
    All,
    Active,
    Completed,
}

impl View for TodoList {
    fn body(self) -> impl View {
        let filtered_items = s!({
            let items = &self.items;
            let filter = &self.filter;
            match filter {
                TodoFilter::All => items.clone(),
                TodoFilter::Active => items.iter().filter(|item| !item.completed).cloned().collect(),
                TodoFilter::Completed => items.iter().filter(|item| item.completed).cloned().collect(),
            }
        });

        vstack((
            todo_input(self.items.clone()),
            todo_filters(self.filter.clone()),
            todo_items_list(filtered_items),
        ))
    }
}

#[cfg(test)]
mod todo_tests {
    use super::*;

    #[test]
    fn test_todo_list_filtering() {
        let items = binding(vec![
            TodoItem { id: 1, text: "Buy milk".to_string(), completed: false },
            TodoItem { id: 2, text: "Walk dog".to_string(), completed: true },
            TodoItem { id: 3, text: "Write code".to_string(), completed: false },
        ));
        let filter = binding(TodoFilter::Active);
        
        let todo_list = TodoList {
            items: items.clone(),
            filter: filter.clone(),
        };
        
        // Test that filtering works correctly
        filter.set(TodoFilter::Active);
        // Verify only active items are shown
        
        filter.set(TodoFilter::Completed);
        // Verify only completed items are shown
    }

    #[test]
    fn test_adding_todo_item() {
        let items = binding(Vec::new());
        
        // Simulate adding an item
        items.update(|items| {
            items.push(TodoItem {
                id: 1,
                text: "New task".to_string(),
                completed: false,
            });
        });
        
        assert_eq!(items.get().len(), 1);
        assert_eq!(items.get()[0].text, "New task");
    }
}
```

## Integration Testing

### Testing User Interactions

```rust,ignore
use waterui::*;
use nami::*;

fn login_form() -> impl View {
    let username = binding(String::new());
    let password = binding(String::new());
    let is_loading = binding(false);
    let error_message = binding(None::<String>);
    
    let submit_action = {
        let username = username.clone();
        let password = password.clone();
        let is_loading = is_loading.clone();
        let error_message = error_message.clone();
        
        move || {
            if username.get().is_empty() || password.get().is_empty() {
                error_message.set(Some("Please fill in all fields".to_string()));
                return;
            }
            
            is_loading.set(true);
            error_message.set(None);
            
            // Simulate async login
            // In real app, this would be an actual API call
        }
    };
    
    vstack((
        text_field("Username", username.clone()),
        secure_field("Password", password.clone()),
        s!(if let Some(error) = error_message.as_ref() {
            Some(text!(error).color(Color::red()))
        } else {
            None
        }),
        button("Login", submit_action)
            .disabled(s!(is_loading))
            .background(s!(if is_loading { Color::gray() } else { Color::blue() })),
    ))
    .padding(20)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[test]
    fn test_login_form_validation() {
        // Test empty fields validation
        let username = binding(String::new());
        let password = binding(String::new());
        
        // Simulate button click with empty fields
        // Verify error message is shown
    }
    
    #[test]
    fn test_login_form_success_flow() {
        // Test successful login flow
        let username = binding("testuser".to_string());
        let password = binding("password123".to_string());
        
        // Simulate successful login
        // Verify loading state and success handling
    }
}
```

## Reactive State Testing

### Testing Signal Dependencies

```rust,ignore
use nami::*;

#[test]
fn test_computed_signal_updates() {
    let width = binding(10.0);
    let height = binding(5.0);
    
    let area = s!(width * height);
    let perimeter = s!(2.0 * (width + height));
    
    assert_eq!(area.get(), 50.0);
    assert_eq!(perimeter.get(), 30.0);
    
    // Update width and verify computations update
    width.set(20.0);
    assert_eq!(area.get(), 100.0);
    assert_eq!(perimeter.get(), 50.0);
    
    // Update height and verify computations update
    height.set(10.0);
    assert_eq!(area.get(), 200.0);
    assert_eq!(perimeter.get(), 60.0);
}

#[test]
fn test_conditional_reactive_logic() {
    let score = binding(85);
    let grade = s!(match score {
        90..=100 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    });
    
    assert_eq!(grade.get(), "B");
    
    score.set(95);
    assert_eq!(grade.get(), "A");
    
    score.set(55);
    assert_eq!(grade.get(), "F");
}
```

### Testing Effect Systems

```rust,ignore
use nami::*;
use std::sync::{Arc, Mutex};

#[test]
fn test_effects_and_cleanup() {
    let counter = binding(0);
    let log = Arc::new(Mutex::new(Vec::new()));
    
    // Create an effect that logs counter changes
    let log_clone = log.clone();
    let _effect = effect({
        let counter = counter.clone();
        move || {
            let value = counter.get();
            log_clone.lock().unwrap().push(format!("Counter: {}", value));
        }
    });
    
    // Initial effect should run
    assert_eq!(log.lock().unwrap().len(), 1);
    assert_eq!(log.lock().unwrap()[0], "Counter: 0");
    
    // Update counter and verify effect runs
    counter.set(1);
    assert_eq!(log.lock().unwrap().len(), 2);
    assert_eq!(log.lock().unwrap()[1], "Counter: 1");
    
    counter.set(2);
    assert_eq!(log.lock().unwrap().len(), 3);
    assert_eq!(log.lock().unwrap()[2], "Counter: 2");
}
```

## Snapshot Testing

### View Structure Testing

```rust,ignore
use waterui::*;
use nami::*;
use serde_json;

// Helper function to serialize view structure for testing
fn view_to_json<V: View>(view: V) -> String {
    // In a real implementation, this would traverse the view hierarchy
    // and create a JSON representation for snapshot testing
    todo!("Implement view serialization for testing")
}

#[test]
fn test_profile_card_snapshot() {
    let user = User {
        name: "Alice Johnson".to_string(),
        email: "alice@example.com".to_string(),
        avatar_url: Some("https://example.com/avatar.jpg".to_string()),
        role: UserRole::Admin,
    };
    
    let view = profile_card(user);
    let snapshot = view_to_json(view);
    
    // Compare against expected snapshot
    let expected_snapshot = include_str!("../snapshots/profile_card.json");
    assert_eq!(snapshot, expected_snapshot);
}
```

## Performance Testing

### Memory Usage Testing

```rust,ignore
use waterui::*;
use nami::*;
use std::mem;

#[test]
fn test_memory_efficiency() {
    let large_list = (0..10000).map(|i| format!("Item {}", i)).collect::<Vec<_>>();
    let list_binding = binding(large_list);
    
    let initial_memory = get_memory_usage(); // Hypothetical function
    
    // Create a large list view
    let view = vstack(s!(list_binding.iter().take(100).map(|item| {
        text!(item)
    }).collect::<Vec<_>>()));
    
    let final_memory = get_memory_usage();
    let memory_increase = final_memory - initial_memory;
    
    // Verify memory usage is within acceptable limits
    assert!(memory_increase < 1024 * 1024); // Less than 1MB increase
}

#[test]
fn test_signal_cleanup() {
    let counter = binding(0);
    let weak_refs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    
    // Create many computed signals
    for i in 0..1000 {
        let computed = s!(counter + i);
        let weak = std::sync::Arc::downgrade(&std::sync::Arc::new(computed));
        weak_refs.lock().unwrap().push(weak);
    }
    
    // Force garbage collection (platform-dependent)
    // std::gc::collect(); // Hypothetical
    
    // Verify weak references are cleaned up
    let alive_count = weak_refs.lock().unwrap()
        .iter()
        .filter(|weak| weak.upgrade().is_some())
        .count();
    
    assert!(alive_count < 100); // Most should be cleaned up
}
```

## Testing Utilities

### Custom Testing Macros

```rust,ignore
// Test macro for view assertions
macro_rules! assert_view_contains {
    ($view:expr, $expected:expr) => {
        let view_structure = debug_view_structure($view);
        assert!(view_structure.contains($expected), 
            "View does not contain expected element: {}", $expected);
    };
}

// Test macro for reactive updates
macro_rules! assert_reactive_update {
    ($signal:expr, $new_value:expr, $computed:expr, $expected:expr) => {
        $signal.set($new_value);
        assert_eq!($computed.get(), $expected, 
            "Reactive computation did not update correctly");
    };
}

#[test]
fn test_using_custom_macros() {
    let name = binding("World".to_string());
    let view = text!("Hello, {}!", name);
    
    assert_view_contains!(view, "Hello, World!");
    
    let count = binding(5);
    let doubled = s!(count * 2);
    
    assert_reactive_update!(count, 10, doubled, 20);
}
```

### Mock Services

```rust,ignore
use async_trait::async_trait;

#[async_trait]
trait ApiService {
    async fn fetch_user(&self, id: u32) -> Result<User, ApiError>;
    async fn update_user(&self, user: User) -> Result<(), ApiError>;
}

struct MockApiService {
    users: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u32, User>>>,
}

#[async_trait]
impl ApiService for MockApiService {
    async fn fetch_user(&self, id: u32) -> Result<User, ApiError> {
        let users = self.users.lock().unwrap();
        users.get(&id)
            .cloned()
            .ok_or(ApiError::NotFound)
    }
    
    async fn update_user(&self, user: User) -> Result<(), ApiError> {
        let mut users = self.users.lock().unwrap();
        users.insert(user.id, user);
        Ok(())
    }
}

#[tokio::test]
async fn test_user_profile_with_mock_service() {
    let mock_service = MockApiService {
        users: std::sync::Arc::new(std::sync::Mutex::new([
            (1, User { id: 1, name: "Test User".to_string(), /* ... */ })
        ].into())),
    };
    
    let user = mock_service.fetch_user(1).await.unwrap();
    assert_eq!(user.name, "Test User");
    
    let updated_user = User { name: "Updated User".to_string(), ..user };
    mock_service.update_user(updated_user).await.unwrap();
    
    let fetched_user = mock_service.fetch_user(1).await.unwrap();
    assert_eq!(fetched_user.name, "Updated User");
}
```

## Testing Best Practices

### 1. Test Structure

- **Arrange**: Set up test data and initial state
- **Act**: Perform the action being tested
- **Assert**: Verify the expected outcome

### 2. Test Naming

```rust,ignore
#[test]
fn should_update_counter_when_increment_clicked() {
    // Test implementation
}

#[test]
fn should_show_error_when_username_is_empty() {
    // Test implementation
}

#[test]
fn should_filter_todos_when_filter_changed() {
    // Test implementation
}
```

### 3. Test Independence

Each test should be independent and not rely on the state from other tests:

```rust,ignore
#[test]
fn test_independent_state() {
    // Create fresh state for each test
    let counter = binding(0);
    let view = counter_component(counter.clone());
    
    // Test specific scenario
}
```

### 4. Testing Edge Cases

```rust,ignore
#[test]
fn test_empty_list() {
    let items = binding(Vec::<String>::new());
    let view = item_list(items);
    // Verify empty state handling
}

#[test]
fn test_very_long_text() {
    let long_text = "a".repeat(10000);
    let view = text!(long_text);
    // Verify text truncation or wrapping
}

#[test]
fn test_special_characters() {
    let special_text = "Hello 🌍 <>&\"'";
    let view = text!(special_text);
    // Verify proper escaping
}
```

## Continuous Testing

### Cargo Integration

```toml
# In Cargo.toml
[dev-dependencies]
tokio-test = "0.4"
proptest = "1.0"

# Test configuration
[[test]]
name = "integration_tests"
path = "tests/integration.rs"
```

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_counter_display

# Run tests with output
cargo test -- --nocapture

# Run tests in release mode for performance testing
cargo test --release
```

Testing ensures your WaterUI applications are reliable, maintainable, and perform well across different scenarios. The reactive nature of WaterUI makes testing straightforward once you understand the patterns for testing signals, computed values, and view hierarchies.
