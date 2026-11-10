//! The `UIKit` toggle: a `UISwitch` or a `UIButton` checkbox, together with
//! the labelled row a toggle lays out.
//!
//! `Toggle` owns a [`ToggleRow`] container holding the control and an
//! optional label, arranged by kind — a checkbox reads box first, then its
//! label; a switch puts its label at the leading end and the switch at the
//! trailing end of the row — and carries the platform's animation and action
//! plumbing.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass and forwards to `UIView`'s
//! own implementation of each method it overrides, constructs framework
//! controls through their designated initializers and convenience
//! constructors, installs the container's Auto Layout constraints, and wraps
//! state changes in `UIView` transition blocks — all documented main-thread
//! APIs, which the [`MainThreadMarker`] `new` requires guarantees.

use std::cell::OnceCell;
use std::fmt;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, ns_string};
use objc2_ui_kit::{
    NSLayoutConstraint, UIAccessibilityContentSizeCategoryImageAdjusting, UIButton, UIButtonType,
    UIColor, UIControl, UIControlState, UIFontTextStyleBody, UIImage, UIImageSymbolConfiguration,
    UILayoutGuide, UILayoutPriorityDefaultHigh, UILayoutPriorityRequired, UIResponder, UISwitch,
    UIView,
};

use crate::action::{ActionTarget, ControlEvents};
use crate::callback::guarded;
use crate::core_animation;
use crate::geometry::{Rect, Size};
use crate::response::{ReportsResponse, ResponseGeometry};
use crate::slider::ControlSize;

/// The gap between the label and the control, in points.
pub const LABEL_SPACING: f64 = 8.0;

/// Which control a [`Toggle`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `UISwitch`.
    Switch,
    /// A `UIButton` checkbox — `square` / `checkmark.square.fill` symbols.
    Checkbox,
}

/// How a state change reaches the control.
#[derive(Debug, Clone, Copy, Default)]
pub enum StateChange {
    /// Writes the new state directly.
    #[default]
    Immediate,
    /// A switch plays its slide; a checkbox writes directly — it has no
    /// slide to play.
    Animated,
    /// A checkbox cross-fades the new state over `seconds`; a switch plays
    /// its slide.
    Dissolve {
        /// The cross-fade duration.
        seconds: f64,
    },
}

#[derive(Clone)]
enum Control {
    Switch(Retained<UISwitch>),
    Checkbox(Retained<UIButton>),
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
    guide: Retained<UILayoutGuide>,
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
    // SAFETY: `UIView`'s designated initializer is `initWithFrame:`, which
    // `ToggleRow::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[name = "CocoaUiToggleRow"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToggleRowIvars]
    #[derive(Debug)]
    /// The row a toggle's control and label are arranged in. Its frame is
    /// the allocation its parent assigned; the control and label are
    /// arranged in its response rectangle, which it reports as its
    /// accessibility frame.
    pub struct ToggleRow;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for ToggleRow {}

    impl ToggleRow {
        // The content guide follows the committed response before the
        // constraints are solved.
        // SAFETY: see the module safety note.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("ToggleRow layoutSubviews", || {
                let geometry = &self.ivars().geometry;
                geometry.commit(self);
                self.arrange_in(geometry.content_rect(self));
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
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
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        let guide = UILayoutGuide::new(mtm);
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
        NSLayoutConstraint::activateConstraints(
            &NSArray::from_slice(&[
                &*content.left,
                &*content.top,
                &*content.width,
                &*content.height,
            ]),
            mtm,
        );
        assert!(
            this.ivars().content.set(content).is_ok(),
            "a toggle row installs its content guide once"
        );
        this
    }

