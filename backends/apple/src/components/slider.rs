//! The `slider` leaf: `Native<SliderConfig>` rendered as a container view
//! holding the platform slider, its top label, and the min/max value
//! labels beside the track.
//!
//! Mirrors `WuiSlider`: the label row plus the slider row give the leaf a
//! fixed intrinsic height while it stretches horizontally, and spacing
//! beside a label that measures empty collapses — a hidden label leaves a
//! bare 31pt track on iOS, exactly like `Slider(...).labelsHidden()`.
//!
//! The `Binding<f64>` is two-way: watchers push value changes onto the
//! platform control (through an animation group when the metadata carries
//! one), and the control's action writes user edits back into the binding.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use cocoa_ui::slider::ValueAnimation;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::animation::Animation;
use waterui::component::slider::SliderConfig;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::components::control_size::platform_control_size;
use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, Slider};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::{HostView, Slider};
}

use platform::{HostView, Slider};

/// The gap between the label row and the slider row.
const VERTICAL_SPACING: f64 = 4.0;
/// The gap between a value label and the track.
const HORIZONTAL_SPACING: f64 = 8.0;
/// The narrowest track the leaf will report: below this the thumb is
/// unusable, matching `WuiSlider`'s `minSliderTrackWidth`.
const MIN_TRACK_WIDTH: f64 = 50.0;

/// The track height `SwiftUI`'s `Slider` reports on iOS — `UISlider` draws
/// the same track and thumb centered in a 34pt frame, so the extra 3pt is
/// pure padding and the parity answer pins 31pt instead.
#[cfg(target_os = "ios")]
const SLIDER_TRACK_HEIGHT: f64 = 31.0;

/// The height the leaf charges for the track — the control's own intrinsic
/// answer on macOS, the `SwiftUI` track height on iOS.
#[cfg(target_os = "ios")]
const fn track_height(_slider: &Slider) -> f64 {
    SLIDER_TRACK_HEIGHT
}

/// The height the leaf charges for the track — the control's own intrinsic
/// answer on macOS, the `SwiftUI` track height on iOS.
#[cfg(target_os = "macos")]
fn track_height(slider: &Slider) -> f64 {
    slider.intrinsic_height()
}

/// A label that measures empty — hidden through `LabelDisplayMode::Hidden`
/// or simply absent — contributes no spacing beside it either.
fn spacing(spacing: f64, beside: f64) -> f64 {
    if beside > 0.0 { spacing } else { 0.0 }
}

/// The measured child extents the layout pass needs: one intrinsic
/// measurement per child, shared between `measure` and `layout`.
#[derive(Debug, Clone, Copy, Default)]
struct ChildSizes {
    label: Size,
    min_label: Size,
    max_label: Size,
}

impl ChildSizes {
    /// `WuiSlider`'s intrinsic height: the label row plus the tallest of the
    /// track and the two value labels.
    fn intrinsic_height(self, slider_height: f64) -> f64 {
        let row = slider_height
            .max(f64::from(self.min_label.height))
            .max(f64::from(self.max_label.height));
        f64::from(self.label.height) + spacing(VERTICAL_SPACING, f64::from(self.label.height)) + row
    }

    /// `WuiSlider`'s minimum width: the wider of the label and a 50pt track
    /// flanked by the value labels.
    fn min_width(self) -> f64 {
        let min = f64::from(self.min_label.width);
        let max = f64::from(self.max_label.width);
        (min + spacing(HORIZONTAL_SPACING, min)
            + MIN_TRACK_WIDTH
            + spacing(HORIZONTAL_SPACING, max)
            + max)
            .max(f64::from(self.label.width))
    }
}

/// The leaf's live state: the platform slider and the three mounted child
/// views, shared between the watchers, the layout face and the host's
/// layout handler.
struct SliderState {
    /// The platform slider.
    slider: Retained<Slider>,
    /// The rendered label, min-value and max-value children. `Option` only
    /// because the host's layout handler is installed before the children
    /// exist; a live `SliderState` always holds `Some`.
    children: Option<Children>,
    /// Keeps the control's action target alive; the field is never read.
    _action: cocoa_ui::ActionTarget,
}

/// The three children mounted on the container.
struct Children {
    label: Mounted,
    min_label: Mounted,
    max_label: Mounted,
}

/// The intrinsic size each mounted child reports under an unspecified
/// proposal — the same offer `WuiSlider` gives its labels.
fn child_sizes(children: &Children) -> ChildSizes {
    ChildSizes {
        label: children
            .label
            .layout()
            .measure(ProposalSize::UNSPECIFIED)
            .size,
        min_label: children
            .min_label
            .layout()
            .measure(ProposalSize::UNSPECIFIED)
            .size,
        max_label: children
            .max_label
            .layout()
            .measure(ProposalSize::UNSPECIFIED)
            .size,
    }
}

