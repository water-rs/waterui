extern crate alloc;

pub mod text_field;
pub mod toggle;
#[doc(inline)]
pub use text_field::{TextField, field};
#[doc(inline)]
pub use toggle::{Toggle, toggle};
pub mod slider;
pub mod stepper;

#[doc(inline)]
pub use stepper::{Stepper, stepper};

uniffi::setup_scaffolding!();
