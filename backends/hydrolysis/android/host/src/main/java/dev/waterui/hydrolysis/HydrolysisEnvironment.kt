package dev.waterui.hydrolysis

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.system.Os
import android.util.Log
import java.io.File
import java.util.concurrent.Executor

/**
 * Syncs the packaged `waterui_assets` tree and prepares the native environment
 * once per process, before any session starts. Intent environment overrides
 * must be applied after [prepare]'s continuation lands.
 *
 * The copy runs on a worker thread: callers never block on it, and the
 * continuation it queues lands on the main thread once the environment is
 * ready — so the native library load and the first session never see a partly
 * copied tree under `WATERUI_ASSETS_ROOT`.
 */
object HydrolysisEnvironment {
    /** Posts the environment hand-off and the queued continuations. */
    private val mainHandler = Handler(Looper.getMainLooper())

    /**
     * Starts the asset sync off the main thread: a dedicated thread per sync,
     * since a process syncs at most once. Tests substitute a captive one.
     */
    @Volatile
    internal var syncExecutor: Executor = Executor { sync ->
        Thread(sync, "hydrolysis-asset-sync").start()
    }

    /** Set once the sync and the environment variables have both landed. */
    private var prepared = false

    /** True while the worker owns a sync; continuations queue behind it. */
    private var syncRunning = false

    /** Continuations awaiting the in-flight sync, in call order. */
    private val pendingContinuations = mutableListOf<() -> Unit>()

    /**
     * Prepares the process environment once, then runs [onPrepared] on the
     * main thread. A caller that arrives mid-sync queues behind it; one that
     * arrives after it runs [onPrepared] inline when it is already on the
     * main thread. A failed sync surfaces as the thrown error on the main
     * thread — never swallowed, and a session never starts without its
     * assets — and leaves the process unprepared so a later call retries.
     */
    fun prepare(context: Context, onPrepared: () -> Unit) {
        var startSync = false
        val ready = synchronized(this) {
            if (!prepared) {
                pendingContinuations += onPrepared
                if (!syncRunning) {
                    syncRunning = true
                    startSync = true
                }
            }
            prepared
        }
        if (ready) {
            if (Looper.myLooper() == Looper.getMainLooper()) {
                onPrepared()
            } else {
                mainHandler.post { onPrepared() }
            }
        } else if (startSync) {
            val appContext = context.applicationContext
            syncExecutor.execute {
                runSync(appContext)
            }
        }
    }

    /**
     * The worker half of [prepare]: the copy itself. Everything after it —
     * the environment variables, the queued continuations, and a failure's
     * rethrow — is handed back to the main thread so ordering against the
     * launch intent's overrides stays exact and the error surfaces there.
     */
    private fun runSync(appContext: Context) {
        val assetsRoot = try {
            syncBundledAssets(appContext)
        } catch (error: Throwable) {
            mainHandler.post {
                synchronized(this) {
                    syncRunning = false
                    pendingContinuations.clear()
                }
                throw error
            }
            return
        }
        mainHandler.post {
            Os.setenv("WATERUI_ASSETS_ROOT", assetsRoot.absolutePath, true)
            Os.setenv("WATER_CACHE_DIR", appContext.cacheDir.absolutePath, true)
            val continuations = synchronized(this) {
                prepared = true
                syncRunning = false
                pendingContinuations.toList().also { pendingContinuations.clear() }
            }
            continuations.forEach { it() }
        }
    }

    /** Test-only reset: drops the once-per-process state for a relaunch. */
    internal fun resetForTest() {
        synchronized(this) {
            prepared = false
            syncRunning = false
            pendingContinuations.clear()
        }
    }

    private fun syncBundledAssets(context: Context): File {
        val assetRoot = File(context.filesDir, "waterui_assets")
        val stampAsset = "waterui_assets/waterui-sync-stamp"
        val bundledStamp = try {
            context.assets.open(stampAsset).bufferedReader().use { it.readText() }
        } catch (e: Exception) {
            throw IllegalStateException(
                "hydrolysis: packaged asset '$stampAsset' is missing or unreadable — " +
                    "the build did not stage the waterui_assets tree",
                e,
            )
        }
        val localStamp = File(assetRoot, "waterui-sync-stamp")
            .takeIf { it.exists() }
            ?.readText()
        if (localStamp == bundledStamp) return assetRoot

        assetRoot.deleteRecursively()
        assetRoot.mkdirs()
        copyAssetTree(context, "waterui_assets", assetRoot)
        File(assetRoot, "waterui-sync-stamp").writeText(bundledStamp)
        Log.d("WaterUI.Environment", "Synced waterui_assets into $assetRoot")
        return assetRoot
    }

    private fun copyAssetTree(context: Context, assetPath: String, dest: File) {
        val children = context.assets.list(assetPath)?.filter { it.isNotEmpty() }.orEmpty()
        if (children.isEmpty()) {
            dest.parentFile?.mkdirs()
            context.assets.open(assetPath).use { input ->
                dest.outputStream().use { output -> input.copyTo(output) }
            }
            return
        }
        dest.mkdirs()
        for (child in children) {
            copyAssetTree(context, "$assetPath/$child", File(dest, child))
        }
    }
}
