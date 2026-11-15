//! Build-time protocol generation from the runtime's Rust definitions.
macro_rules! code_metadata {
    ($($item:item)*) => { $($item)* };
}
#[macro_use]
#[path = "../src/hwui/codes.rs"]
mod codes;
#[path = "../src/hwui/kotlin.rs"]
pub mod kotlin;
#[path = "../src/hwui/protocol.rs"]
pub mod protocol;
#[path = "../src/hwui/text/constants.rs"]
pub mod text_constants;
/// Text constants used by the shared generator.
pub mod text {
    pub use super::text_constants as constants;
}
