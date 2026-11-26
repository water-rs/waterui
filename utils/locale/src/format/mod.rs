//! Locale-aware formatting utilities.

#[cfg(feature = "datetime")]
pub mod date;
#[cfg(feature = "number")]
pub mod number;
#[cfg(feature = "relative-time")]
pub mod relative;
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

/// Numeric types whose formatted decimal representation should be localized
/// (digit shaping and locale-specific grouping and decimal separators) when
/// interpolated into [`text!`].
#[cfg(feature = "number")]
pub trait LocalizedNumber: Display {
    /// Formats `self` as a plain ASCII decimal string suitable for locale digit
    /// and separator shaping, respecting `f.precision()`.
    ///
    /// # Errors
    ///
    /// Propagates formatting errors from `f`.
    fn fmt_decimal(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(precision) = f.precision() {
            write!(f, "{self:.precision$}")
        } else {
            Display::fmt(self, f)
        }
    }
}

#[cfg(feature = "number")]
macro_rules! impl_localized_number_primitives {
    ($($ty:ty),* $(,)?) => {
        $(impl LocalizedNumber for $ty {})*
    };
}

#[cfg(feature = "number")]
impl_localized_number_primitives!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
);

#[cfg(feature = "number")]
macro_rules! impl_localized_number_nonzero {
    ($($ty:ty),* $(,)?) => {
        $(impl LocalizedNumber for core::num::NonZero<$ty> {})*
    };
}

#[cfg(feature = "number")]
impl_localized_number_nonzero!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

#[cfg(feature = "number")]
impl<T: LocalizedNumber> LocalizedNumber for core::num::Wrapping<T> {}

#[cfg(feature = "number")]
impl<T: LocalizedNumber> LocalizedNumber for core::num::Saturating<T> {}

#[cfg(feature = "number")]
impl LocalizedNumber for fixed_decimal::Decimal {
    fn fmt_decimal(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(precision) = f.precision() {
            let position = i16::try_from(precision).map_or(i16::MIN, i16::saturating_neg);
            let mut rounded = self.clone().rounded(position);
            rounded.absolute.pad_end(position);
            Display::fmt(&rounded, f)
        } else {
            Display::fmt(self, f)
        }
    }
}

#[cfg(feature = "number")]
impl<T: LocalizedNumber + ?Sized> LocalizedNumber for &T {}

#[cfg(feature = "number")]
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct __LocalizedArgModeNumeric;

#[cfg(feature = "number")]
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct __LocalizedArgModeToPrimitiveDisplay;

#[cfg(feature = "number")]
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct __LocalizedArgModeToPrimitive;

/// A directionally isolated, locale-aware interpolation argument used by
/// WaterUI's `text!` macro.
#[cfg(feature = "number")]
#[doc(hidden)]
pub struct LocalizedArgument<'a, T: ?Sized, M = ()> {
    value: &'a T,
    locale: &'a Locale,
    _mode: core::marker::PhantomData<fn() -> M>,
}

/// `{:?}` interpolations render the wrapped value, not this adapter.
#[cfg(feature = "number")]
impl<T: fmt::Debug + ?Sized, M> fmt::Debug for LocalizedArgument<'_, T, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.value, f)
    }
}

#[cfg(feature = "number")]
impl<'a, T: ?Sized> LocalizedArgument<'a, T> {
    /// Creates an interpolation adapter formatted through [`LocalizedDisplay`].
    #[must_use]
    pub const fn new(value: &'a T, locale: &'a Locale) -> Self {
        Self {
            value,
            locale,
            _mode: core::marker::PhantomData,
        }
    }
}

#[cfg(feature = "number")]
impl<'a, T: LocalizedNumber + ?Sized> LocalizedArgument<'a, T, __LocalizedArgModeNumeric> {
    /// Creates an interpolation adapter for a [`LocalizedNumber`] value.
    #[must_use]
    pub const fn numeric(value: &'a T, locale: &'a Locale) -> Self {
        Self {
            value,
            locale,
            _mode: core::marker::PhantomData,
        }
    }
}

#[cfg(feature = "number")]
impl<'a, T: num_traits::ToPrimitive + Display + ?Sized>
    LocalizedArgument<'a, T, __LocalizedArgModeToPrimitiveDisplay>
{
    #[doc(hidden)]
    #[must_use]
    pub const fn from_to_primitive_display(value: &'a T, locale: &'a Locale) -> Self {
        Self {
            value,
            locale,
            _mode: core::marker::PhantomData,
        }
    }
}

