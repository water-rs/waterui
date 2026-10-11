//! The `progress` leaf: `Native<ProgressConfig>` rendered as a container
//! view holding the platform progress indicator between two label children.
//!
//! Mirrors `WuiProgress`: the label sits above the indicator and the value
//! label below it, stacked with a fixed gap; a linear indicator stretches
//! horizontally while a circular one stays content-sized and centered.
//! `value` drives determinacy — positive infinity spins, a finite `0...1`
//! fills — and every change is an imperative kit call inside a watcher.
//!
//! The platform mapping is the Swift port's non-four-color path:
//! `NSProgressIndicator` bar/spinning on `AppKit`, `UIProgressView` +
//! `UIActivityIndicatorView` on `UIKit`. Theme tinting exists only where
//! the platform honours it — `UIKit` takes accent and track tints,
//! including the four-color cycle; `AppKit`'s indicator ignores tint, so
//! `four_color` draws the native spinner untinted there.

use alloc::rc::Rc;
#[cfg(target_os = "ios")]
use alloc::rc::Weak;
use alloc::vec::Vec;
use core::cell::RefCell;

#[cfg(target_os = "ios")]
use cocoa_ui::Retained;
use cocoa_ui::{PlatformView, Rect};
use waterui::animation::Animation;
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui::graphics::color::WorkingColor;
use waterui::reactive::watcher::Metadata;
use waterui::reactive::{Signal, SignalExt};
use waterui::resolve::Resolvable;
use waterui::theme::color::{Accent, AccentContainer, Border, Tertiary, TertiaryContainer};
use waterui_backend_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, Progress};

    /// An owned handle to the platform indicator: `Retained` on `AppKit`.
    pub(super) type SharedProgress = cocoa_ui::Retained<Progress>;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::colors;
    pub(super) use cocoa_ui::uikit::{HostView, Progress};

    /// An owned handle to the platform indicator: the wrapper itself on
    /// `UIKit`.
    pub(super) type SharedProgress = Progress;
}

use cocoa_ui::progress::ProgressVariant;
use platform::{HostView, Progress, SharedProgress};

/// The gap between the label, the indicator, and the value label —
/// `WuiProgress`'s `verticalSpacing`.
const VERTICAL_SPACING: f64 = 6.0;

/// The minimum width a linear indicator reports — `WuiProgress`'s floor of
/// `max(50, intrinsicWidth)`.
const MIN_LINEAR_WIDTH: f64 = 50.0;

/// The bar width a linear indicator falls back to when the platform
/// control reports no intrinsic width — `WuiProgress`'s bar width.
const LINEAR_BAR_WIDTH: f64 = 100.0;

/// How long each step of the four-color tint cycle holds, in seconds.
#[cfg(target_os = "ios")]
const TINT_STEP_SECONDS: u64 = 1;

/// The measured child extents the layout pass needs: one intrinsic
/// measurement per child, shared between `measure` and `layout`.
#[derive(Debug, Clone, Copy, Default)]
struct ChildSizes {
    label: Size,
    value_label: Size,
}

/// The leaf's live state: the platform indicator, the mounted children and
/// the theme palette, shared between the watchers, the layout face and the
/// host's layout handler.
struct ProgressState {
    /// The platform progress indicator.
    progress: SharedProgress,
    /// Whether the indicator draws round.
    variant: ProgressVariant,
    /// The mounted label and value-label children. `Option` only because
    /// the host's layout handler is installed before the children exist; a
    /// live `ProgressState` always holds `Some`.
    children: Option<Children>,
    /// The latest value — indeterminate reads as positive infinity.
    value: f64,
    /// The indeterminate palette: accent alone, or accent plus the
    /// container/tertiary slots when `four_color` is set.
    palette: Vec<WorkingColor>,
    /// The palette position currently applied as the tint.
    tint_index: usize,
    /// Whether the tint-cycle timer is armed (`UIKit` only — `AppKit`
    /// indicators ignore tint, so no cycle ever runs there).
    #[cfg(target_os = "ios")]
    cycling: bool,
    /// The most recent track color — the `Border` slot.
    track: Option<WorkingColor>,
    /// Whether `four_color` widens the indeterminate palette.
    #[cfg(target_os = "ios")]
    four_color: bool,
}

/// The children mounted on the container.
struct Children {
    label: Mounted,
    value_label: Mounted,
}

