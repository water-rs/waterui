package dev.waterui.hydrolysis.hwui

import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class CommandDecoderTest {
    /** A frame of `records`, each an opcode and its payload words. */
    private fun frame(sequence: Int, vararg records: IntArray, length: Int? = null, magic: Int = Protocol.MAGIC): Pair<ByteBuffer, Int> {
        val words = mutableListOf(magic, sequence, 0)
        for (record in records) {
            words += record[0] or ((record.size - 1) shl 16)
            for (index in 1 until record.size) words += record[index]
        }
        val bytes = words.size * 4
        words[2] = length ?: bytes
        val buffer = ByteBuffer.allocateDirect(bytes).order(ByteOrder.LITTLE_ENDIAN)
        words.forEach { buffer.putInt(it) }
        return buffer to bytes
    }

    private fun decodeFails(decoder: CommandDecoder, frame: Pair<ByteBuffer, Int>): String =
        assertThrows(CommandBufferException::class.java) { decoder.decode(frame.first, frame.second) }.message.orEmpty()

    private val record = intArrayOf(Protocol.RECORD, 0, 4, 4)
    private val endRecord = intArrayOf(Protocol.END_RECORD)

    @Test
    fun a_frame_with_a_foreign_magic_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, magic = 0x12345678))
        assertTrue(message, message.contains("magic 0x12345678"))
    }

    @Test
    fun a_header_length_that_is_not_the_frames_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, length = 99))
        assertTrue(message, message.contains("declares 99 bytes"))
    }

    @Test
    fun an_unknown_opcode_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, intArrayOf(0x0777)))
        assertTrue(message, message.contains("0x0777") && message.contains("unknown"))
    }

    @Test
    fun a_fixed_op_with_the_wrong_payload_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, intArrayOf(Protocol.CREATE_NODE, 1, 2)))
        assertTrue(message, message.contains("CreateNode") && message.contains("the layout is 1"))
    }

    @Test
    fun a_draw_outside_a_recording_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, intArrayOf(Protocol.DRAW_NODE, 0)))
        assertTrue(message, message.contains("outside a recording"))
    }

    @Test
    fun a_node_op_inside_a_recording_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, record, intArrayOf(Protocol.CREATE_NODE, 1), endRecord))
        assertTrue(message, message.contains("recording is open"))
    }

    @Test
    fun a_recording_left_open_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, record))
        assertTrue(message, message.contains("still open at the frame's end"))
    }

    @Test
    fun saves_must_balance_within_a_recording() {
        val restore = decodeFails(CommandDecoder(LoggingSink()), frame(1, record, intArrayOf(Protocol.RESTORE), endRecord))
        assertTrue(restore, restore.contains("restores more"))
        val save = decodeFails(CommandDecoder(LoggingSink()), frame(1, record, intArrayOf(Protocol.SAVE), endRecord))
        assertTrue(save, save.contains("unrestored"))
    }

    @Test
    fun frames_must_advance_the_sequence() {
        val decoder = CommandDecoder(LoggingSink())
        val (buffer, length) = frame(2)
        decoder.decode(buffer, length)
        val message = decodeFails(decoder, frame(2))
        assertTrue(message, message.contains("does not follow 2"))
    }

    @Test
    fun counts_that_overrun_a_variable_payload_are_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, intArrayOf(Protocol.HOST_ORDER, 2, HostKind.NODE, 0, 0)))
        assertTrue(message, message.contains("exceed"))
    }

    @Test
    fun a_bind_platform_view_with_the_wrong_payload_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, intArrayOf(Protocol.BIND_PLATFORM_VIEW, 5, 7, 0)))
        assertTrue(message, message.contains("payload"))
    }

    @Test
    fun a_bind_platform_view_with_a_nonpositive_density_is_refused() {
        fun bind(densityBits: Int) = intArrayOf(
            Protocol.BIND_PLATFORM_VIEW,
            5, 7, 0,
            0, 0, 0x41A00000, 0x41200000,
            densityBits,
            0x3F800000, 0, 0, 0, 0x3F800000, 0, 0, 0, 0x3F800000,
        )
        for (bits in intArrayOf(0, 0x7FC00000)) {
            val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, bind(bits)))
            assertTrue(message, message.contains("density"))
        }
    }

    @Test
    fun a_variable_payload_longer_than_its_counts_is_refused() {
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, intArrayOf(Protocol.HOST_ORDER, 0, 9)))
        assertTrue(message, message.contains("its counts imply 1"))
    }

    @Test
    fun a_path_whose_verbs_disagree_with_its_points_is_refused() {
        val path = intArrayOf(Protocol.DEFINE_PATH, 0, FillType.WINDING, 1, 2, Verb.MOVE, 0, 0, 0, 0)
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, path))
        assertTrue(message, message.contains("consume 1 points; it carries 2"))
    }

    @Test
    fun a_mesh_band_past_its_patch_limit_is_refused() {
        val patches = MeshInterpolation.BAND_PATCHES + 1
        val mesh = IntArray(3)
        mesh[0] = Protocol.MESH
        mesh[1] = MeshInterpolation.LINEAR
        mesh[2] = patches
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, record, mesh, endRecord))
        assertTrue(message, message.contains("its $patches patches exceed a band of ${MeshInterpolation.BAND_PATCHES}"))
    }

    @Test
    fun a_mesh_with_unknown_colour_weights_is_refused() {
        val mesh = intArrayOf(Protocol.MESH, 2, 0)
        val message = decodeFails(CommandDecoder(LoggingSink()), frame(1, record, mesh, endRecord))
        assertTrue(message, message.contains("mesh interpolation 2 is unknown"))
    }

    @Test
    fun a_well_formed_frame_reaches_the_sink_in_order() {
        val sink = LoggingSink()
        val (buffer, length) = frame(1, intArrayOf(Protocol.CREATE_NODE, 5), intArrayOf(Protocol.RECORD, 5, 2, 3), intArrayOf(Protocol.SAVE), intArrayOf(Protocol.RESTORE), endRecord, intArrayOf(Protocol.RELEASE_NODE, 5))
        CommandDecoder(sink).decode(buffer, length)
        assertEquals(
            listOf("Frame sequence=1", "CreateNode node=5", "Record node=5 width=2 height=3", "Save", "Restore", "EndRecord", "ReleaseNode node=5"),
            sink.lines,
        )
    }

    @Test
    fun decode_leaves_the_callers_cursor_and_order_untouched() {
        val sink = LoggingSink()
        val (buffer, length) = frame(1, record, endRecord)
        buffer.position(4)
        buffer.order(ByteOrder.BIG_ENDIAN)
        CommandDecoder(sink).decode(buffer, length)
        assertEquals(4, buffer.position())
        assertEquals(buffer.capacity(), buffer.limit())
        assertEquals(ByteOrder.BIG_ENDIAN, buffer.order())
        assertEquals(
            listOf("Frame sequence=1", "Record node=0 width=4 height=4", "EndRecord"),
            sink.lines,
        )
    }

    @Test
    fun a_refilled_and_grown_transport_buffer_carries_no_stale_tail() {
        val sink = LoggingSink()
        val decoder = CommandDecoder(sink)

        // The JVM transport refills one direct buffer: a shorter second
        // frame overwrites its head but leaves the first frame's tail.
        val buffer = ByteBuffer.allocateDirect(64).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(Protocol.MAGIC).putInt(1).putInt(20)
        buffer.putInt(Protocol.CREATE_NODE or (1 shl 16)).putInt(5)
        decoder.decode(buffer, 20)

        buffer.clear()
        buffer.putInt(Protocol.MAGIC).putInt(2).putInt(12)
        decoder.decode(buffer, 12)
        assertEquals(listOf("Frame sequence=1", "CreateNode node=5", "Frame sequence=2"), sink.lines)
        // The caller's cursor and order are untouched by the reuse.
        assertEquals(12, buffer.position())
        assertEquals(ByteOrder.LITTLE_ENDIAN, buffer.order())

        // A grown replacement buffer decodes the next frame.
        val grown = ByteBuffer.allocateDirect(128).order(ByteOrder.LITTLE_ENDIAN)
        grown.putInt(Protocol.MAGIC).putInt(3).putInt(12)
        decoder.decode(grown, 12)
        assertEquals(3, decoder.lastSequence)
        assertEquals(listOf("Frame sequence=1", "CreateNode node=5", "Frame sequence=2", "Frame sequence=3"), sink.lines)
    }