#[cfg(feature = "number")]
impl<'a, T: num_traits::ToPrimitive + ?Sized>
    LocalizedArgument<'a, T, __LocalizedArgModeToPrimitive>
{
    #[doc(hidden)]
    #[must_use]
    pub const fn from_to_primitive(value: &'a T, locale: &'a Locale) -> Self {
        Self {
            value,
            locale,
            _mode: core::marker::PhantomData,
        }
    }
}

#[cfg(feature = "number")]
fn write_localized_numeric_str(
    number: &str,
    locale: &Locale,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let formatted = number::format_number_text(locale, number);
    if f.width().is_none() && !f.sign_plus() && !f.sign_aware_zero_pad() {
        // No width, sign or zero-padding flags: emit the localized
        // digits verbatim so a locale-specific minus sign survives.
        f.write_str(&formatted)
    } else {
        // `pad_integral` re-emits the sign and applies width, fill,
        // alignment and sign-aware zero padding exactly as `core::fmt`
        // does for numbers. `pad` cannot be used here: it interprets
        // `precision` as a maximum string length and would truncate
        // the already-rounded text.
        let digits = formatted
            .strip_prefix(|minus| matches!(minus, '-' | '\u{2212}'))
            .unwrap_or(formatted.as_str());
        f.pad_integral(!number.starts_with('-'), "", digits)
    }
}

#[cfg(feature = "number")]
fn fmt_localized_number<T: LocalizedNumber + ?Sized>(
    value: &T,
    locale: &Locale,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    struct DecimalAdapter<'a, T: ?Sized>(&'a T);
    impl<T: LocalizedNumber + ?Sized> Display for DecimalAdapter<'_, T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt_decimal(f)
        }
    }

    let adapter = DecimalAdapter(value);
    let number = f
        .precision()
        .map_or_else(|| adapter.to_string(), |p| format!("{adapter:.p$}"));
    write_localized_numeric_str(&number, locale, f)
}

#[cfg(feature = "number")]
fn fmt_to_primitive_display<T: num_traits::ToPrimitive + Display + ?Sized>(
    value: &T,
    locale: &Locale,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let raw = f
        .precision()
        .map_or_else(|| value.to_string(), |p| format!("{value:.p$}"));
    if raw.parse::<fixed_decimal::Decimal>().is_ok()
        || matches!(raw.as_str(), "NaN" | "inf" | "-inf")
    {
        write_localized_numeric_str(&raw, locale, f)
    } else {
        Display::fmt(value, f)
    }
}

#[cfg(feature = "number")]
fn fmt_to_primitive<T: num_traits::ToPrimitive + ?Sized>(
    value: &T,
    locale: &Locale,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let raw = value.to_i128().map_or_else(
        || {
            value.to_u128().map_or_else(
                || {
                    value.to_f64().map_or_else(String::new, |float| {
                        f.precision()
                            .map_or_else(|| float.to_string(), |p| format!("{float:.p$}"))
                    })
                },
                |unsigned| unsigned.to_string(),
            )
        },
        |signed| signed.to_string(),
    );
    write_localized_numeric_str(&raw, locale, f)
}

#[cfg(feature = "number")]
impl<T: LocalizedDisplay + ?Sized> Display for LocalizedArgument<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\u{2068}")?;
        self.value.fmt(self.locale, f)?;
        f.write_str("\u{2069}")
    }
}

#[cfg(feature = "number")]
impl<T: LocalizedNumber + ?Sized> Display for LocalizedArgument<'_, T, __LocalizedArgModeNumeric> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\u{2068}")?;
        fmt_localized_number(self.value, self.locale, f)?;
        f.write_str("\u{2069}")
    }
}

#[cfg(feature = "number")]
impl<T: num_traits::ToPrimitive + Display + ?Sized> Display
    for LocalizedArgument<'_, T, __LocalizedArgModeToPrimitiveDisplay>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\u{2068}")?;
        fmt_to_primitive_display(self.value, self.locale, f)?;
        f.write_str("\u{2069}")
    }
}

#[cfg(feature = "number")]
impl<T: num_traits::ToPrimitive + ?Sized> Display
    for LocalizedArgument<'_, T, __LocalizedArgModeToPrimitive>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\u{2068}")?;
        fmt_to_primitive(self.value, self.locale, f)?;
        f.write_str("\u{2069}")
    }
}

/// Probe used by `text!` autoref specialization to pick numeric localization
/// ahead of `num_traits::ToPrimitive` and generic `LocalizedDisplay`.
#[cfg(feature = "number")]
#[doc(hidden)]
pub struct __LocalizedArgProbe<'a, T: ?Sized>(pub &'a T);

