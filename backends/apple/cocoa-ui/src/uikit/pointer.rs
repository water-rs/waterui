//! Pointer (trackpad/mouse) behavior on iOS.
//!
//! [`PointerInteraction`] installed on a view gives the pointer the system
//! highlight effect with the view's rounded-rect shape while it is inside
//! the view — the only thing the contract needs beyond the default.
//! [`CursorInteraction`] is the other half: a view that draws its own content
//! picks what the pointer looks like over it, by [`PointerCursor`].
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate class `UIKit` calls for the
//! pointer style. The interaction keeps its delegate weakly, so the
//! returned [`PointerInteraction`] owns it; dropping that handle removes
//! the answer `UIKit` would dereference, which is why the handle must
//! outlive the view's use of the interaction.

use std::cell::{Cell, RefCell};
use std::fmt;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{
    UIAxis, UIFont, UIFontTextStyleBody, UIInteraction, UIPointerInteraction,
    UIPointerInteractionDelegate, UIPointerRegion, UIPointerShape, UIPointerStyle, UIView,
};

use crate::callback::guarded;

/// The ivars of a [`PointerStyleDelegate`]: nothing — the shape always
/// follows the interaction's view bounds.
#[derive(Default)]
pub struct PointerStyleDelegateIvars {
    _marker: RefCell<()>,
}

impl fmt::Debug for PointerStyleDelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PointerStyleDelegateIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSObject` asks a subclass to initialize through `init`, which
    // `PointerStyleDelegate::new` does, and the class does not implement
    // `Drop`.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "CocoaUiPointerStyleDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PointerStyleDelegateIvars]
    #[derive(Debug)]
    /// Answers a [`UIPointerInteraction`]'s style question with the rounded
    /// rect of the interaction's view.
    struct PointerStyleDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for PointerStyleDelegate {}

    // SAFETY: the one method `UIKit` calls through the delegate protocol is
    // implemented with its declared signature, on the main thread.
    unsafe impl UIPointerInteractionDelegate for PointerStyleDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(pointerInteraction:styleForRegion:))]
        fn pointer_interaction_style_for_region(
            &self,
            interaction: &UIPointerInteraction,
            _region: &UIPointerRegion,
        ) -> Option<Retained<UIPointerStyle>> {
            guarded("PointerStyleDelegate styleForRegion", || {
                let mtm = MainThreadMarker::from(self);
                let shape = interaction
                    .view()
                    .map(|view| UIPointerShape::shapeWithRoundedRect(view.bounds(), mtm));
                shape.map(|shape| {
                    UIPointerStyle::styleWithShape_constrainedAxes(&shape, UIAxis::Neither)
                })
            })
        }
    }
);

impl PointerStyleDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PointerStyleDelegateIvars::default());
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// A [`UIPointerInteraction`] installed on a view plus the delegate that
/// answers it, kept alive together.
///
/// `UIKit` holds the delegate weakly; this value owns it. Drop it and the
/// interaction answers with no style — keep it for as long as the pointer
/// effect should apply.
#[derive(Debug)]
pub struct PointerInteraction {
    _interaction: Retained<UIPointerInteraction>,
    _delegate: Retained<PointerStyleDelegate>,
}

impl PointerInteraction {
    /// Installs a rounded-rect pointer interaction on `view`.
    #[must_use]
    pub fn rounded_rect(view: &UIView) -> Self {
        let mtm = MainThreadMarker::from(view);
        let delegate = PointerStyleDelegate::new(mtm);
        let interaction = {
            UIPointerInteraction::initWithDelegate(
                UIPointerInteraction::alloc(mtm),
                Some(ProtocolObject::from_ref(&*delegate)),
            )
        };
        view.addInteraction(ProtocolObject::from_ref(&*interaction));
        Self {
            _interaction: interaction,
            _delegate: delegate,
        }
    }
}

/// What the pointer looks like over a view.
///
/// `iPadOS` draws a fixed pointer vocabulary: the arrow, an I-beam that
/// morphs to the text, and no pointer at all. The hand, resize and busy
/// pointers of a desktop do not exist there, so a caller maps the cursors it
/// has onto these and leaves the rest at the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerCursor {
    /// The system pointer.
    #[default]
    Arrow,
    /// The text I-beam, sized to one line of body text.
    Beam,
    /// No pointer.
    Hidden,
}

/// The ivars of a [`CursorStyleDelegate`].
#[derive(Default)]
pub struct CursorStyleDelegateIvars {
    cursor: Cell<PointerCursor>,
}

impl fmt::Debug for CursorStyleDelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CursorStyleDelegateIvars")
            .field("cursor", &self.cursor.get())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSObject` asks a subclass to initialize through `init`, which
    // `CursorStyleDelegate::new` does, and the class does not implement
    // `Drop`.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "CocoaUiCursorStyleDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = CursorStyleDelegateIvars]
    #[derive(Debug)]
    /// Answers a [`UIPointerInteraction`]'s style question with the
    /// interaction's current [`PointerCursor`].
    struct CursorStyleDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for CursorStyleDelegate {}

    // SAFETY: the one method `UIKit` calls through the delegate protocol is
    // implemented with its declared signature, on the main thread.
    unsafe impl UIPointerInteractionDelegate for CursorStyleDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(pointerInteraction:styleForRegion:))]
        fn pointer_interaction_style_for_region(
            &self,
            _interaction: &UIPointerInteraction,
            _region: &UIPointerRegion,
        ) -> Option<Retained<UIPointerStyle>> {
            guarded("CursorStyleDelegate styleForRegion", || {
                let mtm = MainThreadMarker::from(self);
                match self.ivars().cursor.get() {
                    // No style leaves the system pointer in place.
                    PointerCursor::Arrow => None,
                    PointerCursor::Beam => {
                        // SAFETY: `UIFontTextStyleBody` is a documented,
                        // immutable `UIKit` constant, and `lineHeight` only
                        // reads the font it is sent to.
                        let length = unsafe {
                            UIFont::preferredFontForTextStyle(UIFontTextStyleBody).lineHeight()
                        };
                        let beam = UIPointerShape::beamWithPreferredLength_axis(
                            length,
                            UIAxis::Vertical,
                            mtm,
                        );
                        Some(UIPointerStyle::styleWithShape_constrainedAxes(
                            &beam,
                            UIAxis::Neither,
                        ))
                    }
                    PointerCursor::Hidden => Some(UIPointerStyle::hiddenPointerStyle(mtm)),
                }
            })
        }
    }
);

