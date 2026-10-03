//! The `stepper` leaf: `Native<StepperConfig>` rendered as a container view
//! holding the platform stepper, its label child, and an optional
//! formatted-value child beside the buttons.
//!
//! Mirrors `WuiStepper`: the label hugs the leading edge, the stepper hugs
//! the trailing edge, and the formatted value sits between them — the row
//! stretches horizontally while its height stays intrinsic. The
//! `Binding<i32>` is two-way: watchers push value changes onto the control,
//! and the control's action writes user steps back into the binding.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::component::stepper::StepperConfig;
use waterui::reactive::Signal;
use waterui::text::{StyledStr, TextConfig};
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, Stepper};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::{HostView, Stepper};
}

use platform::{HostView, Stepper};

/// The gap between the label, the formatted value, and the buttons —
/// `WuiStepper`'s `spacing`.
const SPACING: f64 = 8.0;

/// The measured child extents the layout pass needs: one intrinsic
/// measurement per child, shared between `measure` and `layout`.
#[derive(Debug, Clone, Copy, Default)]
struct ChildSizes {
    label: Size,
    formatted: Size,
}

/// The leaf's live state: the platform stepper and the mounted children,
/// shared between the watchers, the layout face and the host's layout
/// handler.
struct StepperState {
    /// The platform stepper.
    stepper: Retained<Stepper>,
    /// The mounted label and formatted-value children; `Some` while the
    /// formatter exists. `Option` only because the host's layout handler is
    /// installed before the children exist; a live `StepperState` always
    /// holds `Some`.
    children: Option<Children>,
    /// The latest formatted value as spoken text — the accessibility value
    /// the control announces instead of the raw number.
    formatted_text: Option<String>,
    /// Keeps the control's action target alive; the field is never read.
    _action: cocoa_ui::ActionTarget,
}

/// The children mounted on the container.
struct Children {
    label: Mounted,
    formatted: Option<Mounted>,
}

/// The intrinsic size each mounted child reports under an unspecified
/// proposal — the same offer `WuiStepper` gives its labels.
fn child_sizes(children: &Children) -> ChildSizes {
    ChildSizes {
        label: children
            .label
            .layout()
            .measure(ProposalSize::UNSPECIFIED)
            .size,
        formatted: children
            .formatted
            .as_ref()
            .map_or_else(Size::default, |mounted| {
                mounted.layout().measure(ProposalSize::UNSPECIFIED).size
            }),
    }
}

/// The minimum row width `WuiStepper.sizeThatFits` reports: the buttons
/// plus, when present and non-empty, the formatted value and the label with
/// their spacing.
fn min_width(sizes: ChildSizes, stepper_width: f64) -> f64 {
    let mut width = stepper_width;
    if sizes.formatted.width > 0.0 {
        width += SPACING + f64::from(sizes.formatted.width);
    }
    if sizes.label.width > 0.0 && sizes.label.height > 0.0 {
        width += SPACING + f64::from(sizes.label.width);
    }
    width
}

/// `WuiStepper.sizeThatFits`'s height: the tallest of buttons, label and
/// formatted value.
fn intrinsic_height(sizes: ChildSizes, stepper_height: f64) -> f64 {
    stepper_height
        .max(f64::from(sizes.label.height))
        .max(f64::from(sizes.formatted.height))
}

