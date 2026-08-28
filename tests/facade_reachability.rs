//! Every public capability must be reachable from a crate that depends only on
//! `waterui`.
//!
//! These are compile-time assertions: an app author should never have to add a
//! direct `waterui-core` (or `filtrate`, or `wgpu`) dependency, nor reach
//! through an inconsistent long path, to use a documented feature. Each `use`
//! below stands for one gap that used to force exactly that.

#![allow(unused_imports, reason = "these imports are the assertion")]

// Tabs sit beside the split-view navigation types instead of behind
// `navigation::tab::…`.
use waterui::navigation::{
    NavigationSplitView, Tab, Tabs, navigation_transition, split_style, tab_style,
};

// The `slider` free function joins `button`/`toggle`/`stepper`/`field`/`label`
// at the controls root.
use waterui::component::{button, field, label, slider, stepper, toggle};

// Custom extractors no longer require `impl_extractor!` as the only route.
use waterui::extract::{Extractor, State, Use};

// Implementing a custom resolvable theme token.
use waterui::resolve::{AnyResolvable, Resolvable};

// View metadata and lifecycle events.
use waterui::event::Event;
use waterui::metadata::{IgnorableMetadata, Metadata, MetadataKey, Retain};

// Implementing `Animatable` for a custom data shape.
use waterui::easing::{EasingCurve, Interpolatable};

// The explicit-intensity haptic modifiers, not just the `_default` variants.
#[cfg(feature = "std")]
use waterui::Intensity;

// `.filter(F)` requires `F: Effect`; the whole family is at the graphics root
// beside the `ViewEffect` family it mirrors.
#[cfg(feature = "gpu")]
use waterui::graphics::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRenderResult, EffectSetupResult,
    ViewEffect, ViewEffectContext, ViewEffectInput, ViewEffectOutput,
};

// Apps implementing `GpuView`/`Effect` must be able to use the same `wgpu` this
// build links, rather than hand-adding a version that may not match.
#[cfg(feature = "gpu")]
use waterui::graphics::wgpu;

#[test]
fn public_capabilities_are_reachable_through_the_facade() {
    // Reaching this point means every import above resolved.
    let _ = Metadata::new(waterui::component::text("water"), Retain::new(0_u8));
}
