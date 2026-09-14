# Add project specific ProGuard rules here.
# See https://developer.android.com/studio/build/shrink-code for more details.

# The WaterUI runtime ships its own consumer rules covering every class and
# member the Rust side reaches by name (FFI structs, watcher metadata, the
# WebView bridge), and the default proguard-android-optimize.txt already
# preserves `native` methods. Nothing else in the generated app needs keeping:
# manifest-declared components and code reached from them are kept by the
# default rules automatically.
