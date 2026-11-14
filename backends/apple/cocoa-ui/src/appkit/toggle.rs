//! The `AppKit` toggle: an `NSSwitch` or an `NSButton` checkbox, together
//! with the labelled row a toggle lays out.
//!
//! `Toggle` owns a [`ToggleRow`] container holding the control and an
//! optional label, arranged by kind — a checkbox reads box first, then its
//! title; a switch puts its title at the leading edge and the switch at the
//! trailing edge of the row — and carries the platform's animation and
//! action plumbing.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSView` subclass and forwards to `NSView`'s
//! own implementation of each method it overrides, constructs framework
//! controls through their designated initializers and convenience
//! constructors, installs the container's Auto Layout constraints, and runs
//! `NSAnimationContext` groups — all documented main-thread APIs, which the
//! [`MainThreadMarker`] `new` requires guarantees.

use std::cell::OnceCell;
use std::fmt;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimationContext, NSButton, NSControl, NSControlSize, NSControlStateValueOff,
    NSControlStateValueOn, NSLayoutConstraint, NSLayoutConstraintOrientation, NSLayoutGuide,
    NSLayoutPriorityDefaultHigh, NSLayoutPriorityRequired, NSSwitch, NSView,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSArray, NSObjectProtocol, ns_string};
use objc2_quartz_core::CAMediaTimingFunction;

use crate::action::ActionTarget;
use crate::callback::guarded;
use crate::geometry::{Rect, Size};
use crate::response::{ReportsResponse, ResponseGeometry};
use crate::slider::ControlSize;

/// The gap between the control and its label, in points.
pub const LABEL_SPACING: f64 = 8.0;

/// Which control a [`Toggle`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An `NSSwitch`.
    Switch,
    /// An `NSButton` checkbox.
    Checkbox,
}

/// The `NSAnimationContext` parameters a state change plays under — the
/// `withPlatformAnimation` table.
#[derive(Debug, Clone, Copy, Default)]
pub enum StateChange {
    /// Writes the new state directly.
    #[default]
    Immediate,
    /// Runs the write inside an implicit-animation group of `seconds`;
    /// `control_points` installs a cubic-bezier timing function when present.
    Implicit {
        /// The group's duration.
        seconds: f64,
        /// Cubic-bezier timing control points `(x1, y1, x2, y2)`.
        control_points: Option<(f32, f32, f32, f32)>,
    },
}

#[derive(Clone)]
enum Control {
    Switch(Retained<NSSwitch>),
    Checkbox(Retained<NSButton>),
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Switch(_) => f.write_str("Switch"),
            Self::Checkbox(_) => f.write_str("Checkbox"),
        }
    }
}

/// The layout guide a [`ToggleRow`] arranges its control and label in, and
/// the constraints that pin it to the row's content rectangle.
struct ContentGuide {
    guide: Retained<NSLayoutGuide>,
    left: Retained<NSLayoutConstraint>,
    top: Retained<NSLayoutConstraint>,
    width: Retained<NSLayoutConstraint>,
    height: Retained<NSLayoutConstraint>,
}

/// The state a [`ToggleRow`] carries.
pub struct ToggleRowIvars {
    /// How the row was placed as a leaf, and its response rectangle.
    geometry: ResponseGeometry,
    /// Where the control and label are arranged.
    content: OnceCell<ContentGuide>,
}

