//! How a `ButtonStyle` and a `ControlSize` realize on the platform button:
//! the label's `Foreground` and `Body`, `AppKit`'s bezel and control size,
//! `UIKit`'s chrome and the accent both tint with.
//!
//! The `button` leaf and the `menu` leaf's pull-down trigger both draw
//! through these, so a menu styled like a button reads like one.

use cocoa_ui::Retained;
use waterui::component::ControlSize;
use waterui::component::button::ButtonStyle;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::{Computed, SignalExt};
use waterui::resolve::Resolvable;
#[cfg(target_os = "macos")]
use waterui::text::font::{Body, ResolvedFont};
#[cfg(target_os = "macos")]
use waterui::theme::color::MutedForeground;
use waterui::theme::color::{AccentForeground, Foreground};
#[cfg(target_os = "macos")]
use waterui::theme::install_font_signal;
use waterui::theme::{install_color_signal, resolve_tint};
use waterui_backend_core::Environment;

#[cfg(target_os = "macos")]
use crate::components::control_size::platform_control_size;
use crate::contract::NativeLeaf;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
    pub use cocoa_ui::objc2_app_kit::NSControlSize;
    pub(super) use cocoa_ui::objc2_app_kit::{NSBezelStyle, NSButton, NSColor};

    /// The platform button a style draws on.
    pub type StyledButton = cocoa_ui::Retained<NSButton>;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor;
    pub(super) use cocoa_ui::uikit::button::Chrome;
    pub(super) use cocoa_ui::uikit::{Button, colors};

    /// The platform button a style draws on.
    pub type StyledButton = Button;
}

#[cfg(target_os = "macos")]
pub(super) use platform::NSControlSize;
pub(super) use platform::StyledButton;

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "ios")]
pub(super) fn platform_color(color: &WorkingColor) -> Retained<platform::UIColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "macos")]
pub(super) fn platform_color(color: &WorkingColor) -> Retained<platform::NSColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// The tint the native controls of `env`'s subtree draw with: the nearest
/// `.tint(...)`, else the `Accent` token. Every native control takes its
/// tint colour from this one resolution.
pub(super) fn tint_signal(env: &Environment) -> Computed<WorkingColor> {
    resolve_tint(env).resolve(env)
}

/// The color the label's `Foreground` slot resolves to under `style` — the
/// `UIKit` table: chrome carries the emphasis for glass, accent for
/// prominent fills. Inside a navigation or bottom bar an automatic or
/// borderless label takes the primary label color instead, as `SwiftUI` and
/// `UIBarButtonItem` draw it there.
#[cfg(target_os = "ios")]
fn label_foreground(style: ButtonStyle, env: &Environment) -> Computed<WorkingColor> {
    match style {
        ButtonStyle::BorderedProminent | ButtonStyle::GlassProminent => {
            AccentForeground.resolve(env).computed()
        }
        ButtonStyle::Plain | ButtonStyle::Glass => Foreground.resolve(env).computed(),
        ButtonStyle::Automatic | ButtonStyle::Borderless
            if env.get::<crate::toolbar_chrome::ToolbarChrome>().is_some() =>
        {
            Foreground.resolve(env).computed()
        }
        ButtonStyle::Automatic
        | ButtonStyle::Link
        | ButtonStyle::Borderless
        | ButtonStyle::Bordered => tint_signal(env),
        _ => {
            waterui::contract_violation!(
                "unsupported WaterUI button style: {style:?}; the automatic style's label color is used"
            );
            label_foreground(ButtonStyle::Automatic, env)
        }
    }
}