impl CursorStyleDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(CursorStyleDelegateIvars::default());
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// A [`UIPointerInteraction`] whose style over the whole view is a settable
/// [`PointerCursor`], kept alive with its delegate.
///
/// `UIKit` holds the delegate weakly; this value owns it, so keep it for as
/// long as the cursor should apply.
#[derive(Debug)]
pub struct CursorInteraction {
    interaction: Retained<UIPointerInteraction>,
    delegate: Retained<CursorStyleDelegate>,
}

impl CursorInteraction {
    /// Installs the interaction on `view`, showing [`PointerCursor::Arrow`].
    #[must_use]
    pub fn install(view: &UIView) -> Self {
        let mtm = MainThreadMarker::from(view);
        let delegate = CursorStyleDelegate::new(mtm);
        let interaction = UIPointerInteraction::initWithDelegate(
            UIPointerInteraction::alloc(mtm),
            Some(ProtocolObject::from_ref(&*delegate)),
        );
        view.addInteraction(ProtocolObject::from_ref(&*interaction));
        Self {
            interaction,
            delegate,
        }
    }

    /// Shows `cursor` over the view, immediately when the pointer is already
    /// inside it.
    pub fn set(&self, cursor: PointerCursor) {
        if self.delegate.ivars().cursor.replace(cursor) != cursor {
            self.interaction.invalidate();
        }
    }
}
