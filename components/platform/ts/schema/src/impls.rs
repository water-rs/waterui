//! [`TsType`] for the types the Rust-to-TypeScript mapping defines.
//!
//! A nested schema is written as `&<T as TsType>::SCHEMA`: inside a `const`
//! initializer the reference is extended to `'static`, so an arbitrarily deep
//! tree is still one constant and the compiler has already resolved every
//! alias and generic parameter by the time it is encoded.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use crate::{TsMapKey, TsType, TypeSchema, tree::NumberKind};

impl TsType for () {
    const SCHEMA: TypeSchema = TypeSchema::Unit;
}

impl TsType for bool {
    const SCHEMA: TypeSchema = TypeSchema::Bool;
}

/// One [`TypeSchema::Number`] impl per Rust numeric type.
macro_rules! number_schemas {
    ($($ty:ty => $kind:ident),* $(,)?) => {
        $(
            impl TsType for $ty {
                const SCHEMA: TypeSchema = TypeSchema::Number(NumberKind::$kind);
            }
        )*
    };
}

number_schemas! {
    f32 => F32,
    f64 => F64,
    i8 => I8,
    i16 => I16,
    i32 => I32,
    u8 => U8,
    u16 => U16,
    u32 => U32,
    i64 => I64,
    u64 => U64,
    isize => Isize,
    usize => Usize,
}

impl TsType for String {
    const SCHEMA: TypeSchema = TypeSchema::String;
}

impl TsMapKey for String {}

impl TsType for &'static str {
    const SCHEMA: TypeSchema = TypeSchema::String;
}

impl TsMapKey for &'static str {}

impl<T: TsType> TsType for Option<T> {
    const SCHEMA: TypeSchema = TypeSchema::Option(&T::SCHEMA);
}

impl<T: TsType> TsType for Vec<T> {
    const SCHEMA: TypeSchema = TypeSchema::List(&T::SCHEMA);
}

impl<T: TsType> TsType for &'static [T] {
    const SCHEMA: TypeSchema = TypeSchema::List(&T::SCHEMA);
}

impl<T: TsType, const N: usize> TsType for [T; N] {
    const SCHEMA: TypeSchema = TypeSchema::List(&T::SCHEMA);
}

impl<K: TsMapKey, V: TsType> TsType for BTreeMap<K, V> {
    const SCHEMA: TypeSchema = TypeSchema::Map {
        key: &K::SCHEMA,
        value: &V::SCHEMA,
    };
}

impl<K: TsMapKey, V: TsType, S> TsType for HashMap<K, V, S> {
    const SCHEMA: TypeSchema = TypeSchema::Map {
        key: &K::SCHEMA,
        value: &V::SCHEMA,
    };
}

/// One [`TypeSchema::Callback`] impl per callable arity, for both boxed and
/// reference-counted closures.
macro_rules! callback_schemas {
    ($($argument:ident),*) => {
        impl<$($argument: TsType),*> TsType for Box<dyn Fn($($argument),*)> {
            const SCHEMA: TypeSchema = TypeSchema::Callback(&[$(<$argument as TsType>::SCHEMA),*]);
        }

        impl<$($argument: TsType),*> TsType for Rc<dyn Fn($($argument),*)> {
            const SCHEMA: TypeSchema = TypeSchema::Callback(&[$(<$argument as TsType>::SCHEMA),*]);
        }
    };
}

callback_schemas!();
callback_schemas!(A);
callback_schemas!(A, B);
callback_schemas!(A, B, C);
callback_schemas!(A, B, C, D);

#[cfg(feature = "waterui")]
mod waterui {
    use super::{TsMapKey, TsType, TypeSchema};

    impl<T: TsType + 'static> TsType for waterui_core::Binding<T> {
        const SCHEMA: TypeSchema = TypeSchema::Signal(&T::SCHEMA);
    }

    impl<T: TsType + 'static> TsType for waterui_core::Computed<T> {
        const SCHEMA: TypeSchema = TypeSchema::Accessor(&T::SCHEMA);
    }

    impl TsType for waterui_core::AnyView {
        const SCHEMA: TypeSchema = TypeSchema::View;
    }

    impl TsType for suiteki::Str {
        const SCHEMA: TypeSchema = TypeSchema::String;
    }

    impl TsMapKey for suiteki::Str {}
}
