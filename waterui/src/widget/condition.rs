use waterui_core::{Environment, View};
use waterui_reactive::{compute::ToComputed, ComputeExt};

use crate::component::Dynamic;

pub struct When<Condition, F, F2> {
    condition: Condition,
    when: F,
    or: F2,
}

impl<Condition, F1, F2, V1, V2> View for When<Condition, F1, F2>
where
    Condition: ToComputed<bool>,
    F1: 'static + Fn() -> V1,
    F2: 'static + Fn() -> V2,
    V1: View,
    V2: View,
{
    fn body(self, _env: &Environment) -> impl View {
        let (view, handle) = Dynamic::new();
        self.condition.to_compute().watch(move |condition| {
            if condition {
                handle.set((self.when)());
            } else {
                handle.set((self.or)());
            }
        });
        view
    }
}
