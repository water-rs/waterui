#![no_std]
#![doc = include_str!("../README.md")]

extern crate self as waterui_apple_os;

include!("../versions.rs");

/// The marker for an Apple OS version: `AppleOs<27, 0>` is OS 27.0 on every
/// Apple platform.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct AppleOs<const MAJOR: u16, const MINOR: u16>;

/// Holds when a build's Apple deployment target reaches `Version`, a
/// [`AppleOs`]; `Api` names the API that needs it, for the error message.
///
/// An API gates itself with a bound on one of its own generic parameters
/// (`F: Available<AppleOs<27, 0>, NavigationPolicy>`), so the error appears at
/// the call that uses it instead of where the API is defined. Every type
/// implements it when the deployment target reaches the version, and on
/// targets that are not Apple.
#[diagnostic::on_unimplemented(
    message = "`{Api}` is unavailable: it needs `{Version}`, above this build's Apple deployment target",
    label = "needs `{Version}`",
    note = "`AppleOs<MAJOR, MINOR>` is Apple OS MAJOR.MINOR on every platform; raise the platform you target under `[apple.deployment-targets]` in `Water.toml` to MAJOR.MINOR or newer"
)]
pub trait Available<Version, Api> {}

macro_rules! implement {
    ($(($major:literal, $minor:literal, $cfg:literal)),* $(,)?) => {
        $(
            #[cfg(waterui_apple_os_met = $cfg)]
            impl<T, Api> Available<AppleOs<$major, $minor>, Api> for T {}

            impl AppleOs<$major, $minor> {
                /// Referenced by `apple_available!`; deprecated once the build's
                /// deployment target reaches this version.
                #[doc(hidden)]
                #[cfg_attr(
                    waterui_apple_os_met = $cfg,
                    deprecated(
                        note = "always true: this build's deployment target already meets this version; delete the older realization"
                    )
                )]
                pub const GATED_VERSION: () = ();
            }
        )*
    };
}
gated_versions!(implement);

#[cfg(target_vendor = "apple")]
#[doc(hidden)]
pub use objc2;
