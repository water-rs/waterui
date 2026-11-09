//! Type-safe unit wrappers with automatic conversion and localized display.

use core::fmt;
use core::marker::PhantomData;
use core::ops::Add;

use crate::format::LocalizedDisplay;
use crate::format::number::format_number;
use crate::locale::Locale;

// =====================================================
// Length Units
// =====================================================

/// Trait for length unit types.
pub trait LengthUnit: Copy + Clone {
    /// Conversion factor: how many meters per one unit.
    const METERS_PER_UNIT: f64;

    /// Get the localized unit symbol.
    fn symbol(locale: &Locale) -> &'static str;
}

/// Meter unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Meter;

/// Feet unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Feet;

/// Kilometer unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Kilometer;

/// Mile unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mile;

impl LengthUnit for Meter {
    const METERS_PER_UNIT: f64 = 1.0;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "米",
            "ja" => "メートル",
            "ko" => "미터",
            _ => "m",
        }
    }
}

impl LengthUnit for Feet {
    const METERS_PER_UNIT: f64 = 0.3048;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "英尺",
            "ja" => "フィート",
            "ko" => "피트",
            _ => "ft",
        }
    }
}

impl LengthUnit for Kilometer {
    const METERS_PER_UNIT: f64 = 1000.0;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "公里",
            "ja" => "キロメートル",
            "ko" => "킬로미터",
            _ => "km",
        }
    }
}

impl LengthUnit for Mile {
    const METERS_PER_UNIT: f64 = 1609.344;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "英里",
            "ja" => "マイル",
            "ko" => "마일",
            _ => "mi",
        }
    }
}

/// Type-safe length wrapper with automatic conversion.
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{Kilometer, Length, Meter, Mile};
///
/// // Create lengths
/// let distance = Length::<Kilometer>::new(5.0);
/// let extra = Length::<Meter>::new(500.0);
///
/// // Add different units seamlessly (result in left-hand unit)
/// let total = distance + extra; // 5.5 km
/// assert_eq!(total.value(), 5.5);
///
/// // Convert between units
/// let in_miles = distance.to::<Mile>(); // ~3.1 miles
/// assert!((in_miles.value() - 3.107).abs() < 0.001);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Length<U: LengthUnit> {
    value: f64,
    _unit: PhantomData<U>,
}

impl<U: LengthUnit> Length<U> {
    /// Create a new length value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self {
            value,
            _unit: PhantomData,
        }
    }

    /// Get the raw value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// Convert to meters.
    #[must_use]
    pub fn as_meters(&self) -> f64 {
        self.value * U::METERS_PER_UNIT
    }

    /// Convert to a different unit.
    #[must_use]
    pub fn to<T: LengthUnit>(&self) -> Length<T> {
        let meters = self.as_meters();
        Length::new(meters / T::METERS_PER_UNIT)
    }
}

impl<U: LengthUnit> Default for Length<U> {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Add different length units seamlessly.
/// Result is in the left-hand unit.
impl<U: LengthUnit, V: LengthUnit> Add<Length<V>> for Length<U> {
    type Output = Self;

    fn add(self, rhs: Length<V>) -> Self::Output {
        Self::new(self.value + rhs.to::<U>().value)
    }
}

/// Localized display for Length.
impl<U: LengthUnit> LocalizedDisplay for Length<U> {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formatted_num = format_number(locale, self.value);
        let symbol = U::symbol(locale);
        write!(f, "{formatted_num}{symbol}")
    }
}

// =====================================================
// Temperature Units
// =====================================================

/// Trait for temperature unit types.
pub trait TemperatureUnit: Copy + Clone {
    /// Convert from Kelvin to this unit.
    fn from_kelvin(k: f64) -> f64;

    /// Convert to Kelvin from this unit.
    fn to_kelvin(value: f64) -> f64;

    /// Get the unit symbol.
    fn symbol() -> &'static str;
}

/// Celsius unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Celsius;

/// Fahrenheit unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fahrenheit;

/// Kelvin unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Kelvin;

impl TemperatureUnit for Celsius {
    fn from_kelvin(k: f64) -> f64 {
        k - 273.15
    }

    fn to_kelvin(value: f64) -> f64 {
        value + 273.15
    }

    fn symbol() -> &'static str {
        "°C"
    }
}

impl TemperatureUnit for Fahrenheit {
    fn from_kelvin(k: f64) -> f64 {
        (k - 273.15) * 9.0 / 5.0 + 32.0
    }

