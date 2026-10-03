//! The `color_picker` leaf: `Native<ColorPickerConfig>` rendered as a
//! container holding a leading label and the platform color well
//! (`NSColorWell` / `UIColorWell`).
//!
//! Mirrors `WuiColorPicker`: the `value` `Binding<Color>` is two-way —
//! watchers resolve the color in the current environment and push it onto
//! the well (under a sync guard so the write doesn't echo back as a user
//! edit), and the well's action reads the picked color back into linear
//! Display-P3 plus HDR headroom. `support_alpha` reaches `NSColorPanel` on
//! `AppKit`; `support_hdr` gates whether headroom is read back and applied.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::{Cell, RefCell};

use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::component::form::picker::color::ColorPickerConfig;
use waterui::graphics::color::{Color, Working, WorkingColor, srgb_to_linear, working};
use waterui::reactive::{Computed, Signal};
use waterui::text::StyledStr;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{ColorWell, HostView, colors};
    pub(super) use cocoa_ui::objc2_app_kit::NSColor as PlatformColor;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor as PlatformColor;
    pub(super) use cocoa_ui::uikit::{ColorWell, HostView};
    pub(super) use cocoa_ui::uikit::{color_well as well_colors, colors};
}

use platform::{ColorWell, HostView};

/// The gap between the leading label and the well.
const LABEL_SPACING: f64 = 8.0;

