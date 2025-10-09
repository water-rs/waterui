# WaterUI Android Backend

This module hosts the prototype Jetpack Compose renderer for WaterUI. The Kotlin sources compile today,
but most components still contain placeholders — expect TODOs until the JNI layer and Compose UX are
fully implemented.

## Prerequisites

Ensure the following tooling is available on your machine:

- **Java 17 or newer.** (The gradle wrapper runs on the JDK on `PATH`; we tested with Temurin 22.)
- **Android SDK** with at least:
  - Platform `android-34` (Android 14)
  - Build tools `34.0.0`
- **Gradle wrapper** generated in the repo (`./gradlew`).

> ⚠️ For local builds we rely on `local.properties` to point at the Android SDK. The repo already contains
>   `sdk.dir=/Users/lexo/Library/Android/sdk`; update this path if your SDK lives elsewhere.

## Compile the Kotlin sources

```bash
./gradlew :backends:android:compileDebugKotlin
```

This resolves dependencies, generates the Compose runtime stubs, and confirms that every source file
compiles. The task succeeds today with Kotlin `1.9.22` and Compose compiler `1.5.10` (see
`backends/android/build.gradle.kts`).

If you upgrade Kotlin/Compose, adjust both the Kotlin plugin version in `settings.gradle.kts` and the
`kotlinCompilerExtensionVersion` inside the module build script to keep them compatible.

## Build an AAR

```bash
./gradlew :backends:android:assembleDebug
```

This produces `backends/android/build/outputs/aar/android-debug.aar`. There is no consumer yet, but
shipping the artefact lets you integrate with sample apps when JNI bindings are ready.

## Run tests

No unit or instrumentation tests exist yet. To add them:

1. Place JVM unit tests under `src/test/java` and run `./gradlew :backends:android:testDebugUnitTest`.
2. Add instrumentation tests under `src/androidTest/java` and run
   `./gradlew :backends:android:connectedDebugAndroidTest` (requires an emulator or device).

Please update this document as the backend matures (e.g., once JNI glue works or when we add CI targets).
