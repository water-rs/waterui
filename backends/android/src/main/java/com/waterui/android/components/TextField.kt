package com.waterui.android.components

import android.content.Context
import android.text.Editable
import android.text.TextWatcher
import android.view.View
import android.widget.EditText
import com.google.android.material.textfield.TextInputLayout
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.reactive.BindingStr
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

object WuiTextField : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_text_field_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val component = CWaterUI.INSTANCE.waterui_force_as_text_field(anyView)
        val bindingPtr = component.value

        val editText = EditText(context)
        val textInputLayout = TextInputLayout(context)
        textInputLayout.addView(editText)

        // TODO: Handle label and prompt

        if (bindingPtr != null) {
            val binding = BindingStr(bindingPtr)

            // Set initial value
            editText.setText(binding.value.value)

            // Watch for updates from binding
            val listener = object : View.OnAttachStateChangeListener {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
                var isUpdatingFromBinding = false

                override fun onViewAttachedToWindow(v: View) {
                    binding.value
                        .onEach { newValue ->
                            if (editText.text.toString() != newValue) {
                                isUpdatingFromBinding = true
                                editText.setText(newValue)
                                isUpdatingFromBinding = false
                            }
                        }
                        .launchIn(scope)
                }

                override fun onViewDetachedFromWindow(v: View) {
                    scope.cancel()
                    binding.close()
                }
            }
            editText.addOnAttachStateChangeListener(listener)

            // Watch for updates from user
            editText.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                    if (!listener.isUpdatingFromBinding) {
                        binding.set(s.toString())
                    }
                }
                override fun afterTextChanged(s: Editable?) {}
            })
        }

        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)
        return textInputLayout
    }
}
