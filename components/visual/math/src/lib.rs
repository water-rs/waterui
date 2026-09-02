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
//!
//! # Displaying a formula
//!
//! ```
//! use waterui_math::view::Math;
//!
//! let quadratic = Math::new(r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}")
//!     .display()
//!     .font_size(28.0);
//! ```
//!
//! # Reading one without drawing it
//!
//! Parsing and the accessibility markup need no font and no GPU, which is what
//! makes them usable from a test or a server.
//!
//! ```
//! use waterui_math::ast::MathStyle;
//! use waterui_math::{latex, mathml};
//!
//! let formula = latex::parse(r"\frac{a}{b}")?;
//! let markup = mathml::to_mathml(&formula, MathStyle::Display)?;
//!
//! assert!(markup.contains("<mfrac>"));
//! assert!(markup.contains(r#"display="block""#));
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! A construct the layout engine does not implement is refused by name rather
//! than silently dropped, so a formula never renders as a quietly wrong one.
//!
//! ```
//! use waterui_math::latex::{self, LatexError};
//!
//! let refused = latex::parse(r"\begin{matrix}a & b\\c & d\end{matrix}");
//! assert!(matches!(refused, Err(LatexError::Unsupported { .. })));
//! ```

extern crate alloc;

pub mod ast;
pub mod font;
pub mod latex;
pub mod layout;
pub mod mathml;
pub mod scene;
pub mod spacing;
pub mod view;
