# Members the native library calls by name through JNI, kept by annotation:
# the rule travels with this library instead of every app's own rules.
-keepclassmembers class * {
    @dev.waterui.hydrolysis.CalledFromNative *;
}

# The annotation itself must survive for the rule above to match.
-keep class dev.waterui.hydrolysis.CalledFromNative

# JNI export names (`Java_dev_waterui_*` on the native side) — R8 cannot see
# the calls either, and renamed `external` functions break the link.
-keepclasseswithmembernames class dev.waterui.hydrolysis.** {
    native <methods>;
}
