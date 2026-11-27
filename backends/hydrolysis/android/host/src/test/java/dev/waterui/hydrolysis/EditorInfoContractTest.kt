package dev.waterui.hydrolysis

import android.view.inputmethod.EditorInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class EditorInfoContractTest {
    private fun payload(
        password: Boolean = false,
        singleLine: Boolean = true,
        hasSubmit: Boolean = false,
        keyboard: String = "text",
        capitalization: String = "sentences",
        autocorrect: Boolean = true,
        submitLabel: String = "return",
    ): EditingStatePayload =
        EditingStatePayload(
            """
            {
              "editor_id": 1,
              "focused": true,
              "revision": 1,
              "text": "",
              "sel_start": 0,
              "sel_end": 0,
              "comp_start": -1,
              "comp_end": -1,
              "password": $password,
              "single_line": $singleLine,
              "has_submit": $hasSubmit,
              "keyboard": "$keyboard",
              "capitalization": "$capitalization",
              "autocorrect": $autocorrect,
              "submit_label": "$submitLabel"
            }
            """.trimIndent(),
        )

    @Test
    fun mapsEmailDecimalAndExplicitOptionsToInputTypeAndImeOptions() {
        val emailAttrs = EditorInfo()
        applyEditorInfo(
            emailAttrs,
            payload(keyboard = "email", capitalization = "none", autocorrect = false),
        )
        assertEquals(
            EditorInfo.TYPE_CLASS_TEXT or
                EditorInfo.TYPE_TEXT_VARIATION_EMAIL_ADDRESS or
                EditorInfo.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
            emailAttrs.inputType,
        )

        val decimalAttrs = EditorInfo()
        applyEditorInfo(decimalAttrs, payload(keyboard = "decimal", submitLabel = "done"))
        assertEquals(
            EditorInfo.TYPE_CLASS_NUMBER or EditorInfo.TYPE_NUMBER_FLAG_DECIMAL,
            decimalAttrs.inputType,
        )
        assertEquals(
            EditorInfo.IME_ACTION_DONE or EditorInfo.IME_FLAG_NO_FULLSCREEN,
            decimalAttrs.imeOptions,
        )

        val nameAttrs = EditorInfo()
        applyEditorInfo(
            nameAttrs,
            payload(capitalization = "words", autocorrect = false, submitLabel = "next"),
        )
        assertEquals(
            EditorInfo.TYPE_CLASS_TEXT or
                EditorInfo.TYPE_TEXT_FLAG_CAP_WORDS or
                EditorInfo.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
            nameAttrs.inputType,
        )
        assertEquals(
            EditorInfo.IME_ACTION_NEXT or EditorInfo.IME_FLAG_NO_FULLSCREEN,
            nameAttrs.imeOptions,
        )

        val postalAttrs = EditorInfo()
        applyEditorInfo(
            postalAttrs,
            payload(capitalization = "characters", autocorrect = false, submitLabel = "search"),
        )
        assertEquals(
            EditorInfo.TYPE_CLASS_TEXT or
                EditorInfo.TYPE_TEXT_FLAG_CAP_CHARACTERS or
                EditorInfo.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
            postalAttrs.inputType,
        )
        assertEquals(
            EditorInfo.IME_ACTION_SEARCH or EditorInfo.IME_FLAG_NO_FULLSCREEN,
            postalAttrs.imeOptions,
        )

        val passwordAttrs = EditorInfo()
        applyEditorInfo(passwordAttrs, payload(password = true, submitLabel = "go"))
        assertEquals(
            EditorInfo.TYPE_CLASS_TEXT or
                EditorInfo.TYPE_TEXT_VARIATION_PASSWORD or
                EditorInfo.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
            passwordAttrs.inputType,
        )
        assertTrue(passwordAttrs.imeOptions and EditorInfo.IME_ACTION_GO == EditorInfo.IME_ACTION_GO)
        assertTrue(
            passwordAttrs.imeOptions and EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING != 0,
        )
    }
}
