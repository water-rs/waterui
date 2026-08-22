//! Validation utilities for form components.

use core::fmt;
use core::{
    error::Error,
    fmt::{Debug, Display},
    ops::Range,
};

use alloc::string::{String, ToString};
use nami::{Binding, SignalExt};
use regex::Regex;
use waterui_controls::text_field::TextField;
use waterui_core::{Str, View};
use waterui_layout::stack::vstack;
use waterui_text::Text;
use waterui_text::styled::StyledStr;

macro_rules! impl_error {
    ($ident:ident,$message:expr) => {
        #[derive(Debug, Clone, Copy)]
        #[doc = $message]
        pub struct $ident;

        impl fmt::Display for $ident {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, $message)
            }
        }

        impl core::error::Error for $ident {}
    };
}

/// Trait for views that can be validated.
/// This trait allows a view to be associated with a validator that can
/// check the validity of the view's value.
pub trait Validatable: View + Sized {
    /// The type of value that this view holds and can be validated.
    type Value;
    /// Returns a mutable reference to the binding of the view's value.
    /// This binding can be used to get or set the value,
    /// as well as to observe changes
    fn validable(&mut self) -> &mut Binding<Self::Value>;
}

/// Trait for validating values of type `T`.
///
/// Implementors of this trait provide a method to validate values
/// and return either success or a reason for validation failure.
pub trait Validator<T>: Clone + 'static {
    /// The error type returned when validation fails.
    type Err: Error;
    /// Validates the given value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to validate.
    ///
    /// # Errors
    ///
    /// Returns an error of type `Self::Err` if validation fails.
    fn validate(&self, value: T) -> Result<(), Self::Err>;

    /// Combines this validator with another using logical AND.
    /// # Arguments
    /// * `other` - The other validator to combine with.
    ///
    /// # Returns
    /// A new validator that succeeds only if both validators succeed.
    fn and<V>(self, other: V) -> And<Self, V>
    where
        Self: Sized,
        V: Validator<T>,
    {
        And(self, other)
    }
    /// Combines this validator with another using logical OR.
    /// # Arguments
    /// * `other` - The other validator to combine with.
    /// # Returns
    /// A new validator that succeeds if either validator succeeds.
    fn or<V>(self, other: V) -> Or<Self, V>
    where
        Self: Sized,
        V: Validator<T>,
    {
        Or(self, other)
    }
}

/// A view that combines a view with a validator.
/// This struct holds a view and a validator, allowing the view's value
/// to be validated
#[derive(Debug, Clone)]
pub struct ValidatableView<V, T> {
    view: V,
    validator: T,
}

impl<V, T> View for ValidatableView<V, T>
where
    T: Validator<V::Value>,
    V: Validatable<Value: Clone>,
{
    fn body(mut self, _env: &waterui_core::Environment) -> impl View {
        let value = {
            let value = self.view.validable();
            let validator = self.validator.clone();
            let new_binding = value.filter(move |v| validator.validate(v.clone()).is_ok());

            *value = new_binding;
            value.clone()
        };
        vstack((
            self.view,
            Text::computed(value.map(move |v| {
                if let Err(reason) = self.validator.validate(v) {
                    reason.to_string().into()
                } else {
                    Str::new()
                }
            })),
        ))
    }
}

/// An error indicating that a value is out of a specified range.
#[derive(Debug, Clone)]
pub struct OutOfRange<T>(pub Range<T>);

impl<T: Display> Display for OutOfRange<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Value is out of range: {} - {}.",
            self.0.start, self.0.end
        )
    }
}

impl<T: Display + Debug> Error for OutOfRange<T> {}

impl<T: Display + Debug + Ord + Clone + 'static> Validator<T> for Range<T> {
    type Err = OutOfRange<T>;
    fn validate(&self, value: T) -> Result<(), Self::Err> {
        self.contains(&value)
            .then_some(())
            .ok_or(OutOfRange(self.clone()))
    }
}

impl<V: Validatable, T> ValidatableView<V, T> {
    /// Creates a new `ValidatableView`.
    ///
    /// # Arguments
    /// * `view` - The view to be validated.
    /// * `validator` - The validator
    pub const fn new(view: V, validator: T) -> Self {
        Self { view, validator }
    }
}
impl_error!(NotMatch, "Value does not match the required pattern.");

impl<T> Validator<T> for Regex
where
    T: AsRef<str>,
{
    type Err = NotMatch;
    fn validate(&self, value: T) -> Result<(), Self::Err> {
        self.is_match(value.as_ref()).then_some(()).ok_or(NotMatch)
    }
}

/// A validator that combines two validators with logical AND.
/// Short-circuits on the first failure.
#[derive(Debug, Clone)]
pub struct And<A, B>(A, B);

/// An error type for the `And` validator, representing which validator failed.
#[derive(Debug, Clone)]
pub enum AndError<A, B> {
    /// The first validator failed.
    A(A),
    /// The second validator failed.
    B(B),
}

impl<A, B> Display for AndError<A, B>
where
    A: Display,
    B: Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::A(a) => write!(f, "{a}"),
            Self::B(b) => write!(f, "{b}"),
        }
    }
}

impl<A, B> Error for AndError<A, B>
where
    A: Error,
    B: Error,
{
}

impl<T, A, B> Validator<T> for And<A, B>
where
    T: Clone,
    A: Validator<T>,
    B: Validator<T>,
{
    type Err = AndError<A::Err, B::Err>;
    fn validate(&self, value: T) -> Result<(), Self::Err> {
        self.0.validate(value.clone()).map_err(AndError::A)?;
        self.1.validate(value).map_err(AndError::B)
    }
}

