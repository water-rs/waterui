mod computed;

mod binding;

mod components;
mod ty;
use alloc::boxed::Box;
use core::mem::ManuallyDrop;
pub(crate) use ty::*;

#[doc(inline)]
pub use components::AnyView;

type Int = isize;

#[no_mangle]
unsafe extern "C" fn waterui_free_action(action: Action) {
    let _: Box<dyn Fn()> = action.into();
}

#[no_mangle]
extern "C" fn waterui_call_action(action: Action) {
    let f: ManuallyDrop<Box<dyn Fn()>> = ManuallyDrop::new(action.into());
    f()
}
