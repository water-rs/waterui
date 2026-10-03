//! The `UIKit` image view: a `UIImageView` that also names system symbols.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIImageView` subclass and calls `objc2`
//! bindings marked unsafe because `UIKit` view, image, and symbol APIs are
//! main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee.

use std::cell::RefCell;
use std::fmt;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_ui_kit::{
    NSObjectUIAccessibility, UIColor, UIImage, UIImageSymbolConfiguration, UIImageView,
    UIViewContentMode,
};

use crate::geometry::Size;
use crate::image::ScaleMode;

/// The named system symbol an [`ImageView`] shows.
///
/// `preferredSymbolConfiguration` needs no re-resolution, but the name is
/// kept anyway: chrome outside the view (a window toolbar, say) asks for it
/// to draw the icon itself.
pub struct ImageViewIvars {
    symbol_name: RefCell<Option<String>>,
}

impl fmt::Debug for ImageViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageViewIvars").finish()
    }
}

define_class!(
    // SAFETY: `UIImageView` inherits `initWithFrame:` as a valid initializer,
    // which `ImageView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIImageView))]
    #[name = "CocoaUiImageView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ImageViewIvars]
    #[derive(Debug)]
    /// A `UIImageView` used as an image leaf, with system-symbol helpers.
    pub struct ImageView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIImageView` subclass.
    unsafe impl NSObjectProtocol for ImageView {}

    impl ImageView {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(symbolName))]
        fn symbol_name_selector(&self) -> Option<Retained<NSString>> {
            self.ivars()
                .symbol_name
                .borrow()
                .as_deref()
                .map(NSString::from_str)
        }
    }
);

impl ImageView {
    /// An empty image view scaled proportionally.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ImageViewIvars {
            symbol_name: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.set_scale_mode(ScaleMode::Fit);
        this
    }

    /// How the image scales inside the view's bounds.
    pub fn set_scale_mode(&self, mode: ScaleMode) {
        let content_mode = match mode {
            ScaleMode::Stretch => UIViewContentMode::ScaleToFill,
            ScaleMode::Fit => UIViewContentMode::ScaleAspectFit,
            ScaleMode::Fill => UIViewContentMode::ScaleAspectFill,
        };
        self.setContentMode(content_mode);
    }

    /// The image the view draws; `None` clears it.
    pub fn set_image(&self, image: Option<&UIImage>) {
        self.ivars().symbol_name.replace(None);
        self.setImage(image);
        self.invalidate_layout();
    }

    /// The named `SF Symbol` as the view's image.
    ///
    /// Answers `false` when the platform catalog has no symbol of that name;
    /// whether an unknown name is a bug is the caller's call.
    pub fn set_system_symbol(&self, name: &str) -> bool {
        let symbol = NSString::from_str(name);
        let image = UIImage::systemImageNamed(&symbol);
        image.is_some_and(|image| {
            self.ivars().symbol_name.replace(Some(name.to_owned()));
            self.setImage(Some(&image));
            self.invalidate_layout();
            true
        })
    }

    /// The name of the system symbol shown, if one is set.
    #[must_use]
    pub fn symbol_name(&self) -> Option<String> {
        self.ivars().symbol_name.borrow().clone()
    }

    /// Re-renders the current symbol to match `font`, the way
    /// `UIImage.SymbolConfiguration(font:)` scales a symbol to a themed
    /// typeface.
    ///
    /// Answers `false` when the view holds no image.
    pub fn set_symbol_font(&self, font: &crate::Font) -> bool {
        if self.image().is_none() {
            return false;
        }
        let configuration = UIImageSymbolConfiguration::configurationWithFont(font);
        self.setPreferredSymbolConfiguration(Some(&configuration));
        self.invalidate_layout();
        true
    }

    /// The color a template-rendered symbol draws in.
    pub fn set_tint_color(&self, color: Option<&UIColor>) {
        // SAFETY: see the module safety note.
        unsafe { self.setTintColor(color) };
    }

    /// Names the view to a screen reader; `None` leaves it unnamed. Setting a
    /// label marks the view an accessibility element.
    /// The window screen's scale — `None` while the view is off-window.
    #[must_use]
    pub fn backing_scale(&self) -> Option<f64> {
        crate::view::window(self).map(|window| window.screen().scale())
    }

    /// The view's `accessibilityValue`.
    pub fn set_accessibility_value(&self, value: Option<&str>) {
        self.setAccessibilityValue(
            value.map(NSString::from_str).as_deref(),
            MainThreadMarker::from(self),
        );
    }

    /// `isAccessibilityElement(true)` + the image accessibility trait.
    pub fn set_image_trait(&self) {
        self.setIsAccessibilityElement(true, MainThreadMarker::from(self));
        let mtm = MainThreadMarker::from(self);
        self.setAccessibilityTraits(
            // SAFETY: `UIAccessibilityTraitImage` is a `UIKit` extern static.
            self.accessibilityTraits(mtm) | unsafe { objc2_ui_kit::UIAccessibilityTraitImage },
            mtm,
        );
    }

    /// Marks the image as an accessibility image carrying `label`.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        let mtm = MainThreadMarker::from(self);
        self.setIsAccessibilityElement(label.is_some(), mtm);
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref(), mtm);
    }

    /// The image's intrinsic point size as the view reports it, when one is
    /// set — `sizeThatFits`'s answer.
    #[must_use]
    pub fn image_size(&self) -> Option<Size> {
        self.image().map(|_| self.intrinsicContentSize().into())
    }

    /// The view's intrinsic size changed: invalidate it and walk the
    /// superview chain so every ancestor re-runs layout.
    fn invalidate_layout(&self) {
        self.invalidateIntrinsicContentSize();
        crate::view::invalidate_layout(self);
    }
}
