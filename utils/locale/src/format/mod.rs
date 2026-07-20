//! Locale-aware formatting utilities.

pub mod date;
pub mod number;
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
/// ```rust,ignore
/// use waterui_locale::{LocalizedDisplay, Length, Meter, locales};
///
/// let distance = Length::<Meter>::new(18.0);
///
/// // Format with different locales
/// assert_eq!(distance.to_localized_string(&locales::EN), "18 m");
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

/// Wrapper for locale-aware list formatting.
///
/// Formats lists according to locale conventions:
/// - English: "A, B, and C"
/// - Chinese: "A、B、C"
/// - Japanese: "A、B、C"
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{LocalizedList, LocalizedDisplay, locales};
///
/// let items = LocalizedList(&["Apple", "Banana", "Orange"]);
///
/// assert_eq!(items.to_localized_string(&locales::EN), "Apple, Banana, and Orange");
/// assert_eq!(items.to_localized_string(&locales::ZH_CN), "Apple、Banana和Orange");
/// ```
#[derive(Debug)]
pub struct LocalizedList<'a>(pub &'a [&'a str]);

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
