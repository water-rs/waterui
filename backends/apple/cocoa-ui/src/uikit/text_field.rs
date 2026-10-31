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

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::ffi;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::{CGFloat, CGRect, CGSize};
use objc2_foundation::{NSArray, NSAttributedString, NSObjectProtocol, NSRange, NSString, NSValue};
use objc2_ui_kit::{
    NSTextAlignment, UICommand, UIKeyboardType, UIMenu, UIMenuElement,
    UITextAutocapitalizationType, UITextAutocorrectionType, UITextBorderStyle,
    UITextContentTypePassword, UITextField, UITextFieldDelegate, UITextInput,
};

use crate::ActionTarget;
use crate::action::ControlEvents;
use crate::callback::guarded;
use crate::geometry::Size;
use crate::uikit::key_commands::{KeyCommandRegistry, KeyCommands};

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

/// Runs when the field asks for its edit menu — the field and the actions
/// the system suggests; the answer replaces the default menu, `None` keeps
/// it.
type EditMenuHandler = Rc<dyn Fn(&TextField, &NSArray<UIMenuElement>) -> Option<Retained<UIMenu>>>;

/// A single-line `UITextField` used as a text-input leaf.
pub struct TextFieldIvars {
    edit_menu: RefCell<Option<EditMenuHandler>>,
    /// The field's own key-command registry: its edit-menu shortcut
    /// commands route back to the first-responder field through
    /// `cocoaUiTextSelectionCommandFired:`, not the application delegate.
    key_commands: Rc<KeyCommandRegistry>,
}

impl fmt::Debug for TextFieldIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextFieldIvars")
            .field("has_edit_menu", &self.edit_menu.borrow().is_some())
            .finish_non_exhaustive()
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

    // SAFETY: `UITextFieldDelegate` is an all-optional protocol; the field
    // answers the edit-menu methods it implements with their declared
    // signatures. `delegate` is an assign reference, so self-delegation
    // retains nothing.
    unsafe impl UITextFieldDelegate for TextField {
        // SAFETY: the signature `UITextFieldDelegate` declares. `method_id`
        // because the implementation hands back a `Retained`.
        #[unsafe(method_id(textField:editMenuForCharactersInRanges:suggestedActions:))]
        fn text_field_edit_menu_for_characters_in_ranges_suggested_actions(
            &self,
            _text_field: &UITextField,
            _ranges: &NSArray<NSValue>,
            suggested_actions: &NSArray<UIMenuElement>,
        ) -> Option<Retained<UIMenu>> {
            self.edit_menu(suggested_actions)
        }
    }

    // SAFETY: the selector is the field's own armed-command route; the
    // sender is the `UICommand` the registry armed.
    impl TextField {
        #[unsafe(method(cocoaUiTextSelectionCommandFired:))]
        fn text_selection_command_fired(&self, command: &UICommand) {
            guarded("cocoaUiTextSelectionCommandFired:", || {
                self.ivars().key_commands.fire(command);
            });
        }

        // SAFETY: `canPerformAction:withSender:` is `UIResponder`'s
        // declared signature; the override answers the field's own route
        // and defers everything else to `super`.
        #[unsafe(method(canPerformAction:withSender:))]
        fn can_perform_action_with_sender(
            &self,
            action: Sel,
            sender: Option<&AnyObject>,
        ) -> objc2::runtime::Bool {
            if action != sel!(cocoaUiTextSelectionCommandFired:) {
                // SAFETY: `canPerformAction:withSender:` is `UIResponder`'s
                // declared signature; `super` answers every selector not
                // ours.
                return unsafe {
                    msg_send![super(self), canPerformAction: action, withSender: sender]
                };
            }
            // Own selector: a `UICommand` sender answers by ownership —
            // foreign-field or retired commands are declined; a plain
            // capability query (nil/other sender) answers whether the
            // field has anything armed.
            objc2::runtime::Bool::from(
                sender
                    .and_then(|sender| sender.downcast_ref::<UICommand>())
                    .map_or_else(
                        || self.ivars().key_commands.any_armed(),
                        |command| self.ivars().key_commands.owns(command),
                    ),
            )
        }

        // SAFETY: `sizeThatFits:` is `UIView`'s declared signature; the
        // override keeps `super`'s width and floors the height at the
        // idiom's canonical field height.
        #[unsafe(method(sizeThatFits:))]
        fn size_that_fits(&self, size: CGSize) -> CGSize {
            // SAFETY: `super` answers the control's own fitting size.
            let mut fitting: CGSize = unsafe { msg_send![super(self), sizeThatFits: size] };
            fitting.height = fitting.height.max(canonical_field_height(self));
            fitting
        }

        // SAFETY: `intrinsicContentSize` is `UIView`'s declared signature;
        // the override floors the height the same way `sizeThatFits:` does.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> CGSize {
            // SAFETY: `super` answers the control's own intrinsic size.
            let mut size: CGSize = unsafe { msg_send![super(self), intrinsicContentSize] };
            size.height = size.height.max(canonical_field_height(self));
            size
        }
    }
);

