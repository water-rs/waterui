package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.reactive.BindingInt
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

object WuiStepper : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_stepper_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val stepperComponent = CWaterUI.INSTANCE.waterui_force_as_stepper(anyView)
        val bindingPtr = stepperComponent.value
        // TODO: Use the step value from stepperComponent.step (Computed_i32)
        val step = 1 
        val range = stepperComponent.range

        val layout = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
        }

        if (bindingPtr != null) {
            val binding = BindingInt(bindingPtr)
            
            val valueText = TextView(context)
            val minusButton = Button(context).apply { text = "-" }
            val plusButton = Button(context).apply { text = "+" }

            layout.addView(minusButton)
            layout.addView(valueText)
            layout.addView(plusButton)

            // Set initial value
            valueText.text = binding.value.value.toString()

            // Set click listeners
            minusButton.setOnClickListener {
                val newValue = binding.value.value - step
                if (range == null || newValue >= range.start) {
                    binding.set(newValue)
                }
            }
            plusButton.setOnClickListener {
                val newValue = binding.value.value + step
                if (range == null || newValue <= range.end) {
                    binding.set(newValue)
                }
            }

            // Watch for updates
            val listener = object : View.OnAttachStateChangeListener {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

                override fun onViewAttachedToWindow(v: View) {
                    binding.value
                        .onEach { newValue ->
                            valueText.text = newValue.toString()
                            minusButton.isEnabled = range == null || newValue > range.start
                            plusButton.isEnabled = range == null || newValue < range.end
                        }
                        .launchIn(scope)
                }

                override fun onViewDetachedFromWindow(v: View) {
                    scope.cancel()
                    binding.close()
                }
            }
            layout.addOnAttachStateChangeListener(listener)
        }

        // Drop the AnyView pointer
        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)

        return layout
    }
}
