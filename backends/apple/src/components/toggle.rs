//! The `toggle` leaf: `Native<ToggleConfig>` rendered through the kit's
//! switch/checkbox row.
//!
//! Mirrors `WuiToggle`: the `toggle` `Binding<bool>` drives `set_on` with the
//! watcher metadata's animation, the control's action writes user edits back
//! into the binding, `Disabled` pushes `set_enabled`, and the label renders
//! as a mounted child beside the control. The payload's style arrives
//! resolved — `ToggleStyle::resolved` owns the platform default — and picks
//! the control; the payload's stretch axis picks how the row fills its
//! offer (`docs/layout-spec.md` §3, §6).

use waterui::animation::{Animation, ChangeAnimation};
use waterui::component::ControlSize;
use waterui::component::toggle::{ToggleConfig, ToggleStyle};
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};

#[cfg(target_os = "ios")]
use crate::components::button_chrome;
use crate::components::control_size::platform_control_size;
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

/// The control a resolved style draws. The native payload never carries
/// `Automatic`: `ToggleConfig`'s resolver replaces it with the platform
/// default before the backend sees the config.
///
/// An unresolved `Automatic` or a style the backend does not know is a
/// contract violation of the framework above; the switch is drawn.
fn kind(style: ToggleStyle) -> Kind {
    match style {
        ToggleStyle::Switch => Kind::Switch,
        ToggleStyle::Checkbox => Kind::Checkbox,
        ToggleStyle::Automatic => {
            waterui::contract_violation!(
                "the toggle payload reached the backend with an unresolved Automatic style; \
                 the switch is drawn"
            );
            Kind::Switch
        }
        _ => {
            waterui::contract_violation!("unsupported WaterUI toggle style; the switch is drawn");
            Kind::Switch
        }
    }
}

/// The `StateChange` the metadata asks for, `withCrossDissolveAnimation`'s
/// table: a change that names no animation or disables it writes directly,
/// and any named animation plays — a switch slides, a checkbox cross-fades
/// for the seconds the table gives.
#[cfg(target_os = "ios")]
fn state_change(metadata: &Metadata) -> StateChange {
    match ChangeAnimation::of(metadata).animation() {
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
/// group lasting the spring's settling duration.
#[cfg(target_os = "macos")]
fn state_change(metadata: &Metadata) -> StateChange {
    match ChangeAnimation::of(metadata).animation() {
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
        Some(spring @ Animation::Spring { .. }) => StateChange::Implicit {
            seconds: spring.duration().as_secs_f64(),
            control_points: None,
        },
    }
}

/// The toggle row's layout face, measured from the control's intrinsic size
/// plus a live read of the label child. A switch with a visible label is
/// `Horizontal` and answers the proposed width; a checkbox or a hidden label
/// is `None` and answers the row's intrinsic size.
struct ToggleSubView {
    /// The kit row; its `control_size` is the control's share of a measure.
    toggle: Toggle,
    /// The payload's `NativeView::stretch_axis` — the one statement of the
    /// rule, read once at install.
    stretch: StretchAxis,
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
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

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
        let width = match (self.stretch, proposal.width) {
            (StretchAxis::Horizontal, Some(proposed)) if proposed.is_finite() => {
                proposed.max(intrinsic_width)
            }
            _ => intrinsic_width,
        };
        ViewDimensions::new(Size::new(width, height))
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
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
        let stretch = waterui_core::NativeView::stretch_axis(&config);
        let (label, binding, style, size) = config.into_parts();
        let toggle = Toggle::new(mtm, kind(style), binding.snapshot());
        // `Toggle`'s documented default size is `Small`; the mapping is
        // relative to it, so a bare toggle draws at the platform's regular
        // size like a bare `NSSwitch` or checkbox. `AppKit` honours every
        // size on the switch and the checkbox; `UIKit` draws one size at
        // every control size, so the platform fixes the size on iOS and
        // `set_control_size` ignores the value there.
        toggle.set_control_size(platform_control_size(size, ControlSize::Small));
        let disabled = Disabled::resolve(ctx.env(), false);

        // The label's spoken name lands on the control; the visual label
        // leaves the accessibility tree — `WuiControlAccessibility`'s split.
        let accessibility_label = label.accessibility_label();

        // The label renders as a mounted child of the row's container.
        let child = ctx.render(waterui_backend_core::AnyView::new(label));
        let mounted = child.mount(toggle.container());
        toggle.set_label(mounted.view());
        cocoa_ui::view::hide_from_accessibility(mounted.view());

        let mut leaf = NativeLeaf::new(
            toggle.container(),
            ToggleSubView {
                toggle: toggle.clone(),
                stretch,
                label: mounted,
            },
        );

        // Signal → control, under the watcher metadata's animation.
        leaf.watch(&binding, {
            let toggle = toggle.clone();
            move |ctx| {
                toggle.set_on(*ctx.value(), state_change(ctx.metadata()));
            }
        });

        // Control → binding: user edits write back through the action.
        leaf.keep(toggle.on_change({
            let toggle = toggle.clone();
            move |_| binding.set(toggle.is_on())
        }));

        leaf.bind(&disabled, {
            let toggle = toggle.clone();
            move |is_disabled| toggle.set_enabled(!is_disabled)
        });

        // The tint colours the on state: a switch's track, a checkbox's
        // glyph. `AppKit` has no per-control tint for `NSSwitch` or the
        // checkbox `NSButton` (`contentTintColor` touches borderless
        // buttons only, `bezelColor` bordered ones), so they draw with the
        // application's accent colour there.
        #[cfg(target_os = "ios")]
        leaf.bind(&button_chrome::tint_signal(ctx.env()), {
            let toggle = toggle.clone();
            move |color| toggle.set_tint(&button_chrome::platform_color(&color))
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

    use waterui::animation::NoAnimation;

    use super::*;

    #[test]
    fn kind_maps_resolved_styles_like_the_swift_table() {
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
            assert_eq!(kind(style.resolved()), expected);
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "unresolved Automatic style")
    )]
    fn kind_draws_a_switch_for_an_unresolved_automatic_style() {
        assert_eq!(kind(ToggleStyle::Automatic), Kind::Switch);
    }

    /// A change that disables animation writes directly, even when an
    /// animation is attached too.
    #[test]
    fn state_change_snaps_when_animation_is_disabled() {
        let disabled = Metadata::new().with(Animation::Default).with(NoAnimation);
        let StateChange::Immediate = state_change(&disabled) else {
            panic!("a change that disables animation must write immediately");
        };
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
                // The spring runs for its own settling duration.
                Animation::spring(170.0, 15.0).duration().as_secs_f64(),
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
