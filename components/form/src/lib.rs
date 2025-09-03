//! # `WaterUI` Form Components
//!
//! This crate provides a comprehensive form system for `WaterUI` applications with ergonomic
//! macros and type-safe form building capabilities.
//!
//! ## Quick Start
//!
//! The easiest way to create forms is using the `#[derive(FormBuilder)]` macro:
//!
//! ```rust
//! use waterui_form::FormBuilder;
//! use waterui_core::View;
//!
//! #[derive(Default, Clone, Debug, FormBuilder)]
//! pub struct LoginForm {
//!     /// The user's username
//!     pub username: String, // will be a text field
//!     /// The user's password  
//!     pub password: String, // will be a text field with password mode
//!     /// Whether to remember the user
//!     pub remember_me: bool, // will be a toggle
//!     /// The user's age
//!     pub age: i32, // will be a stepper
//! }
//!
//! fn login_view() -> impl View {
//!     let form_binding = LoginForm::binding();
//!     form(&form_binding)
//! }
//! ```
//!
//! ## Type-to-Component Mapping
//!
//! The `FormBuilder` derive macro automatically maps Rust types to appropriate form components:
//!
//! | Rust Type | Form Component | Description |
//! |-----------|----------------|-------------|
//! | `String`, `&str` | [`TextField`] | Single-line text input |
//! | `bool` | [`Toggle`] | On/off switch |
//! | `i32`, `i64`, etc. | [`Stepper`] | Numeric input with +/- buttons |
//! | `f32`, `f64` | [`Slider`] | Slider with 0.0-1.0 range |
//! | [`Color`](waterui_core::Color) | [`ColorPicker`](picker::ColorPicker) | Color selection widget |
//!
//! ## Advanced Usage
//!
//! ### Custom Form Layouts
//!
//! You can compose forms with custom layouts by implementing [`FormBuilder`] manually:
//!
//! ```rust
//! use waterui_form::{FormBuilder, TextField, Toggle};
//! use waterui_core::{Binding, View};
//! use waterui_layout::{hstack, vstack};
//!
//! struct CustomForm {
//!     name: String,
//!     active: bool,
//! }
//!
//! impl FormBuilder for CustomForm {
//!     type View = VStack;
//!
//!     fn view(binding: &Binding<Self>) -> Self::View {
//!         vstack((
//!             TextField::new(&binding.name),
//!             hstack((
//!                 text!("Active:"),
//!                 Toggle::new(&binding.active),
//!             ))
//!         ))
//!     }
//! }
//! ```
//!
//! ### Secure Fields
//!
//! For sensitive data like passwords, use [`SecureField`]:
//!
//! ```rust
//! use waterui_form::{SecureField, secure};
//! use waterui_core::{Binding, View};
//!
//! fn password_form() -> impl View {
//!     let password_binding = Binding::<String>::default();
//!     secure(&password_binding)
//! }
//! ```
//!
//! ## Form Validation
//!
//! Forms integrate seamlessly with `WaterUI`'s reactive state system. Use bindings
//! to access form values and implement validation logic:
//!
//! ```rust
//! use waterui_form::FormBuilder;
//! use waterui_core::{Binding, View};
//! use waterui_text::text;
//!
//! #[derive(Default, Clone, FormBuilder)]
//! struct ValidatedForm {
//!     email: String,
//!     age: i32,
//! }
//!
//! fn validated_form_view() -> impl View {
//!     let form_binding = ValidatedForm::binding();
//!     
//!     vstack((
//!         form(&form_binding),
//!         // Add validation feedback
//!         text!(
//!             if form_binding.email.get().contains('@') {
//!                 "Valid email"
//!             } else {
//!                 "Please enter a valid email"
//!             }
//!         ),
//!     ))
//! }
//! ```
//!
//! ## Complete Examples
//!
//! ### E-commerce Product Form
//!
//! ```rust
//! use waterui_form::{FormBuilder, form};
//! use waterui_core::{Binding, View, Color};
//! use waterui_layout::vstack;
//! use waterui_text::text;
//!
//! #[derive(Default, Clone, Debug, FormBuilder)]
//! struct ProductForm {
//!     /// Product name
//!     name: String,
//!     /// Product description
//!     description: String,
//!     /// Price in cents
//!     price_cents: i32,
//!     /// Discount percentage (0.0 to 1.0)
//!     discount: f32,
//!     /// Product is currently active
//!     active: bool,
//!     /// Brand color theme
//!     brand_color: Color,
//! }
//!
//! fn product_editor() -> impl View {
//!     let form_binding = ProductForm::binding();
//!
//!     vstack((
//!         text!("Product Editor"),
//!         form(&form_binding),
//!         // Real-time preview
//!         vstack((
//!             text!(format!("Name: {}", form_binding.name.get())),
//!             text!(format!("Price: ${:.2}", form_binding.price_cents.get() as f32 / 100.0)),
//!             text!(format!("Discount: {:.0}%", form_binding.discount.get() * 100.0)),
//!             text!(format!("Status: {}",
//!                 if form_binding.active.get() { "Active" } else { "Inactive" }
//!             )),
//!         ))
//!     ))
//! }
//! ```
//!
//! ### User Settings with Validation
//!
//! ```rust
//! # use waterui_form::{FormBuilder, form};
//! # use waterui_core::{Binding, View};
//! # use waterui_layout::vstack;
//! # use waterui_text::text;
//! #[derive(Default, Clone, Debug, FormBuilder)]
//! struct UserSettings {
//!     /// Display name (2-50 characters)
//!     display_name: String,
//!     /// Email address for notifications
//!     email: String,
//!     /// Age (must be 13+)
//!     age: i32,
//!     /// Maximum file upload size (MB)
//!     max_upload_mb: i32,
//!     /// Enable email notifications
//!     email_notifications: bool,
//!     /// Enable push notifications
//!     push_notifications: bool,
//!     /// Theme opacity (0.0 = transparent, 1.0 = opaque)
//!     theme_opacity: f32,
//! }
//!
//! fn settings_form_with_validation() -> impl View {
//!     let settings_binding = UserSettings::binding();
//!
//!     vstack((
//!         text!("Account Settings"),
//!         form(&settings_binding),
//!         
//!         // Validation feedback
//!         text!(
//!             validate_settings(&settings_binding.get())
//!         ),
//!     ))
//! }
//!
//! fn validate_settings(settings: &UserSettings) -> &'static str {
//!     if settings.display_name.len() < 2 {
//!         "Display name too short"
//!     } else if settings.age < 13 {
//!         "Must be 13 or older"
//!     } else if !settings.email.contains('@') {
//!         "Please enter a valid email"
//!     } else if settings.max_upload_mb > 100 {
//!         "Upload limit cannot exceed 100MB"
//!     } else {
//!         "All settings are valid ✓"
//!     }
//! }
//! ```
//!
//! ### Multi-step Form with State Management  
//!
//! ```rust
//! # use waterui_form::{FormBuilder, form};
//! # use waterui_core::{Binding, View};
//! # use waterui_layout::{vstack, hstack};
//! # use waterui_text::text;
//! #[derive(Default, Clone, Debug, FormBuilder)]
//! struct PersonalInfo {
//!     /// First name
//!     first_name: String,
//!     /// Last name  
//!     last_name: String,
//!     /// Date of birth (age)
//!     age: i32,
//! }
//!
//! #[derive(Default, Clone, Debug, FormBuilder)]
//! struct ContactInfo {
//!     /// Email address
//!     email: String,
//!     /// Phone number
//!     phone: String,
//!     /// Preferred contact method
//!     prefer_email: bool, // true = email, false = phone
//! }
//!
//! #[derive(Default, Clone, Debug)]
//! struct RegistrationState {
//!     personal: PersonalInfo,
//!     contact: ContactInfo,
//!     current_step: i32,
//! }
//!
//! fn multi_step_registration() -> impl View {
//!     let state_binding = Binding::new(RegistrationState::default());
//!
//!     vstack((
//!         text!(format!("Step {} of 2", state_binding.current_step.get() + 1)),
//!         
//!         match state_binding.current_step.get() {
//!             0 => vstack((
//!                 text!("Personal Information"),
//!                 form(&state_binding.personal),
//!             )),
//!             1 => vstack((
//!                 text!("Contact Information"),
//!                 form(&state_binding.contact),
//!             )),
//!             _ => text!("Registration Complete!"),
//!         },
//!
//!         // Navigation buttons
//!         hstack((
//!             if state_binding.current_step.get() > 0 {
//!                 // Previous button (implementation depends on button component)
//!                 text!("← Previous")
//!             } else {
//!                 text!("")
//!             },
//!             if state_binding.current_step.get() < 2 {
//!                 // Next button (implementation depends on button component)  
//!                 text!("Next →")
//!             } else {
//!                 text!("Submit")
//!             },
//!         ))
//!     ))
//! }
//! ```

