//! Platform lifecycle phase observation for `waterui-apple`.
//!
//! Wires `AppKit` (`NSApplication`) and `UIKit` (`UIApplication`) foreground,
//! inactive, and background notifications into [`PhaseHandle`], keeping the
//! environment's `Computed<AppPhase>` current and running `App::on_phase_change`
//! hooks. On iOS, transitions into [`AppPhase::Background`] hold a
//! `UIApplication` background task assertion around the spawned hook future so
//! async state persistence completes before suspension.

use cocoa_ui::MainThreadMarker;
use cocoa_ui::notification::{NotificationName, NotificationObserver, observe};
use waterui_backend_core::Environment;
#[cfg(target_os = "ios")]
use waterui_backend_core::app::PhaseHandle;
use waterui_backend_core::app::{AppPhase, Phase};

/// Holds the notification observers feeding the application's [`AppPhase`].
/// Dropping it unregisters every observer.
#[derive(Debug)]
#[must_use = "dropping the observation unregisters the lifecycle observers"]
pub struct PhaseObservation {
    _observers: [NotificationObserver; 4],
}

/// Starts `phase` under `env` and registers the platform lifecycle observers.
pub fn start(phase: Phase, env: &Environment, mtm: MainThreadMarker) -> PhaseObservation {
    #[cfg(target_os = "macos")]
    {
        start_macos(phase, env, mtm)
    }
    #[cfg(target_os = "ios")]
    {
        start_ios(phase, env, mtm)
    }
}

#[cfg(target_os = "macos")]
const fn macos_phase(is_active: bool, is_hidden: bool) -> AppPhase {
    if is_hidden {
        AppPhase::Background
    } else if is_active {
        AppPhase::Active
    } else {
        AppPhase::Inactive
    }
}

#[cfg(target_os = "macos")]
fn start_macos(phase: Phase, env: &Environment, mtm: MainThreadMarker) -> PhaseObservation {
    use cocoa_ui::appkit::Application;

    let app = Application::shared(mtm);
    let initial = macos_phase(app.is_active(), app.is_hidden());
    let handle = phase.start_with(env, initial);

    let on_become_active = observe(mtm, &NotificationName::application_did_become_active(), {
        let handle = handle.clone();
        move || handle.set_phase(AppPhase::Active)
    });
    let on_resign_active = observe(mtm, &NotificationName::application_will_resign_active(), {
        let handle = handle.clone();
        move || handle.set_phase(AppPhase::Inactive)
    });
    let on_did_hide = observe(mtm, &NotificationName::application_did_hide(), {
        let handle = handle.clone();
        move || handle.set_phase(AppPhase::Background)
    });
    let on_will_unhide = observe(
        mtm,
        &NotificationName::application_will_unhide(),
        move || {
            let mtm =
                MainThreadMarker::new().expect("notification observer runs on the main thread");
            let next = if Application::shared(mtm).is_active() {
                AppPhase::Active
            } else {
                AppPhase::Inactive
            };
            handle.set_phase(next);
        },
    );

    PhaseObservation {
        _observers: [
            on_become_active,
            on_resign_active,
            on_did_hide,
            on_will_unhide,
        ],
    }
}

#[cfg(target_os = "ios")]
const fn ios_phase(state: cocoa_ui::uikit::ApplicationState) -> AppPhase {
    use cocoa_ui::uikit::ApplicationState;
    match state {
        ApplicationState::Active => AppPhase::Active,
        ApplicationState::Inactive => AppPhase::Inactive,
        ApplicationState::Background => AppPhase::Background,
    }
}

#[cfg(target_os = "ios")]
fn dispatch_ios_background(handle: &PhaseHandle, mtm: MainThreadMarker) {
    let Some(work) = handle.transition_future(AppPhase::Background) else {
        return;
    };
    let background_task = cocoa_ui::uikit::begin_background_task(mtm, "waterui.phase_change");
    waterui_backend_core::task::spawn_local(async move {
        let _background_task = background_task;
        work.await;
    })
    .detach();
}

#[cfg(target_os = "ios")]
fn start_ios(phase: Phase, env: &Environment, mtm: MainThreadMarker) -> PhaseObservation {
    let initial = ios_phase(cocoa_ui::uikit::application_state(mtm));
    let handle = phase.start_with(env, initial);

    let on_become_active = observe(mtm, &NotificationName::application_did_become_active(), {
        let handle = handle.clone();
        move || handle.set_phase(AppPhase::Active)
    });
    let on_resign_active = observe(mtm, &NotificationName::application_will_resign_active(), {
        let handle = handle.clone();
        move || handle.set_phase(AppPhase::Inactive)
    });
    let on_will_enter_foreground = observe(
        mtm,
        &NotificationName::application_will_enter_foreground(),
        {
            let handle = handle.clone();
            move || handle.set_phase(AppPhase::Inactive)
        },
    );
    let on_did_enter_background = observe(
        mtm,
        &NotificationName::application_did_enter_background(),
        move || {
            let mtm =
                MainThreadMarker::new().expect("notification observer runs on the main thread");
            dispatch_ios_background(&handle, mtm);
        },
    );

    PhaseObservation {
        _observers: [
            on_become_active,
            on_resign_active,
            on_will_enter_foreground,
            on_did_enter_background,
        ],
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_phase_maps_hidden_active_and_inactive_states() {
        assert_eq!(macos_phase(true, false), AppPhase::Active);
        assert_eq!(macos_phase(false, false), AppPhase::Inactive);
        assert_eq!(macos_phase(false, true), AppPhase::Background);
        assert_eq!(macos_phase(true, true), AppPhase::Background);
    }
}