impl fmt::Debug for ToggleRowIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToggleRowIvars")
            .field("geometry", &self.geometry)
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `ToggleRow::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiToggleRow"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToggleRowIvars]
    #[derive(Debug)]
    /// The row a toggle's control and label are arranged in. Its frame is
    /// the allocation its parent assigned; the control and label are
    /// arranged in its response rectangle, which it reports as its
    /// accessibility frame.
    pub struct ToggleRow;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for ToggleRow {}

    impl ToggleRow {
        /// Top-down coordinates: the content rectangle is in layout space.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        // The content guide follows the committed response before the
        // constraints are solved.
        // SAFETY: see the module safety note.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("ToggleRow layout", || {
                let geometry = &self.ivars().geometry;
                geometry.commit(self);
                self.arrange_in(geometry.content_rect(self));
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layout] };
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame_override(&self) -> CGRect {
            guarded("ToggleRow accessibilityFrame", || {
                self.ivars()
                    .geometry
                    .accessibility_frame(self)
                    // SAFETY: see the module safety note.
                    .unwrap_or_else(|| unsafe { msg_send![super(self), accessibilityFrame] })
            })
        }
    }
);

impl ReportsResponse for ToggleRow {
    fn response_geometry(&self) -> &ResponseGeometry {
        &self.ivars().geometry
    }
}

impl ToggleRow {
    /// An empty row.
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ToggleRowIvars {
            geometry: ResponseGeometry::default(),
            content: OnceCell::new(),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        let guide = NSLayoutGuide::new();
        this.addLayoutGuide(&guide);
        let content = ContentGuide {
            left: guide
                .leftAnchor()
                .constraintEqualToAnchor_constant(&this.leftAnchor(), 0.0),
            top: guide
                .topAnchor()
                .constraintEqualToAnchor_constant(&this.topAnchor(), 0.0),
            width: guide.widthAnchor().constraintEqualToConstant(0.0),
            height: guide.heightAnchor().constraintEqualToConstant(0.0),
            guide,
        };
        NSLayoutConstraint::activateConstraints(&NSArray::from_slice(&[
            &*content.left,
            &*content.top,
            &*content.width,
            &*content.height,
        ]));
        assert!(
            this.ivars().content.set(content).is_ok(),
            "a toggle row installs its content guide once"
        );
        this
    }

    /// The guide the control and label are arranged in.
    fn guide(&self) -> &NSLayoutGuide {
        &self
            .ivars()
            .content
            .get()
            .expect("a toggle row installs its content guide at creation")
            .guide
    }

    /// Pins the content guide to `rect`, in the row's own coordinates.
    fn arrange_in(&self, rect: Rect) {
        let content = self
            .ivars()
            .content
            .get()
            .expect("a toggle row installs its content guide at creation");
        for (constraint, value) in [
            (&content.left, rect.origin.x),
            (&content.top, rect.origin.y),
            (&content.width, rect.size.width),
            (&content.height, rect.size.height),
        ] {
            if constraint.constant().to_bits() != value.to_bits() {
                constraint.setConstant(value);
            }
        }
    }
}

/// A labelled toggle row in a [`ToggleRow`] container.
///
/// The container is what the leaf mounts and lays out; the control is what
/// user interaction and accessibility target.
#[derive(Debug, Clone)]
pub struct Toggle {
    container: Retained<ToggleRow>,
    control: Control,
}

impl Toggle {
    /// A toggle row of `kind`, initially `is_on`, with no label yet —
    /// `set_label` installs one.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, kind: Kind, is_on: bool) -> Self {
        let state = if is_on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        let control = match kind {
            Kind::Switch => {
                let switch = NSSwitch::new(mtm);
                switch.setState(state);
                Control::Switch(switch)
            }
            Kind::Checkbox => {
                // SAFETY: the convenience constructor yields a configured
                // checkbox; `nil` target/action leave it unarmed until
                // `on_change` installs one.
                let button = unsafe {
                    NSButton::checkboxWithTitle_target_action(ns_string!(""), None, None, mtm)
                };
                button.setState(state);
                Control::Checkbox(button)
            }
        };
        Self {
            container: ToggleRow::new(mtm),
            control,
        }
    }

    /// The container view — what a leaf mounts.
    #[must_use]
    pub fn container(&self) -> &NSView {
        &self.container
    }

    /// The control view — the target of user interaction, accessibility, and
    /// `set_enabled`.
    #[must_use]
    pub fn control(&self) -> &NSView {
        match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        }
    }

    /// Installs `label` beside the control, both vertically centered. A
    /// checkbox pins the box leading and the label `LABEL_SPACING` after it;
    /// a switch pins the label leading and the switch trailing, so the free
    /// space of a row wider than its content falls between them.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_label(&self, label: &NSView) {
        let control = self.control();
        label.setTranslatesAutoresizingMaskIntoConstraints(false);
        control.setTranslatesAutoresizingMaskIntoConstraints(false);
        self.container.addSubview(label);
        self.container.addSubview(control);

        label.setContentCompressionResistancePriority_forOrientation(
            NSLayoutPriorityRequired,
            NSLayoutConstraintOrientation::Horizontal,
        );
        label.setContentHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultHigh,
            NSLayoutConstraintOrientation::Horizontal,
        );

        let container = self.container.guide();
        let centered = [
            control
                .centerYAnchor()
                .constraintEqualToAnchor(&container.centerYAnchor()),
            label
                .centerYAnchor()
                .constraintEqualToAnchor(&container.centerYAnchor()),
        ];
        let placed = match &self.control {
            Control::Checkbox(_) => [
                control
                    .leadingAnchor()
                    .constraintEqualToAnchor(&container.leadingAnchor()),
                label
                    .leadingAnchor()
                    .constraintEqualToAnchor_constant(&control.trailingAnchor(), LABEL_SPACING),
                label
                    .trailingAnchor()
                    .constraintLessThanOrEqualToAnchor(&container.trailingAnchor()),
            ],
            Control::Switch(_) => [
                label
                    .leadingAnchor()
                    .constraintEqualToAnchor(&container.leadingAnchor()),
                control
                    .trailingAnchor()
                    .constraintEqualToAnchor(&container.trailingAnchor()),
                label
                    .trailingAnchor()
                    .constraintLessThanOrEqualToAnchor_constant(
                        &control.leadingAnchor(),
                        -LABEL_SPACING,
                    ),
            ],
        };
        let constraints: Vec<&NSLayoutConstraint> =
            centered.iter().chain(&placed).map(|c| &**c).collect();
        NSLayoutConstraint::activateConstraints(&NSArray::from_slice(&constraints));
    }

    /// How large the control is drawn: `NSControl.controlSize` on the switch
    /// or the checkbox. The intrinsic size [`Self::control_size`] reports
    /// follows it.
    pub fn set_control_size(&self, size: ControlSize) {
        let size = match size {
            ControlSize::Mini => NSControlSize::Mini,
            ControlSize::Small => NSControlSize::Small,
            ControlSize::Regular => NSControlSize::Regular,
            ControlSize::Large => NSControlSize::Large,
        };
        match &self.control {
            Control::Switch(control) => control.setControlSize(size),
            Control::Checkbox(control) => control.setControlSize(size),
        }
    }

    /// The control's intrinsic size — the row's share of a measure.
    #[must_use]
    pub fn control_size(&self) -> Size {
        self.control().intrinsicContentSize().into()
    }

    /// The row's composed size beside a label of `label_size`: control,
    /// `LABEL_SPACING` and label wide, the taller of the two high — the
    /// content `set_label`'s constraints place, in either order.
    #[must_use]
    pub fn row_size(&self, label_size: Size) -> Size {
        let control = self.control_size();
        Size::new(
            control.width + LABEL_SPACING + label_size.width,
            control.height.max(label_size.height),
        )
    }

    /// Writes `on` into the control, under `change`'s animation terms.
    /// A state already equal to `on` is left alone.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_on(&self, on: bool, change: StateChange) {
        let state = if on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        match &self.control {
            Control::Switch(control) if control.state() == state => return,
            Control::Checkbox(control) if control.state() == state => return,
            _ => {}
        }
        match change {
            StateChange::Immediate => set_state(&self.control, state),
            StateChange::Implicit {
                seconds,
                control_points,
            } => {
                let control = self.control.clone();
                let changes =
                    RcBlock::new(move |context: core::ptr::NonNull<NSAnimationContext>| {
                        // SAFETY: the context is the live group AppKit hands
                        // the block.
                        let context = unsafe { context.as_ref() };
                        context.setDuration(seconds);
                        context.setAllowsImplicitAnimation(true);
                        if let Some((x1, y1, x2, y2)) = control_points {
                            let timing =
                                CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2);
                            context.setTimingFunction(Some(&timing));
                        }
                        set_state(&control, state);
                    });
                NSAnimationContext::runAnimationGroup(&changes);
            }
        }
    }

    /// The control's state after user interaction — what an action handler
    /// reports back.
    #[must_use]
    pub fn is_on(&self) -> bool {
        match &self.control {
            Control::Switch(control) => control.state() == NSControlStateValueOn,
            Control::Checkbox(control) => control.state() == NSControlStateValueOn,
        }
    }

    /// Whether the control responds to input.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_enabled(&self, enabled: bool) {
        let control: &NSControl = match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        };
        control.setEnabled(enabled);
    }

    /// Calls `handler` each time the user toggles the control. The returned
    /// [`ActionTarget`] owns the registration — keep it for as long as the
    /// control should respond.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn on_change(&self, handler: impl Fn(MainThreadMarker) + 'static) -> ActionTarget {
        let control: &NSControl = match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        };
        ActionTarget::new(control, handler)
    }
}

/// Writes `state` — the property change an implicit-animation group picks
/// up.
fn set_state(control: &Control, state: objc2_app_kit::NSControlStateValue) {
    match control {
        Control::Switch(control) => control.setState(state),
        Control::Checkbox(control) => control.setState(state),
    }
}
