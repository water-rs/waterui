package dev.waterui.hydrolysis

import android.util.SparseArray
import android.view.View
import android.view.ViewStructure
import android.view.autofill.AutofillManager
import android.view.autofill.AutofillValue
import androidx.core.util.size

/**
 * The autofill adapter, separate from accessibility: it projects the session's
 * editable semantic nodes into a virtual `ViewStructure` so an autofill
 * service can find the text fields, and routes committed values back through
 * the same stable virtual ids (`autofill()` → the node's `SetValue` action).
 *
 * Android requires a virtual structure for a self-drawn hierarchy; the
 * provider supplies the node list and geometry, so both services share the
 * tree's stable identities.
 */
internal class AutofillBridge(
    private val host: HydrolysisHostView,
    private val provider: HydrolysisAccessibilityProvider,
) {
    private var focusedEditableId = INVALID_ID

    private fun manager(): AutofillManager? =
        host.context.getSystemService(AutofillManager::class.java)

    /** Builds the virtual structure: one child per editable node. */
    fun provideVirtualStructure(structure: ViewStructure) {
        val nodes = provider.editableNodes()
        val parentId = checkNotNull(structure.autofillId)
        structure.setChildCount(nodes.size)
        for ((index, node) in nodes.withIndex()) {
            if (node.id > Int.MAX_VALUE) continue
            val child = structure.newChild(index)
            child.setAutofillId(parentId, node.id.toInt())
            child.setClassName("android.widget.EditText")
            child.setAutofillType(View.AUTOFILL_TYPE_TEXT)
            if (node.autofillHints.isNotEmpty()) {
                child.setAutofillHints(node.autofillHints)
            }
            if (node.label.isNotEmpty()) {
                child.setContentDescription(node.label)
            }
            child.setInputType(
                android.text.InputType.TYPE_CLASS_TEXT or
                    if (node.sensitive) {
                        android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
                    } else {
                        0
                    }
            )
            child.setDataIsSensitive(node.sensitive)
            child.setDimens(
                node.bounds.left,
                node.bounds.top,
                0,
                0,
                node.bounds.width(),
                node.bounds.height(),
            )
            child.setAutofillValue(AutofillValue.forText(node.value))
        }
        focusedEditableId = provider.focusedEditableId()
    }

    /**
     * The service's committed values return keyed by virtual id — each one
     * becomes the node's `SetValue` text action on the session.
     */
    fun autofill(values: SparseArray<AutofillValue>) {
        val manager = manager()
        var changed = false
        for (i in 0 until values.size) {
            val id = values.keyAt(i)
            val value = values.valueAt(i)
            if (value.isText) {
                val text = value.textValue?.toString() ?: continue
                if (provider.applyAutofillValue(id.toLong(), text)) {
                    changed = true
                    manager?.notifyValueChanged(host, id, value)
                }
            }
        }
        if (changed) manager?.commit()
    }

    /**
     * Called after each accessibility publish: the service is told when focus
     * enters or leaves an editable node and when a committed value changes.
     */
    fun snapshotChanged() {
        val manager = manager() ?: return
        val focused = provider.focusedEditableId()
        if (focused != focusedEditableId) {
            if (focusedEditableId != INVALID_ID) {
                manager.notifyViewExited(host, focusedEditableId.toInt())
            }
            if (focused != INVALID_ID) {
                val bounds = provider.editableScreenBounds(focused)
                if (bounds != null) {
                    manager.notifyViewEntered(host, focused.toInt(), bounds)
                }
            }
            focusedEditableId = focused
        }
    }

    /** The activity finishing commits or cancels the pending autofill save. */
    fun commit() {
        manager()?.commit()
    }

    fun cancel() {
        manager()?.cancel()
        focusedEditableId = INVALID_ID
    }

    private companion object {
        const val INVALID_ID = -1L
    }
}
