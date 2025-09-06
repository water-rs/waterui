# Form Controls

WaterUI provides a comprehensive form system that makes creating interactive forms both simple and powerful. The centerpiece of this system is the `FormBuilder` derive macro, which automatically generates form UIs from your data structures.

## Quick Start with FormBuilder

The easiest way to create forms in WaterUI is using the `#[derive(FormBuilder)]` macro:

```rust
use waterui_form::{FormBuilder, form};
use waterui::reactive::Binding;

#[derive(Default, Clone, Debug, FormBuilder)]
pub struct LoginForm {
    /// The user's username
    pub username: String,
    /// The user's password  
    pub password: String,
    /// Whether to remember the user
    pub remember_me: bool,
    /// The user's age
    pub age: i32,
}

fn login_view() -> impl View {
    let form_binding = LoginForm::binding();
    form(&form_binding)
}
```

That's it! WaterUI automatically creates appropriate form controls for each field type:

- `String` → Text field
- `bool` → Toggle switch
- `i32` → Number stepper
- `f32` → Slider
- And many more...

## Type-to-Component Mapping

The `FormBuilder` macro automatically maps Rust types to appropriate form components:

| Rust Type | Form Component | Description |
|-----------|----------------|-------------|
| `String`, `&str` | `TextField` | Single-line text input |
| `bool` | `Toggle` | On/off switch |
| `i32`, `i64`, etc. | `Stepper` | Numeric input with +/- buttons |
| `f32`, `f64` | `Slider` | Slider with 0.0-1.0 range |
| `Color` | `ColorPicker` | Color selection widget |

## Complete Example: User Registration Form

Let's build a more comprehensive form:

```rust
use waterui_form::{FormBuilder, form};
use waterui::reactive::Binding;
use waterui::core::Color;
use waterui::component::layout::stack::vstack;
use waterui_text::{text};

#[derive(Default, Clone, Debug, FormBuilder)]
struct RegistrationForm {
    /// Full name (2-50 characters)
    full_name: String,
    /// Email address
    email: String,
    /// Age (must be 18+)
    age: i32,
    /// Subscribe to newsletter
    newsletter: bool,
    /// Account type
    is_premium: bool,
    /// Profile completion (0.0 to 1.0)
    profile_completion: f32,
    /// Theme color preference
    theme_color: Color,
}

fn registration_view() -> impl View {
    let form_binding = RegistrationForm::binding();
    
    vstack((
        "User Registration",
        form(&form_binding),
        // Real-time validation feedback
        validation_feedback(&form_binding),
    ))
}

fn validation_feedback(form: &Binding<RegistrationForm>) -> impl View {
    text!(
        validate_registration(&form.get())
    )
}

fn validate_registration(data: &RegistrationForm) -> &'static str {
    if data.full_name.len() < 2 {
        "Name too short"
    } else if data.age < 18 {
        "Must be 18 or older"
    } else if !data.email.contains('@') {
        "Invalid email"
    } else {
        "Form is valid ✓"
    }
}
```

## Individual Form Controls

You can also use form controls individually:

### Text Fields

```rust
use waterui_form::{TextField, field};
use waterui::reactive::binding;

fn text_field_example() -> impl View {
    let name = binding("".to_string());
    field("Name:", &name)
}
```

### Toggle Switches

```rust
use waterui_form::{Toggle, toggle};

fn toggle_example() -> impl View {
    let enabled = binding(false);
    toggle("Enable notifications", &enabled)
}
```

### Number Steppers

```rust
use waterui_form::{Stepper, stepper};

fn stepper_example() -> impl View {
    let count = binding(0);
    stepper(&count)
}
```

### Sliders

```rust
use waterui_form::Slider;

fn slider_example() -> impl View {
    let volume = binding(0.5);
    Slider::new(0.0..=1.0, &volume)
}
```

## Advanced Form Patterns

### Multi-Step Forms

