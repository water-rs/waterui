# The Rust handle drives the wrapper by JNI: the class and the instance
# methods it calls must keep their names, `create` included. `-keep` (not
# `-keepclassmembers`): nothing on the JVM references the class, so member
# rules alone let R8 drop the whole wrapper.
-keep class dev.waterui.hydrolysis.webview.HydrolysisWebView {
    public <methods>;
    native <methods>;
}

# `nativeAssetRespond` constructs this by name; the constructor signature is
# the JNI contract.
-keep class dev.waterui.hydrolysis.webview.AssetResponse {
    <init>(int, java.lang.String, byte[]);
}