    /// The guide the control and label are arranged in.
    fn guide(&self) -> &UILayoutGuide {
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
    /// When not called on the main thread, or when the system `square` /
    /// `checkmark.square.fill` symbols a checkbox needs are absent — a
    /// fatal authoring error.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, kind: Kind, is_on: bool) -> Self {
        let control = match kind {
            Kind::Switch => {
                let switch = UISwitch::new(mtm);
                switch.setOn(is_on);
                Control::Switch(switch)
            }
            Kind::Checkbox => {
                let unchecked = UIImage::systemImageNamed(ns_string!("square"));
                let checked = UIImage::systemImageNamed(ns_string!("checkmark.square.fill"));
                let (Some(unchecked), Some(checked)) = (unchecked, checked) else {
                    panic!("cocoa-ui checkbox requires the system square symbols");
                };
                let button =
                    crate::uikit::button::CocoaUiButton::with_type(mtm, UIButtonType::System)
                        .into_super();
                // SAFETY: `UIFontTextStyleBody` is a documented, immutable
                // shared text style.
                let configuration = unsafe {
                    UIImageSymbolConfiguration::configurationWithTextStyle(UIFontTextStyleBody)
                };
                button.setPreferredSymbolConfiguration_forImageInState(
                    Some(&configuration),
                    UIControlState::Normal,
                );
                button.setImage_forState(Some(&unchecked), UIControlState::Normal);
                button.setImage_forState(Some(&checked), UIControlState::Selected);
                button.setSelected(is_on);
                button.setChangesSelectionAsPrimaryAction(true);
                button.setAdjustsImageSizeForAccessibilityContentSizeCategory(true);
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
    pub fn container(&self) -> &UIView {
        &self.container
    }

    /// The control view — the target of user interaction, accessibility, and
    /// `set_enabled`.
    #[must_use]
    pub fn control(&self) -> &UIView {
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
    pub fn set_label(&self, label: &UIView) {
        let control = self.control();
        label.setTranslatesAutoresizingMaskIntoConstraints(false);
        control.setTranslatesAutoresizingMaskIntoConstraints(false);
        self.container.addSubview(label);
        self.container.addSubview(control);

        label.setContentCompressionResistancePriority_forAxis(
            UILayoutPriorityRequired,
            objc2_ui_kit::UILayoutConstraintAxis::Horizontal,
        );
        label.setContentHuggingPriority_forAxis(
            UILayoutPriorityDefaultHigh,
            objc2_ui_kit::UILayoutConstraintAxis::Horizontal,
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
        NSLayoutConstraint::activateConstraints(
            &NSArray::from_slice(&constraints),
            MainThreadMarker::from(&*self.container),
        );
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

    /// Writes `on` into the control under `change`'s terms. A state already
    /// equal to `on` is left alone.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_on(&self, on: bool, change: StateChange) {
        match &self.control {
            Control::Switch(switch) => {
                if switch.isOn() == on {
                    return;
                }
                match change {
                    StateChange::Immediate => switch.setOn(on),
                    StateChange::Animated | StateChange::Dissolve { .. } => {
                        switch.setOn_animated(on, true);
                    }
                }
            }
            Control::Checkbox(button) => {
                if button.isSelected() == on {
                    return;
                }
                match change {
                    StateChange::Immediate | StateChange::Animated => button.setSelected(on),
                    StateChange::Dissolve { seconds } => {
                        let button = Retained::clone(button);
                        let view = Retained::clone(&button);
                        core_animation::cross_dissolve(&view, seconds, move || {
                            button.setSelected(on);
                        });
                    }
                }
            }
        }
    }

    /// The control's state after user interaction — what an action handler
    /// reports back.
    #[must_use]
    pub fn is_on(&self) -> bool {
        match &self.control {
            Control::Switch(switch) => switch.isOn(),
            Control::Checkbox(button) => button.isSelected(),
        }
    }

    /// How large the control is drawn.
    ///
    /// `UIKit` has no size classes for a switch or a checkbox: the platform
    /// draws one size at every control size, so the value is ignored.
    pub const fn set_control_size(&self, _size: ControlSize) {}

    /// The colour the control draws its on state with: a switch's track
    /// when it is on (`onTintColor`), a checkbox's glyph (`tintColor`).
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_tint(&self, color: &UIColor) {
        match &self.control {
            Control::Switch(switch) => switch.setOnTintColor(Some(color)),
            // SAFETY: `setTintColor:` is marked unsafe in the bindings; it is
            // an ordinary main-thread `UIKit` setter.
            Control::Checkbox(button) => unsafe { button.setTintColor(Some(color)) },
        }
    }

    /// Whether the control responds to input.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_enabled(&self, enabled: bool) {
        let control: &UIControl = match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        };
        control.setEnabled(enabled);
    }

    /// Calls `handler` each time the user toggles the control — `.valueChanged`
    /// on a switch, `.primaryActionTriggered` on a checkbox. The returned
    /// [`ActionTarget`] owns the registration — keep it for as long as the
    /// control should respond.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn on_change(&self, handler: impl Fn(MainThreadMarker) + 'static) -> ActionTarget {
        match &self.control {
            Control::Switch(switch) => {
                ActionTarget::new(switch, ControlEvents::VALUE_CHANGED, handler)
            }
            Control::Checkbox(button) => {
                ActionTarget::new(button, ControlEvents::PRIMARY_ACTION_TRIGGERED, handler)
            }
        }
    }
}
