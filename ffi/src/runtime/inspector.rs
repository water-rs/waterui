//! Bringing up the inspector from a native backend.
//!
//! A backend knows what a user did — a secondary click, a long press, a menu
//! item — and the runtime knows what to do about it. These two entry points are
//! the whole of that: everything else about inspection already crosses no
//! boundary, because the endpoint runs inside the application.
//!
//! Both are inert unless an inspector endpoint is running, which in practice
//! means a debug build.

use crate::WuiEnv;

/// Opens the inspector for this application.
///
/// Reveals nothing in particular: use this where the backend cannot say which
/// element the user meant, such as a menu item that is about the application
/// rather than about a point on screen.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_open(env: *const WuiEnv) {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
        return;
    };
    inspector.open();
}

/// Reveals one node in the inspector, opening one if none is attached.
///
/// `node` is an accessibility node id, which is what the inspector's tree is
/// keyed by. A backend that publishes no tree has no id to pass and should call
/// [`waterui_inspector_open`] instead.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_inspect_node(env: *const WuiEnv, node: u64) {
    // SAFETY: as above.
    let env = unsafe { crate::borrow_ffi(env) };
    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
        return;
    };
    inspector.inspect_node(waterui::inspector::protocol::NodeId(node));
}

/// Whether this build offers inspection at all.
///
/// A backend asks before putting "Inspect element" in front of a user, so that
/// a release build shows nothing rather than an entry that does nothing.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_is_available(env: *const WuiEnv) -> bool {
    // SAFETY: as above.
    let env = unsafe { crate::borrow_ffi(env) };
    env.get::<waterui::inspector::InspectorRuntime>().is_some()
}
