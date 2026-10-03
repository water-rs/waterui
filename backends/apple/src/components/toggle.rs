//! The `toggle` leaf: `Native<ToggleConfig>` rendered through the kit's
//! switch/checkbox row.
//!
//! Mirrors `WuiToggle`: the `toggle` `Binding<bool>` drives `set_on` with the
//! watcher metadata's animation, the control's action writes user edits back
//! into the binding, `Disabled` pushes `set_enabled`, and the label renders
//! as a mounted child beside the control. Style picks the control — iOS's
//! automatic is the switch, macOS's the checkbox, exactly the Swift table.

use waterui::animation::Animation;
use waterui::component::toggle::{ToggleConfig, ToggleStyle};
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::toggle::{Kind, StateChange, Toggle};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::toggle::{Kind, StateChange, Toggle};
}

use platform::{Kind, StateChange, Toggle};

/// The control a style draws on this platform — `makeToggleControl`'s table.
#[cfg(target_os = "ios")]
const fn kind(style: ToggleStyle) -> Kind {
    match style {
        ToggleStyle::Automatic | ToggleStyle::Switch => Kind::Switch,
        ToggleStyle::Checkbox => Kind::Checkbox,
        _ => panic!("unsupported WaterUI toggle style"),
    }
}

/// The control a style draws on this platform — `makeToggleControl`'s table.
#[cfg(target_os = "macos")]
const fn kind(style: ToggleStyle) -> Kind {
    match style {
        ToggleStyle::Automatic | ToggleStyle::Checkbox => Kind::Checkbox,
        ToggleStyle::Switch => Kind::Switch,
        _ => panic!("unsupported WaterUI toggle style"),
    }
}

/// The `StateChange` the metadata asks for, `withCrossDissolveAnimation`'s
/// table: an absent animation writes directly, and any present animation
/// plays — a switch slides, a checkbox cross-fades for the seconds the
/// table gives.
#[cfg(target_os = "ios")]
fn state_change(metadata: &Metadata) -> StateChange {
    match metadata.try_get::<Animation>() {
        None => StateChange::Immediate,
        Some(Animation::Default) => StateChange::Dissolve { seconds: 0.25 },
        Some(Animation::Bezier { duration, .. }) => StateChange::Dissolve {
            seconds: duration.as_secs_f64(),
        },
        Some(Animation::Spring { .. }) => StateChange::Dissolve { seconds: 0.15 },
    }
}

/// The `StateChange` the metadata asks for, `withPlatformAnimation`'s table:
/// a bezier becomes an implicit group under its curve, a spring an implicit
/// group under its clamped estimate.
#[cfg(target_os = "macos")]
fn state_change(metadata: &Metadata) -> StateChange {
    match metadata.try_get::<Animation>() {
        None => StateChange::Immediate,
        Some(Animation::Default) => StateChange::Implicit {
            seconds: 0.25,
            control_points: None,
        },
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => StateChange::Implicit {
            seconds: duration.as_secs_f64(),
            control_points: Some((x1, y1, x2, y2)),
        },
        Some(Animation::Spring { stiffness, damping }) => {
            let estimate = 2.0 * f64::sqrt(1.0 / f64::from(stiffness)) * f64::from(damping);
            StateChange::Implicit {
                seconds: estimate.clamp(0.1, 2.0),
                control_points: None,
            }
        }
    }
}

/// The toggle row's layout face: horizontally stretching, measured from the
/// control's intrinsic size plus a live read of the label child — the
/// `sizeThatFits` contract.
struct ToggleSubView {
    /// The kit row; its `control_size` is the control's share of a measure.
    toggle: Toggle,
    /// The mounted label, measured under an unspecified proposal each time —
    /// natively hosted children measure against their ideal, not their frame.
    label: Mounted,
}

impl core::fmt::Debug for ToggleSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToggleSubView").finish_non_exhaustive()
    }
}

