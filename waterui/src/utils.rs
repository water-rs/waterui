use core::fmt::Debug;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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

pub trait HandleBorrowed<F> {
    fn handle(self, f: F) -> impl Fn() + 'static;
}

macro_rules! impl_handle_borrowed {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <F,$($ty:Clone+'static,)*>HandleBorrowed<F> for ($(&$ty),*)
        where
            F: Fn($(&$ty),*) + 'static{

            fn handle(self, f: F) -> impl Fn() + 'static {
                let ($($ty),*) = self;
                let ($($ty),*) = ($($ty.clone()),*);

                move || f($(&$ty),*)
            }
        }
    };
}

tuples!(impl_handle_borrowed);
