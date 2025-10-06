# Form Controls

WaterUI provides a comprehensive form system that makes creating interactive forms both simple and powerful. The centerpiece of this system is the `FormBuilder` derive macro, which automatically generates form UIs from your data structures.

## Two-Way Data Binding

WaterUI's forms are built on a powerful concept called **two-way data binding**. This means that the state of your data model and the state of your UI controls are always kept in sync automatically.

Here's how it works:
1.  You provide a `Binding` of your data structure (e.g., `Binding<LoginForm>`) to a form control.
2.  The form control (e.g., a `TextField`) reads the initial value from the binding to display it.
3.  When the user interacts with the control (e.g., types into the text field), the control **automatically updates the value inside your original `Binding`**.

This creates a seamless, reactive loop:
-   **Model → View:** If you programmatically change the data in your `Binding`, the UI control will instantly reflect that change.
-   **View → Model:** If the user changes the value in the UI control, your underlying data `Binding` is immediately updated.

This eliminates a huge amount of boilerplate code. You don't need to write manual event handlers to update your state for every single input field. The binding handles it for you. All form components in WaterUI, whether used individually or through the `FormBuilder`, use this two-way binding mechanism.

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
    let login_form = LoginForm::binding();
    form(&login_form)
}
```

That's it! WaterUI automatically creates appropriate form controls for each field type:

- `String` → Text field
- `bool` → Toggle switch
- `i32` → Number stepper
- `f64` → Slider
- And many more...

## Type-to-Component Mapping

The `FormBuilder` macro automatically maps Rust types to appropriate form components:

| Rust Type | Form Component | Description |
|-----------|----------------|-------------|
| `String`, `&str` | `TextField` | Single-line text input |
| `bool` | `Toggle` | On/off switch |
| `i32`, `i64`, etc. | `Stepper` | Numeric input with +/- buttons |
| `f64` | `Slider` | Slider with 0.0-1.0 range |
| `Color` | `ColorPicker` | Color selection widget |

## Complete Example: User Registration Form

Let's build a more comprehensive form:

```rust
use waterui_form::{FormBuilder, form};
use waterui::reactive::Binding;
use waterui::Color;
use waterui::component::layout::stack::vstack;
use waterui_text::text;

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

    // Create a computed signal for the validation message
    let validation_message = form_binding.map(|data| {
        if data.full_name.len() < 2 {
            "Name too short"
        } else if data.age < 18 {
            "Must be 18 or older"
        } else if !data.email.contains('@') {
            "Invalid email"
        } else {
            "Form is valid ✓"
        }
    });
    
    vstack((
        "User Registration",
        form(&form_binding),
        // Real-time validation feedback
        text(validation_message),
    ))
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
use waterui::reactive::binding;

fn toggle_example() -> impl View {
    let enabled = binding(false);
    toggle("Enable notifications", &enabled)
}
```

### Number Steppers

```rust
use waterui_form::{Stepper, stepper};
use waterui::reactive::binding;

fn stepper_example() -> impl View {
    let count = binding(0);
    stepper(&count)
}
```

### Sliders

```rust
use waterui_form::Slider;
use waterui::reactive::binding;

fn slider_example() -> impl View {
    let volume = binding(0.5);
    Slider::new(0.0..=1.0, &volume)
}
```

## Advanced Form Patterns

### Multi-Step Forms

```rust
use waterui::reactive::binding;
use waterui::widget::condition::when;

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
    
    let step_display = Dynamic::new(wizard.current_step.map(|step| {
        match step {
            0 => vstack((
                "Personal Information",
                form(wizard.map_project(|w| &w.personal)),
            )).any(),
            1 => vstack((
                "Contact Information", 
                form(wizard.map_project(|w| &w.contact)),
            )).any(),
            _ => text("Registration Complete!").any(),
        }
    }));

    vstack((
        text(s!("Step {} of 2", wizard.current_step.map(|s| s + 1))),
        step_display,
        navigation_buttons(wizard),
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
use waterui::reactive::binding;

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
use waterui::reactive::binding;

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
    text(s!("{}", pwd.zip(confirm).map(|(p, c)| {
        if p == c && !p.is_empty() {
            "Passwords match ✓"
        } else {
            "Passwords do not match"
        }
    })))
}
```

# Form Validation Best Practices

### Real-time Validation with Computed Signals

For more complex forms, it's a good practice to encapsulate your validation logic into a separate struct. This makes your code more organized and reusable.

Let's create a `Validation` struct that holds computed signals for each validation rule.

```rust
use waterui::reactive::binding;

#[derive(Default, Clone, FormBuilder)]
struct ValidatedForm {
    email: String,
    password: String,
    age: i32,
}

struct Validation {
    is_valid_email: Computed<bool>,
    is_valid_password: Computed<bool>,
    is_valid_age: Computed<bool>,
    is_form_valid: Computed<bool>,
}

impl Validation {
    fn new(form: &Binding<ValidatedForm>) -> Self {
        let is_valid_email = form.map(|f| f.email.contains('@') && f.email.contains('.'));
        let is_valid_password = form.map(|f| f.password.len() >= 8);
        let is_valid_age = form.map(|f| f.age >= 18);
        let is_form_valid = is_valid_email.zip(is_valid_password).zip(is_valid_age).map(|((email, pass), age)| email && pass && age);

        Self {
            is_valid_email,
            is_valid_password,
            is_valid_age,
            is_form_valid,
        }
    }
}

fn validated_form_view() -> impl View {
    let form = binding(ValidatedForm::default());
    let validation = Validation::new(&form);
    
    vstack((
        form(form),
        
        // Validation messages
        text(validation.is_valid_email.map(|is_valid| if is_valid { "✓ Valid email" } else { "✗ Please enter a valid email" })),
        text(validation.is_valid_password.map(|is_valid| if is_valid { "✓ Password is strong enough" } else { "✗ Password must be at least 8 characters" })),
        text(validation.is_valid_age.map(|is_valid| if is_valid { "✓ Age requirement met" } else { "✗ Must be 18 or older" })),
        
        // Submit button - only enabled when form is valid
        when(validation.is_form_valid.clone(), || {
            button("Submit").action(|| {
                println!("Form submitted!");
            })
        })
        .or(|| text("Fill every requirement to enable submission.")),
    ))
}
```

## Integration with State Management

Forms integrate seamlessly with WaterUI's reactive state system:

```rust
use nami::s;
use waterui::widget::condition::when;

#[derive(Default, Clone, FormBuilder)]
struct UserSettings {
    name: String,
    theme: String,
    notifications: bool,
}

fn settings_panel() -> impl View {
    let settings = UserSettings::binding();
    
    // Computed values based on form state
    let has_changes = settings.map(|s| {
        s.name != "Default Name" ||
        s.theme != "Light" ||
        s.notifications
    });
    
    let settings_summary = s!("User: {} | Theme: {} | Notifications: {}", 
        settings.map_project(|s| &s.name),
        settings.map_project(|s| &s.theme),
        settings.map_project(|s| &s.notifications).map(|n| if n { "On" } else { "Off" })
    );
    
    vstack((
        "Settings",
        form(&settings),
        
        // Live preview
        "Preview:",
        text(settings_summary),
        
        // Save button
        when(has_changes.clone(), || {
            button("Save Changes").action_with(&settings, |s| {
                save_settings(s);
            })
        })
        .or(|| text("No changes to save.")),
    ))
}

fn save_settings(settings: &UserSettings) {
    println!("Saving settings: {settings:?}");
    // Save to database, file, etc.
}
```
