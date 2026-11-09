// The Apple OS versions a WaterUI API can require, written once: the build
// script and the library both `include!` this file.
//
// Each row is `(major, minor, "major.minor")`. Apple OS versions are unified
// across iOS, macOS, tvOS, watchOS and visionOS since 26, so one number
// covers every platform. A row is the value of the
// `waterui_apple_os_met` cfg the build script sets when the deployment target
// reaches it.
macro_rules! gated_versions {
    ($callback:ident) => {
        $callback! {
            (27, 0, "27.0")
        }
    };
}