/// The `AppKit` table: bordered titles draw in the primary label color;
/// only link and borderless styles are accent-tinted. Inside the window
/// toolbar a borderless label takes the toolbar's secondary label color
/// instead, as `SwiftUI` draws it there.
#[cfg(target_os = "macos")]
fn label_foreground(style: ButtonStyle, env: &Environment) -> Computed<WorkingColor> {
    match style {
        ButtonStyle::BorderedProminent | ButtonStyle::GlassProminent => {
            AccentForeground.resolve(env).computed()
        }
        ButtonStyle::Automatic
        | ButtonStyle::Bordered
        | ButtonStyle::Plain
        | ButtonStyle::Glass => Foreground.resolve(env).computed(),
        ButtonStyle::Borderless if env.get::<crate::toolbar_chrome::ToolbarChrome>().is_some() => {
            MutedForeground.resolve(env).computed()
        }
        ButtonStyle::Link | ButtonStyle::Borderless => tint_signal(env),
        _ => {
            waterui::contract_violation!(
                "unsupported WaterUI button style: {style:?}; the automatic style's label color is used"
            );
            label_foreground(ButtonStyle::Automatic, env)
        }
    }
}

/// The label's `Body` slot: the environment's resolved body font resized to
/// `size`'s own system font size. `AppKit` pairs each `NSControlSize` with a
/// point size — 11 pt for small, 9 pt for mini — and [`apply_bezel`] sets
/// the same `NSControlSize` on the bezel. Weight, family and typography
/// metrics keep the values the theme resolved.
#[cfg(target_os = "macos")]
fn label_font(env: &Environment, size: NSControlSize) -> Computed<ResolvedFont> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ResolvedFont is f32; AppKit's control-size point sizes are small exact values"
    )]
    let points = cocoa_ui::font::system_size_for_control_size(size) as f32;
    Body.resolve(env)
        .map(move |mut font| {
            font.set_size(points);
            font
        })
        .computed()
}

/// The `AppKit` control size a button of `size` draws at. A button's
/// documented default is `Small`, so a bare one draws at `regular` like a
/// bare `NSButton`.
#[cfg(target_os = "macos")]
pub(super) const fn control_size(size: ControlSize) -> NSControlSize {
    match platform_control_size(size, ControlSize::Small) {
        cocoa_ui::slider::ControlSize::Mini => NSControlSize::Mini,
        cocoa_ui::slider::ControlSize::Small => NSControlSize::Small,
        cocoa_ui::slider::ControlSize::Regular => NSControlSize::Regular,
        cocoa_ui::slider::ControlSize::Large => NSControlSize::Large,
    }
}

/// The environment a button's label renders against: `Foreground` is the
/// color `style` prescribes for its platform, and on `AppKit` the default
/// `Body` follows the control size's own system font size. Another slot or
/// an explicit font keeps what it resolves to.
pub(super) fn label_environment(
    style: ButtonStyle,
    size: ControlSize,
    env: &Environment,
) -> Environment {
    let mut label_env = env.clone();
    install_color_signal::<Foreground>(&mut label_env, label_foreground(style, env));
    #[cfg(target_os = "macos")]
    install_font_signal::<Body>(&mut label_env, label_font(env, control_size(size)));
    #[cfg(target_os = "ios")]
    let _ = size;
    label_env
}

/// Whether the style draws chrome around its label — `AppKit`'s bezel, plus
/// `Automatic`, which resolves to the bordered push bezel. Chrome-less
/// styles pad nothing.
#[cfg(target_os = "macos")]
pub(super) const fn draws_chrome(style: ButtonStyle) -> bool {
    matches!(
        style,
        ButtonStyle::Automatic
            | ButtonStyle::Bordered
            | ButtonStyle::BorderedProminent
            | ButtonStyle::Glass
            | ButtonStyle::GlassProminent
    )
}

