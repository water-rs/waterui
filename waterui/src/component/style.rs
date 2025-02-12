use crate::color::Color;

#[derive(Debug, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub offset: Vector<f32>,
    pub radius: f32,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Vector<T> {
    pub x: T,
    pub y: T,
}