/// Lays out the children inside `view`'s bounds: label at the leading edge,
/// stepper at the trailing edge, formatted value immediately before the
/// buttons — `WuiStepper`'s `AutoLayout` constraints as manual frames, with
/// the arrangement mirrored when the view is right-to-left.
fn layout_children(view: &PlatformView, state: &StepperState) {
    let Some(children) = &state.children else {
        return;
    };
    let width = cocoa_ui::view::bounds(view).size.width;
    let height = cocoa_ui::view::bounds(view).size.height;
    let sizes = child_sizes(children);
    let stepper_size = state.stepper.intrinsic_size();
    let rtl = cocoa_ui::view::is_right_to_left(view);

    let place = |child: &PlatformView, x: f64, w: f64, h: f64| {
        let x = if rtl { width - x - w } else { x };
        cocoa_ui::view::set_frame(child, Rect::new(x, (height - h) / 2.0, w, h));
    };

    let stepper_x = width - stepper_size.width;
    let leading_limit = children
        .formatted
        .as_ref()
        .map_or(stepper_x - SPACING, |formatted| {
            let formatted_width = f64::from(sizes.formatted.width);
            let formatted_x = stepper_x - SPACING - formatted_width;
            place(
                formatted.view(),
                formatted_x.max(0.0),
                formatted_width,
                f64::from(sizes.formatted.height),
            );
            formatted_x - SPACING
        });
    let label_width = f64::from(sizes.label.width).min(leading_limit.max(0.0));
    place(
        children.label.view(),
        0.0,
        label_width,
        f64::from(sizes.label.height),
    );
    let stepper_frame_x = if rtl { 0.0 } else { stepper_x };
    cocoa_ui::view::set_frame(
        &state.stepper,
        Rect::new(
            stepper_frame_x,
            (height - stepper_size.height) / 2.0,
            stepper_size.width,
            stepper_size.height,
        ),
    );
}

/// The container's layout face: reports `WuiStepper.sizeThatFits`'s answer —
/// the proposed width (never below the minimum) when a label shares the
/// row, the minimum otherwise; always the intrinsic height. Stretches
/// horizontally at priority 0.
struct StepperSubView {
    state: Rc<RefCell<StepperState>>,
}

impl core::fmt::Debug for StepperSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StepperSubView").finish_non_exhaustive()
    }
}

impl SubView for StepperSubView {
    // `measure` speaks f32; the geometry math runs in f64 — the narrowing is
    // the layout contract, as in `text`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        let sizes = state
            .children
            .as_ref()
            .map_or_else(ChildSizes::default, child_sizes);
        let stepper_size = state.stepper.intrinsic_size();
        let min_width = min_width(sizes, stepper_size.width);
        let has_label = sizes.label.width > 0.0 && sizes.label.height > 0.0;
        let width = if has_label {
            proposal
                .width
                .map_or(min_width, |w| f64::from(w).max(min_width))
        } else {
            min_width
        };
        ViewDimensions::new(Size::new(
            width as f32,
            intrinsic_height(sizes, stepper_size.height) as f32,
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The stepper as its platform view, for mounting and frame placement.
fn as_view(stepper: &Stepper) -> &PlatformView {
    stepper
}

/// A platform value as the `i32` the binding speaks — `Int32(stepper.value)`
/// truncates the same way.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the binding is i32; the control speaks f64 and stepping yields whole values"
)]
const fn stepped(value: f64) -> i32 {
    value as i32
}

/// What the control announces: the formatted text when the row has one, the
/// raw number otherwise — `applyAccessibilityValue`'s table.
fn accessibility_value(formatted: Option<&String>, value: i32) -> String {
    formatted.map_or_else(|| value.to_string(), Clone::clone)
}

/// Installs the `stepper` handler on the dispatcher: `Native<StepperConfig>`
/// maps to a container view with the platform stepper, the label child and,
/// when a `value_formatter` is present, a text child showing the formatted
/// value.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<StepperConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let stepper = Stepper::new(mtm);
        stepper.set_range(
            f64::from(*config.range.start()),
            f64::from(*config.range.end()),
        );
        stepper.set_step(f64::from(config.step.snapshot()));
        stepper.set_value(f64::from(config.value.snapshot()));
        host.add_subview(as_view(&stepper));

        let action = stepper.install_action({
            let binding = config.value.clone();
            move |value| binding.set(stepped(value))
        });

        // The label's semantic text is announced on the control itself; the
        // visual children are hidden from the accessibility tree.
        let accessibility_label = config.label.accessibility_label();

        let host_view: &PlatformView = &host;
        let value_formatter = config.value_formatter;
        let children = Children {
            label: ctx
                .render(waterui_backend_core::AnyView::new(config.label))
                .mount(host_view),
            formatted: value_formatter.clone().map(|formatter| {
                ctx.render(waterui_backend_core::AnyView::new(
                    waterui_core::Native::new(TextConfig::new(formatter)),
                ))
                .mount(host_view)
            }),
        };
        cocoa_ui::view::hide_from_accessibility(children.label.view());
        if let Some(formatted) = &children.formatted {
            cocoa_ui::view::hide_from_accessibility(formatted.view());
        }

