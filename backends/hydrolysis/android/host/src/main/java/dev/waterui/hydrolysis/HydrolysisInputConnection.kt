package dev.waterui.hydrolysis

import android.content.Context
import android.graphics.RectF
import android.os.Build
import android.text.Editable
import android.text.Selection
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import org.json.JSONObject

/**
 * The IME boundary: a [BaseInputConnection] backed by a real [Editable] that
 * mirrors the authoritative editing session in the Rust runner.
 *
 * Every mutator forwards to native first (`nativeEditOp`); only an accepted
 * op is replayed onto the local mirror through `super` — which, by the same
 * span semantics AOSP defines, produces the identical edit the Rust side
 * recorded. Queries (`getTextBeforeCursor`, `getSelectedText`,
 * `getExtractedText`, …) are inherited and read the mirror for free.
 * Authoritative pushes from the renderer (`onNativeEditingState`) adopt the
 * whole state, so the mirror converges whenever Rust's normalization is
 * stricter.
 *
 * The `editorId` is the connection's generation token: it is pulled
 * synchronously at bind time (`nativeEditingState`) and every op carries it,
 * so a connection that outlives its focused editor is rejected by the
 * session rather than editing stale text.
 */
internal class HydrolysisInputConnection(
    private val target: HydrolysisHostView,
    private val session: HydrolysisSession?,
    private var editorId: Long,
    private var live: Boolean,
) : BaseInputConnection(target, true) {

    private val imm: InputMethodManager =
        target.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager

    /** The request token the IMM monitors extracted text under, or none. */
    private var extractedTextMonitorToken = -1

    /** True while [applyNativeState] replays the session's state onto the mirror. */
    private var adopting = false

    /** One `nativeEditOp` dispatch; `false` on a dead session or stale id. */
    private fun op(code: Int, arg1: Int = 0, arg2: Int = 0, text: String = ""): Boolean {
        // Adopting replays the session's own edits back onto the mirror —
        // forwarding them would echo them into the session, whose push
        // re-enters this very method: an infinite Kotlin->native->Kotlin
        // cycle. The op is accepted so the super.* mirror update still runs.
        if (adopting) return true
        if (!live) return false
        val ptr = session?.nativePtr ?: return false
        return NativeBridge.nativeEditOp(ptr, editorId, code, arg1, arg2, text)
    }

    // ------------------------------------------------------------------
    // Mutators — forward to native first, then move the mirror identically.

    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!op(EDIT_OP_SET_COMPOSING_TEXT, arg1 = newCursorPosition, text = text?.toString().orEmpty())) {
            return false
        }
        return super.setComposingText(text, newCursorPosition)
    }

    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!op(EDIT_OP_COMMIT_TEXT, arg1 = newCursorPosition, text = text?.toString().orEmpty())) {
            return false
        }
        return super.commitText(text, newCursorPosition)
    }

    override fun setComposingRegion(start: Int, end: Int): Boolean {
        if (!op(EDIT_OP_SET_COMPOSING_REGION, arg1 = start, arg2 = end)) return false
        return super.setComposingRegion(start, end)
    }

    override fun finishComposingText(): Boolean {
        if (!op(EDIT_OP_FINISH_COMPOSING_TEXT)) return false
        return super.finishComposingText()
    }

    override fun setSelection(start: Int, end: Int): Boolean {
        if (!op(EDIT_OP_SET_SELECTION, arg1 = start, arg2 = end)) return false
        return super.setSelection(start, end)
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (!op(EDIT_OP_DELETE_SURROUNDING, arg1 = beforeLength, arg2 = afterLength)) return false
        return super.deleteSurroundingText(beforeLength, afterLength)
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
        if (!op(EDIT_OP_DELETE_SURROUNDING_POINTS, arg1 = beforeLength, arg2 = afterLength)) {
            return false
        }
        return super.deleteSurroundingTextInCodePoints(beforeLength, afterLength)
    }

    override fun beginBatchEdit(): Boolean {
        if (!op(EDIT_OP_BEGIN_BATCH)) return false
        return super.beginBatchEdit()
    }

    override fun endBatchEdit(): Boolean {
        if (!op(EDIT_OP_END_BATCH)) return false
        return super.endBatchEdit()
    }

    override fun performContextMenuAction(id: Int): Boolean {
        val action =
            when (id) {
                android.R.id.selectAll -> CONTEXT_ACTION_SELECT_ALL
                android.R.id.cut -> CONTEXT_ACTION_CUT
                android.R.id.copy -> CONTEXT_ACTION_COPY
                android.R.id.paste, android.R.id.pasteAsPlainText -> CONTEXT_ACTION_PASTE
                else -> return false
            }
        return op(EDIT_OP_CONTEXT_ACTION, arg1 = action)
    }

    override fun performEditorAction(actionCode: Int): Boolean =
        op(EDIT_OP_EDITOR_ACTION, arg1 = actionCode)

    override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean =
        op(EDIT_OP_CURSOR_UPDATES, arg1 = cursorUpdateMode, arg2 = 0)

    override fun requestCursorUpdates(cursorUpdateMode: Int, filter: Int): Boolean =
        op(EDIT_OP_CURSOR_UPDATES, arg1 = cursorUpdateMode, arg2 = filter)

    /**
     * `sendKeyEvent` — hardware keys the IME routes through the connection.
     * This is the key channel, not an editing op, so it crosses the same
     * `nativeKeyEvent` path the view's own key dispatch uses.
     */
    override fun sendKeyEvent(event: KeyEvent): Boolean {
        val session = session ?: return false
        val key =
            when {
                event.unicodeChar != 0 -> event.unicodeChar.toChar().toString()
                event.keyCode == KeyEvent.KEYCODE_DEL -> "Backspace"
                event.keyCode == KeyEvent.KEYCODE_ENTER -> "Enter"
                event.keyCode == KeyEvent.KEYCODE_DPAD_LEFT -> "ArrowLeft"
                event.keyCode == KeyEvent.KEYCODE_DPAD_RIGHT -> "ArrowRight"
                event.keyCode == KeyEvent.KEYCODE_DPAD_UP -> "ArrowUp"
                event.keyCode == KeyEvent.KEYCODE_DPAD_DOWN -> "ArrowDown"
                else -> return false
            }
        NativeBridge.nativeKeyEvent(
            session.nativePtr,
            key,
            event.action == KeyEvent.ACTION_DOWN,
            event.isShiftPressed,
            event.isCtrlPressed,
            event.isAltPressed,
            event.isMetaPressed,
        )
        // IMEs pair down/up; a lone down without the up leaves stuck presses.
        if (event.action == KeyEvent.ACTION_DOWN) {
            NativeBridge.nativeKeyEvent(
                session.nativePtr,
                key,
                false,
                event.isShiftPressed,
                event.isCtrlPressed,
                event.isAltPressed,
                event.isMetaPressed,
            )
        }
        return true
    }

    // ------------------------------------------------------------------
    // Queries — the ones the mirror must answer beyond the inherited set.

    override fun getExtractedText(request: ExtractedTextRequest, flags: Int): ExtractedText {
        if (flags and InputConnection.GET_EXTRACTED_TEXT_MONITOR != 0) {
            extractedTextMonitorToken = request.token
        }
        return extractedText()
    }

    private fun extractedText(): ExtractedText {
        val editable = editable
        val out = ExtractedText()
        val text = editable?.toString() ?: ""
        out.text = text
        out.startOffset = 0
        out.partialStartOffset = 0
        out.partialEndOffset = text.length
        out.selectionStart = editable?.let { Selection.getSelectionStart(it) } ?: 0
        out.selectionEnd = editable?.let { Selection.getSelectionEnd(it) } ?: 0
        return out
    }

    // ------------------------------------------------------------------
    // Native pushes.

    /**
     * Adopts the authoritative editing state: whole-text replace, selection
     * and composing span, then `updateSelection`/`updateExtractedText` so the
     * IMM and the editor agree. Idempotent — it also converges the mirror
     * after an external binding update mid-composition.
     */
    internal fun applyNativeState(state: EditingStatePayload) {
        editorId = state.editorId
        live = state.focused
        val editable = editable ?: return
        if (editable.toString() != state.text) {
            editable.replace(0, editable.length, state.text)
        }
        Selection.setSelection(
            editable,
            state.selStart.coerceIn(0, editable.length),
            state.selEnd.coerceIn(0, editable.length),
        )
        adopting = true
        try {
            if (state.compStart >= 0) {
                // setComposingRegion marks the range without rewriting it —
                // the span itself is what the inherited queries read.
                super.setComposingRegion(state.compStart, state.compEnd)
            } else {
                super.finishComposingText()
            }
        } finally {
            adopting = false
        }
        imm.updateSelection(target, state.selStart, state.selEnd, state.compStart, state.compEnd)
        if (extractedTextMonitorToken >= 0) {
            imm.updateExtractedText(target, extractedTextMonitorToken, extractedText())
        }
    }

    /** A pushed cursor anchor update — the subscribed candidate geometry. */
    internal fun applyCursorAnchorInfo(info: AnchorInfoPayload) {
        imm.updateCursorAnchorInfo(
            target,
            info.build(target.resources.displayMetrics.density, target.viewToScreenMatrix()),
        )
    }

    override fun closeConnection() {
        live = false
        if (target.inputConnection === this) target.inputConnection = null
        super.closeConnection()
    }

    private companion object {
        // `nativeEditOp` opcodes — mirrored in `src/runner/android/ime.rs`;
        // the JNI schema bump refuses a skewed pair.
        const val EDIT_OP_SET_COMPOSING_REGION = 0
        const val EDIT_OP_FINISH_COMPOSING_TEXT = 1
        const val EDIT_OP_SET_SELECTION = 2
        const val EDIT_OP_DELETE_SURROUNDING = 3
        const val EDIT_OP_DELETE_SURROUNDING_POINTS = 4
        const val EDIT_OP_BEGIN_BATCH = 5
        const val EDIT_OP_END_BATCH = 6
        const val EDIT_OP_CONTEXT_ACTION = 7
        const val EDIT_OP_EDITOR_ACTION = 8
        const val EDIT_OP_CURSOR_UPDATES = 9
        const val EDIT_OP_SET_COMPOSING_TEXT = 10
        const val EDIT_OP_COMMIT_TEXT = 11

        // `performContextMenuAction` actions — mirrored in ime.rs.
        const val CONTEXT_ACTION_SELECT_ALL = 0
        const val CONTEXT_ACTION_CUT = 1
        const val CONTEXT_ACTION_COPY = 2
        const val CONTEXT_ACTION_PASTE = 3
    }
}

