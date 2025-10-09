package dev.waterui.android.runtime

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/**
 * Android counterpart to the WaterUI environment handle. Responsible for owning the native pointer
 * and providing a lifecycle-aware coroutine scope for reactive updates.
 */
class WuiEnvironment(
    envPtr: Long
) : NativePointer(envPtr) {

    /** Coroutine scope tied to the environment lifetime for asynchronous watchers. */
    val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    fun clone(): WuiEnvironment {
        val cloned = NativeBindings.waterui_env_clone(raw())
        return WuiEnvironment(cloned)
    }

    override fun close() {
        super.close()
        scope.cancel()
    }

    override fun release(ptr: Long) {
        NativeBindings.waterui_env_drop(ptr)
    }
}
