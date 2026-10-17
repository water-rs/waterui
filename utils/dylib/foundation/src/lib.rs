//! Foundation layer of the layered `waterui` dylib chain.
//!
//! Crates reached by two or more dylib images that share a link are embedded
//! here, at or below their lowest common layer; every image above this one
//! resolves them through `waterui_dylib_foundation.dll` instead of embedding a
//! second copy. See the companion rule in `utils/dylib/foundation/Cargo.toml`.

pub use executor_core;
pub use lazy_static;
pub use nami;
pub use native_executor;
pub use pastey;
pub use quick_xml;
pub use regex;
pub use waterui_core;
pub use waterui_locale;
pub use waterui_url;
pub use waterui_watcher_set;