// ---- DefineEffect records ----

private fun effectRecord(effect: Int, count: Int, stages: IntArray): IntArray {
    val payload = intArrayOf(effect, count) + stages
    return intArrayOf(Protocol.DEFINE_EFFECT or (payload.size shl 16)) + payload
}

private fun blurStage(radius: Float): IntArray =
    intArrayOf(EffectKind.BLUR) + IntArray(12) { if (it == 0) java.lang.Float.floatToIntBits(radius) else 0 }

private fun colorStage(): IntArray =
    intArrayOf(EffectKind.LINEAR_COLOR) + IntArray(12) { if (it % 5 == 0) java.lang.Float.floatToIntBits(1f) else 0 }

    @Test
    fun a_valid_mixed_chain_decodes() {
        val sink = LoggingSink()
        val (buffer, length) = frame(1, effectRecord(0, 2, blurStage(8f) + colorStage()))
        CommandDecoder(sink).decode(buffer, length)
        assertEquals(1, sink.lines.count { it.startsWith("DefineEffect") })
    }

    @Test
    fun an_unknown_stage_kind_is_refused() {
        val bad = intArrayOf(99) + IntArray(12)
        val message =
            decodeFails(CommandDecoder(LoggingSink()), frame(1, effectRecord(0, 1, bad)))
        assertTrue(message.contains("unknown"))
    }

    @Test
    fun a_short_stage_is_refused() {
        val record = effectRecord(0, 1, blurStage(8f).dropLast(1).toIntArray())
        decodeFails(CommandDecoder(LoggingSink()), frame(1, record))
    }

    @Test
    fun a_nonfinite_value_is_refused() {
        val nan = intArrayOf(EffectKind.BLUR) + IntArray(12) {
            if (it == 0) java.lang.Float.floatToIntBits(Float.NaN) else 0
        }
        val inf = intArrayOf(EffectKind.BLUR) + IntArray(12) {
            if (it == 0) java.lang.Float.floatToIntBits(Float.POSITIVE_INFINITY) else 0
        }
        for (stage in listOf(nan, inf)) {
            val message = decodeFails(
                CommandDecoder(LoggingSink()),
                frame(1, effectRecord(0, 1, stage)),
            )
            assertTrue(message.contains("non-finite"))
        }
    }

    @Test
    fun an_oversized_count_is_refused() {
        val record = intArrayOf(Protocol.DEFINE_EFFECT or (2 shl 16), 0, 9999)
        decodeFails(CommandDecoder(LoggingSink()), frame(1, record))
    }

    @Test
    fun a_negative_count_is_refused() {
        val record = intArrayOf(Protocol.DEFINE_EFFECT or (2 shl 16), 0, -1)
        decodeFails(CommandDecoder(LoggingSink()), frame(1, record))
    }

    @Test
    fun effect_scratch_arrays_are_reused_across_frames() {
        val captured = mutableListOf<Pair<IntArray, FloatArray>>()
        val sink = object : LoggingSink() {
            override fun defineEffect(effect: Int, count: Int, kinds: IntArray, values: FloatArray) {
                captured += kinds to values
                super.defineEffect(effect, count, kinds, values)
            }
        }
        val decoder = CommandDecoder(sink)
        val first = frame(1, effectRecord(0, 1, blurStage(8f)))
        decoder.decode(first.first, first.second)
        val second = frame(2, effectRecord(0, 2, blurStage(4f) + colorStage()))
        decoder.decode(second.first, second.second)
        assertEquals(2, captured.size)
        assertTrue(captured[0].first === captured[1].first)
        assertTrue(captured[0].second === captured[1].second)
    }

    @Test
    fun a_trailing_word_after_the_stages_is_refused() {
        // The extra word is inside the record's payload: count-derived reads
        // must not consume it, and the exact-length check rejects it.
        val record = effectRecord(0, 1, blurStage(8f) + intArrayOf(0))
        decodeFails(CommandDecoder(LoggingSink()), frame(1, record))
    }

}
