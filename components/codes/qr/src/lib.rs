//! QR code rendering for `WaterUI`.
//!
//! No platform ships a QR primitive — a code is a picture everywhere — so this
//! is a self-drawn component: the payload is encoded into a module grid, and
//! the grid is drawn through the engine-independent `Scene2D` contract, which
//! means it renders on whichever engine the backend supplies and stays vector
//! sharp at any size instead of being rasterized once into a bitmap.
//!
//! Two things separate this from drawing squares. Module edges are snapped to
//! whole units of the box the code is drawn in, because a boundary that falls
//! inside a pixel is antialiased into a seam a decoder's binarizer has to guess
//! at; and the default colours preserve a code's polarity across the colour
//! scheme, because a light-on-dark code is one most detectors will not find at
//! all. Both are explained where they happen, in [`view`].
//!
//! # Showing a code
//!
//! ```
//! use waterui_qr::qr_code;
//!
//! let ticket = qr_code("https://waterui.dev");
//! ```
//!
//! The payload takes a signal, so a code bound to state re-encodes and redraws
//! without its subtree being rebuilt:
//!
//! ```
//! use nami::Binding;
//! use waterui_qr::qr_code;
//! use waterui_str::Str;
//!
//! let link = Binding::container(Str::from_static("https://waterui.dev"));
//! let code = qr_code(link.clone());
//!
//! link.set(Str::from_static("https://waterui.dev/docs"));
//! ```
//!
//! Style is an attribute rather than another type — the correction level, the
//! quiet zone and the two colours are set on the code:
//!
//! ```
//! use waterui_graphics::color::Color;
//! use waterui_qr::{ErrorCorrection, qr_code};
//!
//! let code = qr_code("https://waterui.dev")
//!     .correction(ErrorCorrection::High)
//!     .module_size(6.0)
//!     .module_color(Color::srgb(13, 26, 51));
//! ```
//!
//! # Encoding without drawing
//!
//! Encoding needs no GPU and no environment, which is what makes it usable from
//! a test or a server:
//!
//! ```
//! use waterui_qr::{ErrorCorrection, QrMatrix};
//!
//! let matrix = QrMatrix::encode("https://waterui.dev", ErrorCorrection::Medium)?;
//! assert!(matrix.is_dark(0, 0), "every symbol opens with a finder pattern");
//! # Ok::<(), waterui_qr::QrError>(())
//! ```

extern crate alloc;

pub mod matrix;
pub mod view;

pub use matrix::{ErrorCorrection, QrError, QrMatrix};
pub use view::{DEFAULT_MODULE_SIZE, DEFAULT_QUIET_ZONE, QrCode, QrContent, qr_code};
