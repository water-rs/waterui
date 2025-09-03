# Conditional Rendering

Conditional rendering is a fundamental technique for creating dynamic user interfaces that respond to changing application state. WaterUI provides powerful and ergonomic components for conditional rendering through the `when` function and related components.

## The `when` Function

The `when` function is the primary tool for conditional rendering in WaterUI. It takes a reactive boolean condition and a closure that returns a view to display when the condition is true.

```rust,ignore
use waterui::{when, text};
use nami::{binding, s};

let is_logged_in = binding(false);

when(is_logged_in, || {
    text!("Welcome back!")
})
```

## Basic Conditional Rendering

Here's a simple example showing how to conditionally display content:

```rust,ignore
use waterui::{when, text, button, vstack};
use nami::binding;

pub fn login_view() -> impl View {
    let is_authenticated = binding(false);
    
    vstack((
        when(is_authenticated, || {
            text!("You are logged in!")
        }),
        
        // Binding implements Not trait - no s!() needed for negation
        when(!is_authenticated, || {
            button("Login", {
                let auth = is_authenticated.clone();
                move || auth.set(true)
            })
        })
    ))
}
```

## Reactive Negation with `!`

WaterUI's `Binding` type implements the `Not` trait, which means you can use `!` directly on bindings without wrapping them in `s!()`. This maintains full reactivity:

```rust,ignore
use waterui::{when, text};
use nami::binding;

let is_visible = binding(true);

// Both of these are reactive and equivalent:
when(!is_visible, || text!("Hidden"));  // ✅ Preferred - cleaner syntax
when(s!(!is_visible), || text!("Hidden"));  // ✅ Works but unnecessary

// This applies to any boolean binding
let is_loading = binding(false);
let has_errors = binding(false);

when(!is_loading, || text!("Content loaded"))
when(!has_errors, || text!("No errors"))
```

## Complete Conditional Rendering with `or`

For situations where you need to display one of two views, use the `.or()` method:

```rust,ignore
use waterui::{when, text, button};
use nami::binding;

let has_data = binding(false);

when(has_data, || {
    text!("Data loaded successfully!")
}).or(|| {
    text!("Loading...")
})
```

## Advanced Conditional Patterns

### Loading States

A common pattern is to show different UI based on loading state:

```rust,ignore
use waterui::{when, text, vstack, button};
use nami::binding;

#[derive(Debug, Clone)]
enum LoadingState {
    Idle,
    Loading,
    Success(String),
    Error(String),
}

pub fn data_view() -> impl View {
    let state = binding(LoadingState::Idle);
    
    vstack((
        // Show loading spinner when loading
        when(s!(matches!(state, LoadingState::Loading)), || {
            text!("Loading...")
        }),
        
        // Show success message when data loaded
        when(s!(matches!(state, LoadingState::Success(_))), || {
            if let LoadingState::Success(data) = state.get() {
                text!("Success: {}", data)
            } else {
                text!("Success")
            }
        }),
        
        // Show error message on failure
        when(s!(matches!(state, LoadingState::Error(_))), || {
            if let LoadingState::Error(error) = state.get() {
                text!("Error: {}", error)
            } else {
                text!("Error occurred")
            }
        }),
        
        // Show load button when idle
        when(s!(matches!(state, LoadingState::Idle)), || {
            button("Load Data", {
                let state = state.clone();
                move || {
                    state.set(LoadingState::Loading);
                    // Simulate async operation
                    // In real app, this would be an async call
                }
            })
        })
    ))
}
```

### User Permissions

Conditional rendering is perfect for implementing user permission systems:

```rust,ignore
use waterui::{when, text, button, vstack};
use nami::binding;

#[derive(Debug, Clone, PartialEq)]
enum UserRole {
    Guest,
    User,
    Admin,
}

pub fn admin_panel() -> impl View {
    let user_role = binding(UserRole::Guest);
    
    vstack((
        // Always visible
        text!("Application Dashboard"),
        
        // User-only features
        when(s!(user_role != UserRole::Guest), || {
            vstack((
                text!("User Dashboard"),
                button("View Profile", || {}),
            ))
        }),
        
        // Admin-only features
        when(s!(user_role == UserRole::Admin), || {
            vstack((
                text!("Admin Controls"),
                button("Manage Users", || {}),
                button("System Settings", || {}),
            ))
        }).or(|| {
            when(s!(user_role != UserRole::Guest), || {
                text!("Contact admin for elevated permissions")
            })
        })
    ))
}
```

### Feature Flags

Conditional rendering can be used to implement feature flags:

```rust,ignore
use waterui::{when, text, button, vstack};
use nami::binding;

pub fn feature_flagged_view() -> impl View {
    let new_feature_enabled = binding(true);
    let beta_features = binding(false);
    
    vstack((
        text!("Main Application"),
        
        // New feature behind a flag
        when(new_feature_enabled, || {
            vstack((
                text!("🆕 New Feature Available!"),
                button("Try New Feature", || {}),
            ))
        }),
        
        // Beta features for testing
        when(beta_features, || {
            vstack((
                text!("🧪 Beta Features"),
                text!("Warning: These features are experimental"),
                button("Enable Beta Mode", || {}),
            ))
        })
    ))
}
```

## Reactive Computations in Conditions

Conditions can be complex reactive computations using the `s!` macro:

