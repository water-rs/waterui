use std::sync::mpsc::Sender;

use cef::rc::Rc as _;
use cef::{
    App, BrowserProcessHandler, CefString, CommandLine, ImplApp, ImplBrowserProcessHandler,
    ImplCommandLine, WrapApp, WrapBrowserProcessHandler,
};

use crate::runtime::PumpDeadline;

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
            ] {
                command_line.append_switch(Some(&switch.into()));
            }
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
