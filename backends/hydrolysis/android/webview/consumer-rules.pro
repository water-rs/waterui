# `AssetResponse.<init>` is the one member reached only by name — Rust's
# `nativeAssetRespond` constructs it through `GetMethodID` — so it is kept
# by exact signature as well as by the `@CalledFromNative` annotation the
# host module's rules preserve.
-keep class dev.waterui.hydrolysis.webview.AssetResponse {
    <init>(int, java.lang.String, byte[]);
}

# `postBridgeReply` takes the listener's `JavaScriptReplyProxy`, and Rust
# resolves it through `GetMethodID` by a descriptor that names the class, so
# the class keeps its name. Only the name: the class itself is reachable
# from the listener, and its members shrink as usual.
-keepnames class androidx.webkit.JavaScriptReplyProxy
