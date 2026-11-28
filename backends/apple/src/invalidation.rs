//! `invalidateCapturedRendering`: the superview walk to an enclosing
//! effect view's rendered-content sink.
//!
//! Leaves that change what they drew (a shape's fill, a picture's bitmap)
//! call [`invalidate_rendered_content`]; an enclosing `view_effect` leaf
//! registered its content view through [`register_sink`] and re-renders its
//! capture — `WuiRenderedContentInvalidationSink`.

#[cfg(any(
    target_os = "macos",
    feature = "applied_filter",
    feature = "view_effect"
))]
use alloc::rc::Rc;
use alloc::rc::Weak;
use cocoa_ui::PlatformView;
use std::collections::HashMap;
use std::sync::Mutex;

/// Mounted content view → the owning effect's invalidation callback.
/// `Weak` entries are only ever upgraded on the main queue — the sinks are
/// main-thread objects crossing through the shared table.
struct SendableWeak(Weak<dyn Fn()>);

#[allow(clippy::non_send_fields_in_send_ty)]
// SAFETY: `invalidate` runs the closure through the main queue, so the weak
// target is only touched where it lives.
unsafe impl Send for SendableWeak {}

static SINKS: Mutex<Option<HashMap<usize, SendableWeak>>> = Mutex::new(None);

/// The invariant behind locking [`SINKS`]: every critical section is a
/// single map operation that cannot panic, so the lock is never poisoned.
const REGISTRY_UNPOISONED: &str =
    "the rendered-content invalidation registry's critical sections cannot panic";

/// The map key for a platform view.
fn key(view: &PlatformView) -> usize {
    core::ptr::from_ref(view).cast::<u8>() as usize
}

/// Registers `callback` as the rendered-content invalidation sink for
/// `view` — the identity `WuiRenderedContentInvalidationSink` conformed to.
///
/// The entry is weak: it clears itself when the owning leaf drops.
#[allow(clippy::needless_pass_by_value)]
#[cfg(any(
    target_os = "macos",
    feature = "applied_filter",
    feature = "view_effect"
))]
pub fn register_sink(view: &PlatformView, callback: Rc<dyn Fn()>) {
    SINKS
        .lock()
        .expect(REGISTRY_UNPOISONED)
        .get_or_insert_with(HashMap::new)
        .insert(key(view), SendableWeak(Rc::downgrade(&callback)));
}

/// Removes the sink for `view`, if any — a leaf tearing down before its
/// callback would die on its own.
#[cfg(any(
    target_os = "macos",
    feature = "applied_filter",
    feature = "view_effect"
))]
pub fn unregister_sink(view: &PlatformView) {
    if let Some(sinks) = SINKS.lock().expect(REGISTRY_UNPOISONED).as_mut() {
        sinks.remove(&key(view));
    }
}

/// The full layout invalidation shared by reactive layout watchers and the
/// GPU measure-change path — `invalidateLayoutHierarchy`: bump the measure
/// epoch so no leaf answers a stale size, mark `view` and every native
/// ancestor for layout, then notify an enclosing rendered-content capture.
///
/// All three halves are required together: the epoch bump alone never
/// reschedules a layout pass, the native walk alone leaves memoized leaf
/// measures serving the size that is no longer true, and skipping the
/// capture walk leaves an enclosing effect compositing stale pixels.
pub fn invalidate_layout_hierarchy(view: &PlatformView) {
    crate::measure_memo::invalidate();
    cocoa_ui::view::invalidate_layout(view);
    invalidate_rendered_content(view);
}

/// Walks `view`'s superview chain to the nearest registered sink and calls
/// it — `PlatformView.invalidateCapturedRendering`.
pub fn invalidate_rendered_content(view: &PlatformView) {
    let mut ancestor = cocoa_ui::view::superview(view);
    while let Some(current) = ancestor {
        let sink = SINKS
            .lock()
            .expect(REGISTRY_UNPOISONED)
            .as_ref()
            .and_then(|sinks| sinks.get(&key(&current)).map(|sink| sink.0.clone()))
            .and_then(|sink| sink.upgrade());
        if let Some(sink) = sink {
            sink();
            return;
        }
        ancestor = cocoa_ui::view::superview(&current);
    }
}
