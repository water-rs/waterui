//! A window's reactive background realized at the window.
//!
//! The framework resolves [`Window::background`](waterui::window::Window) to
//! a colour or a material. A colour is painted behind the window's content. A
//! material is the window's material: the platform's visual-effect view —
//! `NSVisualEffectView` on `AppKit`, `UIVisualEffectView` on `UIKit` — fills
//! the window behind its content, with the level's own blending.
//!
//! On `AppKit` the window's content is mounted inside the effect view while
//! a material is installed, so text drawn in a system label colour is
//! vibrant (#3683); it moves back to the window's content view when the
//! material is removed. `UIKit` keeps the effect view and the content as
//! siblings.
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

use crate::contract::KeepAlive;

/// The `Material` thickness as the kit's neutral level.
pub const fn level(material: Material) -> material::MaterialLevel {
    match material {
        Material::UltraThin => material::MaterialLevel::UltraThin,
        Material::Thin => material::MaterialLevel::Thin,
        Material::Regular => material::MaterialLevel::Regular,
        Material::Thick => material::MaterialLevel::Thick,
        Material::UltraThick => material::MaterialLevel::UltraThick,
    }
}

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
    /// The window's content: a subview of the installed effect view, else of
    /// the container.
    #[cfg(target_os = "macos")]
    content: Retained<PlatformView>,
    /// The installed effect and the material it realizes.
    installed: RefCell<Option<(Material, Retained<PlatformView>)>>,
    mtm: MainThreadMarker,
}

impl EffectView {
    /// Shows `material`'s effect behind the container's content, or none:
    /// a change of material replaces the effect view, `None` removes it. On
    /// `AppKit` the content moves into the effect view and back out.
    fn show(&self, material: Option<Material>) {
        let mut installed = self.installed.borrow_mut();
        if installed.as_ref().map(|(shown, _)| *shown) == material {
            return;
        }
        let previous = installed.take();
        if let Some(material) = material {
            let effect = material::material_view(self.mtm, level(material));
            view::set_frame(&effect, view::bounds(&self.container));
            view::set_autoresizing_flexible_size(&effect);
            view::add_subview_at_bottom(&self.container, &effect);
            #[cfg(target_os = "macos")]
            view::add_subview(&effect, &self.content);
            *installed = Some((material, effect));
        } else {
            #[cfg(target_os = "macos")]
            view::add_subview_at_bottom(&self.container, &self.content);
        }
        if let Some((_, effect)) = previous {
            view::remove_from_superview(&effect);
        }
    }
}

impl Drop for EffectView {
    /// Removes the installed effect; on `AppKit` the content moves back to
    /// the container first, so it outlives the binding.
    fn drop(&mut self) {
        self.show(None);
    }
}

/// Realizes `resolved` at a window whose content lives in `container`: the
/// colour to paint behind the content goes to `paint` now and on every
/// change — of the background, or of the theme background a material is
/// painted over — and a material installs its effect view at the bottom of
/// `container`, filling it. On `AppKit`, `content` — a subview of
/// `container` — is mounted inside the effect view while one is installed.
pub fn bind(
    keepalive: &mut KeepAlive,
    container: &PlatformView,
    #[cfg(target_os = "macos")] content: &PlatformView,
    resolved: &Computed<ResolvedWindowBackground>,
    env: &Environment,
    mtm: MainThreadMarker,
    paint: impl Fn(WorkingColor) + 'static,
) {
    let effect = EffectView {
        container: view::retain_base(container),
        #[cfg(target_os = "macos")]
        content: view::retain_base(content),
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
