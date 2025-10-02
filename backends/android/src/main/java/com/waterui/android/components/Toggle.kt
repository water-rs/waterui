package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.LinearLayout
import androidx.appcompat.widget.SwitchCompat
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.core.createView
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.reactive.BindingBool
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

object WuiToggle : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_toggle_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val component = CWaterUI.INSTANCE.waterui_force_as_toggle(anyView)
        val bindingPtr = component.toggle
        val labelPtr = component.label

        // A Switch in Android has its own text property, but the label in WaterUI can be any view.
        // We'll create a horizontal layout to hold the label and the switch.
        val layout = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
        }

        val switchView = SwitchCompat(context)

        if (labelPtr != null) {
            val labelView = createView(labelPtr, env, context)
            layout.addView(labelView)
        }
        
        layout.addView(switchView)

        if (bindingPtr != null) {
            val binding = BindingBool(bindingPtr)

            // Set initial value
            switchView.isChecked = binding.value.value

            // Watch for updates from binding
            val listener = object : View.OnAttachStateChangeListener {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

                override fun onViewAttachedToWindow(v: View) {
                    binding.value
                        .onEach { newValue ->
                            if (switchView.isChecked != newValue) {
                                switchView.isChecked = newValue
                            }
                        }
                        .launchIn(scope)
                }

                override fun onViewDetachedFromWindow(v: View) {
                    scope.cancel()
                    binding.close()
                }
            }
            switchView.addOnAttachStateChangeListener(listener)

            // Watch for updates from user
            switchView.setOnCheckedChangeListener { _, isChecked ->
                binding.set(isChecked)
            }
        }

        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)
        return layout
    }
}
