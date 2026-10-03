//! The `UIKit` text field: a `UITextField` driving single-line text input.
//!
//! The control carries no chrome: `new` leaves `UITextField`'s border
//! style at its default `None` — no border, no fill. User edits surface through `install_change_handler`
//! (`editingChanged`) and Return through `install_submit_handler`
//! (`editingDidEndOnExit`).
//!
//! # Safety
//!
//! The `unsafe` here defines a `UITextField` subclass and calls `objc2`
//! bindings marked unsafe because `UIKit` control APIs are main-thread
//! only — which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::fmt;

use objc2::ffi;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::{CGFloat, CGRect, CGSize};
use objc2_foundation::{NSAttributedString, NSObjectProtocol};
use objc2_ui_kit::{
    NSTextAlignment, UIKeyboardType, UITextAutocapitalizationType, UITextAutocorrectionType,
    UITextBorderStyle, UITextContentTypePassword, UITextField,
};

use crate::ActionTarget;
use crate::action::ControlEvents;
use crate::geometry::Size;

/// The on-screen keyboard a [`TextField`] requests while editing.
///
/// Maps onto `UIKeyboardType`; platforms without a software keyboard never
/// see this hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Keyboard {
    /// General text input.
    #[default]
    Text,
    /// Email input — `@` and `.` on the primary layout.
    Email,
    /// URL input.
    Url,
    /// Numeric input.
    Number,
    /// Phone-number input.
    PhoneNumber,
}

impl Keyboard {
    /// The `UIKeyboardType` the kind maps onto.
    const fn native(self) -> UIKeyboardType {
        match self {
            Self::Text => UIKeyboardType::Default,
            Self::Email => UIKeyboardType::EmailAddress,
            Self::Url => UIKeyboardType::URL,
            Self::Number => UIKeyboardType::NumberPad,
            Self::PhoneNumber => UIKeyboardType::PhonePad,
        }
    }
}

/// A single-line `UITextField` used as a text-input leaf.
pub struct TextFieldIvars {}

