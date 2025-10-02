package com.waterui.android.components

import android.content.Context
import android.view.View
import android.widget.LinearLayout
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.core.createView
import com.waterui.android.ffi.CWaterUI

package com.waterui.android.components

import android.content.Context
import android.view.View
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.core.createView
import com.waterui.android.ffi.CWaterUI
import com.waterui.android.layout.RustLayoutView

object WuiContainer : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_container_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val containerComponent = CWaterUI.INSTANCE.waterui_force_as_container(anyView)
        val layoutPtr = containerComponent.layout
        val contents = containerComponent.contents

        if (layoutPtr == null) {
            // If there's no layout, we can't do anything.
            CWaterUI.INSTANCE.waterui_drop_any_view(anyView)
            return View(context) // Return an empty view
        }

        val rustLayout = RustLayoutView(context, layoutPtr)

        // Create and add child views
        if (contents != null && contents.data != null && contents.vtable?.slice != null) {
            val slice = contents.vtable!!.slice!!.invoke(contents.data)
            if (slice.head != null && slice.len > 0) {
                val children = slice.head!!.getPointerArray(0, slice.len.toInt())
                for (childPtr in children) {
                    val childView = createView(childPtr, env, context)
                    
                    // Attach metadata to the view for the layout pass
                    val childId = CWaterUI.INSTANCE.waterui_view_id(childPtr)
                    val isSpacer = childId == WuiSpacer.id
                    childView.tag = RustLayoutView.ChildData(isSpacer = isSpacer, pointer = childPtr)

                    rustLayout.addView(childView)
                }
            }
            // Drop the array data
            contents.vtable!!.drop!!.invoke(contents.data)
        }

        // The RustLayoutView is now responsible for dropping the layoutPtr.
        // We still need to drop the container's AnyView pointer.
        CWaterUI.INSTANCE.waterui_drop_any_view(anyView)

        return rustLayout
    }
}