#![no_std]
extern crate alloc;

#[doc(inline)]
pub use waterui_form_derive::{FormBuilder, form};

/// Text field form component module.
pub mod text_field;
/// Toggle (switch) form component module.
pub mod toggle;
#[doc(inline)]
pub use text_field::{TextField, field};
#[doc(inline)]
pub use toggle::{Toggle, toggle};
/// Slider form component module.
pub mod slider;
pub use slider::Slider;
/// Picker form component module.
pub mod picker;
/// Stepper form component module.
pub mod stepper;
#[doc(inline)]
pub use stepper::{Stepper, stepper};
use waterui_core::{Binding, Color, Str, View};

/// Trait for types that can be automatically converted to form UI components.
///
/// This trait enables ergonomic form creation by mapping Rust data structures
/// to interactive UI components. It can be implemented manually for custom layouts
/// or automatically derived using `#[derive(FormBuilder)]`.
///
/// # Automatic Implementation via Derive Macro
///
/// The most convenient way to use this trait is through the derive macro:
///
/// ```rust
/// use waterui_form::FormBuilder;
///
/// #[derive(Default, Clone, Debug, FormBuilder)]
/// pub struct UserProfile {
///     /// User's display name
///     pub name: String,
///     /// Account active status  
///     pub active: bool,
///     /// User's current level
///     pub level: i32,
/// }
/// ```
///
/// This automatically generates appropriate form components for each field type.
///
/// # Manual Implementation
///
/// For custom layouts or specialized form behavior, implement the trait manually:
///
/// ```rust
/// use waterui_form::{FormBuilder, TextField, Toggle};
/// use waterui_core::{Binding, View};
/// use waterui_layout::vstack;
///
/// struct CustomForm {
///     title: String,
///     enabled: bool,
/// }
///
/// impl FormBuilder for CustomForm {
///     type View = VStack;
///
///     fn view(binding: &Binding<Self>) -> Self::View {
///         vstack((
///             TextField::new(&binding.title),
///             Toggle::new(&binding.enabled),
///         ))
///     }
/// }
/// ```
///
/// # Reactive Form State
///
/// Forms created with `FormBuilder` are automatically reactive. Changes to form
/// fields immediately update the bound data, enabling real-time validation and
/// UI updates:
///
/// ```rust
/// # use waterui_form::FormBuilder;
/// # use waterui_core::{Binding, View};
/// # #[derive(Default, Clone, FormBuilder)]
/// # struct MyForm { name: String }
/// fn reactive_example() -> impl View {
///     let form_binding = MyForm::binding();
///     
///     // Form updates are immediately reflected in the binding
///     vstack((
///         form(&form_binding),
///         text!(format!("Hello, {}!", form_binding.name.get())),
///     ))
/// }
/// ```
pub trait FormBuilder: Sized {
    /// The view type that represents this form field.
    ///
    /// This associated type determines what UI component will be rendered
    /// for this form. For derived implementations, this is typically a
    /// layout containing multiple form fields.
    type View: View;

