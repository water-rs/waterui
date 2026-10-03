//! `Native<()>` — the unit view every unconditional render must answer.
//!
//! The platform object is `android.widget.Space`: a view whose entire job is
//! to be a zero-size, never-drawn gap — exactly the role `()` plays in the
//! tree.

use waterui_backend_core::Native;

use crate::contract::{NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;
use crate::jvm;
use crate::native_layout::EmptySubView;

/// Claims `Native<()>`.
pub(crate) fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<()>(|(), _ctx| {
        let space = jvm::with_env(|env| {
            jvm::globals()
                .bindings()
                .new_space(env)
                .expect("a Space constructs against the host context")
        });
        NativeLeaf::new(space, EmptySubView)
    });
}
