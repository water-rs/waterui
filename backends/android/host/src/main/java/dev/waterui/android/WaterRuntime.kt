package dev.waterui.android

/**
 * The JNI face of the `waterui-android` Rust backend.
 *
 * The host loads the app library named by the manifest's
 * `dev.waterui.LIBRARY` meta-data and every call below forwards into it on
 * the main thread. The library owns all rendering logic; this class only
 * carries the lifecycle.
 */
object WaterRuntime {
    /**
     * The loaded library name, from `application meta-data
     * `dev.waterui.LIBRARY`. Read once by [WaterActivity] before load.
     */
    const val LIBRARY_METADATA: String = "dev.waterui.LIBRARY"

    /**
     * Mounts the app's declared content into [root] against [activity]'s
     * context. Runs on the main thread; returns the opaque runtime handle.
     */
    external fun nativeCreate(activity: WaterActivity, root: android.view.ViewGroup): Long

    /** Tears the runtime down; the handle must not be used again. */
    external fun nativeDestroy(handle: Long)

    /** Loads the app library named in the manifest's `dev.waterui.LIBRARY`. */
    fun loadLibrary(activity: WaterActivity) {
        val info = activity.packageManager.getApplicationInfo(
            activity.packageName,
            android.content.pm.PackageManager.GET_META_DATA,
        )
        val name = info.metaData?.getString(LIBRARY_METADATA)
            ?: error("manifest is missing dev.waterui.LIBRARY meta-data")
        System.loadLibrary(name)
    }

    /** Forwards `Activity.onConfigurationChanged` — theme, locale, density. */
    external fun nativeOnConfigurationChanged(handle: Long)

    /**
     * Pushes the window's insets, in pixels, split by safe-area region:
     * [container*] the container region (system bars, cutouts, the caption
     * bar), [keyboard*] the keyboard region (the IME). Fires on inset
     * dispatch and on every `WindowInsetsAnimationCompat` frame.
     */
    external fun nativeInsetsChanged(
        handle: Long,
        containerLeft: Int,
        containerTop: Int,
        containerRight: Int,
        containerBottom: Int,
        keyboardLeft: Int,
        keyboardTop: Int,
        keyboardRight: Int,
        keyboardBottom: Int,
    )

    /** Forwards `Activity.onTrimMemory`. */
    external fun nativeOnTrimMemory(handle: Long, level: Int)
}
