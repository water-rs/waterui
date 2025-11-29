
use waterui_core::{Dynamic, binding, Binding, View, Environment};

fn main() {
    let counter: Binding<i32> = binding(0);
    let _view = Dynamic::watch(counter, |count: i32| {
         format!("Current value: {}", count)
    });
}

