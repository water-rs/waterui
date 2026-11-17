//! Views that are one of several view types, chosen when they are built.
//!
//! A branch that produces different view types — an `if`/`else`, a `match` —
//! returns a `OneOfN`, whose variant holds the branch taken. The view keeps
//! every branch's type, so the active branch dispatches statically:
//!
//! ```rust
//! use waterui_core::{OneOf2, View};
//!
//! fn status(online: bool) -> impl View {
//!     if online {
//!         OneOf2::A("Online")
//!     } else {
//!         OneOf2::B(())
//!     }
//! }
//! ```
//!
//! `#[view_builder]` and `view!` lower `if` and `match` branches to these
//! types, so view code rarely names them.

use crate::backend::Backend;
use crate::{AnyView, Environment, View};

macro_rules! one_of {
    ($(#[$doc:meta])* $name:ident { $($variant:ident),+ }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[expect(
            clippy::exhaustive_enums,
            reason = "a choice among exactly N views: callers build and match every variant"
        )]
        pub enum $name<$($variant),+> {
            $(
                #[doc = concat!("The `", stringify!($variant), "` branch.")]
                $variant($variant),
            )+
        }

        impl<$($variant: View),+> View for $name<$($variant),+> {
            /// The erased form, for code that expands views by hand.
            fn body(self, _env: &Environment) -> impl View {
                match self {
                    $(Self::$variant(view) => AnyView::new(view),)+
                }
            }

            fn dispatch<Target: Backend>(
                self,
                backend: &mut Target,
                env: &Environment,
                cx: Target::Ctx,
            ) -> Target::Output {
                match self {
                    $(Self::$variant(view) => view.dispatch(backend, env, cx),)+
                }
            }
        }
    };
}

one_of!(
    /// A view that is one of two view types.
    OneOf2 { A, B }
);
one_of!(
    /// A view that is one of three view types.
    OneOf3 { A, B, C }
);
one_of!(
    /// A view that is one of four view types.
    OneOf4 { A, B, C, D }
);
one_of!(
    /// A view that is one of five view types.
    OneOf5 { A, B, C, D, E }
);
one_of!(
    /// A view that is one of six view types.
    OneOf6 { A, B, C, D, E, F }
);
one_of!(
    /// A view that is one of seven view types.
    OneOf7 {
        A,
        B,
        C,
        D,
        E,
        F,
        G
    }
);
one_of!(
    /// A view that is one of eight view types. A choice among more types
    /// nests: the last variant holds another `OneOfN`.
    OneOf8 {
        A,
        B,
        C,
        D,
        E,
        F,
        G,
        H
    }
);

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::any::type_name;

    use super::*;
    use crate::metadata::MetadataKey;
    use crate::{Metadata, NativeView};

    /// Records the payload type each view reaches.
    #[derive(Default)]
    struct Recorder(Vec<&'static str>);

    impl Backend for Recorder {
        type Output = ();
        type Ctx = ();
        type Slot = AnyView;

        fn native<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {
            self.0.push(type_name::<T>());
        }

        fn metadata<K: MetadataKey>(&mut self, _metadata: Metadata<K>, _env: &Environment, (): ()) {
            self.0.push(type_name::<K>());
        }

        fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) {
            self.0.push("erased");
        }
    }

    #[test]
    fn the_active_branch_dispatches_through_its_own_type() {
        let mut recorder = Recorder::default();
        let env = Environment::new();
        OneOf3::<(), &'static str, ()>::B("text").dispatch(&mut recorder, &env, ());
        OneOf3::<&'static str, (), ()>::C(()).dispatch(&mut recorder, &env, ());
        assert!(!recorder.0.contains(&"erased"));
        assert_eq!(recorder.0.len(), 2);
    }
}
