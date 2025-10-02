//! # `WaterUI` Form Components
//!
//! This crate provides a comprehensive form system for `WaterUI` applications with ergonomic
//! macros and type-safe form building capabilities.

#![no_std]
extern crate alloc;

use waterui_color::Color;
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
use waterui_core::{AnyView, Binding, Str, View};

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
/// # #[derive(Default, Clone, Project, FormBuilder)]
/// # struct MyForm { name: String }
/// fn reactive_example() -> impl View {
///     let form_binding = MyForm::binding();
///     
///     // Form updates are immediately reflected in the binding
///     vstack((
///         form(&form_binding),
///         text!("Hello, {}!", form_binding.project().name),
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
    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View;

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

// TextField has prompt, so handle it specially
impl FormBuilder for Str {
    type View = TextField;
    #[allow(unused_variables)]
    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View {
        let mut field = TextField::new(binding).label(label);
        if !placeholder.is_empty() {
            field = field.prompt(placeholder);
        }
        field
    }
}

// String also uses TextField with prompt
impl FormBuilder for alloc::string::String {
    type View = TextField;

    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View {
        use alloc::string::ToString;
        use nami::Binding as NamiBinding;
        // Map String to Str binding
        let str_binding = NamiBinding::mapping(binding, Str::from, |binding, str_val: Str| {
            *binding.get_mut() = str_val.to_string();
        });
        let mut field = TextField::new(&str_binding).label(label);
        if !placeholder.is_empty() {
            field = field.prompt(placeholder);
        }
        field
    }
}

// Other components don't have prompt
impl FormBuilder for i32 {
    type View = Stepper;
    #[allow(unused_variables)]
    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View {
        Stepper::new(binding).label(label)
    }
}

impl FormBuilder for bool {
    type View = Toggle;
    #[allow(unused_variables)]
    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View {
        Toggle::new(binding).label(label)
    }
}

impl FormBuilder for Color {
    type View = picker::ColorPicker;
    #[allow(unused_variables)]
    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View {
        picker::ColorPicker::new(binding).label(label)
    }
}

impl FormBuilder for f64 {
    type View = Slider;
    #[allow(unused_variables)]
    fn view(binding: &Binding<Self>, label: AnyView, placeholder: Str) -> Self::View {
        Slider::new(0.0..=1.0, binding).label(label)
    }
}

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
    T::view(binding, AnyView::default(), Str::default())
}
