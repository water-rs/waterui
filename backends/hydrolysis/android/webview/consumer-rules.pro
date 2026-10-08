# `AssetResponse.<init>` is the one member reached only by name — Rust's
# `nativeAssetRespond` constructs it through `GetMethodID` — so it is kept
# by exact signature as well as by the `@CalledFromNative` annotation the
# host module's rules preserve.
-keep class dev.waterui.hydrolysis.webview.AssetResponse {
    <init>(int, java.lang.String, byte[]);
}