```rust
#[derive(Default, Clone, FormBuilder)]
struct PersonalInfo {
    first_name: String,
    last_name: String,
    birth_year: i32,
}

#[derive(Default, Clone, FormBuilder)]
struct ContactInfo {
    email: String,
    phone: String,
    preferred_contact: bool, // true = email, false = phone
}

#[derive(Default, Clone)]
struct RegistrationWizard {
    personal: PersonalInfo,
    contact: ContactInfo,
    current_step: usize,
}

fn registration_wizard() -> impl View {
    let wizard = binding(RegistrationWizard::default());
    
    vstack((
        text!(format!("Step {} of 2", wizard.current_step.get() + 1)),
        
        match wizard.current_step.get() {
            0 => vstack((
                "Personal Information",
                form(&wizard.personal),
            )),
            1 => vstack((
                "Contact Information", 
                form(&wizard.contact),
            )),
            _ => "Registration Complete!",
        },
        
        navigation_buttons(&wizard),
    ))
}
```

### Custom Form Layouts

For complete control over form layout, implement `FormBuilder` manually:

```rust
use waterui_form::{FormBuilder, TextField, Toggle};
use waterui::{
    core::Binding,
    component::layout::stack::{vstack, hstack},
};

struct CustomForm {
    title: String,
    active: bool,
}

impl FormBuilder for CustomForm {
    type View = VStack;

    fn view(binding: &Binding<Self>) -> Self::View {
        vstack((
            hstack((
                "Title:",
                TextField::new(&binding.title),
            )),
            hstack((
                "Active:",
                Toggle::new(&binding.active),
            )),
        ))
    }
}
```

### Secure Fields

For sensitive data like passwords:

```rust
use waterui_form::{SecureField, secure};

fn password_form() -> impl View {
    let password = binding(String::new());
    let confirm_password = binding(String::new());
    
    vstack((
        secure("Password:", &password),
        secure("Confirm Password:", &confirm_password),
        password_validation(&password, &confirm_password),
    ))
}

fn password_validation(pwd: &Binding<String>, confirm: &Binding<String>) -> impl View {
    text!(
        if pwd.get() == confirm.get() && !pwd.get().is_empty() {
            "Passwords match ✓"
        } else {
            "Passwords don't match"
        }
    )
}
```

## Form Validation Best Practices

### Real-time Validation

```rust
#[derive(Default, Clone, FormBuilder)]
struct ValidatedForm {
    email: String,
    password: String,
    age: i32,
}

fn validated_form_view() -> impl View {
    let form = ValidatedForm::binding();
    
    vstack((
        form(&form),
        
        // Email validation
        text!(
            if form.email.get().contains('@') && form.email.get().contains('.') {
                "✓ Valid email"
            } else {
                "✗ Please enter a valid email"
            }
        ),
        
        // Password validation
        text!(
            if form.password.get().len() >= 8 {
                "✓ Password is strong enough"
            } else {
                "✗ Password must be at least 8 characters"
            }
        ),
        
        // Age validation
        text!(
            if form.age.get() >= 18 {
                "✓ Age requirement met"
            } else {
                "✗ Must be 18 or older"
            }
        ),
        
        // Submit button - only enabled when form is valid
        button("Submit")
            .disabled(s!(
                !form.email.get().contains('@') ||
                form.password.get().len() < 8 ||
                form.age.get() < 18
            ))
            .action(|| {
                // Handle form submission
                println!("Form submitted!");
            }),
    ))
}
```

## Integration with State Management

Forms integrate seamlessly with WaterUI's reactive state system:

```rust
use nami::s;

#[derive(Default, Clone, FormBuilder)]
struct UserSettings {
    name: String,
    theme: String,
    notifications: bool,
}

fn settings_panel() -> impl View {
    let settings = UserSettings::binding();
    
    // Computed values based on form state
    let has_changes = s!(
        settings.name.get() != "Default Name" ||
        settings.theme.get() != "Light" ||
        settings.notifications.get()
    );
    
    let settings_summary = s!(
        format!("User: {} | Theme: {} | Notifications: {}",
            settings.name.get(),
            settings.theme.get(), 
            if settings.notifications.get() { "On" } else { "Off" }
        )
    );
    
    vstack((
        "Settings",
        form(&settings),
        
        // Live preview
        "Preview:",
        text!(settings_summary),
        
        // Save button
        button("Save Changes")
            .disabled(s!(!has_changes))
            .action({
                let settings = settings.clone();
                move |_| {
                    save_settings(&settings.get());
                }
            }),
    ))
}

fn save_settings(settings: &UserSettings) {
    println!("Saving settings: {settings:?}");
    // Save to database, file, etc.
}
```
