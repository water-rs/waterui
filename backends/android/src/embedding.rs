//! Mounting an `AnyView` into the host's root `ViewGroup`.
//!
//! One function, once, at `nativeCreate`: dispatch renders the window's
//! content into a leaf, the leaf mounts into the host's root, and the
//! window's resolved background is painted onto that root. Everything else
//! about the platform object is the leaf's own business.

use waterui_backend_core::{AnyView, Environment};

use crate::contract::{Mounted, NativeLeaf, PlatformView};
use crate::jvm;

/// Renders `content` and adds it to `root`, installing the intrinsic-measure
/// bridge and owning the result. The returned handle is the window's content
/// lifetime: dropping it detaches the view and releases the leaf.
pub(crate) fn mount_content(
    content: AnyView,
    root: &PlatformView,
    env: &Environment,
) -> jni::errors::Result<Mounted> {
    let leaf: NativeLeaf = crate::dispatch::render(content, env);
    Ok(leaf.mount(root))
}

/// Paints `color` onto `root` — the window's resolved background, which no
/// leaf owns.
pub(crate) fn set_root_background(
    root: &PlatformView,
    color: waterui::graphics::color::WorkingColor,
) {
    jvm::with_env(|env| {
        jvm::globals()
            .bindings()
            .set_background_color(env, root.as_ref(), crate::theme::working_to_argb(color))
            .expect("setBackgroundColor must not throw")
    });
}
