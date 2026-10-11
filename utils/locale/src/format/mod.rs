//! Locale-aware formatting utilities.

#[cfg(feature = "datetime")]
pub mod date;
#[cfg(feature = "number")]
pub mod number;
#[cfg(feature = "number")]
pub mod unit;

use core::fmt::{self, Display};

use crate::locale::Locale;

/// Trait for locale-aware display.
///
/// Types implementing this trait can format themselves differently
/// based on the current locale.
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{Length, LocalizedDisplay, Meter, locales};
///
/// let distance = Length::<Meter>::new(18.0);
///
/// // Format with different locales. The unit symbol follows the number with
/// // no separator; CLDR would put a space in before a Latin-script symbol,
/// // which this hand-rolled unit table does not yet do.
/// assert_eq!(distance.to_localized_string(&locales::EN), "18m");
/// assert_eq!(distance.to_localized_string(&locales::ZH_CN), "18米");
/// assert_eq!(distance.to_localized_string(&locales::JA), "18メートル");
/// ```
pub trait LocalizedDisplay {
    /// Format the value for the given locale.
    ///
    /// # Errors
    ///
    /// Returns the formatter error propagated from `fmt::Formatter`.
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result;

    /// Convert to a localized string.
    fn to_localized_string(&self, locale: &Locale) -> String {
        struct Adapter<'a, T: LocalizedDisplay + ?Sized>(&'a T, &'a Locale);

        impl<T: LocalizedDisplay + ?Sized> Display for Adapter<'_, T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(self.1, f)
            }
        }

        Adapter(self, locale).to_string()
    }

    /// Get a Display adapter for use with format strings.
    fn localized_fmt<'a>(&'a self, locale: &'a Locale) -> impl Display + 'a
    where
        Self: Sized,
    {
        struct LocalizedFmt<'a, T: LocalizedDisplay>(&'a T, &'a Locale);

        impl<T: LocalizedDisplay> Display for LocalizedFmt<'_, T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(self.1, f)
            }
        }

        LocalizedFmt(self, locale)
    }
}

/// Blanket implementation for any type that implements Display.
///
/// This allows standard types to be used with `LocalizedDisplay`,
/// though they won't have locale-specific formatting.
impl<T: Display> LocalizedDisplay for T {
    fn fmt(&self, _locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

/// A directionally isolated, locale-aware interpolation argument used by
/// WaterUI's `text!` macro.
#[cfg(feature = "number")]
#[doc(hidden)]
pub struct LocalizedArgument<'a, T> {
    value: &'a T,
    locale: &'a Locale,
}

/// `{:?}` interpolations render the wrapped value, not this adapter.
#[cfg(feature = "number")]
impl<T: fmt::Debug> fmt::Debug for LocalizedArgument<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.value, f)
    }
}

#[cfg(feature = "number")]
impl<'a, T> LocalizedArgument<'a, T> {
    /// Creates an interpolation adapter.
    #[must_use]
    pub const fn new(value: &'a T, locale: &'a Locale) -> Self {
        Self { value, locale }
    }
}

#[cfg(feature = "number")]
struct RawArgument<'a, T: LocalizedDisplay + ?Sized>(&'a T, &'a Locale);

#[cfg(feature = "number")]
impl<T: LocalizedDisplay + ?Sized> Display for RawArgument<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(self.1, f)
    }
}

#[cfg(feature = "number")]
fn numeric_argument<T: LocalizedDisplay + ?Sized>(
    value: &T,
    locale: &Locale,
    precision: Option<usize>,
) -> Option<String> {
    let numeric = matches!(
        core::any::type_name::<T>(),
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    );
    let argument = RawArgument(value, locale);
    numeric.then(|| precision.map_or_else(|| argument.to_string(), |p| format!("{argument:.p$}")))
}

#[cfg(feature = "number")]
impl<T: LocalizedDisplay> Display for LocalizedArgument<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\u{2068}")?;
        if let Some(number) = numeric_argument(self.value, self.locale, f.precision()) {
            let formatted = number::format_number_text(self.locale, &number);
            if f.width().is_none() && !f.sign_plus() && !f.sign_aware_zero_pad() {
                // No width, sign or zero-padding flags: emit the localized
                // digits verbatim so a locale-specific minus sign survives.
                f.write_str(&formatted)?;
            } else {
                // `pad_integral` re-emits the sign and applies width, fill,
                // alignment and sign-aware zero padding exactly as `core::fmt`
                // does for numbers. `pad` cannot be used here: it interprets
                // `precision` as a maximum string length and would truncate
                // the already-rounded text.
                let digits = formatted
                    .strip_prefix(|minus| matches!(minus, '-' | '\u{2212}'))
                    .unwrap_or(formatted.as_str());
                f.pad_integral(!number.starts_with('-'), "", digits)?;
            }
        } else {
            self.value.fmt(self.locale, f)?;
        }
        f.write_str("\u{2069}")
    }
}

