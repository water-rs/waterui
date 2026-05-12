extern crate alloc;

use alloc::rc::Rc;

use nami::Binding;
use waterui_core::{Environment, LocalStateScope, LocalStateStore};

fn local_scope(env: &Environment) -> LocalStateScope {
    env.get::<LocalStateScope>()
        .unwrap_or_else(|| {
            panic!("waterui-chart requires renderer LocalStateScope support for internal state")
        })
        .clone()
}

pub fn local_binding<T: Clone + 'static>(
    env: &Environment,
    init: impl FnOnce() -> T + 'static,
) -> Binding<T> {
    let scope = local_scope(env);
    env.get::<LocalStateStore>()
        .unwrap_or_else(|| {
            panic!("waterui-chart requires renderer LocalStateStore support for internal state")
        })
        .binding(&scope, init)
}

pub fn local_shared<T: 'static>(env: &Environment, init: impl FnOnce() -> T + 'static) -> Rc<T> {
    let scope = local_scope(env);
    env.get::<LocalStateStore>()
        .unwrap_or_else(|| {
            panic!("waterui-chart requires renderer LocalStateStore support for internal state")
        })
        .get_or_init(&scope, init)
}
