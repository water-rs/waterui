# Add project specific ProGuard rules here.
# See https://developer.android.com/studio/build/shrink-code for more details.

# The app reaches the Hydrolysis host through `System.loadLibrary` and JNI
# symbol names; the default proguard-android-optimize.txt keeps cover that
# direction. The reverse direction — native code calling host methods by
# name — is covered by the host library's own consumer keep rule: every
# such member carries @dev.waterui.hydrolysis.CalledFromNative.
