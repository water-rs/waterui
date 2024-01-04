use async_executor::Task;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::future::Future;
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[repr(C)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: f64,
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

    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum Background {
    Default,
    Color(Color),
}

impl_from!(Background, Color);

impl Default for Background {
    fn default() -> Self {
        Self::Default
    }
}

pub fn task<Fut>(fut: Fut) -> Task<Fut::Output>
where
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    smol::spawn(fut)
}
