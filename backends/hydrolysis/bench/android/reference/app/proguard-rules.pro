# Reference app keep rules. R8 prunes each per-fixture variant to the twins
# its BuildConfig.FIXTURE selects; what survives is exactly what a per-fixture
# Hydrolysis APK is size-compared against.

# The launcher activity and every twin entry point are reached reflectively
# through twinFor's when-dispatch only — keep the registry intact so R8's
# reachability, not hand-keeps, decides what each variant ships.
-keep class dev.waterui.android.reference.MainActivity { *; }
-keep class dev.waterui.android.reference.BuildConfig { *; }
