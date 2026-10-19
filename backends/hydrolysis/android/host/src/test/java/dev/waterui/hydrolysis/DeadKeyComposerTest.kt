package dev.waterui.hydrolysis

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The hardware dead-key composition contract: a `COMBINING_ACCENT` press
 * holds its spacing accent until the next character key press composes
 * with it or commits it — the same sequences the platform's own editor
 * composition produces for a dead-key keyboard layout.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class DeadKeyComposerTest {

    @Test
    fun `a held accent composes with the next character`() {
        val composer = DeadKeyComposer()
        assertNull(composer.accentDown(0x00B4)) // ´
        assertEquals(listOf("é"), composer.charDown('e'.code))
    }

    @Test
    fun `a pair with no composition commits the accent then the character`() {
        val composer = DeadKeyComposer()
        assertNull(composer.accentDown(0x00B4)) // ´
        assertEquals(listOf("´", "x"), composer.charDown('x'.code))
    }

    @Test
    fun `a doubled accent composes to its own spacing char`() {
        val composer = DeadKeyComposer()
        assertNull(composer.accentDown(0x00B4)) // ´
        assertEquals(listOf("´"), composer.accentDown(0x00B4))
    }

    @Test
    fun `space after an accent commits the accent alone`() {
        val composer = DeadKeyComposer()
        assertNull(composer.accentDown(0x00B4)) // ´
        assertEquals(listOf("´"), composer.charDown(' '.code))
    }

    @Test
    fun `an uncomposable accent commits then holds the next accent`() {
        val composer = DeadKeyComposer()
        assertNull(composer.accentDown(0x00B4)) // ´
        // ´ does not compose with the dead circumflex, so it commits and
        // the circumflex is held in turn for the next character.
        assertEquals(listOf("´"), composer.accentDown('^'.code))
        assertEquals(listOf("ê"), composer.charDown('e'.code))
    }

    @Test
    fun `the accent is spent by one composition`() {
        val composer = DeadKeyComposer()
        assertNull(composer.accentDown(0x00B4)) // ´
        assertEquals(listOf("é"), composer.charDown('e'.code))
        // A second character arrives with nothing held — the caller's own
        // key value goes through.
        assertNull(composer.charDown('e'.code))
    }

    @Test
    fun `a character press with nothing held passes through`() {
        val composer = DeadKeyComposer()
        assertNull(composer.charDown('e'.code))
    }
}