/** The `onNativeEditingState` JSON, parsed. */
internal class EditingStatePayload(json: String) {
    private val o = JSONObject(json)
    val editorId: Long = o.getLong("editor_id")
    val focused: Boolean = o.getBoolean("focused")
    val revision: Long = o.getLong("revision")
    val text: String = o.getString("text")
    val selStart: Int = o.getInt("sel_start")
    val selEnd: Int = o.getInt("sel_end")
    val compStart: Int = o.getInt("comp_start")
    val compEnd: Int = o.getInt("comp_end")
    val password: Boolean = o.getBoolean("password")
    val singleLine: Boolean = o.getBoolean("single_line")
    val hasSubmit: Boolean = o.getBoolean("has_submit")
}

/** The `onNativeCursorAnchorInfo` JSON — rect arrays in logical units. */
internal class AnchorInfoPayload(json: String) {
    private val o = JSONObject(json)
    val insertion: FloatArray = o.getJSONArray("insertion").toFloats()
    val editorBounds: FloatArray? =
        if (o.isNull("editor_bounds")) null else o.getJSONArray("editor_bounds").toFloats()
    val selStart: Int = o.getInt("sel_start")
    val selEnd: Int = o.getInt("sel_end")
    val compStart: Int = o.getInt("comp_start")
    val compEnd: Int = o.getInt("comp_end")
    val composingText: String? =
        if (o.isNull("composing_text")) null else o.getString("composing_text")
    val charBounds: List<FloatArray> =
        o.getJSONArray("char_bounds").let { arr ->
            (0 until arr.length()).map { arr.getJSONArray(it).toFloats() }
        }

