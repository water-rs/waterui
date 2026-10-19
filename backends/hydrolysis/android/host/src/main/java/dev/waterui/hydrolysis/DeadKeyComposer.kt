package dev.waterui.hydrolysis

import android.view.KeyCharacterMap

/**
 * Hardware dead-key composition for the view key path: the state an input
 * method carries between a `COMBINING_ACCENT` press and the character
 * that follows it, so a self-drawn field composes a hardware keyboard's
 * accent sequence exactly as the IME path does.
 *
 * The held accent survives anything that is not a character key press —
 * releases, named keys, navigation — the way the platform's own
 * composition state does; only the press that follows answers it.
 */
internal class DeadKeyComposer {
    /** The pending spacing-accent code point, or 0 while nothing is held. */
    private var pending = 0

    /**
     * A `COMBINING_ACCENT` key press arrived. While no accent is held this
     * one is — `null`, and the press still reports `"Dead"`. While one is,
     * the pair composes through [KeyCharacterMap.getDeadChar] — a doubled
     * accent resolves to its own spacing char on standard tables — or the
     * held accent commits as its spacing char and this accent is held in
     * turn.
     */
    fun accentDown(accent: Int): List<String>? {
        val held = pending
        if (held == 0) {
            pending = accent
            return null
        }
        val composed = KeyCharacterMap.getDeadChar(held, accent)
        if (composed != 0) {
            pending = 0
            return listOf(String(Character.toChars(composed)))
        }
        pending = accent
        return listOf(String(Character.toChars(held)))
    }

    /**
     * A character key press arrived. A held accent composes with it
     * through [KeyCharacterMap.getDeadChar] — the table the input method
     * composes with on the IME path — or, when the pair has no
     * composition, commits the accent's spacing char ahead of the key's
     * own character.
     */
    fun charDown(codePoint: Int): List<String>? {
        if (pending == 0) return null
        val held = pending
        pending = 0
        val composed = KeyCharacterMap.getDeadChar(held, codePoint)
        return if (composed != 0) {
            listOf(String(Character.toChars(composed)))
        } else {
            listOf(String(Character.toChars(held)), String(Character.toChars(codePoint)))
        }
    }
}