/// Lays out the children inside `view`'s bounds: the label at top-leading,
/// then the slider row — min label, track, max label — below it. Manual
/// frames replicate `WuiSlider`'s `AutoLayout` constraints, with the whole
/// arrangement mirrored when the view is right-to-left (leading and
/// trailing swap edges).
fn layout_children(view: &PlatformView, state: &SliderState) {
    let Some(children) = &state.children else {
        return;
    };
    let bounds = cocoa_ui::view::bounds(view);
    let width = bounds.size.width;
    let sizes = child_sizes(children);
    let slider_height = track_height(&state.slider);
    let rtl = cocoa_ui::view::is_right_to_left(view);

    // Mirroring the frames reproduces the leading/trailing anchors a
    // right-to-left `NSLayoutConstraint` pass would give.
    let place = |child: &PlatformView, x: f64, y: f64, w: f64, h: f64| {
        let x = if rtl { width - x - w } else { x };
        cocoa_ui::view::set_frame(child, Rect::new(x, y, w, h));
    };

    place(
        children.label.view(),
        0.0,
        0.0,
        f64::from(sizes.label.width),
        f64::from(sizes.label.height),
    );

    let row_top =
        f64::from(sizes.label.height) + spacing(VERTICAL_SPACING, f64::from(sizes.label.height));
    let min_w = f64::from(sizes.min_label.width);
    let max_w = f64::from(sizes.max_label.width);
    let slider_x = min_w + spacing(HORIZONTAL_SPACING, min_w);
    let slider_right = width - max_w - spacing(HORIZONTAL_SPACING, max_w);
    let slider_center_y = row_top + slider_height / 2.0;

    place(
        children.min_label.view(),
        0.0,
        slider_center_y - f64::from(sizes.min_label.height) / 2.0,
        min_w,
        f64::from(sizes.min_label.height),
    );
    place(
        children.max_label.view(),
        width - max_w,
        slider_center_y - f64::from(sizes.max_label.height) / 2.0,
        max_w,
        f64::from(sizes.max_label.height),
    );
    place(
        as_view(&state.slider),
        slider_x,
        row_top,
        (slider_right - slider_x).max(0.0),
        slider_height,
    );
}

/// The container's layout face: reports `WuiSlider.sizeThatFits`'s answer —
/// the proposed width (never below the minimum) and always the intrinsic
/// height, stretching horizontally at priority 0.
struct SliderSubView {
    state: Rc<RefCell<SliderState>>,
}

impl core::fmt::Debug for SliderSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SliderSubView").finish_non_exhaustive()
    }
}

impl SubView for SliderSubView {
    // `measure` speaks f32; the geometry math runs in f64 — the narrowing is
    // the layout contract, as in `text`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        let (sizes, slider_height) = match &state.children {
            Some(children) => (child_sizes(children), track_height(&state.slider)),
            None => (ChildSizes::default(), 0.0),
        };
        let min_width = sizes.min_width();
        let intrinsic_height = sizes.intrinsic_height(slider_height);
        let width = proposal
            .width
            .map_or(min_width, |w| f64::from(w).max(min_width));
        let height = proposal
            .height
            .map_or(intrinsic_height, |h| f64::from(h).max(intrinsic_height));
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The `ValueAnimation` the watcher's metadata calls for: `None` applies the
/// change directly. `Spring` has no `AppKit` equivalent — `WuiSlider` runs it
/// as a timed animation of an estimated duration, matching
/// `withPlatformAnimation`.
fn value_animation(metadata: &Metadata) -> Option<ValueAnimation> {
    match metadata.try_get::<Animation>() {
        None => None,
        Some(Animation::Default) => Some(ValueAnimation::Duration(0.25)),
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => Some(ValueAnimation::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        }),
        Some(Animation::Spring { stiffness, damping }) => {
            let estimated = 2.0 * (1.0 / f64::from(stiffness)).sqrt() * f64::from(damping);
            Some(ValueAnimation::Duration(estimated.clamp(0.1, 2.0)))
        }
    }
}

