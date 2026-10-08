# Members the native library calls by name through JNI, kept by annotation:
# the rule travels with this library instead of every app's own rules.
# `keepclasseswithmembers`, not `keepclassmembers`: a class reachable only
# through `FindClass` — `dev.waterui.hydrolysis.webview.HydrolysisWebView` —
# would otherwise be dropped whole, taking its kept members with it.
-keepclasseswithmembers class * {
    @dev.waterui.hydrolysis.CalledFromNative *;
}

# The annotation itself must survive for the rule above to match.
-keep class dev.waterui.hydrolysis.CalledFromNative

# `AssetResponse.<init>` is the one member reached only by name — Rust's
# `nativeAssetRespond` constructs it through `GetMethodID` — so it is kept
# by exact signature as well as by annotation.
-keep class dev.waterui.hydrolysis.webview.AssetResponse {
    <init>(int, java.lang.String, byte[]);
}
