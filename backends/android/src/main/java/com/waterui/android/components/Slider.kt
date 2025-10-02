package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.SeekBar
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.reactive.BindingDouble
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

object WuiSlider : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_slider_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val sliderComponent = CWaterUI.INSTANCE.waterui_force_as_slider(anyView)
        val bindingPtr = sliderComponent.value
        val range = sliderComponent.range

        val seekBar = SeekBar(context)

        if (bindingPtr != null && range != null) {
            val binding = BindingDouble(bindingPtr)
            val rangeStart = range.start
            val rangeEnd = range.end
            
            // SeekBar works with integers, so we'll use a scale of 1000
            val scale = 1000
            seekBar.max = scale

            fun toSeekBarProgress(value: Double): Int {
                val normalized = ((value - rangeStart) / (rangeEnd - rangeStart)).coerceIn(0.0, 1.0)
                return (normalized * scale).toInt()
            }

            fun fromSeekBarProgress(progress: Int): Double {
                val normalized = progress.toDouble() / scale
                return normalized * (rangeEnd - rangeStart) + rangeStart
            }

            // Set initial value
            seekBar.progress = toSeekBarProgress(binding.value.value)

            // Watch for updates from the binding
            val listener = object : View.OnAttachStateChangeListener {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

                override fun onViewAttachedToWindow(v: View) {
                    binding.value
                        .onEach { newValue ->
                            seekBar.progress = toSeekBarProgress(newValue)
                        }
                        .launchIn(scope)
                }

                override fun onViewDetachedFromWindow(v: View) {
                    scope.cancel()
                    binding.close()
                }
            }
            seekBar.addOnAttachStateChangeListener(listener)

            // Watch for updates from the user
            seekBar.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                    if (fromUser) {
                        binding.set(fromSeekBarProgress(progress))
                    }
                }
                override fun onStartTrackingTouch(seekBar: SeekBar?) {}
                override fun onStopTrackingTouch(seekBar: SeekBar?) {}
            })
        }

        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)
        return seekBar
    }
}