/// Whether `style` draws round: circular and loading both project to the
/// platform spinner — `isRound` in the Swift port.
const fn is_round(style: ProgressStyle) -> bool {
    match style {
        ProgressStyle::Circular | ProgressStyle::Loading => true,
        // `Linear` and any future style draw the platform's bar.
        _ => false,
    }
}

/// Whether the value reads as "work in flight, duration unknown" —
/// `Progress::infinity()` writes `f64::INFINITY`.
fn is_indeterminate(value: f64) -> bool {
    value == f64::INFINITY
}

/// The measured size each mounted child reports under a width-bounded
/// offer — the same proposal `WuiProgress` measures its labels with.
fn child_sizes(children: &Children, width: Option<f32>, value: f64) -> ChildSizes {
    let proposal = ProposalSize {
        width,
        height: None,
    };
    ChildSizes {
        label: children.label.layout().measure(proposal).size,
        value_label: if value.is_finite() {
            children.value_label.layout().measure(proposal).size
        } else {
            Size::default()
        },
    }
}

/// The indicator's share of the container: the spinner's intrinsic size
/// when round, the fallback bar width and the control height when linear.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; measured points always fit"
)]
fn control_size(progress: &Progress, variant: ProgressVariant) -> Size {
    let size = progress.intrinsic_size();
    match variant {
        ProgressVariant::Circular => Size::new(size.width as f32, size.height as f32),
        ProgressVariant::Linear => {
            let intrinsic = if size.width > 0.0 {
                size.width
            } else {
                LINEAR_BAR_WIDTH
            };
            Size::new(intrinsic.max(MIN_LINEAR_WIDTH) as f32, size.height as f32)
        }
    }
}

/// `WuiProgress.sizeThatFits`'s width: the proposed width (never below the
/// floor) for a linear indicator, the intrinsic width for a round one.
fn measure_width(style_width: ProgressVariant, proposal: ProposalSize, intrinsic: f64) -> f64 {
    match style_width {
        ProgressVariant::Linear => {
            let floor = MIN_LINEAR_WIDTH.max(intrinsic);
            proposal.width.map_or(floor, |w| f64::from(w).max(floor))
        }
        ProgressVariant::Circular => intrinsic,
    }
}

/// The stacked height `WuiProgress.sizeThatFits` reports: children and the
/// control joined by a gap wherever the neighbor above is non-empty.
fn measure_height(sizes: ChildSizes, control: Size) -> f64 {
    let mut height = 0.0;
    if sizes.label.height > 0.0 {
        height += f64::from(sizes.label.height) + VERTICAL_SPACING;
    }
    height += f64::from(control.height);
    if f64::from(sizes.value_label.height) > 0.0 {
        height += VERTICAL_SPACING + f64::from(sizes.value_label.height);
    }
    height
}

/// Lays out the children inside `view`'s bounds: label at the top,
/// indicator centered when round or full-width when linear, value label at
/// the bottom — `WuiProgress.performLayout` as manual frames.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; measured points always fit"
)]
fn layout_children(view: &PlatformView, state: &ProgressState) {
    let Some(children) = &state.children else {
        return;
    };
    let width = cocoa_ui::view::bounds(view).size.width;
    let sizes = child_sizes(children, Some(width as f32), state.value);
    let control = control_size(&state.progress, state.variant);

    let mut y = 0.0;
    let label_width = f64::from(sizes.label.width).min(width);
    cocoa_ui::view::set_frame(
        children.label.view(),
        Rect::new(0.0, y, label_width, f64::from(sizes.label.height)),
    );
    y += f64::from(sizes.label.height);
    if sizes.label.height > 0.0 {
        y += VERTICAL_SPACING;
    }

    let (control_x, control_width) = match state.variant {
        ProgressVariant::Circular => (
            (width - f64::from(control.width)) / 2.0,
            f64::from(control.width),
        ),
        ProgressVariant::Linear => (0.0, width),
    };
    cocoa_ui::view::set_frame(
        as_view(&state.progress),
        Rect::new(control_x, y, control_width, f64::from(control.height)),
    );
    y += f64::from(control.height);

    if sizes.value_label.height > 0.0 {
        y += VERTICAL_SPACING;
        let value_width = f64::from(sizes.value_label.width).min(width);
        cocoa_ui::view::set_frame(
            children.value_label.view(),
            Rect::new(0.0, y, value_width, f64::from(sizes.value_label.height)),
        );
    } else {
        cocoa_ui::view::set_frame(children.value_label.view(), Rect::ZERO);
    }
}

/// The container's layout face: reports `WuiProgress.sizeThatFits`'s
/// answer and stretches horizontally only for the linear variant —
/// `Progress`'s own stretch axis.
struct ProgressSubView {
    state: Rc<RefCell<ProgressState>>,
}

