package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.LinearLayout
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI

object WuiSpacer : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_spacer_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val spacer = View(context)
        // A spacer in a LinearLayout expands to fill available space.
        val layoutParams = LinearLayout.LayoutParams(
            0, // width
            0, // height
            1.0f // weight
        )
        spacer.layoutParams = layoutParams
        
        // A spacer is an empty view, so we drop the Rust view pointer immediately.
        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)

        return spacer
    }
}
