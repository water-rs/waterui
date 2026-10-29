//! Checks for the Reply sample: its semantic behavior on the accessibility
//! tree, and the remote images repainting on the rendered runtime.

use core::time::Duration;
use reply_example::{app, demo};
use waterui::Environment;
use waterui_testing::{Role, SemanticApp, mount_app};

fn reply_all_buttons(app: &mut SemanticApp) -> usize {
    app.query()
        .role(Role::BUTTON)
        .label("Reply all")
        .all()
        .len()
}

/// Opening a thread shows its messages in the reading pane, and the search
/// field narrows the inbox.
#[waterui::test(demo, viewport = (1000, 700))]
fn thread_opens_and_search_filters(app: &mut SemanticApp) {
    app.query()
        .role(Role::LIST_ITEM)
        .label_contains("Dinner Club")
        .assert_exists();
    // The initial selection is the three-message Dinner Club thread.
    assert_eq!(reply_all_buttons(app), 3);

    app.query()
        .role(Role::LIST_ITEM)
        .label_contains("This food show")
        .tap();
    assert_eq!(reply_all_buttons(app), 1);

    app.query().label("Search replies").set_text("noodle");
    app.query()
        .role(Role::LIST_ITEM)
        .label_contains("豆花鱼")
        .assert_not_exists();
    app.query()
        .role(Role::LIST_ITEM)
        .label_contains("Dinner Club")
        .assert_exists();
}

/// Remote `Photo`/`avatar` sources fetch over the network on a background
/// thread and publish into the tree asynchronously. This test mounts the real
/// app env, lets the runtime pump while fetch work is in flight, and verifies
/// the frame repaints with decoded image content rather than keeping the flat
/// initials fallback.
#[test]
#[ignore = "requires network access to the sample's published image assets"]
fn remote_avatars_render() {
    // Mirror the generated backend: `mount_app` installs the M3 style's
    // tokens on the app env.
    let mut app = mount_app(
        app(Environment::new()),
        hydrolysis_m3::Material3::defaults(),
    );
    let before = app.snapshot();
    // Pump hot while spawned fetch work is pending; the timeout is wall-clock
    // because the network round-trip lives outside the virtual clock.
    let _ = app.pump_until(Duration::from_secs(15), || false);
    let after = app.snapshot();

    assert_ne!(
        before.rgba8, after.rgba8,
        "the frame must repaint once the remote images decode"
    );
}