impl core::fmt::Debug for ProgressSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProgressSubView").finish_non_exhaustive()
    }
}

impl SubView for ProgressSubView {
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
            .map_or_else(ChildSizes::default, |children| {
                child_sizes(children, proposal.width, state.value)
            });
        let control = control_size(&state.progress, state.variant);
        let intrinsic_width = f64::from(
            sizes
                .label
                .width
                .max(sizes.value_label.width)
                .max(control.width),
        );
        ViewDimensions::new(Size::new(
            measure_width(state.variant, proposal, intrinsic_width) as f32,
            measure_height(sizes, control) as f32,
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        match self.state.borrow().variant {
            ProgressVariant::Linear => StretchAxis::Horizontal,
            ProgressVariant::Circular => StretchAxis::None,
        }
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The indicator as its platform view, for mounting and frame placement.
#[cfg(target_os = "macos")]
fn as_view(progress: &Progress) -> &PlatformView {
    progress
}

#[cfg(target_os = "ios")]
fn as_view(progress: &Progress) -> &PlatformView {
    progress.container()
}

/// Whether `metadata` asks for an animated fill — `WuiProgress` passed the
/// flag through `UIProgressView.setProgress(_:animated:)`.
fn should_animate(metadata: &Metadata) -> bool {
    metadata.try_get::<Animation>().is_some()
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object — the
/// `allowHdr: false` variant `wuiProgressPlatformColor` produced.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// Applies the current palette position as the indicator's tint — a no-op
/// on `AppKit`, whose indicator ignores tint.
fn apply_tint(state: &ProgressState) {
    let Some(color) = state
        .palette
        .get(state.tint_index % state.palette.len().max(1))
    else {
        return;
    };
    #[cfg(target_os = "ios")]
    state.progress.set_tint(&platform_color(color));
    #[cfg(target_os = "macos")]
    let _ = color;
}

/// Applies the track color — `UIProgressView`'s `trackTintColor`.
#[cfg(target_os = "ios")]
fn apply_track(state: &ProgressState) {
    let Some(color) = &state.track else {
        return;
    };
    state.progress.set_track_tint(&platform_color(color));
}

/// `AppKit` indicators ignore tint, so the track color never applies.
#[cfg(target_os = "macos")]
const fn apply_track(_state: &ProgressState) {}

/// Pushes a new value onto the indicator: indeterminate spins, a finite
/// `0...1` fills — `applyValue` in the Swift port.
fn apply_value(
    state: &Rc<RefCell<ProgressState>>,
    host: &PlatformView,
    value: f64,
    animated: bool,
) {
    assert!(
        is_indeterminate(value) || (value.is_finite() && (0.0..=1.0).contains(&value)),
        "Progress value must be finite within 0...1 or positive infinity"
    );
    let indeterminate = is_indeterminate(value);
    // `animated` reaches the kit only where the control supports it.
    #[cfg(target_os = "macos")]
    let _ = animated;

    let value_label = {
        let mut state = state.borrow_mut();
        state.value = value;
        state.progress.set_indeterminate(indeterminate);
        if indeterminate {
            #[cfg(target_os = "macos")]
            state.progress.start_animation();
        } else {
            #[cfg(target_os = "macos")]
            {
                state.progress.stop_animation();
                state.progress.set_value(value);
            }
            #[cfg(target_os = "ios")]
            state.progress.set_progress(value, animated);
        }
        state
            .children
            .as_ref()
            .map(|children| cocoa_ui::view::retain_base(children.value_label.view()))
    };

    // `valueLabelView.isHidden = isIndeterminate`.
    if let Some(value_label) = &value_label {
        cocoa_ui::view::set_hidden(value_label, indeterminate);
    }
    cocoa_ui::view::invalidate_layout(host);
    update_tint_cycle(state, host);
}

/// Starts or stops the four-color tint cycle to match
/// `updateTintCycleState`: it runs only while an indeterminate,
/// `four_color` indicator sits in a window. `AppKit` never cycles — its
/// indicator ignores tint — so the scheduling exists on `UIKit` alone.
#[cfg(target_os = "ios")]
fn update_tint_cycle(state: &Rc<RefCell<ProgressState>>, host: &PlatformView) {
    let mut state_mut = state.borrow_mut();
    let should_cycle = cycling_condition(&state_mut, host);
    if should_cycle && !state_mut.cycling {
        state_mut.cycling = true;
        let host = cocoa_ui::view::retain_base(host);
        drop(state_mut);
        schedule_tint_tick(Rc::downgrade(state), host);
    } else if !should_cycle && state_mut.cycling {
        state_mut.cycling = false;
        state_mut.tint_index = 0;
        apply_tint(&state_mut);
    }
}

/// `AppKit` indicators ignore tint, so no cycle ever runs.
#[cfg(target_os = "macos")]
const fn update_tint_cycle(_state: &Rc<RefCell<ProgressState>>, _host: &PlatformView) {}

/// Whether the tint cycle should be running: the `fourColor &&
/// value == .infinity && window != nil` conjunction of the Swift port.
#[cfg(target_os = "ios")]
fn cycling_condition(state: &ProgressState, host: &PlatformView) -> bool {
    state.four_color && is_indeterminate(state.value) && cocoa_ui::view::has_window(host)
}

/// Arms the next step of the tint cycle on the main queue, one second
/// out. `state` travels weak: once the leaf drops, the cycle dies instead
/// of writing into dead state.
#[cfg(target_os = "ios")]
fn schedule_tint_tick(state: Weak<RefCell<ProgressState>>, host: Retained<PlatformView>) {
    use dispatch2::{DispatchQueue, DispatchTime};
    let work = dispatch2::MainThreadBound::new(
        move |_| tint_tick(&state, &host),
        // The scheduling sites all run on the main thread.
        cocoa_ui::MainThreadMarker::new().expect("the tint cycle is main-thread work"),
    );
    let Ok(when) = DispatchTime::try_from(core::time::Duration::from_secs(TINT_STEP_SECONDS))
    else {
        return;
    };
    // `after` delivers the block on the main queue; a stopped cycle just
    // sees the flag clear and does not re-arm.
    let _ = DispatchQueue::main().after(when, move || {
        let mtm = cocoa_ui::MainThreadMarker::new()
            .expect("the main dispatch queue runs its work on the main thread");
        work.into_inner(mtm)(mtm);
    });
}

/// One step of the tint cycle: advances the palette and re-arms, or stops
/// when the cycling condition no longer holds — `tintCycleTask`'s loop.
#[cfg(target_os = "ios")]
fn tint_tick(state: &Weak<RefCell<ProgressState>>, host: &PlatformView) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let mut state_mut = state.borrow_mut();
    if !state_mut.cycling || !cycling_condition(&state_mut, host) {
        state_mut.cycling = false;
        state_mut.tint_index = 0;
        apply_tint(&state_mut);
        return;
    }
    state_mut.tint_index = (state_mut.tint_index + 1) % state_mut.palette.len().max(1);
    apply_tint(&state_mut);
    drop(state_mut);
    schedule_tint_tick(Rc::downgrade(&state), cocoa_ui::view::retain_base(host));
}

/// Installs the `progress` handler on the dispatcher: `Native<ProgressConfig>`
/// maps to a container view with the platform indicator and the label pair
/// `WuiProgress` stacks around it.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ProgressConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let variant = if is_round(config.style) {
            ProgressVariant::Circular
        } else {
            ProgressVariant::Linear
        };
        let progress = Progress::new(mtm);
        progress.set_variant(variant);
        // `SwiftUI` draws a circular indicator at the small control size on
        // macOS; the kit leaves the size to the backend.
        #[cfg(target_os = "macos")]
        if variant == ProgressVariant::Circular {
            progress.set_control_size(cocoa_ui::slider::ControlSize::Small);
        }
        cocoa_ui::view::add_subview(host_view, as_view(&progress));

        let value = config.value.snapshot();
        let children = Children {
            label: ctx
                .render(waterui_backend_core::AnyView::new(config.label))
                .mount(host_view),
            value_label: ctx
                .render(waterui_backend_core::AnyView::new(config.value_label))
                .mount(host_view),
        };

        let state = Rc::new(RefCell::new(ProgressState {
            progress,
            variant,
            children: Some(children),
            value,
            palette: Vec::new(),
            tint_index: 0,
            #[cfg(target_os = "ios")]
            cycling: false,
            track: None,
            #[cfg(target_os = "ios")]
            four_color: config.four_color,
        }));

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state.borrow())
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            ProgressSubView {
                state: Rc::clone(&state),
            },
        );

        // Signal → indicator: a value change selects determinate or
        // indeterminate drawing and refreshes the value label's visibility.
        leaf.watch(&config.value, {
            let state = Rc::clone(&state);
            let host = cocoa_ui::view::retain_base(host_view);
            move |change| {
                let animated = should_animate(change.metadata());
                apply_value(&state, &host, *change.value(), animated);
            }
        });

        // Theme → tint: the accent is the indicator's color; the border is
        // the bar's track. `four_color` widens the palette with the
        // container and tertiary slots the cycle steps through.
        install_tint(&mut leaf, &state, ctx.env(), config.four_color);

        // The initial value applies without animation.
        apply_value(&state, host_view, value, false);

        leaf.keep(state);
        leaf
    });
}