    fn to_kelvin(value: f64) -> f64 {
        (value - 32.0) * 5.0 / 9.0 + 273.15
    }

    fn symbol() -> &'static str {
        "°F"
    }
}

impl TemperatureUnit for Kelvin {
    fn from_kelvin(k: f64) -> f64 {
        k
    }

    fn to_kelvin(value: f64) -> f64 {
        value
    }

    fn symbol() -> &'static str {
        "K"
    }
}

/// Type-safe temperature wrapper with automatic conversion.
#[derive(Debug, Clone, Copy)]
pub struct Temperature<U: TemperatureUnit> {
    value: f64,
    _unit: PhantomData<U>,
}

impl<U: TemperatureUnit> Temperature<U> {
    /// Create a new temperature value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self {
            value,
            _unit: PhantomData,
        }
    }

    /// Get the raw value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// Convert to Kelvin.
    #[must_use]
    pub fn as_kelvin(&self) -> f64 {
        U::to_kelvin(self.value)
    }

    /// Convert to a different unit.
    #[must_use]
    pub fn to<T: TemperatureUnit>(&self) -> Temperature<T> {
        let kelvin = self.as_kelvin();
        Temperature::new(T::from_kelvin(kelvin))
    }
}

impl<U: TemperatureUnit> Default for Temperature<U> {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Localized display for Temperature.
impl<U: TemperatureUnit> LocalizedDisplay for Temperature<U> {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formatted_num = format_number(locale, self.value);
        let symbol = U::symbol();
        write!(f, "{formatted_num}{symbol}")
    }
}

// =====================================================
// Mass Units
// =====================================================

/// Trait for mass unit types.
pub trait MassUnit: Copy + Clone {
    /// Conversion factor: how many kilograms per one unit.
    const KILOGRAMS_PER_UNIT: f64;

    /// Get the localized unit symbol.
    fn symbol(locale: &Locale) -> &'static str;
}

/// Kilogram unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Kilogram;

/// Gram unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gram;

/// Pound unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pound;

/// Ounce unit marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ounce;

impl MassUnit for Kilogram {
    const KILOGRAMS_PER_UNIT: f64 = 1.0;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "公斤",
            "ja" => "キログラム",
            "ko" => "킬로그램",
            _ => "kg",
        }
    }
}

impl MassUnit for Gram {
    const KILOGRAMS_PER_UNIT: f64 = 0.001;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "克",
            "ja" => "グラム",
            "ko" => "그램",
            _ => "g",
        }
    }
}

impl MassUnit for Pound {
    const KILOGRAMS_PER_UNIT: f64 = 0.453_592_37;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "磅",
            "ja" => "ポンド",
            "ko" => "파운드",
            _ => "lb",
        }
    }
}

impl MassUnit for Ounce {
    const KILOGRAMS_PER_UNIT: f64 = 0.028_349_523_125;

    fn symbol(locale: &Locale) -> &'static str {
        match locale.language.as_str() {
            "zh" => "盎司",
            "ja" => "オンス",
            "ko" => "온스",
            _ => "oz",
        }
    }
}

/// Type-safe mass wrapper with automatic conversion.
#[derive(Debug, Clone, Copy)]
pub struct Mass<U: MassUnit> {
    value: f64,
    _unit: PhantomData<U>,
}

impl<U: MassUnit> Mass<U> {
    /// Create a new mass value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self {
            value,
            _unit: PhantomData,
        }
    }

    /// Get the raw value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// Convert to kilograms.
    #[must_use]
    pub fn as_kilograms(&self) -> f64 {
        self.value * U::KILOGRAMS_PER_UNIT
    }

    /// Convert to a different unit.
    #[must_use]
    pub fn to<T: MassUnit>(&self) -> Mass<T> {
        let kilograms = self.as_kilograms();
        Mass::new(kilograms / T::KILOGRAMS_PER_UNIT)
    }
}

impl<U: MassUnit> Default for Mass<U> {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Add different mass units seamlessly.
impl<U: MassUnit, V: MassUnit> Add<Mass<V>> for Mass<U> {
    type Output = Self;

    fn add(self, rhs: Mass<V>) -> Self::Output {
        Self::new(self.value + rhs.to::<U>().value)
    }
}

/// Localized display for Mass.
impl<U: MassUnit> LocalizedDisplay for Mass<U> {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formatted_num = format_number(locale, self.value);
        let symbol = U::symbol(locale);
        write!(f, "{formatted_num}{symbol}")
    }
}
