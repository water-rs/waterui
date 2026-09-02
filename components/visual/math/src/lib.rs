//! Mathematical formula rendering for `WaterUI`.
//!
//! A formula is parsed into a semantic tree ([`ast`]), laid out against the
//! chosen face's OpenType `MATH` table ([`font`], [`layout`]), and drawn
//! through the engine-independent `Scene2D` contract ([`scene`]) — so it
//! renders on whichever engine the backend supplies, including the CPU/GPU
//! split engine that adapters without compute shaders fall to.
//!
//! The semantic tree is kept rather than discarded after layout, because it is
//! also the accessibility representation: a formula drawn as anonymous vector
//! paths is unreadable to a screen reader, and the tree is what `MathML` is
//! published from.

extern crate alloc;

pub mod ast;
pub mod font;
pub mod latex;
pub mod layout;
pub mod mathml;
pub mod scene;
pub mod spacing;
pub mod view;
