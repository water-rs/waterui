//! Integration tests for `#[track_caller]` panic attribution and selector,
//! nearest-candidate, and wait-elapsed failure diagnostics (water-rs/waterui#4320).

use std::any::Any;
use std::cell::RefCell;
use std::sync::Once;
use std::time::Duration;

use waterui::component::{button, text, vstack};
use waterui_testing::{Role, Selector, ui};

thread_local! {
    static LAST_PANIC_LOCATION: RefCell<Option<(String, u32)>> = const { RefCell::new(None) };
}

static INSTALL_HOOK: Once = Once::new();

fn ensure_location_hook() {
    INSTALL_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            LAST_PANIC_LOCATION.with(|slot| {
                *slot.borrow_mut() = info
                    .location()
                    .map(|loc| (loc.file().to_owned(), loc.line()));
            });
            previous(info);
        }));
    });
}

fn catch_panic_with_location(f: impl FnOnce()) -> (String, Option<(String, u32)>) {
    ensure_location_hook();
    LAST_PANIC_LOCATION.with(|slot| {
        *slot.borrow_mut() = None;
    });
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .expect_err("expected assertion to panic");
    let location = LAST_PANIC_LOCATION.with(|slot| slot.borrow_mut().take());
    (panic_message(&*payload), location)
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| String::from("<non-string panic payload>"))
}

#[test]
fn query_assertions_report_caller_location_selector_and_nearest_candidates() {
    let mut app = ui().mount(|| vstack((text("Count: 2"), button("Save").action(|| {}))));

    let expected_line = line!() + 5;
    let (message, location) = catch_panic_with_location(|| {
        app.query()
            .role(Role::LABEL)
            .label("Count: 3")
            .assert_exists();
    });

    let (file, line) = location.expect("panic must record caller location");
    assert!(
        file.ends_with("track_caller_diagnostics.rs"),
        "caller file should point to integration test, got {file}"
    );
    assert_eq!(
        line, expected_line,
        "caller line should point to assertion line"
    );
    assert!(
        message.contains("label=\"Count: 3\""),
        "failure message must report selector: {message}"
    );
    assert!(
        message.contains("nearest:") && message.contains("Count: 2"),
        "failure message must report nearest candidate nodes: {message}"
    );
}

#[test]
fn timed_and_frame_assertions_report_caller_location_candidates_and_elapsed_wait() {
    let mut app = ui().mount(|| vstack((text("Status"), button("Retry").action(|| {}))));

    let frame_line = line!() + 5;
    let (frame_msg, frame_loc) = catch_panic_with_location(|| {
        app.query()
            .role(Role::BUTTON)
            .label("Retry")
            .assert_value_eq_within_frames("done", 3);
    });
    let (file, line) = frame_loc.expect("frame assertion panic must record caller location");
    assert!(file.ends_with("track_caller_diagnostics.rs"));
    assert_eq!(line, frame_line);
    assert!(
        frame_msg.contains("label=\"Retry\"") && frame_msg.contains("value=\"done\""),
        "frame wait assertion must report selector and expected value: {frame_msg}"
    );
    assert!(
        frame_msg.contains("nearest:") && frame_msg.contains("Button \"Retry\""),
        "frame wait assertion must report nearest candidate role and label: {frame_msg}"
    );
    assert!(
        frame_msg.contains("waited ") && frame_msg.contains("frames"),
        "frame wait assertion must report waited frames/time: {frame_msg}"
    );

    let timed_line = line!() + 5;
    let (timed_msg, timed_loc) = catch_panic_with_location(|| {
        app.query()
            .role(Role::LABEL)
            .label("Missing")
            .assert_exists_within(Duration::from_millis(5));
    });
    let (timed_file, timed_actual_line) =
        timed_loc.expect("timed assertion panic must record caller location");
    assert!(timed_file.ends_with("track_caller_diagnostics.rs"));
    assert_eq!(timed_actual_line, timed_line);
    assert!(
        timed_msg.contains("label=\"Missing\""),
        "timed wait assertion must report selector: {timed_msg}"
    );
    assert!(
        timed_msg.contains("waited ") && timed_msg.contains("frames"),
        "timed wait assertion must report waited frames/time: {timed_msg}"
    );
}

#[test]
fn semantic_app_and_element_terminals_report_caller_location() {
    let mut app = ui().mount(|| vstack((button("Save").action(|| {}),)));
    let selector = Selector::new().role(Role::BUTTON).label("Save");

    let value_line = line!() + 2;
    let (value_msg, value_loc) = catch_panic_with_location(|| {
        app.assert_value_eq(&selector, "expected");
    });
    let (file, line) = value_loc.expect("assert_value_eq panic must record caller location");
    assert!(file.ends_with("track_caller_diagnostics.rs"));
    assert_eq!(line, value_line);
    assert!(
        value_msg.contains("label=\"Save\"") && value_msg.contains("expected \"expected\""),
        "assert_value_eq must report selector and expected/actual: {value_msg}"
    );

    let save = app.query().role(Role::BUTTON).label("Save").single();
    let elem_line = line!() + 2;
    let (_elem_msg, elem_loc) = catch_panic_with_location(|| {
        save.increment(&mut app);
    });
    let (elem_file, elem_actual_line) =
        elem_loc.expect("ElementRef action panic must record caller location");
    assert!(elem_file.ends_with("track_caller_diagnostics.rs"));
    assert_eq!(elem_actual_line, elem_line);
}
