// The Gradle build for the native Android backend: `:host` is the host
// library a generated app consumes, `:tests:app` the test application that
// exercises it. No repositories or dependencies live here — the host binds
// only the platform itself.
plugins {
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
}
