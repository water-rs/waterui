//! # `WaterUI` Form Components
//!
//! This crate provides a comprehensive form system for `WaterUI` applications with ergonomic
//! macros and type-safe form building capabilities.

#![doc = include_str!("../README.md")]
#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "text-field")]
use alloc::string::String;
#[cfg(feature = "color-picker")]
use waterui_graphics::color::Color;

/// Picker form component module.
#[cfg(feature = "calendar")]
pub mod calendar;
#[cfg(feature = "calendar")]
pub use calendar::{Calendar, CalendarConfig, CalendarGrid};
/// Native picker controls for colors, dates, files, and multiple dates.
pub mod picker;
#[cfg(feature = "color-picker")]
pub use picker::ColorPicker;
#[cfg(feature = "multi-date-picker")]
pub use picker::MultiDatePicker;
#[cfg(feature = "file-picker")]
pub use picker::file::FilePicker;
#[cfg(feature = "date-picker")]
pub use picker::{
    DatePickable, DatePicker, DatePickerComponents, DateTimePickerComponents, TimePickable,
    TimePickerComponents,
};
#[cfg(feature = "picker")]
pub use picker::{Picker, PickerItem, PickerStyle, picker};
/// Validation helpers for form fields.
#[cfg(feature = "validation")]
pub mod valid;

#[cfg(feature = "slider")]
use waterui_controls::slider::slider;
#[cfg(feature = "stepper")]
use waterui_controls::stepper::stepper;
use waterui_controls::{IntoLabel, LabelKind, TitleLabel};
use waterui_core::{Binding, Str, View};

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
/// ```text
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
/// ```text
/// use waterui_form::FormBuilder;
/// use waterui::prelude::*;
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
///             TextField::new("Title", &binding.title),
///             Toggle::new("Enabled", &binding.enabled),
///         ))
///     }
/// }
/// ```
pub trait FormBuilder: Sized {
    /// The view type that represents this form field.
    ///
    /// This associated type determines what UI component will be rendered
    /// for this form. For derived implementations, this is typically a
    /// layout containing multiple form fields.
    type View<L: LabelKind>: View;

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
    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        placeholder: Str,
    ) -> Self::View<L::Kind>;

    /// Creates a new binding with the default value for this form.
    ///
    /// This convenience method creates a new binding initialized with the default
    /// values for all form fields. It requires the form type to implement
    /// [`Default`] and [`Clone`].
    ///
    /// # Examples
    ///
    /// ```text
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

#[cfg(feature = "text-field")]
fn with_optional_prompt<L: LabelKind>(
    mut field: waterui_controls::TextField<L>,
    placeholder: Str,
) -> waterui_controls::TextField<L> {
    if !placeholder.is_empty() {
        field = field.prompt(placeholder);
    }
    field
}

#[cfg(feature = "text-field")]
fn map_string_binding(binding: &Binding<String>) -> Binding<Str> {
    Binding::mapping(binding, Str::from, |binding, str_val: Str| {
        binding.set(str_val.into());
    })
}

#[cfg(feature = "text-field")]
impl FormBuilder for Str {
    type View<L: LabelKind> = waterui_controls::TextField<L>;

    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        placeholder: Str,
    ) -> Self::View<L::Kind> {
        with_optional_prompt(
            waterui_controls::TextField::new(label, binding),
            placeholder,
        )
    }
}

#[cfg(feature = "text-field")]
impl FormBuilder for String {
    type View<L: LabelKind> = waterui_controls::TextField<L>;

    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        placeholder: Str,
    ) -> Self::View<L::Kind> {
        let str_binding = map_string_binding(binding);
        with_optional_prompt(
            waterui_controls::TextField::new(label, &str_binding),
            placeholder,
        )
    }
}

// Other components don't have prompt
#[cfg(feature = "stepper")]
impl FormBuilder for i32 {
    type View<L: LabelKind> = waterui_controls::Stepper<L>;
    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        _placeholder: Str,
    ) -> Self::View<L::Kind> {
        stepper(label, binding).range(Self::MIN..=Self::MAX)
    }
}

#[cfg(feature = "toggle")]
impl FormBuilder for bool {
    type View<L: LabelKind> = waterui_controls::Toggle<L>;
    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        _placeholder: Str,
    ) -> Self::View<L::Kind> {
        waterui_controls::Toggle::new(label, binding)
    }
}

#[cfg(feature = "color-picker")]
impl FormBuilder for Color {
    type View<L: LabelKind> = picker::ColorPicker<L>;
    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        _placeholder: Str,
    ) -> Self::View<L::Kind> {
        picker::ColorPicker::new(label, binding)
    }
}

#[cfg(feature = "slider")]
impl FormBuilder for f64 {
    type View<L: LabelKind> = waterui_controls::Slider<L, (), ()>;
    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        _placeholder: Str,
    ) -> Self::View<L::Kind> {
        slider(label, binding)
    }
}

#[cfg(feature = "slider")]
impl FormBuilder for f32 {
    type View<L: LabelKind> = waterui_controls::Slider<L, (), ()>;
    fn view<L: IntoLabel>(
        binding: &Binding<Self>,
        label: L,
        _placeholder: Str,
    ) -> Self::View<L::Kind> {
        slider(
            label,
            #[allow(clippy::cast_possible_truncation)]
            &Binding::mapping(binding, f64::from, |binding, val| binding.set(val as Self)),
        )
    }
}

/// Secure form components for handling sensitive data like passwords.
#[cfg(feature = "secure-field")]
pub mod secure;
#[cfg(feature = "secure-field")]
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
/// ```text
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
/// ```text
/// # use waterui_form::{FormBuilder, form};
/// # use waterui_core::{Binding, View};
/// # #[derive(Default, Clone, Debug, FormBuilder)]
/// # struct ContactForm { name: String, email: String }
/// fn pre_filled_form() -> impl View {
///     let initial_data = ContactForm {
///         name: "Lexo Liu".to_string(),
///         email: "me@lexo.cool".to_string(),
///     };
///     let form_binding = Binding::new(initial_data);
///     form(&form_binding)
/// }
/// ```
///
/// ## Accessing Form Data
///
/// ```text
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
///         text!(format!("Name: {}", form_binding.name.snapshot())),
///         text!(format!("Email: {}", form_binding.email.snapshot())),
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
#[must_use = "this builds the form's view; dropping it renders no controls"]
pub fn form<T: FormBuilder>(binding: &Binding<T>) -> T::View<TitleLabel> {
    T::view(binding, Str::default(), Str::default())
}

#[cfg(all(test, feature = "text-field"))]
mod tests {
    use alloc::string::String;

    use nami::{Binding, Signal};
    use waterui_core::Str;

    use super::map_string_binding;

    #[test]
    fn map_string_binding_round_trip() {
        let source = Binding::new(String::from("hello"));
        let mapped = map_string_binding(&source);
        assert_eq!(mapped.snapshot().as_str(), "hello");

        mapped.set(Str::from(String::from("updated")));
        assert_eq!(source.snapshot(), "updated");
    }
}
