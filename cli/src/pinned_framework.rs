//! Access to the framework checkout this crate lives inside.
//!
//! The framework revision is the repository's own HEAD and the "checkout" is
//! the workspace root — the standalone repository's git-pin dance is gone.
//! Tests that need real framework source — a `cargo metadata` graph over real
//! manifests, a checkout layout — read the enclosing tree.

mod clone;
pub use clone::checkout;
