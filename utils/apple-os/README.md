# waterui-apple-os

Compile-time gates on the Apple OS version a build targets.

An API whose platform requirement is newer than the build's Apple deployment
target carries a bound on `Available<AppleOs<MAJOR, MINOR>, Api>`. The build
script reads `rustc --print deployment-target`, so the bound is satisfied only
when the deployment target (the `*_DEPLOYMENT_TARGET` environment the `water`
CLI sets from `[apple.deployment-targets]` in `Water.toml`) reaches the version.
Non-Apple targets always satisfy it.

`apple_available!(27.0)` is the runtime selection between a realization for
the newer OS and one for the older: `objc2::available!` for every Apple
platform, plus a `#[deprecated]` warning at the call site once the deployment
target already meets the version, so a floor raise fails CI at every older
realization left to delete.

Zero dependencies apart from `objc2` on Apple targets; `no_std`.