    /** Logical-unit geometry scaled into view pixels. */
    fun build(density: Float, matrix: android.graphics.Matrix): CursorAnchorInfo {
        val builder =
            CursorAnchorInfo.Builder()
                .setMatrix(matrix)
                .setSelectionRange(selStart, selEnd)
                .setInsertionMarkerLocation(
                    insertion[0] * density,
                    insertion[1] * density,
                    insertion[3] * density,
                    insertion[3] * density,
                    CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION,
                )
        if (compStart >= 0) {
            builder.setComposingText(compStart, composingText.orEmpty())
        }
        for (bounds in charBounds) {
            builder.addCharacterBounds(
                bounds[0].toInt(),
                bounds[1] * density,
                bounds[2] * density,
                bounds[3] * density,
                bounds[4] * density,
                CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION,
            )
        }
        val editorBounds = editorBounds
        if (Build.VERSION.SDK_INT >= 34 && editorBounds != null) {
            builder.setEditorBoundsInfo(
                android.view.inputmethod.EditorBoundsInfo.Builder()
                    .setEditorBounds(
                        RectF(
                            editorBounds[0] * density,
                            editorBounds[1] * density,
                            editorBounds[2] * density,
                            editorBounds[3] * density,
                        ),
                    )
                    .setHandwritingBounds(
                        RectF(
                            editorBounds[0] * density,
                            editorBounds[1] * density,
                            editorBounds[2] * density,
                            editorBounds[3] * density,
                        ),
                    )
                    .build(),
            )
        }
        return builder.build()
    }
}

private fun org.json.JSONArray.toFloats(): FloatArray =
    FloatArray(length()) { getDouble(it).toFloat() }
