//! `NSAnimationContext` groups for the navigation stack's cross-fade.
//!
//! # Safety
//!
//! The `unsafe` here calls `objc2`/`AppKit` bindings marked unsafe —
//! `runAnimationGroup:completionHandler:` retains the blocks and the
//! `animator` proxy — all main-thread calls guaranteed by
//! [`MainThreadMarker`] on every [`crate::appkit`] entry point.

use std::rc::Rc;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2_app_kit::{NSAnimationContext, NSView};

/// Runs `changes` inside an `NSAnimationContext` group and calls
/// `completion` when the animations settle — the cross-fade
/// a navigation container performs between two pages.
pub fn run_animation(changes: Rc<dyn Fn()>, completion: Rc<dyn Fn()>) {
    NSAnimationContext::runAnimationGroup_completionHandler(
        &RcBlock::new(move |_context| changes()),
        Some(&RcBlock::new(move || completion())),
    );
}

/// Sets `view`'s alpha through its animation proxy — inside
/// [`run_animation`] the change animates.
pub fn set_animated_alpha(view: &NSView, alpha: f64) {
    // SAFETY: `animator` returns the view's `NSAnimatablePropertyContainer`
    // proxy; setting `alphaValue` through it animates inside an active group.
    let proxy: Retained<NSView> = unsafe { msg_send![view, animator] };
    proxy.setAlphaValue(alpha);
}
