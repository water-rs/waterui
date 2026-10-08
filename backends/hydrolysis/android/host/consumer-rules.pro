# Members the native library calls by name through JNI, kept by annotation:
# the rule travels with this library instead of every app's own rules.
-keepclassmembers class * {
    @dev.waterui.hydrolysis.CalledFromNative *;
}

# The annotation itself must survive for the rule above to match.
-keep class dev.waterui.hydrolysis.CalledFromNative