```rust,ignore
use waterui::{when, text, vstack};
use nami::{binding, s};

pub fn complex_conditions_view() -> impl View {
    let user_score = binding(0);
    let user_level = binding(1);
    let is_premium = binding(false);
    
    vstack((
        // Complex condition with multiple signals
        when(s!(user_score >= 1000 && user_level >= 5), || {
            text!("🏆 Achievement Unlocked: Expert Level!")
        }),
        
        // Nested conditions
        when(is_premium, || {
            when(s!(user_level >= 10), || {
                text!("🌟 Premium Elite Status!")
            }).or(|| {
                text!("⭐ Premium Member")
            })
        }).or(|| {
            when(s!(user_score >= 500), || {
                text!("💎 Consider upgrading to Premium!")
            })
        }),
        
        // Mathematical conditions
        when(s!(user_score % 100 == 0 && user_score > 0), || {
            text!("🎯 Score milestone reached!")
        })
    ))
}
```

## Performance Considerations

WaterUI's conditional rendering is highly optimized:

- **Lazy Evaluation**: Views in conditional branches are only created when the condition is true
- **Automatic Cleanup**: When conditions change, unused views are automatically cleaned up
- **Minimal Re-renders**: Only the affected conditional branches re-render when conditions change

```rust,ignore
use waterui::{when, text, vstack};
use nami::{binding, s};

pub fn optimized_conditional_view() -> impl View {
    let show_expensive_view = binding(false);
    
    vstack((
        text!("Main Content"),
        
        // Expensive view only created when needed
        when(show_expensive_view, || {
            // This complex view is only built when show_expensive_view is true
            expensive_computation_view()
        })
    ))
}

fn expensive_computation_view() -> impl View {
    // This function only runs when the condition is true
    let computed_data = compute_expensive_data();
    text!("Expensive result: {}", computed_data)
}

fn compute_expensive_data() -> String {
    // Expensive computation here
    "Computed result".to_string()
}
```

## Best Practices

### 1. Use `!` for Negation, `s!` for Complex Conditions

```rust,ignore
// Good: Use ! directly for negation
when(!is_authenticated, || login_form())

// Good: Use s! for complex conditions
when(s!(user.is_authenticated() && user.has_permission("admin")), || {
    admin_panel()
})

// Avoid: Wrapping simple negation in s!
when(s!(!is_authenticated), || login_form())  // Unnecessary

// Avoid: Manual signal operations
when(user.map(|u| u.is_authenticated() && u.has_permission("admin")), || {
    admin_panel()
})
```

### 2. Keep Condition Logic Simple

```rust,ignore
// Good: Simple, readable conditions
let can_edit = s!(user.role == Role::Admin || user.id == post.author_id);
when(can_edit, || edit_button())

// Avoid: Complex nested conditions in the when call
when(s!(user.role == Role::Admin || (user.role == Role::Moderator && post.category == Category::News)), || {
    edit_button()
})
```

### 3. Use Descriptive Variable Names

```rust,ignore
// Good: Descriptive condition names
let is_loading = s!(state == LoadingState::Loading);
let has_permission = s!(user.can_access_admin_panel());
let should_show_tutorial = s!(user.is_first_time && !tutorial_completed);

when(is_loading, || spinner())
when(!is_loading, || content())  // Use ! for negation
when(has_permission, || admin_panel())
when(should_show_tutorial, || tutorial_overlay())
```

### 4. Consider Using Enums for Complex State

```rust,ignore
#[derive(Debug, Clone, PartialEq)]
enum ViewState {
    Loading,
    Empty,
    Data(Vec<Item>),
    Error(String),
}

pub fn data_list_view() -> impl View {
    let state = binding(ViewState::Loading);
    
    vstack((
        when(s!(matches!(state, ViewState::Loading)), || {
            loading_spinner()
        }),
        
        when(s!(matches!(state, ViewState::Empty)), || {
            empty_state_view()
        }),
        
        when(s!(matches!(state, ViewState::Data(_))), || {
            data_list(state.clone())
        }),
        
        when(s!(matches!(state, ViewState::Error(_))), || {
            error_view(state.clone())
        })
    ))
}
```

## Common Patterns

### Toggle Visibility

```rust,ignore
let is_visible = binding(true);

when(is_visible, || content_view())
when(!is_visible, || placeholder_view())  // Use ! for negation
```

### Authentication Gates

```rust,ignore
when(user_is_authenticated, || {
    protected_content()
}).or(|| {
    login_form()
})
```

### Progressive Disclosure

```rust,ignore
let show_advanced = binding(false);

vstack((
    basic_options(),
    
    button("Advanced Options", {
        let show = show_advanced.clone();
        move || show.update(|s| !s)
    }),
    
    when(show_advanced, || {
        advanced_options_panel()
    })
))
```

## Summary

Conditional rendering in WaterUI provides:

- **Reactive Updates**: UI automatically updates when conditions change
- **Performance**: Lazy evaluation and automatic cleanup
- **Ergonomic API**: Simple `when().or()` syntax with `!` negation support
- **Type Safety**: Compile-time guarantees for your conditions
- **Composability**: Conditions can be complex reactive computations using `s!()`
- **Clean Negation**: Direct `!` operator support for bindings without wrapping

Master conditional rendering to build dynamic, responsive user interfaces that elegantly handle changing application state.

Next: [Component Composition](06-composition.md)