/// The field height the idiom's own convention holds — under the pad
/// idiom the platform's plain field is 23 pt while a borderless
/// `UITextField` measures 21.5, so its reports floor at 23 (#3722). Every
/// other idiom already agrees with `UIKit`'s measure, so the floor
/// releases there.
fn canonical_field_height(field: &UITextField) -> CGFloat {
    if objc2_ui_kit::UIDevice::currentDevice(field.mtm()).userInterfaceIdiom()
        == objc2_ui_kit::UIUserInterfaceIdiom::Pad
    {
        23.0
    } else {
        0.0
    }
}

impl TextField {
    /// An empty, chromeless, enabled single-line field.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TextFieldIvars {
            edit_menu: RefCell::new(None),
            key_commands: Rc::new(KeyCommandRegistry::default()),
        });
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setBorderStyle(UITextBorderStyle::None);
        // `delegate` is assign; self-delegation retains nothing.
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));
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

    /// The declared content type — `UITextContentType` is what autofill and
    /// the one-time-code suggestion above the keyboard key on.
    pub fn set_content_type(&self, content_type: crate::ContentType) {
        // SAFETY: `setTextContentType:` is a `UITextInputTraits` setter on
        // `UITextField` — see `send_trait_setter`. The constants are
        // framework statics, so passing one cannot alias.
        unsafe {
            send_trait_setter(self, sel!(setTextContentType:), Some(content_type.native()));
        }
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

    /// Answers `handler`'s menu each time the field asks for its edit
    /// menu — `textField:editMenuForCharactersInRanges:suggestedActions:` —
    /// `None` keeping the system's default.
    pub fn install_edit_menu_handler(
        &self,
        handler: impl Fn(&Self, &NSArray<UIMenuElement>) -> Option<Retained<UIMenu>> + 'static,
    ) {
        self.ivars().edit_menu.replace(Some(Rc::new(handler)));
    }

    /// A key-command scope over the field's own registry: commands it
    /// arms send `cocoaUiTextSelectionCommandFired:` back to the
    /// first-responder field — no application delegate is involved.
    pub fn edit_menu_commands(&self) -> KeyCommands {
        KeyCommands::in_registry_with_action(
            Rc::clone(&self.ivars().key_commands),
            sel!(cocoaUiTextSelectionCommandFired:),
        )
    }

    /// How many commands the field's registry has armed — the
    /// native-test seam for scope retirement.
    #[cfg(feature = "native-test")]
    pub fn armed_command_count(&self) -> usize {
        self.ivars().key_commands.armed()
    }

    /// Runs the installed edit-menu handler on `suggested_actions` —
    /// `None` (no handler, or its answer) means the system's default menu.
    #[must_use]
    pub fn edit_menu(
        &self,
        suggested_actions: &NSArray<UIMenuElement>,
    ) -> Option<Retained<UIMenu>> {
        let handler = self.ivars().edit_menu.borrow().clone();
        handler.and_then(|handler| handler(self, suggested_actions))
    }

    /// Makes the field first responder — begins an editing session, the
    /// state `selectedTextRange` writes require. Answers `becomeFirstResponder`'s
    /// result — false for a field that cannot take input (no window,
    /// disabled).
    pub fn begin_editing(&self) -> bool {
        self.becomeFirstResponder()
    }

    /// The selected UTF-16 range of the live edit — `selectedTextRange`'s
    /// offsets from the document start. `None` while the field has no
    /// selection, which includes not being edited.
    #[must_use]
    pub fn selected_range(&self) -> Option<NSRange> {
        let range = self.selectedTextRange()?;
        let beginning = self.beginningOfDocument();
        let location = self
            .offsetFromPosition_toPosition(&beginning, &range.start())
            .cast_unsigned();
        let end = self
            .offsetFromPosition_toPosition(&beginning, &range.end())
            .cast_unsigned();
        Some(NSRange::new(location, end - location))
    }

    /// The `UITextRange` spanning `utf16` code units from the document
    /// start — `None` when the range reaches past the text.
    fn text_range(&self, utf16: NSRange) -> Option<Retained<objc2_ui_kit::UITextRange>> {
        let beginning = self.beginningOfDocument();
        let start =
            self.positionFromPosition_offset(&beginning, utf16.location.try_into().ok()?)?;
        let end = self.positionFromPosition_offset(&start, utf16.length.try_into().ok()?)?;
        self.textRangeFromPosition_toPosition(&start, &end)
    }

    /// Selects `utf16` code units from the document start — a caret when
    /// `utf16` is empty. Requires an editing session; see
    /// [`Self::begin_editing`].
    ///
    /// # Panics
    ///
    /// If `utf16` reaches past the field's text.
    pub fn set_selected_range(&self, utf16: NSRange) {
        let range = self
            .text_range(utf16)
            .expect("set_selected_range reaches past the field's text");
        self.setSelectedTextRange(Some(&range));
    }

    /// Replaces `utf16` code units of the live edit with `text` through
    /// `replaceRange:withText:` — the field's normal edit path, so the
    /// change surfaces as the same edit a typed replacement produces and
    /// the caret lands after the inserted text.
    ///
    /// # Panics
    ///
    /// If `utf16` reaches past the field's text.
    pub fn replace_range(&self, utf16: NSRange, text: &NSString) {
        let range = self
            .text_range(utf16)
            .expect("replace_range reaches past the field's text");
        self.replaceRange_withText(&range, text);
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
        let this = Self::alloc(mtm).set_ivars(TextFieldIvars {
            edit_menu: RefCell::new(None),
            key_commands: Rc::new(KeyCommandRegistry::default()),
        });
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