/// A validator that combines two validators with logical OR.
/// Succeeds if at least one validator succeeds.
/// Short-circuits on the first success.
#[derive(Debug, Clone)]
pub struct Or<A, B>(A, B);

/// An error type for the `Or` validator, representing both validation failures.
#[derive(Debug, Clone)]
pub struct OrError<A, B>(pub A, pub B);

impl<A, B> Display for OrError<A, B>
where
    A: Display,
    B: Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "At least one of the following errors occurred:\n1. {}\n2. {}",
            self.0, self.1
        )
    }
}

impl<A, B> Error for OrError<A, B>
where
    A: Error,
    B: Error,
{
}

impl<T, A, B> Validator<T> for Or<A, B>
where
    T: Clone,
    A: Validator<T>,
    B: Validator<T>,
{
    type Err = OrError<A::Err, B::Err>;
    fn validate(&self, value: T) -> Result<(), Self::Err> {
        self.0
            .validate(value.clone())
            .or_else(|e1| self.1.validate(value).map_err(|e2| OrError(e1, e2)))
    }
}

/// A validator that checks if a value is present (not None or not empty).
#[derive(Debug, Clone, Copy)]
pub struct Required;

impl_error!(RequiredError, "Value is required.");

impl<T> Validator<Option<T>> for Required {
    type Err = RequiredError;
    fn validate(&self, value: Option<T>) -> Result<(), Self::Err> {
        value.is_some().then_some(()).ok_or(RequiredError)
    }
}

macro_rules! impl_required_for_text {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Validator<$ty> for Required {
                type Err = RequiredError;

                fn validate(&self, value: $ty) -> Result<(), Self::Err> {
                    // we consider a string with only whitespace as empty
                    (!AsRef::<str>::as_ref(&value).trim().is_empty())
                        .then_some(())
                        .ok_or(RequiredError)
                }
            }
        )*
    };
}

impl_required_for_text!(&str, Str, String);

/// Lifts a validator over plain text onto styled text.
///
/// Text inputs are backed by [`StyledStr`], but validators are naturally written
/// against plain text (`Required`, [`Regex`], …). `Plain` bridges the two so a
/// single plain-text validator works on a styled field, instead of every
/// validator needing a second styled implementation:
///
/// ```rust,ignore
/// ValidatableView::new(field, Plain(Required))
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Plain<V>(pub V);

impl<V> Validator<StyledStr> for Plain<V>
where
    V: Validator<Str>,
{
    type Err = V::Err;

    fn validate(&self, value: StyledStr) -> Result<(), Self::Err> {
        self.0.validate(value.to_plain())
    }
}

impl Validatable for TextField {
    type Value = StyledStr;

    fn validable(&mut self) -> &mut Binding<Self::Value> {
        self.value_binding()
    }
}

#[cfg(test)]
mod tests {
    use super::{Required, Validator};
    use alloc::string::ToString;
    use nami::Binding;
    use regex::Regex;
    use waterui_core::Str;

    #[test]
    fn required_accepts_non_empty_str_and_rejects_blank() {
        assert!(Required.validate("water").is_ok());
        assert!(Required.validate(" ui ").is_ok());
        assert!(Required.validate("").is_err());
        assert!(Required.validate("   \t\n").is_err());
    }

    #[test]
    fn required_covers_the_owned_string_types() {
        assert!(Required.validate(Str::from("water")).is_ok());
        assert!(Required.validate(Str::new()).is_err());
        assert!(Required.validate("water".to_string()).is_ok());
        assert!(Required.validate("  ".to_string()).is_err());
    }

    #[test]
    fn required_option_impl_agrees_with_the_string_impls() {
        assert!(Required.validate(Some(1)).is_ok());
        assert!(Required.validate(Option::<i32>::None).is_err());
    }

    #[test]
    fn plain_lifts_a_text_validator_onto_a_styled_field() {
        use super::Plain;
        use waterui_text::styled::StyledStr;

        let validator = Plain(Required);
        assert!(validator.validate(StyledStr::plain("water")).is_ok());
        assert!(validator.validate(StyledStr::plain("  ")).is_err());
        // Styling is irrelevant to a plain-text rule.
        assert!(
            validator
                .validate(StyledStr::plain("water").italic(true))
                .is_ok()
        );
    }

    #[test]
    fn validatable_view_accepts_a_text_field() {
        use super::{Plain, ValidatableView};
        use waterui_controls::text_field::TextField;
        use waterui_core::View;

        // `Validatable` had no implementations, so `ValidatableView` could not be
        // constructed at all. Building one over a `TextField` and asserting it is
        // a `View` is what pins the surface as usable.
        fn assert_view<V: View>(_: &V) {}

        let value = Binding::container(Str::from(""));
        let view = ValidatableView::new(TextField::new("Value", &value), Plain(Required));
        assert_view(&view);
    }

    #[test]
    fn required_and_regex_rejects_blank_input() {
        let validator = Validator::<&str>::and(Required, Regex::new("^[a-z]+$").unwrap());
        assert!(validator.validate("water").is_ok());
        // The blank string matches no pattern arm, but `Required` must be the
        // one that rejects it — this is the regression guard for the inverted
        // condition, which made `Required` pass on empty input.
        assert!(validator.validate("").is_err());
        assert!(Required.validate("").is_err());
    }
}
