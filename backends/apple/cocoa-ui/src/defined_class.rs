//! `objc2` `define_class` support for objects an Objective-C factory
//! allocates.
//!
//! `Self::alloc(mtm).set_ivars(..)` writes both the `#[ivars]` storage and
//! `objc2`'s `drop_flag` marker. A factory invoked on the subclass —
//! `+[UIButton buttonWithType:]`, `+[NSTextField labelWithString:]` —
//! allocates through `self`, so the object is the subclass and its ivar
//! storage is zero-filled, which is the initial state the `#[ivars]`
//! structs here start in; the factory just never runs `set_ivars`. These
//! helpers cover that gap: mark the storage initialized after the factory
//! returns, and read it without the initialized check for overrides the
//! factory itself may invoke first.

use objc2::{ClassType, DefinedClass};

/// Marks `this`'s ivars initialized — `0x0f`, `objc2`'s `InitializedIvars`
/// value.
///
/// Call after constructing `this` through an Objective-C factory: the
/// factory's `alloc` zero-fills the ivar storage but never runs
/// `set_ivars`, so `drop_flag` still reads `Allocated`, which both the
/// debug `ivars()` checks and the ivar destructor gate on. The zero-filled
/// storage already equals the initial ivars state, so this only records
/// what is already true.
pub fn mark_ivars_initialized<T: ClassType>(this: &T) {
    let cls = T::class();
    let Some(ivar) = cls.instance_variable(c"drop_flag") else {
        return;
    };
    // SAFETY: `drop_flag` is `objc2`'s ivar-state marker; `0x0f` is its
    // `InitializedIvars` value. The ivars hold valid zeroed state, so
    // this only records what is already true.
    unsafe {
        std::ptr::from_ref::<T>(this)
            .cast::<u8>()
            .cast_mut()
            .offset(ivar.offset())
            .write(0x0f);
    }
}

/// The `#[ivars]` storage read without the debug initialized-ivars check.
///
/// A factory may invoke an overridden method during its own
/// initialization — `layoutSubviews` under `labelWithString:`,
/// `setEnabled:` under `buttonWithType:` — before
/// [`mark_ivars_initialized`] could have run, so such overrides cannot go
/// through `ivars()`.
///
/// # Safety
///
/// `this` must be an instance of `T` whose `ivars` storage is either
/// `set_ivars`-written, [`mark_ivars_initialized`]-marked, or still
/// zero-filled, and that state must be valid for `T::Ivars`.
pub unsafe fn raw_ivars<T: ClassType + DefinedClass>(this: &T) -> &T::Ivars {
    // Looked up per call: a `static` in a generic function is one item
    // shared by every `T`, so caching the offset there would hand one
    // class's ivar offset to another.
    let offset = T::class()
        .instance_variable(c"ivars")
        .expect("objc2 stores DefinedClass ivars under the `ivars` ivar")
        .offset();
    // SAFETY: `ivars` is the storage `#[ivars]` registers, so the offset
    // points at a correctly aligned `T::Ivars`; the caller guarantees the
    // storage is in a valid state.
    unsafe {
        &*std::ptr::from_ref::<T>(this)
            .cast::<u8>()
            .offset(offset)
            .cast::<T::Ivars>()
    }
}
