use serde::{Deserialize, Serialize};
use std::{fmt::Debug, future::Future};
use url::Url;
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: f64,
}

pub fn task(_fut: impl Future) {
    todo!()
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENCY: Self = Self::rgba(0, 0, 0, 0.0);
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 1.0)
    }

    pub const fn rgba(red: u8, green: u8, blue: u8, opacity: f64) -> Self {
        Self {
            red,
            green,
            blue,
            opacity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum Background {
    Default,
    Image(Url),
    Color(Color),
}

impl_from!(Background, Color);
impl_from!(Background, Url, Image);

impl Default for Background {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub enum Resource {
    URL(Url),
    Data(Vec<u8>),
}

impl_from!(Resource, Url, URL);
impl_from!(Resource, Vec<u8>, Data);

impl Resource {
    pub fn url<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: Debug,
    {
        Self::URL(url.try_into().unwrap())
    }
}
