//! `waterui::component::ControlSize` mapped onto the platform's size
//! classes.
//!
//! The platform offers four sizes — `mini`, `small`, `regular`, `large` —
//! against `WaterUI`'s five-step Material scale. The mapping is relative to
//! the control's documented default rather than positional: the default is
//! the platform's `regular` (what a bare platform control draws at), one
//! step below is `small`, two or more below is `mini`, and any step above
//! is `large`.

use cocoa_ui::slider::ControlSize;
use waterui::component::ControlSize as WuiControlSize;

/// `size` drawn for a control whose documented default is `default`.
///
/// `iOS` has no control size classes; callers honour the value only where
/// the platform supports it (`cocoa-ui`'s `UIKit` setters already no-op).
pub const fn platform_control_size(size: WuiControlSize, default: WuiControlSize) -> ControlSize {
    match ordinal(size) - ordinal(default) {
        0 => ControlSize::Regular,
        -1 => ControlSize::Small,
        d if d <= -2 => ControlSize::Mini,
        _ => ControlSize::Large,
    }
}

/// A `ControlSize`'s position on the five-step scale.
const fn ordinal(size: WuiControlSize) -> i8 {
    match size {
        WuiControlSize::ExtraSmall => 0,
        WuiControlSize::Small => 1,
        WuiControlSize::Large => 3,
        WuiControlSize::ExtraLarge => 4,
        // `Medium`, plus any step the scale gains later.
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_maps_to_regular_whatever_it_is() {
        for default in [
            WuiControlSize::ExtraSmall,
            WuiControlSize::Small,
            WuiControlSize::Medium,
            WuiControlSize::Large,
            WuiControlSize::ExtraLarge,
        ] {
            assert_eq!(
                platform_control_size(default, default),
                ControlSize::Regular
            );
        }
    }

    #[test]
    fn steps_below_the_default_shrink_and_steps_above_grow() {
        // A slider's documented default is `ExtraSmall`.
        assert_eq!(
            platform_control_size(WuiControlSize::Small, WuiControlSize::ExtraSmall),
            ControlSize::Large
        );
        // A button's documented default is `Small`.
        assert_eq!(
            platform_control_size(WuiControlSize::ExtraSmall, WuiControlSize::Small),
            ControlSize::Small
        );
        assert_eq!(
            platform_control_size(WuiControlSize::ExtraSmall, WuiControlSize::Medium),
            ControlSize::Mini
        );
        assert_eq!(
            platform_control_size(WuiControlSize::Small, WuiControlSize::Medium),
            ControlSize::Small
        );
        assert_eq!(
            platform_control_size(WuiControlSize::ExtraSmall, WuiControlSize::Large),
            ControlSize::Mini
        );
        assert_eq!(
            platform_control_size(WuiControlSize::ExtraLarge, WuiControlSize::Medium),
            ControlSize::Large
        );
    }
}
