package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.ScrollView
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.core.createView
import com.waterui.android.ffi.CWaterUI

object WuiScrollView : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_scroll_view_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val scrollComponent = CWaterUI.INSTANCE.waterui_force_as_scroll_view(anyView)
        val contentPtr = scrollComponent.content
        val axis = CWaterUI.WuiAxis.entries.getOrElse(scrollComponent.axis) { CWaterUI.WuiAxis.Vertical }

        val contentView = if (contentPtr != null) {
            createView(contentPtr, env, context)
        } else {
            View(context) // Create an empty view if content is null
        }

        val scrollView: View = when (axis) {
            CWaterUI.WuiAxis.Horizontal -> HorizontalScrollView(context).apply {
                addView(contentView)
            }
            CWaterUI.WuiAxis.Vertical -> ScrollView(context).apply {
                addView(contentView)
            }
            CWaterUI.WuiAxis.All -> {
                // Android doesn't have a built-in two-directional scroll view.
                // We'll default to a vertical ScrollView as a fallback.
                // A proper implementation would require a custom view.
                ScrollView(context).apply {
                    addView(contentView)
                }
            }
        }
        
        // The WuiScrollView struct and its content pointer are consumed, so we don't drop them here.
        // The createView function for the content is responsible for its pointer.

        return scrollView
    }
}
