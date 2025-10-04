package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.TextView
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI

package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.TextView
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.reactive.ComputedAttributedStr
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach

object WuiText : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_text_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val textComponent = CWaterUI.INSTANCE.waterui_force_as_text(anyView)
        val computedStrPtr = textComponent.content

        val textView = TextView(context)

        if (computedStrPtr != null) {
            val computedStr = ComputedAttributedStr(computedStrPtr)

            textView.text = computedStr.value.value // Set initial value

            // Create a CoroutineScope that is active while the view is attached
            val listener = object : View.OnAttachStateChangeListener {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

                override fun onViewAttachedToWindow(v: View) {
                    computedStr.value
                        .onEach { newText -> textView.text = newText }
                        .launchIn(scope)
                }

                override fun onViewDetachedFromWindow(v: View) {
                    scope.cancel() // Cancel the scope to avoid leaks
                    computedStr.close() // Release native resources
                }
            }
            textView.addOnAttachStateChangeListener(listener)
        }

        // The WuiText struct from force_as_text is a value type and doesn't need dropping,
        // but the AnyView that produced it does.
        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)

        return textView
    }
}
