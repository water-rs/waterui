package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.ProgressBar
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.reactive.ComputedDouble
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

object WuiProgress : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_progress_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val progressComponent = CWaterUI.INSTANCE.waterui_force_as_progress(anyView)
        val computedDoublePtr = progressComponent.value
        val style = CWaterUI.WuiProgressStyle.entries.getOrElse(progressComponent.style) { CWaterUI.WuiProgressStyle.Linear }

        val progressBar = when (style) {
            CWaterUI.WuiProgressStyle.Linear -> ProgressBar(context, null, android.R.attr.progressBarStyleHorizontal)
            CWaterUI.WuiProgressStyle.Circular -> ProgressBar(context, null, android.R.attr.progressBarStyle)
        }

        if (computedDoublePtr != null) {
            // Determinate progress
            progressBar.isIndeterminate = false
            progressBar.max = 100 // Assuming progress is 0.0 to 1.0

            val computedDouble = ComputedDouble(computedDoublePtr)

            // Set initial value
            progressBar.progress = (computedDouble.value.value * 100).toInt()

            // Watch for updates
            val listener = object : View.OnAttachStateChangeListener {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

                override fun onViewAttachedToWindow(v: View) {
                    computedDouble.value
                        .onEach { newProgress -> progressBar.progress = (newProgress * 100).toInt() }
                        .launchIn(scope)
                }

                override fun onViewDetachedFromWindow(v: View) {
                    scope.cancel()
                    computedDouble.close()
                }
            }
            progressBar.addOnAttachStateChangeListener(listener)

        } else {
            // Indeterminate progress (loading)
            progressBar.isIndeterminate = true
        }

        // Drop the AnyView pointer
        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)

        return progressBar
    }
}
