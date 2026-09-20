//! Visual checks for the Reply sample against the offscreen runtime.

use core::time::Duration;
use reply_example::app;
use waterui::Environment;
use waterui_testing::{Snapshot, mount_app};

fn pixel(snapshot: &Snapshot, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * snapshot.width + x) * 4) as usize;
    snapshot.rgba8[offset..offset + 4]
        .try_into()
        .expect("pixel in bounds")
}

/// The first list card's avatar sits at (104,112)–(144,152) logical points;
/// the default scale factor renders one pixel per point.
const AVATAR_CENTER: (u32, u32) = (124, 132);

/// Remote `Photo`/`avatar` sources fetch over the network on a background
/// thread and publish into the tree asynchronously. This test mounts the real
/// app env, lets the runtime pump while fetch work is in flight, and verifies
/// the avatar region repaints with decoded image content rather than keeping
/// the flat initials fallback.
#[test]
#[ignore = "requires network access to the sample's published image assets"]
fn remote_avatars_render() {
    // Mirror the generated backend: `mount_app` installs the M3 style's
    // tokens on the app env; the example's `themed` overlay then applies the
    // sample's custom scheme.
    let mut app = mount_app(
        app(Environment::new()),
        hydrolysis_m3::Material3::defaults(),
    );
    let before = app.snapshot();
    // Pump hot while spawned fetch work is pending; the timeout is wall-clock
    // because the network round-trip lives outside the virtual clock.
    let _ = app.pump_until(Duration::from_secs(15), || false);
    let after = app.snapshot();

    let before_px = pixel(&before, AVATAR_CENTER.0, AVATAR_CENTER.1);
    let after_px = pixel(&after, AVATAR_CENTER.0, AVATAR_CENTER.1);
    assert_ne!(
        before_px, after_px,
        "avatar region must repaint once the remote image decodes (before={before_px:?} after={after_px:?})"
    );
}