#[cfg(feature = "number")]
impl<T: ?Sized> fmt::Debug for __LocalizedArgProbe<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("__LocalizedArgProbe").finish_non_exhaustive()
    }
}

#[cfg(feature = "number")]
#[doc(hidden)]
pub trait __LocalizedArgNumber<'a, T: ?Sized> {
    fn __waterui_localized_arg(
        &self,
        locale: &'a Locale,
    ) -> LocalizedArgument<'a, T, __LocalizedArgModeNumeric>;
}

#[cfg(feature = "number")]
impl<'a, T: LocalizedNumber + ?Sized> __LocalizedArgNumber<'a, T>
    for &&&__LocalizedArgProbe<'a, T>
{
    fn __waterui_localized_arg(
        &self,
        locale: &'a Locale,
    ) -> LocalizedArgument<'a, T, __LocalizedArgModeNumeric> {
        LocalizedArgument::numeric(self.0, locale)
    }
}

#[cfg(feature = "number")]
#[doc(hidden)]
pub trait __LocalizedArgToPrimitiveDisplay<'a, T: ?Sized> {
    fn __waterui_localized_arg(
        &self,
        locale: &'a Locale,
    ) -> LocalizedArgument<'a, T, __LocalizedArgModeToPrimitiveDisplay>;
}

#[cfg(feature = "number")]
impl<'a, T: num_traits::ToPrimitive + Display + ?Sized> __LocalizedArgToPrimitiveDisplay<'a, T>
    for &&__LocalizedArgProbe<'a, T>
{
    fn __waterui_localized_arg(
        &self,
        locale: &'a Locale,
    ) -> LocalizedArgument<'a, T, __LocalizedArgModeToPrimitiveDisplay> {
        LocalizedArgument::from_to_primitive_display(self.0, locale)
    }
}

#[cfg(feature = "number")]
#[doc(hidden)]
pub trait __LocalizedArgToPrimitive<'a, T: ?Sized> {
    fn __waterui_localized_arg(
        &self,
        locale: &'a Locale,
    ) -> LocalizedArgument<'a, T, __LocalizedArgModeToPrimitive>;
}

#[cfg(feature = "number")]
impl<'a, T: num_traits::ToPrimitive + ?Sized> __LocalizedArgToPrimitive<'a, T>
    for &__LocalizedArgProbe<'a, T>
{
    fn __waterui_localized_arg(
        &self,
        locale: &'a Locale,
    ) -> LocalizedArgument<'a, T, __LocalizedArgModeToPrimitive> {
        LocalizedArgument::from_to_primitive(self.0, locale)
    }
}

#[cfg(feature = "number")]
#[doc(hidden)]
pub trait __LocalizedArgFallback<'a, T: ?Sized> {
    fn __waterui_localized_arg(&self, locale: &'a Locale) -> LocalizedArgument<'a, T>;
}

