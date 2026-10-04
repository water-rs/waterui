# Add project specific ProGuard rules here.
# See https://developer.android.com/studio/build/shrink-code for more details.

# The app reaches the Hydrolysis host through `System.loadLibrary` and JNI
# symbol names; the default proguard-android-optimize.txt keeps cover that
# direction. The only reachability R8 cannot see is the reverse direction:
# the Rust session calls these HydrolysisSession methods by name.
-keepclassmembers class dev.waterui.hydrolysis.HydrolysisSession {
    void onNativeRequestRedraw();
    void onNativeTextInputState(float, float, float, float, int);
    void onNativeAccessibilityTreeChanged();
    void onNativeFatalError(java.lang.String);
    void onNativeCloseRequested();
}