/// The bezel `style` draws on `AppKit`: `None` for the transparent,
/// chrome-less styles.
#[cfg(target_os = "macos")]
pub(super) fn bezel_of(style: ButtonStyle) -> Option<platform::NSBezelStyle> {
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered | ButtonStyle::BorderedProminent => {
            Some(platform::NSBezelStyle::FlexiblePush)
        }
        ButtonStyle::Glass | ButtonStyle::GlassProminent => Some(platform::NSBezelStyle::Glass),
        ButtonStyle::Plain | ButtonStyle::Link | ButtonStyle::Borderless => None,
        _ => {
            waterui::contract_violation!(
                "unsupported WaterUI button style: {style:?}; the automatic style's bezel is used"
            );
            bezel_of(ButtonStyle::Automatic)
        }
    }
}

/// Sets `button`'s bezel for `style` at control size `size`.
#[cfg(target_os = "macos")]
pub(super) fn apply_bezel(button: &platform::NSButton, style: ButtonStyle, size: NSControlSize) {
    button.setControlSize(size);
    if let Some(bezel) = bezel_of(style) {
        button.setBordered(true);
        button.setBezelStyle(bezel);
    } else {
        button.setBordered(false);
        button.setTransparent(true);
    }
}

/// The chrome `style` draws on `UIKit`, matching what `SwiftUI`'s button
/// styles resolve to.
#[cfg(target_os = "ios")]
pub(super) const fn chrome_of(style: ButtonStyle) -> Option<platform::Chrome> {
    match style {
        // SwiftUI's .bordered is the neutral gray capsule; .filled() is
        // reserved for the prominent accent fill.
        ButtonStyle::Bordered => Some(platform::Chrome::Gray),
        ButtonStyle::BorderedProminent => Some(platform::Chrome::Filled),
        ButtonStyle::Glass => Some(platform::Chrome::Glass),
        ButtonStyle::GlassProminent => Some(platform::Chrome::ProminentGlass),
        _ => None,
    }
}

/// The padding `style`'s `UIKit` chrome keeps around the label on each
/// side, horizontal then vertical, read from the configuration's own
/// content insets.
#[cfg(target_os = "ios")]
pub(super) fn chrome_padding(style: ButtonStyle, mtm: cocoa_ui::MainThreadMarker) -> (f32, f32) {
    let insets = chrome_of(style).map_or(cocoa_ui::EdgeInsets::ZERO, |chrome| {
        platform::Button::chrome_content_insets(chrome, mtm)
    });
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; chrome insets are a few points"
    )]
    (insets.left as f32, insets.top as f32)
}

