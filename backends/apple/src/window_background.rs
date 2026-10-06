//! A window's reactive background realized at the window.
//!
//! The framework resolves [`Window::background`](waterui::window::Window) to
//! a colour or a material. A colour is painted behind the window's content. A
//! material is the window's material: the platform's visual-effect view —
//! `NSVisualEffectView` on `AppKit`, `UIVisualEffectView` on `UIKit` — fills
//! the window behind its content, with the level's own blending.
//!
//! What is painted under a material:
//!
//! - **macOS**: a level blended behind the window makes the window non-opaque
//!   with a clear background, so the desktop shows through the effect view;
//!   a within-window level keeps the window opaque in the theme background,
//!   which the effect view blends over.
//! - **iOS**: nothing lies behind a window, so every level blends over the
//!   theme background.

use core::cell::RefCell;

use cocoa_ui::material;
use cocoa_ui::{MainThreadMarker, PlatformView, Retained, view};
use waterui::background::Material;
use waterui::graphics::Color;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::{Computed, SignalExt as _};
use waterui::theme::color::Background;
use waterui::window::ResolvedWindowBackground;
use waterui_backend_core::Environment;

use crate::components::material_background::level;
use crate::contract::KeepAlive;

/// The colour painted behind the window's content while `material` is its
/// background, given the theme background `theme`.
#[cfg_attr(
    target_os = "ios",
    expect(
        unused_variables,
        reason = "nothing lies behind an iOS window: every level blends over the theme"
    )
)]
const fn under_material(material: Material, theme: WorkingColor) -> WorkingColor {
    #[cfg(target_os = "macos")]
    if material::blends_behind_window(level(material)) {
        // Clear: the desktop shows through the non-opaque window.
        return WorkingColor {
            components: [0.0; 4],
        };
    }
    theme
}

/// The effect view a material background fills `container` with.
struct EffectView {
    /// The view the effect fills, behind its content.
    container: Retained<PlatformView>,
    /// The installed effect and the material it realizes.
    installed: RefCell<Option<(Material, Retained<PlatformView>)>>,
    mtm: MainThreadMarker,
}

impl EffectView {
    /// Shows `material`'s effect behind the container's content, or none:
    /// a change of material replaces the effect view, `None` removes it.
    fn show(&self, material: Option<Material>) {
        let mut installed = self.installed.borrow_mut();
        if installed.as_ref().map(|(shown, _)| *shown) == material {
            return;
        }
        if let Some((_, effect)) = installed.take() {
            view::remove_from_superview(&effect);
        }
        if let Some(material) = material {
            let effect = material::material_view(self.mtm, level(material));
            view::set_frame(&effect, view::bounds(&self.container));
            view::set_autoresizing_flexible_size(&effect);
            view::add_subview_at_bottom(&self.container, &effect);
            *installed = Some((material, effect));
        }
    }
}

impl Drop for EffectView {
    fn drop(&mut self) {
        if let Some((_, effect)) = self.installed.take() {
            view::remove_from_superview(&effect);
        }
    }
}

/// Realizes `resolved` at a window whose content lives in `container`: the
/// colour to paint behind the content goes to `paint` now and on every
/// change — of the background, or of the theme background a material is
/// painted over — and a material installs its effect view at the bottom of
/// `container`, filling it.
pub fn bind(
    keepalive: &mut KeepAlive,
    container: &PlatformView,
    resolved: &Computed<ResolvedWindowBackground>,
    env: &Environment,
    mtm: MainThreadMarker,
    paint: impl Fn(WorkingColor) + 'static,
) {
    let effect = EffectView {
        container: view::retain_base(container),
        installed: RefCell::new(None),
        mtm,
    };
    let theme = Color::new(Background).resolve(env);
    keepalive.bind(
        &resolved.zip(&theme),
        move |(resolved, theme)| match resolved {
            ResolvedWindowBackground::Color(color) => {
                effect.show(None);
                paint(color);
            }
            ResolvedWindowBackground::Material(material) => {
                paint(under_material(material, theme));
                effect.show(Some(material));
            }
        },
    );
}