/// Subscribes the theme signals the tint and track follow.
fn install_tint(
    leaf: &mut NativeLeaf,
    state: &Rc<RefCell<ProgressState>>,
    env: &Environment,
    four_color: bool,
) {
    let accent = Accent.resolve(env);
    {
        let mut state_mut = state.borrow_mut();
        state_mut.palette = Vec::from([accent.snapshot()]);
    }
    leaf.bind(&accent, {
        let state = Rc::clone(state);
        move |color| {
            let mut state_mut = state.borrow_mut();
            state_mut.palette[0] = color;
            apply_tint(&state_mut);
        }
    });
    if four_color {
        let extra = [
            AccentContainer.resolve(env).computed(),
            Tertiary.resolve(env).computed(),
            TertiaryContainer.resolve(env).computed(),
        ];
        {
            let mut state_mut = state.borrow_mut();
            state_mut.palette.extend(extra.iter().map(Signal::snapshot));
        }
        for (index, signal) in extra.into_iter().enumerate() {
            leaf.bind(&signal, {
                let state = Rc::clone(state);
                move |color| {
                    let mut state_mut = state.borrow_mut();
                    state_mut.palette[index + 1] = color;
                    apply_tint(&state_mut);
                }
            });
        }
    }
    let border = Border.resolve(env);
    {
        let mut state_mut = state.borrow_mut();
        state_mut.track = Some(border.snapshot());
        apply_track(&state_mut);
    }
    leaf.bind(&border, {
        let state = Rc::clone(state);
        move |color| {
            let mut state_mut = state.borrow_mut();
            state_mut.track = Some(color);
            apply_track(&state_mut);
        }
    });
    apply_tint(&state.borrow());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_round_covers_circular_and_loading() {
        assert!(!is_round(ProgressStyle::Linear));
        assert!(is_round(ProgressStyle::Circular));
        assert!(is_round(ProgressStyle::Loading));
    }

    #[test]
    fn is_indeterminate_is_positive_infinity_only() {
        assert!(is_indeterminate(f64::INFINITY));
        assert!(!is_indeterminate(f64::NEG_INFINITY));
        assert!(!is_indeterminate(0.5));
        assert!(!is_indeterminate(f64::NAN));
    }

    #[test]
    fn measure_width_follows_the_variant() {
        // Linear: the proposal wins when it beats the floor.
        let linear = measure_width(
            ProgressVariant::Linear,
            ProposalSize {
                width: Some(200.0),
                height: None,
            },
            80.0,
        );
        assert_eq!(linear.to_bits(), 200.0_f64.to_bits());
        // Linear: a small proposal or none clamps to the floor.
        let floor = measure_width(
            ProgressVariant::Linear,
            ProposalSize {
                width: Some(20.0),
                height: None,
            },
            80.0,
        );
        assert_eq!(floor.to_bits(), 80.0_f64.to_bits());
        // Circular: the proposal never stretches a round indicator.
        let round = measure_width(
            ProgressVariant::Circular,
            ProposalSize {
                width: Some(200.0),
                height: None,
            },
            20.0,
        );
        assert_eq!(round.to_bits(), 20.0_f64.to_bits());
    }

    #[test]
    fn measure_height_stacks_with_gaps() {
        let sizes = ChildSizes {
            label: Size::new(40.0, 16.0),
            value_label: Size::new(20.0, 12.0),
        };
        let control = Size::new(100.0, 4.0);
        let height = measure_height(sizes, control);
        assert_eq!(
            height.to_bits(),
            (16.0_f64 + 6.0 + 4.0 + 6.0 + 12.0).to_bits()
        );
        // An empty label contributes no gap.
        let bare = measure_height(ChildSizes::default(), control);
        assert_eq!(bare.to_bits(), 4.0_f64.to_bits());
    }

    #[test]
    fn should_animate_only_with_animation_metadata() {
        assert!(!should_animate(&Metadata::new()));
        assert!(should_animate(&Metadata::new().with(Animation::Default)));
    }
}
