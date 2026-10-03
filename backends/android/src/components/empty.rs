//! `Native<()>` — the unit view every unconditional render must answer.
//!
//! The platform object is `android.widget.Space`: a view whose entire job is
//! to be a zero-size, never-drawn gap — exactly the role `()` plays in the
//! tree.

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;
use crate::jvm;
use crate::native_layout::EmptySubView;

/// Claims `Native<()>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<()>(|(), ctx| {
        let platform = ctx.platform();
        let space = jvm::with_env(|env| {
            platform
                .new_space(env)
                .expect("a Space constructs against the host context")
        });
        NativeLeaf::new(space, EmptySubView, platform)
    });
}
