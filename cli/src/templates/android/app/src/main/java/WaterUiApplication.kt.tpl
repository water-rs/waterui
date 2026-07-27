package {{ ctx.android_package_name() }}

import android.app.Activity
import android.app.Application
import dev.waterui.android.runtime.WuiEnvironment
import dev.waterui.android.runtime.WaterUiRuntimeOwner
import dev.waterui.android.runtime.bootstrapWaterUiRuntime
import dev.waterui.android.runtime.releaseWaterUiRuntime

class WaterUiApplication : Application(), WaterUiRuntimeOwner {
    private data class ActiveRuntime(
        val generation: Long,
        val owner: Long,
    )

    private var nextGeneration = 0L
    private var activeRuntime: ActiveRuntime? = null
    private var processEnvironment: WuiEnvironment? = null

    fun acquireRuntime(activity: Activity): AndroidRuntimeLease {
        activeRuntime?.let { previous ->
            activeRuntime = null
            releaseWaterUiRuntime(previous.owner)
        }

        nextGeneration = Math.addExact(nextGeneration, 1L)
        val runtime = ActiveRuntime(
            generation = nextGeneration,
            owner = bootstrapWaterUiRuntime(activity),
        )
        activeRuntime = runtime
        if (processEnvironment == null) {
            processEnvironment = WuiEnvironment.create()
        }
        return AndroidRuntimeLease(this, runtime.generation)
    }

    override fun createWaterUiEnvironment(): WuiEnvironment =
        checkNotNull(processEnvironment) {
            "WaterUI runtime must be acquired before creating an environment"
        }.clone()

    internal fun releaseRuntime(generation: Long): Boolean {
        val runtime = activeRuntime ?: return false
        if (runtime.generation != generation) return false

        activeRuntime = null
        releaseWaterUiRuntime(runtime.owner)
        return true
    }
}

class AndroidRuntimeLease internal constructor(
    private val application: WaterUiApplication,
    private val generation: Long,
) {
    private var closed = false

    fun close(): Boolean {
        check(!closed) { "Android runtime lease is already closed" }
        closed = true
        return application.releaseRuntime(generation)
    }
}
