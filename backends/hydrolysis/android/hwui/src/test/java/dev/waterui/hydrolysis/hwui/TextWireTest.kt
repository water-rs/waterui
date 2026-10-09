package dev.waterui.hydrolysis.hwui

import org.junit.Assert.assertEquals
import org.junit.Test

/** The packings the Rust `positions_and_ranges_unpack_as_the_provider_packs_them` reads. */
class TextWireTest {
    @Test
    fun positions_pack_the_offset_above_the_affinity_bit() {
        assertEquals(11L, TextWire.packPosition(5, true))
        assertEquals(10L, TextWire.packPosition(5, false))
    }

    @Test
    fun ranges_pack_the_start_above_the_end() {
        assertEquals(12_884_901_897L, TextWire.packRange(3, 9))
        assertEquals(0x7fff_ffff_0000_0000L, TextWire.packRange(Int.MAX_VALUE, 0))
    }
}