    /// Creates a view representation of this form field bound to the given binding.
    ///
    /// This method is the core of the form building system. It takes a binding
    /// to the form data and returns a view that displays interactive form controls.
    ///
    /// # Parameters
    ///
    /// * `binding` - A reference to a binding that holds the form's data state
    ///
    /// # Returns
    ///
    /// A view that renders the form's UI components, automatically bound to the data
    fn view(binding: &Binding<Self>) -> Self::View;

    /// Creates a new binding with the default value for this form.
    ///
    /// This convenience method creates a new binding initialized with the default
    /// values for all form fields. It requires the form type to implement
    /// [`Default`] and [`Clone`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use waterui_form::FormBuilder;
    /// # #[derive(Default, Clone, FormBuilder)]
    /// # struct LoginForm { username: String, password: String }
    /// let form_binding = LoginForm::binding();
    /// // form_binding now holds a LoginForm with default values
    /// ```
    #[must_use]
    fn binding() -> Binding<Self>
    where
        Self: Default + Clone,
    {
        Binding::default()
    }
}

macro_rules! impl_form_builder {
    ($ty:ty,$view:ty) => {
        impl FormBuilder for $ty {
            type View = $view;
            fn view(binding: &Binding<Self>) -> Self::View {
                <$view>::new(binding)
            }
        }
    };
}