#[cfg(feature = "number")]
impl<'a, T: ?Sized> __LocalizedArgFallback<'a, T> for __LocalizedArgProbe<'a, T> {
    fn __waterui_localized_arg(&self, locale: &'a Locale) -> LocalizedArgument<'a, T> {
        LocalizedArgument::new(self.0, locale)
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
    use core::num::{NonZeroU32, Saturating, Wrapping};

    use fixed_decimal::Decimal;
    use num_traits::ToPrimitive;

    use super::*;
    use crate::locale::locales;

    macro_rules! dispatch_arg {
        ($value:expr, $locale:expr) => {
            (&&&&__LocalizedArgProbe($value)).__waterui_localized_arg($locale)
        };
    }

    #[test]
    fn localized_argument_formats_numbers_and_adds_bidi_isolation() {
        let value = 1234.5_f64;
        let formatted = format!("{}", dispatch_arg!(&value, &locales::DE));

        assert!(formatted.starts_with('\u{2068}'));
        assert!(formatted.ends_with('\u{2069}'));
        assert!(formatted.contains("1.234,5"));
    }

    #[test]
    fn localized_argument_shows_non_finite_floats_as_rust_prints_them() {
        assert_eq!(
            dispatch_arg!(&f64::NAN, &locales::EN).to_string(),
            "\u{2068}NaN\u{2069}"
        );
        assert_eq!(
            dispatch_arg!(&f64::INFINITY, &locales::EN).to_string(),
            "\u{2068}inf\u{2069}"
        );
        assert_eq!(
            dispatch_arg!(&f64::NEG_INFINITY, &locales::EN).to_string(),
            "\u{2068}-inf\u{2069}"
        );
    }

    #[test]
    fn localized_argument_accepts_non_static_borrowed_text() {
        let owned = String::from("مرحبا");
        let borrowed = owned.as_str();

        assert_eq!(
            dispatch_arg!(&borrowed, &locales::AR).to_string(),
            "\u{2068}مرحبا\u{2069}"
        );
    }

    #[test]
    fn localized_argument_applies_width_and_zero_fill_to_integers() {
        assert_eq!(
            format!("{:06}", dispatch_arg!(&0_u32, &locales::EN)),
            "\u{2068}000000\u{2069}"
        );
        assert_eq!(
            format!("{:06}", dispatch_arg!(&-42_i32, &locales::EN)),
            "\u{2068}-00042\u{2069}"
        );
        assert_eq!(
            format!("{:>8}", dispatch_arg!(&1234_i64, &locales::EN)),
            "\u{2068}   1,234\u{2069}"
        );
    }

    #[test]
    fn localized_argument_applies_width_to_string_arguments() {
        let value = "hi";
        assert_eq!(
            format!("{:>8}", dispatch_arg!(&value, &locales::EN)),
            "\u{2068}      hi\u{2069}"
        );
    }

    #[test]
    fn localized_argument_applies_precision_and_width_to_floats() {
        let value = 1234.567_f64;
        assert_eq!(
            format!("{:.2}", dispatch_arg!(&value, &locales::EN)),
            "\u{2068}1,234.57\u{2069}"
        );
        assert_eq!(
            format!("{:>10.2}", dispatch_arg!(&1234.5_f64, &locales::EN)),
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
            format!("{:?}", dispatch_arg!(&Fruit::Apple, &locales::EN)),
            "Apple"
        );
        assert_eq!(
            format!("{:#?}", dispatch_arg!(&Fruit::Apple, &locales::EN)),
            "Apple"
        );
    }

    #[test]
    fn autoref_dispatch_formats_nonzero_wrapping_saturating_decimal_and_toprimitive() {
        struct CustomDecimal(&'static str, f64);
        impl Display for CustomDecimal {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0)
            }
        }
        impl ToPrimitive for CustomDecimal {
            fn to_i64(&self) -> Option<i64> {
                None
            }
            fn to_u64(&self) -> Option<u64> {
                None
            }
            fn to_f64(&self) -> Option<f64> {
                Some(self.1)
            }
        }

        struct BareToPrimitive(u32);
        impl ToPrimitive for BareToPrimitive {
            fn to_i64(&self) -> Option<i64> {
                Some(i64::from(self.0))
            }
            fn to_u64(&self) -> Option<u64> {
                Some(u64::from(self.0))
            }
        }

        let ar_eg: Locale = "ar-EG".parse().unwrap();
        let nonzero = NonZeroU32::new(1234).unwrap();
        let wrapping = Wrapping(1234_u32);
        let saturating = Saturating(1234_u32);
        let decimal: Decimal = "1234.56".parse().unwrap();
        let custom_dec = CustomDecimal("1234.56", 1234.56);
        let bare = BareToPrimitive(1234);

        assert_eq!(
            format!("{:.1}", dispatch_arg!(&decimal, &locales::EN_US)),
            "\u{2068}1,234.6\u{2069}"
        );
        assert_eq!(
            format!("{:.3}", dispatch_arg!(&decimal, &locales::EN_US)),
            "\u{2068}1,234.560\u{2069}"
        );

        for locale in [
            ar_eg,
            locales::DE,
            locales::EN_US,
            locales::ZH_CN,
            locales::JA,
            locales::KO,
        ] {
            let expected_int = dispatch_arg!(&1234_u32, &locale).to_string();
            assert_eq!(dispatch_arg!(&nonzero, &locale).to_string(), expected_int);
            assert_eq!(dispatch_arg!(&wrapping, &locale).to_string(), expected_int);
            assert_eq!(
                dispatch_arg!(&saturating, &locale).to_string(),
                expected_int
            );
            assert_eq!(dispatch_arg!(&bare, &locale).to_string(), expected_int);

            let expected_float = dispatch_arg!(&1234.56_f64, &locale).to_string();
            assert_eq!(dispatch_arg!(&decimal, &locale).to_string(), expected_float);
            assert_eq!(
                dispatch_arg!(&custom_dec, &locale).to_string(),
                expected_float
            );
        }
    }
}
