//! The four declarations every `WebViewHandle` method needs, written once.
//!
//! A method a backend implements has to appear four times before a caller can
//! reach it: on the public trait, on the object-safe shim behind it, in the
//! blanket impl bridging the two, and on the wrapper holding the boxed handle.
//! Adding one meant four coordinated edits, with the documentation copied
//! verbatim between the first and the last — and the failure mode is quiet, in
//! that a method left off the wrapper simply cannot be called and nothing says
//! so.
//!
//! Most of those four are mechanical, so [`webview_handle`] writes them from one
//! signature. A few methods are not mechanical: their shape changes at the
//! boundary, because the public form takes `impl Trait` and an object-safe one
//! cannot. Those are written out per layer in the `…_extra` sections, where the
//! `Box::new`, `into_computed` and `Box::pin` that bridge them stay visible.
//! Encoding those conversions as macro syntax would hide the only interesting
//! part behind the most punctuation.

/// Writes the public trait, the object-safe shim, the bridge between them, and
/// the wrapper's inherent methods.
///
/// A `uniform` entry has one signature that serves all four layers, and its
/// documentation is attached once and appears on both public surfaces. The
/// `_extra` sections are spliced in verbatim, for the methods whose signature
/// differs between the ergonomic form and the object-safe one.
macro_rules! webview_handle {
    (
        $(#[doc = $trait_doc:literal])*
        pub trait $public:ident;
        trait $shim:ident;
        impl $wrapper:ident . $field:ident;

        uniform {
            $(
                $(#[doc = $doc:literal])*
                fn $name:ident(&self $(, $arg:ident: $ty:ty)* $(,)?) $(-> $ret:ty)?;
            )*
        }

        public extra { $($public_extra:tt)* }
        shim extra { $($shim_extra:tt)* }
        bridge extra { $($bridge_extra:tt)* }
        wrapper extra { $($wrapper_extra:tt)* }
    ) => {
        $(#[doc = $trait_doc])*
        pub trait $public: Any {
            $(
                $(#[doc = $doc])*
                fn $name(&self $(, $arg: $ty)*) $(-> $ret)?;
            )*
            $($public_extra)*
        }

        /// The object-safe form of the trait above.
        ///
        /// `WebViewHandle` is written for the person implementing a backend —
        /// `impl Signal`, `impl Future`, `impl Fn` — which is not object safe,
        /// and the wrapper needs a `dyn` to hold. Type erasure is an
        /// implementation detail, so it lives here rather than in the public
        /// signature.
        trait $shim: Any {
            $(
                fn $name(&self $(, $arg: $ty)*) $(-> $ret)?;
            )*
            $($shim_extra)*
        }

        impl<T: $public> $shim for T {
            $(
                fn $name(&self $(, $arg: $ty)*) $(-> $ret)? {
                    $public::$name(self $(, $arg)*)
                }
            )*
            $($bridge_extra)*
        }

        impl $wrapper {
            $(
                $(#[doc = $doc])*
                pub fn $name(&self $(, $arg: $ty)*) $(-> $ret)? {
                    self.$field.$name($($arg),*)
                }
            )*
            $($wrapper_extra)*
        }
    };
}
