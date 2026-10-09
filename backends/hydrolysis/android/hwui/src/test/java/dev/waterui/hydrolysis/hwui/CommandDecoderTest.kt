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
}