        let state = Rc::new(RefCell::new(StepperState {
            stepper: stepper.clone(),
            children: Some(children),
            formatted_text: None,
            _action: action,
        }));

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state.borrow())
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            StepperSubView {
                state: Rc::clone(&state),
            },
        );

        // Signal → control: binding changes land on the stepper and refresh
        // the announced value.
        leaf.watch(&config.value, {
            let state = Rc::clone(&state);
            move |change| {
                let value = *change.value();
                let state = state.borrow();
                state.stepper.set_value(f64::from(value));
                state.stepper.set_accessibility_value(&accessibility_value(
                    state.formatted_text.as_ref(),
                    value,
                ));
            }
        });

        // The step size is computed: every change re-arms the control.
        leaf.bind(&config.step, {
            let stepper = stepper.clone();
            move |step| stepper.set_step(f64::from(step))
        });

        // Announce the label's semantic text on the control.
        leaf.bind(&accessibility_label, {
            let stepper = stepper.clone();
            move |styled: StyledStr| {
                let text = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
                stepper.set_accessibility_label(if text.is_empty() {
                    None
                } else {
                    Some(text.as_str())
                });
            }
        });

        // The formatted value drives the spoken value too; the state keeps
        // the latest plain text so value changes announce formatted text.
        if let Some(formatter) = value_formatter {
            leaf.bind(&formatter, {
                let state = Rc::clone(&state);
                move |styled: StyledStr| {
                    let text = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
                    let mut state = state.borrow_mut();
                    state.formatted_text = Some(text.clone());
                    let value = stepped(state.stepper.value());
                    state
                        .stepper
                        .set_accessibility_value(&accessibility_value(Some(&text), value));
                }
            });
        }

        // A disabled subtree must not respond to input.
        if let Some(disabled) = ctx.env().get::<Disabled>() {
            leaf.bind(disabled.signal(), move |is_disabled: bool| {
                stepper.set_enabled(!is_disabled);
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
    fn min_width_stacks_children_with_spacing() {
        // Bare: just the buttons.
        let bare = min_width(ChildSizes::default(), 94.0);
        assert_eq!(bare.to_bits(), 94.0_f64.to_bits());
        // Label + formatted value: an 8pt gap per present child.
        let full = min_width(
            ChildSizes {
                label: Size::new(60.0, 20.0),
                formatted: Size::new(30.0, 20.0),
            },
            94.0,
        );
        assert_eq!(
            full.to_bits(),
            (94.0_f64 + 8.0 + 30.0 + 8.0 + 60.0).to_bits()
        );
        // A zero-height label counts as absent — `hasLabel` in the Swift
        // measure requires both extents.
        let flat_label = min_width(
            ChildSizes {
                label: Size::new(60.0, 0.0),
                formatted: Size::new(0.0, 0.0),
            },
            94.0,
        );
        assert_eq!(flat_label.to_bits(), 94.0_f64.to_bits());
    }

    #[test]
    fn intrinsic_height_takes_the_tallest() {
        let sizes = ChildSizes {
            label: Size::new(10.0, 40.0),
            formatted: Size::new(10.0, 25.0),
        };
        assert_eq!(intrinsic_height(sizes, 22.0).to_bits(), 40.0_f64.to_bits());
        assert_eq!(
            intrinsic_height(ChildSizes::default(), 22.0).to_bits(),
            22.0_f64.to_bits()
        );
    }

    #[test]
    fn stepped_truncates_like_int32_of_double() {
        assert_eq!(stepped(3.0), 3);
        assert_eq!(stepped(-2.0), -2);
    }

    #[test]
    fn accessibility_value_prefers_the_formatted_text() {
        let formatted = String::from("3 items");
        assert_eq!(accessibility_value(Some(&formatted), 3), "3 items");
        assert_eq!(accessibility_value(None, 3), "3");
    }
}
