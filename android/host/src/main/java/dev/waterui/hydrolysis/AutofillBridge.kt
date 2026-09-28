package dev.waterui.hydrolysis

import android.util.SparseArray
import android.view.View
import android.view.ViewStructure
import android.view.autofill.AutofillValue
import androidx.core.util.size

/**
 * The autofill adapter: exposes the session's editable accessibility nodes as
 * a virtual `ViewStructure` so autofill services can find the text fields, and
 * routes committed values back through the accessibility action edge
 * (`ACTION_SET_TEXT` → accesskit `ReplaceSelectedText`).
 */
internal class AutofillBridge(
    private val provider: HydrolysisAccessibilityProvider,
) {
    /** Builds the virtual structure: one child per editable node. */
    fun provideVirtualStructure(structure: ViewStructure) {
        val ids = provider.editableIds()
        var index = 0
        for (id in ids) {
            val node = provider.nodeFor(id) ?: continue
            val child = structure.asyncNewChild(index++) ?: continue
            child.setAutofillId(structure.autofillId!!, id.toInt())
            child.setClassName("android.widget.EditText")
            child.setAutofillType(View.AUTOFILL_TYPE_TEXT)
            val name = node.optString("name")
            if (name.isNotEmpty()) {
                child.setAutofillHints(arrayOf(name))
                child.setContentDescription(name)
            }
            val bounds = node.optJSONObject("bounds")
            if (bounds != null) {
                child.setDimens(
                    bounds.optDouble("x0", 0.0).toInt(),
                    bounds.optDouble("y0", 0.0).toInt(),
                    0,
                    0,
                    (bounds.optDouble("x1", 0.0) - bounds.optDouble("x0", 0.0)).toInt(),
                    (bounds.optDouble("y1", 0.0) - bounds.optDouble("y0", 0.0)).toInt(),
                )
            }
            val value = node.optString("value")
            child.setAutofillValue(AutofillValue.forText(value))
        }
        structure.setChildCount(index)
    }

    /**
     * The service's committed values return here — each one becomes a
     * `SET_TEXT` action on the matching virtual node.
     */
    fun autofill(values: SparseArray<AutofillValue>) {
        for (i in 0 until values.size) {
            val id = values.keyAt(i)
            val value = values.valueAt(i)
            if (value.isText) {
                provider.performAction(
                    id,
                    android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT,
                    android.os.Bundle().apply {
                        putCharSequence(
                            android.view.accessibility.AccessibilityNodeInfo
                                .ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                            value.textValue,
                        )
                    },
                )
            }
        }
    }
}
