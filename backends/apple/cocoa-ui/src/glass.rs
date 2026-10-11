//! Glass effect surfaces.
//!
//! iOS uses a `UIVisualEffectView` driven by a `UIGlassEffect`; macOS uses
//! `NSGlassEffectView`. The wrapper keeps the effect object reachable so
//! tint and corner updates are re-applied to the view, matching the
//! behavior of effect objects that require re-assignment after mutation.
//!
//! # Safety
//!
//! `NSGlassEffectView` exposes an unsafe initializer in `objc2-app-kit`;
//! constructing the wrapper on the main thread satisfies it.

use crate::PlatformView;

/// The visual style of a glass surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassStyle {
    /// The default glass appearance.
    Regular,
    /// A clearer, more transparent glass appearance.
    Clear,
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2_ui_kit::{
        UIColor, UICornerConfiguration, UICornerRadius, UIGlassEffect, UIGlassEffectStyle, UIView,
        UIVisualEffect, UIVisualEffectView,
    };

    use super::GlassStyle;
    use crate::{MainThreadMarker, Retained};

    /// A glass surface wrapping a `UIVisualEffectView`.
    pub struct GlassView {
        view: Retained<UIVisualEffectView>,
        effect: Retained<UIGlassEffect>,
    }

    impl core::fmt::Debug for GlassView {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("GlassView")
                .field("view", &self.view)
                .finish_non_exhaustive()
        }
    }

    impl GlassView {
        /// Creates a glass surface.
        #[must_use]
        pub fn new(mtm: MainThreadMarker, style: GlassStyle, interactive: bool) -> Self {
            let effect = UIGlassEffect::effectWithStyle(
                match style {
                    GlassStyle::Regular => UIGlassEffectStyle::Regular,
                    GlassStyle::Clear => UIGlassEffectStyle::Clear,
                },
                mtm,
            );
            effect.setInteractive(interactive);
            let view = UIVisualEffectView::new(mtm);
            view.setEffect(Some(&effect));
            Self { view, effect }
        }

        /// The backing `UIView` for hierarchy and layout operations.
        #[must_use]
        pub fn as_view(&self) -> &UIVisualEffectView {
            &self.view
        }

        /// The content view where embedded content should be installed.
        #[must_use]
        pub fn content_view(&self) -> Retained<UIView> {
            self.view.contentView()
        }

        /// Applies a tint color to the glass.
        pub fn set_tint_color(&self, tint_color: &UIColor) {
            self.effect.setTintColor(Some(tint_color));
            let effect: &UIVisualEffect = &self.effect;
            self.view.setEffect(Some(effect));
        }

        /// Sets a uniform corner radius for the glass.
        pub fn set_corner_radius(&self, radius: f64) {
            let radii = UICornerConfiguration::configurationWithUniformRadius(
                &UICornerRadius::fixedRadius(radius),
            );
            self.view.setCornerConfiguration(&radii);
        }

        /// Sets per-corner radii for the glass.
        pub fn set_corner_radii(
            &self,
            top_left: f64,
            top_right: f64,
            bottom_left: f64,
            bottom_right: f64,
        ) {
            let radii = UICornerConfiguration::configurationWithTopLeftRadius_topRightRadius_bottomLeftRadius_bottomRightRadius(
                Some(&UICornerRadius::fixedRadius(top_left)),
                Some(&UICornerRadius::fixedRadius(top_right)),
                Some(&UICornerRadius::fixedRadius(bottom_left)),
                Some(&UICornerRadius::fixedRadius(bottom_right)),
            );
            self.view.setCornerConfiguration(&radii);
        }

        /// Sets a capsule corner shape for the glass.
        pub fn set_corner_capsule(&self) {
            let radii = UICornerConfiguration::capsuleConfiguration();
            self.view.setCornerConfiguration(&radii);
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSColor, NSGlassEffectView, NSGlassEffectViewStyle, NSView};

    use super::GlassStyle;
    use crate::{MainThreadMarker, Retained};

    /// A glass surface wrapping an `NSGlassEffectView`.
    pub struct GlassView {
        view: Retained<NSGlassEffectView>,
    }

    impl core::fmt::Debug for GlassView {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("GlassView")
                .field("view", &self.view)
                .finish_non_exhaustive()
        }
    }

    impl GlassView {
        /// Creates a glass surface.
        ///
        /// The `interactive` parameter is accepted for parity with iOS;
        /// `NSGlassEffectView` has no equivalent switch.
        #[must_use]
        pub fn new(mtm: MainThreadMarker, style: GlassStyle, _interactive: bool) -> Self {
            let view = NSGlassEffectView::new(mtm);
            view.setStyle(match style {
                GlassStyle::Regular => NSGlassEffectViewStyle::Regular,
                GlassStyle::Clear => NSGlassEffectViewStyle::Clear,
            });
            Self { view }
        }

        /// The backing `NSView` for hierarchy and layout operations.
        #[must_use]
        pub fn as_view(&self) -> &NSGlassEffectView {
            &self.view
        }

        /// Installs embedded content inside the glass surface.
        pub fn set_content_view(&self, content: &NSView) {
            self.view.setContentView(Some(content));
        }

        /// Applies a tint color to the glass.
        pub fn set_tint_color(&self, tint_color: &NSColor) {
            self.view.setTintColor(Some(tint_color));
        }

        /// Sets a uniform corner radius for the glass.
        pub fn set_corner_radius(&self, radius: f64) {
            self.view.setCornerRadius(radius);
        }

        /// Sets a capsule corner shape for the glass.
        ///
        /// `NSGlassEffectView` only exposes a uniform corner radius, so the
        /// capsule shape is represented by half of the view's current shorter
        /// dimension at the time of the call; callers re-apply it from their
        /// layout handler when the surface resizes.
        pub fn set_corner_capsule(&self) {
            let bounds = self.view.bounds();
            self.view
                .setCornerRadius(bounds.size.height.min(bounds.size.width) / 2.0);
        }
    }
}

pub use imp::GlassView;

impl GlassView {
    /// The backing platform view as the crate's platform alias.
    #[must_use]
    pub fn platform_view(&self) -> &PlatformView {
        self.as_view()
    }
}
