use std::sync::mpsc::Sender;

use cef::rc::Rc as _;
use cef::{
    App, BrowserProcessHandler, CefString, CommandLine, ImplApp, ImplBrowserProcessHandler,
    ImplCommandLine, WrapApp, WrapBrowserProcessHandler,
};

use crate::runtime::PumpDeadline;

#[cfg(target_os = "macos")]
const MACOS_SYSTEM_NOTIFICATION_FEATURES: [&str; 2] =
    ["NativeNotifications", "SystemNotifications"];

#[cfg(target_os = "macos")]
fn merge_disabled_features(existing: &str, required: &[&str]) -> String {
    let mut features = existing
        .split(',')
        .filter(|feature| !feature.is_empty())
        .collect::<Vec<_>>();
    for feature in required {
        if !features.contains(feature) {
            features.push(feature);
        }
    }
    features.join(",")
}

#[cfg(target_os = "macos")]
fn disable_features(command_line: &CommandLine, required: &[&str]) {
    let switch_name: CefString = "disable-features".into();
    let existing = command_line.switch_value(Some(&switch_name));
    let existing = CefString::from(&existing).to_string();
    let features: CefString = merge_disabled_features(&existing, required).as_str().into();
    command_line.append_switch_with_value(Some(&switch_name), Some(&features));
}

cef::wrap_browser_process_handler! {
    struct WaterBrowserProcessHandler {
        schedule: Sender<PumpDeadline>,
    }

    impl BrowserProcessHandler {
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            let _ = self.schedule.send(PumpDeadline::after_millis(delay_ms));
        }
    }
}

cef::wrap_app! {
    struct WaterCefApp {
        handler: BrowserProcessHandler,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let command_line = command_line
                .expect("CEF must provide a command line before process launch");
            for switch in [
                "disable-background-networking",
                "disable-component-update",
                "disable-default-apps",
                "disable-domain-reliability",
                "disable-notifications",
                "disable-sync",
                "enable-gpu",
                "enable-gpu-compositing",
                "no-default-browser-check",
                "no-first-run",
                "no-startup-window",
            ] {
                command_line.append_switch(Some(&switch.into()));
            }
            #[cfg(target_os = "macos")]
            disable_features(command_line, &MACOS_SYSTEM_NOTIFICATION_FEATURES);
            #[cfg(all(target_os = "macos", debug_assertions))]
            command_line.append_switch(Some(&"use-mock-keychain".into()));
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(self.handler.clone())
        }
    }
}

pub fn new_app(schedule: Sender<PumpDeadline>) -> App {
    WaterCefApp::new(WaterBrowserProcessHandler::new(schedule))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{MACOS_SYSTEM_NOTIFICATION_FEATURES, merge_disabled_features};

    #[test]
    fn disables_macos_system_notifications_without_erasing_existing_features() {
        assert_eq!(
            merge_disabled_features(
                "ExistingFeature,NativeNotifications",
                &MACOS_SYSTEM_NOTIFICATION_FEATURES,
            ),
            "ExistingFeature,NativeNotifications,SystemNotifications"
        );
    }
}
