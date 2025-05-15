pub mod font;
use font::Font;
pub mod attributed;
pub mod link;
pub mod locale;
mod macros;
extern crate alloc;

use alloc::string::ToString;
use core::fmt::Display;
use locale::Formatter;
use uniffi::custom_type;
use waterui_core::configurable;
use waterui_core::view::ConfigurableView;
use waterui_reactive::compute::IntoCompute;
use waterui_reactive::zip::FlattenMap;
use waterui_reactive::{Compute, Computed, compute::IntoComputed};
use waterui_reactive::{ComputeExt, ffi_computed};

use waterui_core::Str;

configurable!(Text, TextConfig);

#[derive(Debug, Clone, uniffi::Record)]
#[non_exhaustive]
pub struct TextConfig {
    pub content: Computed<Str>,
    pub font: Computed<Font>,
}

ffi_computed!(Font);

impl Clone for Text {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl core::cmp::PartialEq for Text {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl core::cmp::PartialOrd for Text {
    fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
        None
    }
}

impl Default for Text {
    fn default() -> Self {
        text("")
    }
}

impl Text {
    pub fn new(content: impl IntoComputed<Str>) -> Self {
        Self(TextConfig {
            content: content.into_computed(),
            font: Computed::default(),
        })
    }

    pub fn display<T: Display>(source: impl IntoComputed<T>) -> Self {
        Self::new(source.into_compute().map(|value| value.to_string()))
    }

    pub fn format<T>(value: impl IntoComputed<T>, formatter: impl Formatter<T> + 'static) -> Self {
        Self::new(
            value
                .into_compute()
                .map(move |value| formatter.format(&value)),
        )
    }

    pub fn content(&self) -> Computed<Str> {
        self.0.content.clone()
    }

    pub fn font(mut self, font: impl Compute<Output = Font>) -> Self {
        self.0.font = font.computed();
        self
    }

    pub fn size(mut self, size: impl IntoComputed<f64>) -> Self {
        self.0.font = self
            .0
            .font
            .zip(size.into_compute())
            .flatten_map(|mut old, size| {
                old.size = size;
                old
            })
            .computed();
        self
    }
}

pub fn text(text: impl IntoComputed<Str>) -> Text {
    Text::new(text)
}

impl<T> From<T> for Text
where
    T: IntoComputed<Str>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

uniffi::setup_scaffolding!();
