//! Input conventions a platform defines and its user can change: how close
//! in time and space two presses must be to form a multi-click, and which
//! chord is the paste shortcut.
//!
//! A window reports them through [`PlatformWindow::input_conventions`] and the
//! runner hands the current value to the renderer with every input batch, so
//! a setting the user changes while the app runs applies to the next press.
//!
//! [`PlatformWindow::input_conventions`]: super::PlatformWindow::input_conventions

use std::time::Duration;

use waterui_graphics::input::Key;

use super::{Modifiers, ScrollPlatform};

/// How two presses combine into a multi-click run.
///
/// A press continues the run when it lands on the same target within
/// [`interval`](Self::interval) of the previous press and no further than
/// [`tolerance`](Self::tolerance) from it on either axis. Distances are in
/// logical points: a platform that reports device pixels divides by its
/// scale before handing the value over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MultiClickConfig {
    /// The longest time between two presses of one run.
    pub interval: Duration,
    /// The farthest the second press may sit from the first, per axis. The
    /// rectangle form is the platforms' own: Windows
    /// (`SM_CXDOUBLECLK`/`SM_CYDOUBLECLK`) and GTK (`gtk-double-click-distance`)
    /// both compare each axis separately.
    pub tolerance: kurbo::Vec2,
}

impl MultiClickConfig {
    /// The Apple default: the HID system's `HIDClickTime` of 500 ms (the
    /// value `NSEvent.doubleClickInterval` reports until the user changes
    /// it). `AppKit` publishes no click distance — `NXGetClickSpace` is
    /// deprecated since macOS 10.12 and the registry reports
    /// `HIDClickSpace = (0, 0)`, the driver's internal default — so the
    /// distance is this backend's own 6 points.
    pub const APPLE: Self = Self {
        interval: Duration::from_millis(500),
        tolerance: kurbo::Vec2::new(6.0, 6.0),
    };

    /// GTK's documented defaults, the closest thing Linux has to a platform
    /// value: `gtk-double-click-time` 400 ms and `gtk-double-click-distance`
    /// 5 px, from the
    /// [`GtkSettings`](https://docs.gtk.org/gtk4/class.Settings.html)
    /// reference. Desktops without a GTK settings daemon, and every
    /// offscreen surface, run on these.
    pub const GTK: Self = Self {
        interval: Duration::from_millis(400),
        tolerance: kurbo::Vec2::new(5.0, 5.0),
    };

    /// The data default of the system family Hydrolysis is compiled for.
    #[must_use]
    pub const fn for_platform(platform: ScrollPlatform) -> Self {
        match platform {
            ScrollPlatform::Apple => Self::APPLE,
            ScrollPlatform::Other => Self::GTK,
        }
    }

    /// Whether a press `elapsed` after the previous one and `offset` away
    /// from it continues the run.
    #[must_use]
    pub fn continues(&self, elapsed: Duration, offset: kurbo::Vec2) -> bool {
        elapsed <= self.interval
            && offset.x.abs() <= self.tolerance.x
            && offset.y.abs() <= self.tolerance.y
    }
}

/// The modifier of the platform's paste shortcut. It is binary by definition
/// — the Apple Command key or Control — and a third convention is a new
/// platform family, which must fail every match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteShortcut {
    /// Command-V, on Apple platforms.
    Command,
    /// Control-V, everywhere else.
    Control,
}

impl PasteShortcut {
    /// The shortcut of a system family.
    #[must_use]
    pub const fn for_platform(platform: ScrollPlatform) -> Self {
        match platform {
            ScrollPlatform::Apple => Self::Command,
            ScrollPlatform::Other => Self::Control,
        }
    }

    /// Whether pressing `key` with `modifiers` held is this shortcut: the
    /// letter V with the shortcut's modifier and no other — Control-V on a
    /// Mac is a terminal's literal-next, not a paste, and Shift or Alt make
    /// the chord a different one.
    #[must_use]
    pub fn matches(self, key: &Key, modifiers: Modifiers) -> bool {
        let Key::Character(character) = key else {
            return false;
        };
        let (command, control) = match self {
            Self::Command => (true, false),
            Self::Control => (false, true),
        };
        character.eq_ignore_ascii_case("v")
            && modifiers.super_key == command
            && modifiers.control == control
            && !modifiers.shift
            && !modifiers.alt
    }
}

/// Everything about pointer and keyboard input that a platform defines and
/// its user can change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputConventions {
    /// How presses combine into a multi-click run.
    pub multi_click: MultiClickConfig,
    /// The paste shortcut.
    pub paste_shortcut: PasteShortcut,
}

impl InputConventions {
    /// The data defaults of a system family, for a window with no platform
    /// settings of its own to read.
    #[must_use]
    pub const fn for_platform(platform: ScrollPlatform) -> Self {
        Self {
            multi_click: MultiClickConfig::for_platform(platform),
            paste_shortcut: PasteShortcut::for_platform(platform),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MultiClickConfig, ScrollPlatform};
    use std::time::Duration;

    #[test]
    fn a_run_continues_inside_the_interval_and_the_tolerance_only() {
        let config = MultiClickConfig::for_platform(ScrollPlatform::Other);
        let near = kurbo::Vec2::new(5.0, -5.0);
        assert!(config.continues(Duration::from_millis(400), near));
        assert!(!config.continues(Duration::from_millis(401), near));
        assert!(!config.continues(Duration::ZERO, kurbo::Vec2::new(5.1, 0.0)));
        assert!(!config.continues(Duration::ZERO, kurbo::Vec2::new(0.0, -5.1)));
    }
}