impl SubView for ToggleSubView {
    // Measured points come back in f64; `ViewDimensions` speaks f32 — the
    // narrowing is the layout contract.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let label = self
            .label
            .layout()
            .measure(ProposalSize::new(None, None))
            .size;
        let control = self.toggle.control_size();
        let has_label = label.width > 0.0 && label.height > 0.0;
        // `row_size` is the kit's single definition of the row's
        // composition — the same arithmetic `set_label`'s constraints
        // place.
        let (intrinsic_width, height) = if has_label {
            let row = self.toggle.row_size(cocoa_ui::geometry::Size::new(
                f64::from(label.width),
                f64::from(label.height),
            ));
            (row.width as f32, row.height as f32)
        } else {
            (control.width as f32, control.height as f32)
        };
        // A phone's toggle fills the row it's offered when a label shares it;
        // a Mac's takes its own width — the box pushed to the far edge of a
        // window is not a thing macOS draws.
        #[cfg(target_os = "ios")]
        let width = if has_label && let Some(proposed) = proposal.width {
            proposed.max(intrinsic_width)
        } else {
            intrinsic_width
        };
        #[cfg(target_os = "macos")]
        let width = {
            let _ = proposal;
            intrinsic_width
        };
        ViewDimensions::new(Size::new(width, height))
    }

    fn stretch_axis(&self) -> StretchAxis {
        // `WuiToggle` answers the default `.none` on both platforms; the iOS
        // row fill comes from `measure` echoing the proposed width instead.
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `toggle` handler on the dispatcher: `Native<ToggleConfig>`
/// maps to a kit row whose binding, enabled state, and accessibility name
/// stay live through watchers.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ToggleConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let toggle = Toggle::new(mtm, kind(config.style), config.toggle.snapshot());
        let disabled = Disabled::resolve(ctx.env(), false);

        // The label's spoken name lands on the control; the visual label
        // leaves the accessibility tree — `WuiControlAccessibility`'s split.
        let accessibility_label = config.label.accessibility_label();

        // The label renders as a mounted child of the row's container.
        let child = ctx.render(waterui_backend_core::AnyView::new(config.label));
        let mounted = child.mount(toggle.container());
        toggle.set_label(mounted.view());
        cocoa_ui::view::hide_from_accessibility(mounted.view());

        let mut leaf = NativeLeaf::new(
            toggle.container(),
            ToggleSubView {
                toggle: toggle.clone(),
                label: mounted,
            },
        );

        // Signal → control, under the watcher metadata's animation.
        leaf.watch(&config.toggle, {
            let toggle = toggle.clone();
            move |ctx| {
                toggle.set_on(*ctx.value(), state_change(ctx.metadata()));
            }
        });

        // Control → binding: user edits write back through the action.
        leaf.keep(toggle.on_change({
            let binding = config.toggle.clone();
            let toggle = toggle.clone();
            move |_| binding.set(toggle.is_on())
        }));

        leaf.bind(&disabled, {
            let toggle = toggle.clone();
            move |is_disabled| toggle.set_enabled(!is_disabled)
        });

        let control = cocoa_ui::view::retain_base(toggle.control());
        leaf.bind(&accessibility_label, move |styled| {
            let plain = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
            cocoa_ui::view::set_accessibility_label(&control, &plain);
        });

        leaf
    });
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    #[test]
    fn kind_maps_styles_like_the_swift_table() {
        #[cfg(target_os = "macos")]
        let table = [
            (ToggleStyle::Automatic, Kind::Checkbox),
            (ToggleStyle::Switch, Kind::Switch),
            (ToggleStyle::Checkbox, Kind::Checkbox),
        ];
        #[cfg(target_os = "ios")]
        let table = [
            (ToggleStyle::Automatic, Kind::Switch),
            (ToggleStyle::Switch, Kind::Switch),
            (ToggleStyle::Checkbox, Kind::Checkbox),
        ];
        for (style, expected) in table {
            assert_eq!(kind(style), expected);
        }
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn state_change_maps_animation_metadata() {
        let StateChange::Immediate = state_change(&Metadata::new()) else {
            panic!("absent animation must write immediately");
        };
        let table: [(Animation, f64); 3] = [
            (Animation::Default, 0.25),
            (
                Animation::Bezier {
                    duration: Duration::from_millis(300),
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 1.0,
                },
                0.3,
            ),
            (
                Animation::Spring {
                    stiffness: 170.0,
                    damping: 15.0,
                },
                0.15,
            ),
        ];
        for (animation, expected) in table {
            let StateChange::Dissolve { seconds } = state_change(&Metadata::new().with(animation))
            else {
                panic!("a present animation must animate");
            };
            assert_eq!(seconds.to_bits(), expected.to_bits());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn state_change_maps_animation_metadata() {
        type Row = (Animation, f64, Option<(f32, f32, f32, f32)>);
        let StateChange::Immediate = state_change(&Metadata::new()) else {
            panic!("absent animation must write immediately");
        };
        let table: [Row; 3] = [
            (Animation::Default, 0.25, None),
            (
                Animation::Bezier {
                    duration: Duration::from_millis(300),
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 1.0,
                },
                0.3,
                Some((0.0, 0.0, 1.0, 1.0)),
            ),
            (
                Animation::Spring {
                    stiffness: 170.0,
                    damping: 15.0,
                },
                2.0,
                None,
            ),
        ];
        for (animation, expected_seconds, expected_points) in table {
            let StateChange::Implicit {
                seconds,
                control_points,
            } = state_change(&Metadata::new().with(animation))
            else {
                panic!("a present animation must animate implicitly");
            };
            assert_eq!(seconds.to_bits(), expected_seconds.to_bits());
            assert_eq!(control_points, expected_points);
        }
    }
}
