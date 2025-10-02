package com.waterui.android.components

import android.content.Context
import android.graphics.Color
import android.view.View
import android.view.ViewGroup
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI

object WuiDivider : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_divider_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val divider = View(context)
        // A simple 1dp high line
        val height = (context.resources.displayMetrics.density * 1).toInt()
        divider.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            height
        )
        divider.setBackgroundColor(Color.LTGRAY)

        // A divider is an empty view, so we drop the Rust view pointer immediately.
        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)

        return divider
    }
}