impl fmt::Debug for TextFieldIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextFieldIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UITextField`'s designated initializer is `initWithFrame:`,
    // which `TextField::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UITextField))]
    #[name = "CocoaUiTextField"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextFieldIvars]
    #[derive(Debug)]
    /// A `UITextField` with installable change and submit actions.
    pub struct TextField;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITextField` subclass.
    unsafe impl NSObjectProtocol for TextField {}
);

impl TextField {
    /// An empty, chromeless, enabled single-line field.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TextFieldIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setBorderStyle(UITextBorderStyle::None);
        this
    }

    /// The committed plain text the field currently shows.
    #[must_use]
    pub fn string(&self) -> String {
        self.text()
            .map_or_else(String::new, |text| text.to_string())
    }

    /// Writes `text` into the field. Programmatic writes do not fire the
    /// change action, so an external update cannot echo back as an edit.
    pub fn set_attributed_text(&self, text: &NSAttributedString) {
        self.setAttributedText(Some(text));
    }

    /// The placeholder shown while the field is empty.
    pub fn set_placeholder(&self, placeholder: &NSAttributedString) {
        self.setAttributedPlaceholder(Some(placeholder));
    }

    /// The on-screen keyboard the field requests while editing.
    pub fn set_keyboard(&self, keyboard: Keyboard) {
        // SAFETY: `setKeyboardType:` is a `UITextInputTraits` setter on
        // `UITextField` — see `send_trait_setter`.
        unsafe {
            send_trait_setter(self, sel!(setKeyboardType:), keyboard.native());
        }
    }

    /// How the field aligns its text — prompt alignment applies to both the
    /// placeholder and the entered text on `UIKit`.
    pub fn set_text_alignment(&self, alignment: NSTextAlignment) {
        self.setTextAlignment(alignment);
    }

    /// Whether the field responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// The height the input wants at `width` — the control's `sizeThatFits:`
    /// answer floored at its intrinsic height.
    #[must_use]
    pub fn measured_height(&self, width: f64) -> f64 {
        let fitting = self.sizeThatFits(CGSize {
            width: width as CGFloat,
            height: CGFloat::MAX,
        });
        let intrinsic = self.intrinsicContentSize();
        fitting.height.max(intrinsic.height)
    }

    /// Calls `handler` with the field each time the user edits it. The
    /// returned target must be kept for as long as the control should
    /// respond; dropping it detaches the target.
    pub fn install_change_handler(&self, handler: impl Fn(&Self) + 'static) -> ActionTarget {
        let this = Weak::new(self);
        ActionTarget::new(self, ControlEvents::EDITING_CHANGED, move |_mtm| {
            if let Some(field) = this.load() {
                handler(&field);
            }
        })
    }

    /// Calls `handler` when the user submits with Return — the
    /// `editingDidEndOnExit` event. The returned target must be kept for as
    /// long as the control should respond.
    pub fn install_submit_handler(&self, handler: impl Fn(&Self) + 'static) -> ActionTarget {
        let this = Weak::new(self);
        ActionTarget::new(self, ControlEvents::EDITING_DID_END_ON_EXIT, move |_mtm| {
            if let Some(field) = this.load() {
                handler(&field);
            }
        })
    }
}

/// Sends a `UITextInputTraits` property setter through a raw
/// `objc_msgSend`.
///
/// `UIKit` resolves those accessors dynamically — on iOS 26 they never
/// enter `UITextField`'s method list, so the `class_getInstanceMethod`
/// lookup in `objc2`'s debug message-send verification reports
/// `method not found` on a call `UIKit` answers. A raw send performs the
/// same message without the check.
///
/// # Safety
/// `sel` must name a `UITextInputTraits` property setter that `field`'s
/// class dynamically resolves, taking one argument of type `A` and
/// returning void.
unsafe fn send_trait_setter<A>(field: &UITextField, sel: Sel, arg: A) {
    // SAFETY: `objc_msgSend` is the ABI entry point every `msg_send!`
    // resolves to; the caller pins the signature to `sel`'s declaration.
    let msg: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, A) =
        unsafe { std::mem::transmute(ffi::objc_msgSend as *const ()) };
    // SAFETY: upheld by the function's safety contract — `sel` is a
    // setter `field` resolves dynamically and `arg` matches its type.
    unsafe {
        msg(
            std::ptr::from_ref::<UITextField>(field)
                .cast::<AnyObject>()
                .cast_mut(),
            sel,
            arg,
        );
    }
}

/// The field's intrinsic size as a kit [`Size`], for measures that report
/// the control's share of a layout pass.
#[must_use]
pub fn intrinsic_size(field: &TextField) -> Size {
    field.intrinsicContentSize().into()
}

define_class!(
    // SAFETY: `UITextField`'s designated initializer is `initWithFrame:`,
    // which `SecureField::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UITextField))]
    #[name = "CocoaUiSecureField"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextFieldIvars]
    #[derive(Debug)]
    /// A `UITextField` masking its input — the secure twin of [`TextField`],
    /// with the same installable change and submit actions.
    pub struct SecureField;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITextField` subclass.
    unsafe impl NSObjectProtocol for SecureField {}
);

impl SecureField {
    /// An empty, enabled secure field: masked entry, the password content
    /// type, no autocorrection or autocapitalization — the configuration a
    /// password field needs, drawn with `UIKit`'s rounded-rect border.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TextFieldIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setBorderStyle(UITextBorderStyle::RoundedRect);
        // SAFETY: every call below is a `UITextInputTraits` setter on
        // `UITextField` — see `send_trait_setter`. `UITextContentTypePassword`
        // is a framework static, so reading it cannot alias.
        unsafe {
            send_trait_setter(&this, sel!(setSecureTextEntry:), true);
            send_trait_setter(
                &this,
                sel!(setTextContentType:),
                Some(UITextContentTypePassword),
            );
            send_trait_setter(
                &this,
                sel!(setAutocorrectionType:),
                UITextAutocorrectionType::No,
            );
            send_trait_setter(
                &this,
                sel!(setAutocapitalizationType:),
                UITextAutocapitalizationType::None,
            );
        }
        this
    }

    /// The committed plain text the field currently shows.
    #[must_use]
    pub fn string(&self) -> String {
        self.text()
            .map_or_else(String::new, |text| text.to_string())
    }

    /// Writes `text` into the field. Programmatic writes do not fire the
    /// change action, so an external update cannot echo back as an edit.
    pub fn set_string(&self, text: &str) {
        self.setText(Some(&objc2_foundation::NSString::from_str(text)));
    }

    /// The placeholder shown while the field is empty.
    pub fn set_placeholder(&self, placeholder: &NSAttributedString) {
        self.setAttributedPlaceholder(Some(placeholder));
    }

    /// How the field aligns its text.
    pub fn set_text_alignment(&self, alignment: NSTextAlignment) {
        self.setTextAlignment(alignment);
    }

    /// The font the field draws with.
    pub fn set_font(&self, font: &objc2_ui_kit::UIFont) {
        self.setFont(Some(font));
    }

    /// Whether the field responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// The height the input wants at `width` — see [`TextField::measured_height`].
    #[must_use]
    pub fn measured_height(&self, width: f64) -> f64 {
        let fitting = self.sizeThatFits(CGSize {
            width: width as CGFloat,
            height: CGFloat::MAX,
        });
        let intrinsic = self.intrinsicContentSize();
        fitting.height.max(intrinsic.height)
    }

    /// Calls `handler` with the field each time the user edits it. The
    /// returned target must be kept for as long as the control should
    /// respond; dropping it detaches the target.
    pub fn install_change_handler(&self, handler: impl Fn(&Self) + 'static) -> ActionTarget {
        let this = Weak::new(self);
        ActionTarget::new(self, ControlEvents::EDITING_CHANGED, move |_mtm| {
            if let Some(field) = this.load() {
                handler(&field);
            }
        })
    }

    /// Calls `handler` when the user submits with Return — the
    /// `editingDidEndOnExit` event. The returned target must be kept for as
    /// long as the control should respond.
    pub fn install_submit_handler(&self, handler: impl Fn(&Self) + 'static) -> ActionTarget {
        let this = Weak::new(self);
        ActionTarget::new(self, ControlEvents::EDITING_DID_END_ON_EXIT, move |_mtm| {
            if let Some(field) = this.load() {
                handler(&field);
            }
        })
    }
}