/// Wrapper for locale-aware list formatting.
///
/// Formats lists according to locale conventions:
/// - English: "A, B, and C"
/// - Chinese: "A、B、C"
/// - Japanese: "A、B、C"
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{LocalizedDisplay, LocalizedList, locales};
///
/// let items = LocalizedList(&["Apple", "Banana", "Orange"]);
///
/// assert_eq!(items.to_localized_string(&locales::EN), "Apple, Banana, and Orange");
/// assert_eq!(items.to_localized_string(&locales::ZH_CN), "Apple、Banana和Orange");
/// ```
#[cfg(feature = "list")]
#[derive(Debug)]
pub struct LocalizedList<'a>(pub &'a [&'a str]);

#[cfg(feature = "list")]
impl LocalizedDisplay for LocalizedList<'_> {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use icu_list::{
            ListFormatter,
            options::{ListFormatterOptions, ListLength},
        };

        let options = ListFormatterOptions::default().with_length(ListLength::Wide);
        let formatter = ListFormatter::try_new_and(locale.0.clone().into(), options)
            .unwrap_or_else(|_| {
                ListFormatter::try_new_and(icu_locale::locale!("en").into(), options)
                    .expect("English list formatter should always be available")
            });

        let result = formatter.format(self.0.iter().copied());
        write!(f, "{result}")
    }
}

#[cfg(all(test, feature = "number"))]
mod tests {
    use super::*;
    use crate::locale::locales;

    #[test]
    fn localized_argument_formats_numbers_and_adds_bidi_isolation() {
        let value = 1234.5_f64;
        let formatted = format!("{}", LocalizedArgument::new(&value, &locales::DE));

        assert!(formatted.starts_with('\u{2068}'));
        assert!(formatted.ends_with('\u{2069}'));
        assert!(formatted.contains("1.234,5"));
    }

    #[test]
    fn localized_argument_accepts_non_static_borrowed_text() {
        let owned = String::from("مرحبا");
        let borrowed = owned.as_str();

        assert_eq!(
            LocalizedArgument::new(&borrowed, &locales::AR).to_string(),
            "\u{2068}مرحبا\u{2069}"
        );
    }

    #[test]
    fn localized_argument_applies_width_and_zero_fill_to_integers() {
        assert_eq!(
            format!("{:06}", LocalizedArgument::new(&0_u32, &locales::EN)),
            "\u{2068}000000\u{2069}"
        );
        assert_eq!(
            format!("{:06}", LocalizedArgument::new(&-42_i32, &locales::EN)),
            "\u{2068}-00042\u{2069}"
        );
        assert_eq!(
            format!("{:>8}", LocalizedArgument::new(&1234_i64, &locales::EN)),
            "\u{2068}   1,234\u{2069}"
        );
    }

    #[test]
    fn localized_argument_applies_width_to_string_arguments() {
        let value = "hi";
        assert_eq!(
            format!("{:>8}", LocalizedArgument::new(&value, &locales::EN)),
            "\u{2068}      hi\u{2069}"
        );
    }

    #[test]
    fn localized_argument_applies_precision_and_width_to_floats() {
        let value = 1234.567_f64;
        assert_eq!(
            format!("{:.2}", LocalizedArgument::new(&value, &locales::EN)),
            "\u{2068}1,234.57\u{2069}"
        );
        assert_eq!(
            format!(
                "{:>10.2}",
                LocalizedArgument::new(&1234.5_f64, &locales::EN)
            ),
            "\u{2068}  1,234.50\u{2069}"
        );
    }

    #[test]
    fn localized_argument_debug_forwards_to_the_wrapped_value() {
        #[derive(Debug)]
        enum Fruit {
            Apple,
        }

        assert_eq!(
            format!("{:?}", LocalizedArgument::new(&Fruit::Apple, &locales::EN)),
            "Apple"
        );
        assert_eq!(
            format!("{:#?}", LocalizedArgument::new(&Fruit::Apple, &locales::EN)),
            "Apple"
        );
    }
}