/// The tint ([`tint_signal`]): `UIKit` tints chrome and title with it and
/// re-applies `style`'s chrome each time, matching `applyThemeAppearance`;
/// `AppKit` paints it only into prominent bezels.
pub(super) fn bind_tint(
    leaf: &mut NativeLeaf,
    button: StyledButton,
    style: ButtonStyle,
    env: &Environment,
    mtm: cocoa_ui::MainThreadMarker,
) {
    let tint = tint_signal(env);
    #[cfg(target_os = "macos")]
    let _ = mtm;
    leaf.bind(&tint, move |color| {
        #[cfg(target_os = "ios")]
        {
            button.set_tint_color(&platform_color(&color));
            button.set_chrome(chrome_of(style).unwrap_or(platform::Chrome::Plain), mtm);
        }
        #[cfg(target_os = "macos")]
        if matches!(
            style,
            ButtonStyle::BorderedProminent | ButtonStyle::GlassProminent
        ) {
            button.setBezelColor(Some(&platform_color(&color)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::graphics::color::{Color, Working};
    use waterui::reactive::{Binding, Signal, constant};
    use waterui::theme::Tint;
    use waterui::theme::color::Accent;

    fn components(signal: &Computed<WorkingColor>) -> [f32; 4] {
        signal.snapshot().components
    }

    /// The native controls' tint is the nearest `.tint(...)`, else the
    /// accent; a link label, which draws in the tint, follows it.
    #[test]
    fn native_controls_take_the_tint_else_the_accent() {
        let accent = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        let red = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        let mut env = Environment::new();
        install_color_signal::<Accent>(&mut env, constant(accent).computed());
        assert_eq!(components(&tint_signal(&env)), accent.components);
        assert_eq!(
            components(&label_foreground(ButtonStyle::Link, &env)),
            accent.components
        );

        let tinted = env.extending(Tint::new(Color::new(Working(red))));
        assert_eq!(components(&tint_signal(&tinted)), red.components);
        assert_eq!(
            components(&label_foreground(ButtonStyle::Link, &tinted)),
            red.components
        );
    }

    /// A tint signal's change reaches the resolved tint in place.
    #[test]
    fn a_tint_signal_change_updates_the_resolved_tint() {
        let source = Binding::container(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
        let tint = Tint::new(waterui::graphics::color::signal_color(
            source.map(|color| Color::new(Working(color))),
        ));
        let env = Environment::new().extending(tint);
        let resolved = tint_signal(&env);
        assert_eq!(components(&resolved), [1.0, 0.0, 0.0, 1.0]);

        source.set(WorkingColor::new([0.0, 1.0, 0.0, 1.0]));

        assert_eq!(components(&resolved), [0.0, 1.0, 0.0, 1.0]);
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn chrome_mapping_matches_swift() {
        use platform::Chrome;
        assert!(matches!(
            chrome_of(ButtonStyle::Bordered),
            Some(Chrome::Gray)
        ));
        assert!(matches!(
            chrome_of(ButtonStyle::BorderedProminent),
            Some(Chrome::Filled)
        ));
        assert!(matches!(chrome_of(ButtonStyle::Glass), Some(Chrome::Glass)));
        assert!(matches!(
            chrome_of(ButtonStyle::GlassProminent),
            Some(Chrome::ProminentGlass)
        ));
        // Automatic, Plain, Link, Borderless carry no configuration and draw
        // through `.plain` with zeroed content insets.
        assert!(chrome_of(ButtonStyle::Automatic).is_none());
        assert!(chrome_of(ButtonStyle::Plain).is_none());
        assert!(chrome_of(ButtonStyle::Link).is_none());
        assert!(chrome_of(ButtonStyle::Borderless).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn chrome_mapping_matches_swift() {
        // AppKit draws chrome for Automatic, Bordered, and the prominent and
        // glass styles; Plain, Link, and Borderless are transparent.
        assert!(draws_chrome(ButtonStyle::Automatic));
        assert!(draws_chrome(ButtonStyle::Bordered));
        assert!(draws_chrome(ButtonStyle::BorderedProminent));
        assert!(draws_chrome(ButtonStyle::Glass));
        assert!(draws_chrome(ButtonStyle::GlassProminent));
        assert!(!draws_chrome(ButtonStyle::Plain));
        assert!(!draws_chrome(ButtonStyle::Link));
        assert!(!draws_chrome(ButtonStyle::Borderless));
        for style in [
            ButtonStyle::Automatic,
            ButtonStyle::Bordered,
            ButtonStyle::BorderedProminent,
            ButtonStyle::Glass,
            ButtonStyle::GlassProminent,
            ButtonStyle::Plain,
            ButtonStyle::Link,
            ButtonStyle::Borderless,
        ] {
            assert_eq!(
                bezel_of(style).is_some(),
                draws_chrome(style),
                "{style:?}: a bezel exactly where the style draws chrome"
            );
        }
    }

    /// A button's documented default size draws at `regular`, like a bare
    /// `NSButton`; one step down is `small`, two `mini`, any step up `large`.
    #[cfg(target_os = "macos")]
    #[test]
    fn control_sizes_step_from_the_buttons_default() {
        assert_eq!(control_size(ControlSize::Small), NSControlSize::Regular);
        assert_eq!(control_size(ControlSize::ExtraSmall), NSControlSize::Small);
        assert_eq!(control_size(ControlSize::Medium), NSControlSize::Large);
        assert_eq!(control_size(ControlSize::Large), NSControlSize::Large);
    }
}
