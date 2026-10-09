package dev.waterui.hydrolysis

import org.junit.Assert.assertEquals
import org.junit.Test

class TouchOrderTest {
    @Test
    fun the_topmost_claiming_entry_owns_the_touch() {
        val claims = booleanArrayOf(true, true, false)
        assertEquals(1, TouchOrder.topmost(claims.size) { claims[it] })
    }

    @Test
    fun the_hit_test_asks_top_first_and_stops_at_the_first_claim() {
        val asked = mutableListOf<Int>()
        val owner = TouchOrder.topmost(4) { index -> asked += index; index == 2 }
        assertEquals(2, owner)
        assertEquals(listOf(3, 2), asked)
    }

    @Test
    fun no_claim_is_minus_one() {
        assertEquals(-1, TouchOrder.topmost(3) { false })
        assertEquals(-1, TouchOrder.topmost(0) { true })
    }
}