impl_form_builder!(Str, TextField);
impl_form_builder!(i32, Stepper);
impl_form_builder!(bool, Toggle);
impl_form_builder!(Color, picker::ColorPicker);

/// Secure form components for handling sensitive data like passwords.
pub mod secure;
pub use secure::{SecureField, secure};

/// Creates a form view from a binding to a type that implements [`FormBuilder`].
///
/// This is the primary entry point for creating forms using the `FormBuilder` system.
/// It takes a binding to your form data structure and returns a view that renders
/// all the appropriate form controls.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use waterui_form::{FormBuilder, form};
/// use waterui_core::{Binding, View};
///
/// #[derive(Default, Clone, Debug, FormBuilder)]
/// struct ContactForm {
///     name: String,
///     email: String,
///     age: i32,
///     newsletter: bool,
/// }
///
/// fn contact_form_view() -> impl View {
///     let form_binding = ContactForm::binding();
///     form(&form_binding)
/// }
/// ```
///
/// ## With Custom Initialization
///
/// ```rust
/// # use waterui_form::{FormBuilder, form};
/// # use waterui_core::{Binding, View};
/// # #[derive(Default, Clone, Debug, FormBuilder)]
/// # struct ContactForm { name: String, email: String }
/// fn pre_filled_form() -> impl View {
///     let initial_data = ContactForm {
///         name: "John Doe".to_string(),
///         email: "john@example.com".to_string(),
///     };
///     let form_binding = Binding::new(initial_data);
///     form(&form_binding)
/// }
/// ```
///
/// ## Accessing Form Data
///
/// ```rust
/// # use waterui_form::{FormBuilder, form};
/// # use waterui_core::{Binding, View};
/// # use waterui_layout::vstack;
/// # use waterui_text::text;
/// # #[derive(Default, Clone, Debug, FormBuilder)]
/// # struct ContactForm { name: String, email: String }
/// fn form_with_output() -> impl View {
///     let form_binding = ContactForm::binding();
///     
///     vstack((
///         form(&form_binding),
///         // Display current form values
///         text!(format!("Name: {}", form_binding.name.get())),
///         text!(format!("Email: {}", form_binding.email.get())),
///     ))
/// }
/// ```
///
/// # Parameters
///
/// * `binding` - A reference to a binding containing the form's data
///
/// # Returns
///
/// A view that renders interactive form controls for all fields in the bound data structure
#[must_use]
pub fn form<T: FormBuilder>(binding: &Binding<T>) -> T::View {
    T::view(binding)
}
