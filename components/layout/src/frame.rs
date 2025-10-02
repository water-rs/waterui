//! Placeholder for fixed-size frame layouts.
//!
//! A future iteration will add a public `Frame` view capable of overriding a
//! child's incoming proposal. The struct below documents the intent so that
//! renderers and component authors have a reference point.

use crate::Layout;

/// Planned layout that clamps a single child's proposal.
#[allow(dead_code)]
struct FrameLayout {}
