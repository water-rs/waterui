//! The keyboard region: what a `UIKit` keyboard notification carries.
//!
//! `UIKit` posts `UIKeyboardWillShowNotification`,
//! `UIKeyboardWillChangeFrameNotification` and
//! `UIKeyboardWillHideNotification` with a `userInfo` holding the frame the
//! keyboard ends at — in screen coordinates — and the duration and curve of
//! the animation `UIKit` plays to get there. A consumer that mirrors the
//! keyboard's motion applies its own change inside a `UIView` animation with
//! the same parameters.
//!
//! # Safety
//!
//! The `unsafe` here reads `userInfo` by messaging the dictionary directly —
//! the keys are constants `UIKit` exports — and unpacks the `NSValue` the
//! frame key names into a `CGRect`, a copy `NSValue` documents as typed.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSNotification, NSNumber, NSString, NSValue};
use objc2_ui_kit::{
    UIKeyboardAnimationCurveUserInfoKey, UIKeyboardAnimationDurationUserInfoKey,
    UIKeyboardFrameEndUserInfoKey,
};

/// The frame and animation a keyboard notification reports.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardChange {
    /// The keyboard's end frame in screen coordinates.
    pub frame: CGRect,
    /// The animation's duration in seconds.
    pub duration: f64,
    /// The `UIViewAnimationCurve` raw value; `curve << 16` is the matching
    /// `UIViewAnimationOptions` curve option.
    pub curve: usize,
}

/// One `userInfo` lookup: the dictionary's value for `key`, `None` when the
/// key is absent.
fn user_info(note: &NSNotification, key: &'static NSString) -> Option<Retained<AnyObject>> {
    let info = note.userInfo()?;
    // SAFETY: `userInfo` is the dictionary `UIKit` filled for the
    // notification; `key` is one of the constants it exports.
    unsafe { msg_send![&*info, objectForKey: key] }
}

/// The change `notification` reports, or `None` when it is not a keyboard
/// notification (a `userInfo` without a frame end).
pub fn change(notification: &NSNotification) -> Option<KeyboardChange> {
    // SAFETY: the frame-end value is an `NSValue` wrapping a `CGRect`, the
    // duration and curve values `NSNumber`s — the types `UIKit` documents
    // for each key.
    let frame: CGRect = unsafe {
        user_info(notification, UIKeyboardFrameEndUserInfoKey)?
            .downcast_ref::<NSValue>()?
            .get()
    };
    // SAFETY: `UIKit` exports the duration and curve keys as `NSString`
    // constants.
    let duration = user_info(notification, unsafe {
        UIKeyboardAnimationDurationUserInfoKey
    })
    .and_then(|value| value.downcast_ref::<NSNumber>().map(NSNumber::doubleValue))
    .unwrap_or(0.0);
    // SAFETY: same constants, exported by `UIKit`.
    let curve = user_info(notification, unsafe { UIKeyboardAnimationCurveUserInfoKey })
        .and_then(|value| value.downcast_ref::<NSNumber>().map(NSNumber::as_usize))
        .unwrap_or(0);
    Some(KeyboardChange {
        frame,
        duration,
        curve,
    })
}
