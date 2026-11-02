//! Awaiting the content a captured subtree owes — the Apple half of the
//! [`ContentLedger`] contract the `preview` entry and the `view_renderer`
//! service share.
//!
//! A capture mounts its subtree under a fresh ledger, then awaits it here
//! before rasterizing: the wait yields to the main run loop, so the mount's
//! implicit transaction commits, `Appear` hooks run and the loads they start
//! land; between passes the subtree is laid out, displayed and its
//! transaction flushed, so content that resolved — a decoded photo taking its
//! size — is in the layer tree the capture reads.

use cocoa_ui::PlatformView;
use waterui_core::ContentLedger;
use waterui_core::content_ledger::ContentError;

/// Awaits `ledger` for the subtree rooted at `root`, bounded by `deadline`.
///
/// # Errors
///
/// [`ContentError::Pending`] naming the views still owing content when the
/// deadline passes, and [`ContentError::Failed`] when a load fails.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread with the views it waits for"
)]
pub async fn await_content(
    ledger: &ContentLedger,
    root: &PlatformView,
    deadline: core::time::Duration,
) -> Result<(), ContentError> {
    let root = cocoa_ui::view::retain_base(root);
    ledger
        .wait(deadline, waterui::task::sleep(deadline), || {
            drain(&root);
            // The main executor's next turn runs after the work the drain
            // queued — a load an `Appear` hook spawned.
            waterui::task::yield_now()
        })
        .await
}

/// Commits what resolved content changed: layout, display and the implicit
/// transaction, so `Appear` hooks run and the layer tree carries the
/// content.
fn drain(root: &PlatformView) {
    cocoa_ui::view::layout_immediately(root);
    cocoa_ui::view::display_immediately(root);
    cocoa_ui::core_animation::flush_transaction();
}
