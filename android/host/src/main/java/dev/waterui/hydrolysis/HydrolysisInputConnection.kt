package dev.waterui.hydrolysis

import android.os.SystemClock
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection

/**
 * The IME boundary: an [InputConnection] whose composition and commit calls
 * cross JNI straight into the session. There is no editable text buffer on
 * the Kotlin side — the Rust renderer owns the text model — so every method
 * that would normally edit a local buffer forwards its intent instead.
 *
 * `setComposingText` / `commitText` / `finishComposingText` map to the same
 * events a desktop IME sends; `sendKeyEvent` decodes hardware presses the IME
 * routes through the connection (DPAD navigation, backspace on a dead field).
 */
internal class HydrolysisInputConnection(
    target: HydrolysisHostView,
    private val session: HydrolysisSession?,
    outAttrs: EditorInfo,
) : BaseInputConnection(target, true) {

    private val sessionPtr: Long get() = session?.nativePtr ?: 0L

    init {
        // No extract-mode UI: the app draws its own editing chrome.
        outAttrs.imeOptions = outAttrs.imeOptions or EditorInfo.IME_FLAG_NO_EXTRACT_UI
    }

    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (sessionPtr == 0L) return false
        val string = text?.toString().orEmpty()
        // The connection's caret is 1-based relative to the composing region;
        // the runner wants a byte offset inside the preedit text.
        val caret = string.toByteArray(Charsets.UTF_8).size.takeIf { newCursorPosition > 0 } ?: 0
        NativeBridge.nativeSetComposingText(sessionPtr, string, caret)
        return true
    }

    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (sessionPtr == 0L) return false
        NativeBridge.nativeCommitText(sessionPtr, text?.toString().orEmpty())
        return true
    }

    override fun finishComposingText(): Boolean {
        if (sessionPtr == 0L) return false
        NativeBridge.nativeFinishComposingText(sessionPtr)
        return true
    }

    override fun sendKeyEvent(event: KeyEvent): Boolean {
        if (sessionPtr == 0L) return false
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
        val now = SystemClock.uptimeMillis()
        NativeBridge.nativeKeyEvent(
            sessionPtr,
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
                sessionPtr,
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

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (sessionPtr == 0L) return false
        // No granular delete on the session edge yet: backspace is the
        // semantic the IME actually asks for in practice.
        repeat(beforeLength.coerceAtLeast(1)) {
            NativeBridge.nativeKeyEvent(sessionPtr, "Backspace", true, false, false, false, false)
            NativeBridge.nativeKeyEvent(sessionPtr, "Backspace", false, false, false, false, false)
        }
        return true
    }

    override fun getTextBeforeCursor(n: Int, flags: Int): CharSequence = ""

    override fun getTextAfterCursor(n: Int, flags: Int): CharSequence = ""

    override fun getSelectedText(flags: Int): CharSequence? = null

    override fun performEditorAction(actionCode: Int): Boolean {
        if (sessionPtr == 0L) return false
        NativeBridge.nativeKeyEvent(sessionPtr, "Enter", true, false, false, false, false)
        NativeBridge.nativeKeyEvent(sessionPtr, "Enter", false, false, false, false, false)
        return true
    }
}