/// A styled string as spoken text: plain text with the bidi control
/// characters interpolation inserts for layout stripped, as
/// `WuiControlAccessibility.apply` does.
fn accessibility_text(styled: &StyledStr) -> String {
    styled
        .to_plain()
        .chars()
        .filter(|c| {
            !matches!(c,
                '\u{200e}' | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

/// The platform color a resolved color sets on the well — `toUIColor` /
/// `toNSColor` semantics: extended-range components when
/// `allow_hdr`, clamped linear sRGB otherwise.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor, allow_hdr: bool) -> Retained<platform::PlatformColor> {
    if allow_hdr {
        {
            let [red, green, blue, alpha] = color.components;
            platform::colors::extended_linear_display_p3(
                f64::from(red),
                f64::from(green),
                f64::from(blue),
                f64::from(alpha),
            )
        }
    } else {
        let [red, green, blue, alpha] = color.components;
        platform::colors::linear(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The `AppKit` variant — channels are straight working-color values.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor, allow_hdr: bool) -> Retained<platform::PlatformColor> {
    let [red, green, blue, alpha] = color.components;
    let (red, green, blue, alpha) = (
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    );
    if allow_hdr {
        platform::colors::extended_linear_display_p3(red, green, blue, alpha)
    } else {
        platform::colors::linear(red, green, blue, alpha)
    }
}

/// The well's current color as a `WorkingColor`: the SDR base read as sRGB,
/// converted into linear Display-P3, with HDR headroom carried in the working
/// channels — `updateBindingWithColor` in `WuiColorPicker`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the components fit comfortably in f32 — the binding speaks it"
)]
fn to_working(rgba: cocoa_ui::Rgba, headroom: f64) -> WorkingColor {
    let color = working::from_linear_srgb(
        [
            srgb_to_linear(rgba.red as f32),
            srgb_to_linear(rgba.green as f32),
            srgb_to_linear(rgba.blue as f32),
        ],
        rgba.alpha as f32,
    );
    working::with_headroom(color, headroom as f32)
}

/// The well's current color on `AppKit`.
#[cfg(target_os = "macos")]
fn read_well_color(well: &ColorWell, support_hdr: bool) -> WorkingColor {
    let color = well.color();
    let (base, headroom) = if support_hdr {
        platform::colors::sdr_base_and_headroom(&color)
    } else {
        (color, 0.0)
    };
    let rgba = platform::colors::srgb_components(&base)
        .expect("NSColorWell returned a color with no sRGB form");
    to_working(rgba, headroom)
}

/// The `UIKit` variant; `UIColorWell.selectedColor` may be `nil`, which
/// `WuiColorPicker` guards.
#[cfg(target_os = "ios")]
fn read_well_color(well: &ColorWell, support_hdr: bool) -> Option<WorkingColor> {
    let color = well.color()?;
    let (base, headroom) = if support_hdr {
        platform::well_colors::sdr_base_and_headroom(&color)
    } else {
        (color, 0.0)
    };
    let rgba = platform::well_colors::srgb_components(&base)
        .expect("UIColorWell returned a color that cannot convert to sRGB");
    Some(to_working(rgba, headroom))
}

/// The leaf's live state, shared between the watchers, the layout face and
/// the host's layout handler.
struct ColorPickerState {
    /// The platform color well, shared with the action closure.
    well: Rc<ColorWell>,
    /// The rendered label child; `Option` only because the layout handler
    /// is installed before the child exists.
    label: Option<Mounted>,
    /// `isSyncingFromBinding`: set while a binding write pushes onto the
    /// well so the well's action doesn't echo the same color back. Shared
    /// with the action closure directly — outside the `RefCell`, so a user
    /// pick inside a watcher never deadlocks.
    syncing: Rc<Cell<bool>>,
    /// The watcher observing the currently bound `Color`'s resolved value;
    /// replaced every time the binding emits a new `Color`.
    color_guard: Option<<Computed<WorkingColor> as Signal>::Guard>,
    /// Keeps the well's action target alive; the field is never read.
    _action: cocoa_ui::ActionTarget,
}

/// Lays out the children inside `view`'s bounds: the label leading and
/// vertically centered, then the well spanning the rest of the row,
/// centered on its intrinsic height — `configureSubviews`' constraints.
/// RTL mirrors the row.
fn layout_children(view: &PlatformView, state: &ColorPickerState) {
    let bounds = cocoa_ui::view::bounds(view);
    let width = bounds.size.width;
    let height = bounds.size.height;
    let rtl = cocoa_ui::view::is_right_to_left(view);

    let place = |child: &PlatformView, x: f64, y: f64, w: f64, h: f64| {
        let x = if rtl { width - x - w } else { x };
        cocoa_ui::view::set_frame(child, Rect::new(x, y, w, h));
    };

    let label_size = state.label.as_ref().map_or_else(Size::default, |label| {
        label.layout().measure(ProposalSize::UNSPECIFIED).size
    });
    let well_size = state.well.intrinsic_size();
    // `WuiColorPicker` pins the well to `labelView.trailing + spacing`
    // unconditionally — a hidden label still leaves the 8pt gap.
    let well_x = f64::from(label_size.width) + LABEL_SPACING;

    place(
        state.well.view(),
        well_x,
        (height - well_size.height) / 2.0,
        (width - well_x).max(0.0),
        well_size.height,
    );
    if let Some(label) = &state.label {
        let label_h = f64::from(label_size.height);
        place(
            label.view(),
            0.0,
            (height - label_h) / 2.0,
            f64::from(label_size.width),
            label_h,
        );
    }
}

/// The container's layout face: `label + 8 + well` wide, the taller of the
/// two high — `WuiColorPicker.sizeThatFits`, which ignores the proposal.
struct ColorPickerSubView {
    state: Rc<RefCell<ColorPickerState>>,
}

impl core::fmt::Debug for ColorPickerSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ColorPickerSubView").finish_non_exhaustive()
    }
}

impl SubView for ColorPickerSubView {
    // `measure` speaks f32; the geometry math runs in f64 — the narrowing
    // is the layout contract, as in `text`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        let label_size = state.label.as_ref().map_or_else(Size::default, |label| {
            label.layout().measure(ProposalSize::UNSPECIFIED).size
        });
        let well_size = state.well.intrinsic_size();
        let has_label = label_size.width > 0.0 && label_size.height > 0.0;
        let intrinsic_width = well_size.width
            + if has_label {
                LABEL_SPACING + f64::from(label_size.width)
            } else {
                0.0
            };
        let intrinsic_height = f64::from(label_size.height).max(well_size.height);
        // The well's trailing edge is pinned, so the row stretches
        // horizontally to whatever it's offered.
        let width = proposal
            .width
            .map_or(intrinsic_width, |w| f64::from(w).max(intrinsic_width));
        ViewDimensions::new(Size::new(width as f32, intrinsic_height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `color_picker` handler on the dispatcher:
/// `Native<ColorPickerConfig>` maps to a container view with the label
/// child and the platform color well.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ColorPickerConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let well = Rc::new(ColorWell::new(mtm));
        let host_view: &PlatformView = &host;
        cocoa_ui::view::add_subview(host_view, well.view());
        #[cfg(target_os = "macos")]
        well.set_supports_alpha(config.support_alpha, mtm);
        #[cfg(target_os = "ios")]
        well.set_supports_alpha(config.support_alpha);

        // The sync flag is shared with the state installed below.
        let syncing = Rc::new(Cell::new(false));
        let support_hdr = config.support_hdr;

        // Well → binding: a picked color reads back as a WorkingColor with
        // linear Display-P3 channels and HDR headroom already encoded.
        let action = well.install_action({
            let well = Rc::clone(&well);
            let syncing = Rc::clone(&syncing);
            let value = config.value.clone();
            move || {
                if syncing.get() {
                    return;
                }
                #[cfg(target_os = "macos")]
                let resolved = read_well_color(&well, support_hdr);
                #[cfg(target_os = "ios")]
                let Some(resolved) = read_well_color(&well, support_hdr) else {
                    return;
                };
                value.set(Color::new(Working(resolved)));
            }
        });

        // The label is always mounted — a hidden one collapses to zero
        // size, matching `WuiColorPicker`'s always-present `labelView`.
        let label = ctx
            .render(waterui_backend_core::AnyView::new(config.label.clone()))
            .mount(host_view);
        // The semantic text is announced on the well itself.
        cocoa_ui::view::hide_from_accessibility(label.view());

        let state = Rc::new(RefCell::new(ColorPickerState {
            well,
            label: Some(label),
            syncing,
            color_guard: None,
            _action: action,
        }));

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state.borrow())
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            ColorPickerSubView {
                state: Rc::clone(&state),
            },
        );

        // Binding → well: each `Color` the binding produces is resolved in
        // this environment and observed — `observeColor` re-arming on every
        // `Color` the binding emits, as `WuiColorPicker` does.
        leaf.bind(&config.value, {
            let state = Rc::clone(&state);
            let env = ctx.env().clone();
            move |color| {
                let resolved = color.resolve(&env);
                let apply = |resolved: &WorkingColor| {
                    let state = state.borrow();
                    state.syncing.set(true);
                    state.well.set_color(&platform_color(resolved, support_hdr));
                    state.syncing.set(false);
                };
                apply(&resolved.snapshot());
                let guard = resolved.watch({
                    let state = Rc::clone(&state);
                    move |ctx| {
                        let state = state.borrow();
                        state.syncing.set(true);
                        state
                            .well
                            .set_color(&platform_color(ctx.value(), support_hdr));
                        state.syncing.set(false);
                    }
                });
                state.borrow_mut().color_guard = Some(guard);
            }
        });

        // The label's semantic text is announced on the well.
        let accessibility_label = config.label.accessibility_label();
        leaf.bind(&accessibility_label, {
            let state = Rc::clone(&state);
            move |styled| {
                cocoa_ui::view::set_accessibility_label(
                    state.borrow().well.view(),
                    &accessibility_text(&styled),
                );
            }
        });

        // A disabled subtree must not respond to input.
        if let Some(disabled) = ctx.env().get::<Disabled>() {
            leaf.bind(disabled.signal(), {
                let state = Rc::clone(&state);
                move |is_disabled: bool| state.borrow().well.set_enabled(!is_disabled)
            });
        }

        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_text_strips_bidi_marks() {
        assert_eq!(
            accessibility_text(&StyledStr::from("a\u{202a}b\u{202c}")),
            "ab"
        );
    }

    #[test]
    fn linear_rgb_roundtrip_stays_in_unit_range() {
        // sRGB gamma conversion is monotone over the unit interval the
        // color well can return.
        assert!((0.0..=1.0).contains(&srgb_to_linear(0.0)));
        assert!((0.0..=1.0).contains(&srgb_to_linear(1.0)));
        assert!(srgb_to_linear(0.5) < 0.5);
    }
}