/// A styled string as spoken text: plain text with the bidi control
/// characters interpolation inserts for layout stripped, as
/// `WuiControlAccessibility.apply` does for labels and tooltips.
fn accessibility_text(styled: &waterui::text::StyledStr) -> String {
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

/// Clamps `value` into the track's range, as `WuiSlider.clampedValue` does —
/// a binding may briefly hold an out-of-range value while range and value
/// updates race.
const fn clamped(value: f64, start: f64, end: f64) -> f64 {
    value.clamp(start, end)
}

/// The slider as its platform view, for mounting and frame placement.
fn as_view(slider: &Slider) -> &PlatformView {
    slider
}

/// Installs the `slider` handler on the dispatcher: `Native<SliderConfig>`
/// maps to a container view with the platform slider and the three label
/// children mounted in it.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<SliderConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let slider = Slider::new(mtm);
        // `Slider`'s documented default size is `ExtraSmall`; the mapping is
        // relative to it, so a bare slider draws at the platform's regular
        // track like a bare `NSSlider`/`UISlider` does.
        slider.set_control_size(platform_control_size(
            config.size,
            waterui::component::ControlSize::ExtraSmall,
        ));
        slider.set_range(*config.range.start(), *config.range.end());
        slider.set_value(
            clamped(
                config.value.snapshot(),
                *config.range.start(),
                *config.range.end(),
            ),
            None,
        );
        host.add_subview(as_view(&slider));

        let range = config.range.clone();
        let action = slider.install_action({
            let binding = config.value.clone();
            move |value| binding.set(value)
        });

        // The label's semantic text is announced on the control itself; the
        // visual label child is hidden from the accessibility tree below.
        let accessibility_label = config.label.accessibility_label();

        let host_view: &PlatformView = &host;
        let children = Children {
            label: ctx
                .render(waterui_backend_core::AnyView::new(config.label))
                .mount(host_view),
            min_label: ctx.render(config.min_value_label).mount(host_view),
            max_label: ctx.render(config.max_value_label).mount(host_view),
        };
        cocoa_ui::view::hide_from_accessibility(children.label.view());

        let state = Rc::new(RefCell::new(SliderState {
            slider: slider.clone(),
            children: Some(children),
            _action: action,
        }));

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state.borrow())
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            SliderSubView {
                state: Rc::clone(&state),
            },
        );

        // Signal → control: push binding changes (clamped into range) with
        // the animation the watcher metadata carries.
        leaf.watch(&config.value, {
            let slider = slider.clone();
            move |ctx| {
                let animation = value_animation(ctx.metadata());
                slider.set_value(
                    clamped(*ctx.value(), *range.start(), *range.end()),
                    animation,
                );
            }
        });

        // Announce the label's semantic text on the control.
        leaf.bind(&accessibility_label, {
            let slider = slider.clone();
            move |styled| {
                let text = accessibility_text(&styled);
                slider.set_accessibility_label(if text.is_empty() {
                    None
                } else {
                    Some(text.as_str())
                });
            }
        });

        // A disabled subtree must not respond to input.
        if let Some(disabled) = ctx.env().get::<Disabled>() {
            leaf.bind(disabled.signal(), {
                move |is_disabled: bool| slider.set_enabled(!is_disabled)
            });
        }

        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    #[test]
    fn spacing_collapses_beside_an_empty_label() {
        assert_eq!(spacing(4.0, 0.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(spacing(8.0, 10.0).to_bits(), 8.0_f64.to_bits());
    }

    #[test]
    fn min_width_enforces_track_and_label_extents() {
        // No labels: just the minimum track width.
        let bare = ChildSizes::default().min_width();
        assert_eq!(bare.to_bits(), MIN_TRACK_WIDTH.to_bits());
        // Both value labels: 8pt gaps flank the track.
        let labelled = ChildSizes {
            label: Size::new(120.0, 20.0),
            min_label: Size::new(20.0, 10.0),
            max_label: Size::new(30.0, 10.0),
        }
        .min_width();
        assert_eq!(
            labelled.to_bits(),
            120.0_f64.max(20.0 + 8.0 + 50.0 + 8.0 + 30.0).to_bits()
        );
    }

    #[test]
    fn intrinsic_height_stacks_label_and_row() {
        let sizes = ChildSizes {
            label: Size::new(50.0, 20.0),
            min_label: Size::new(10.0, 40.0),
            max_label: Size::new(0.0, 0.0),
        };
        // Row height follows the tallest of track and value labels.
        assert_eq!(sizes.intrinsic_height(31.0).to_bits(), 64.0_f64.to_bits());
        // An empty label contributes no spacing.
        let no_label = ChildSizes::default();
        assert_eq!(
            no_label.intrinsic_height(31.0).to_bits(),
            31.0_f64.to_bits()
        );
    }

    #[test]
    fn value_animation_follows_metadata_table() {
        assert_eq!(value_animation(&Metadata::new()), None);
        assert_eq!(
            value_animation(&Metadata::new().with(Animation::Default)),
            Some(ValueAnimation::Duration(0.25))
        );
        assert_eq!(
            value_animation(&Metadata::new().with(Animation::Bezier {
                duration: Duration::from_millis(300),
                x1: 0.1,
                y1: 0.2,
                x2: 0.3,
                y2: 0.4,
            })),
            Some(ValueAnimation::Bezier {
                duration: 0.3,
                control_points: [0.1, 0.2, 0.3, 0.4],
            })
        );
        // Spring estimates a clamped duration: 2*sqrt(1/s)*d in [0.1, 2.0].
        let spring = value_animation(&Metadata::new().with(Animation::Spring {
            stiffness: 170.0,
            damping: 15.0,
        }));
        match spring {
            Some(ValueAnimation::Duration(d)) => {
                assert!((0.1..=2.0).contains(&d));
            }
            other => panic!("expected a timed duration for spring, got {other:?}"),
        }
    }

    #[test]
    fn accessibility_text_strips_bidi_marks() {
        use waterui::text::StyledStr;
        let styled = StyledStr::plain("Volume\u{202a}up\u{202c}");
        assert_eq!(accessibility_text(&styled), "Volumeup");
    }
}
