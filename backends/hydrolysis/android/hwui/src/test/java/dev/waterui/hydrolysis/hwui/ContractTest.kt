package dev.waterui.hydrolysis.hwui

import java.io.File
import java.nio.ByteBuffer
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The JVM half of the command-buffer contract: decodes the frames the Rust
 * half (`hwui::contract`) encoded for this run and requires its event log.
 */
class ContractTest {
    @Test
    fun every_scene_decodes_to_the_event_log_the_encoder_wrote() {
        val path = System.getProperty("hwui.contract.dir")
        checkNotNull(path) { "hwui.contract.dir is unset: run through Gradle, whose contractScenes task encodes the scenes" }
        val dir = File(path)
        val logs = dir.listFiles { file -> file.name.endsWith(".log") }.orEmpty().sortedBy { it.name }
        assertEquals("the scenes the Rust half encodes", listOf("direct.log", "tree.log"), logs.map { it.name })
        for (log in logs) {
            val scene = log.name.removeSuffix(".log")
            val frames =
                dir.listFiles { file -> file.name.startsWith("$scene-") && file.name.endsWith(".bin") }
                    .orEmpty()
                    .sortedBy { it.name.removePrefix("$scene-").removeSuffix(".bin").toInt() }
            assertTrue("scene $scene has frames", frames.isNotEmpty())
            val sink = LoggingSink()
            val decoder = CommandDecoder(sink)
            for (frame in frames) {
                val bytes = frame.readBytes()
                val buffer = ByteBuffer.allocateDirect(bytes.size)
                buffer.put(bytes)
                decoder.decode(buffer, bytes.size)
            }
            assertEquals("scene $scene", log.readText(), sink.lines.joinToString("\n", postfix = "\n"))
        }
    }

    @Test
    fun packed_text_runs_read_back_as_the_runs_the_engine_packed() {
        val dir = File(checkNotNull(System.getProperty("hwui.contract.dir")) { "hwui.contract.dir is unset" })
        val wire = File(dir, "text-runs.wire").readLines()
        val locale = wire[0]
        val maxWidth = wire[1].toFloat()
        val maxLines = wire[2].toInt()
        val paragraph = wire[3].toInt()
        val familyCount = wire[4].toInt()
        val families = wire.subList(5, 5 + familyCount).toTypedArray()
        val spanCount = wire[5 + familyCount].toInt()
        val spans = wire[6 + familyCount].split(" ").map { it.toInt() }.toIntArray()
        assertEquals("packed words", spanCount * TextWire.SPAN_WORDS, spans.size)
        val described =
            describeParagraph(locale, maxWidth, maxLines, paragraph) + "\n" +
                (0 until spanCount).joinToString("") { describeRun(spans, it, families) + "\n" }
        assertEquals(File(dir, "text-runs.expected").readText(), described)
    }

    /** The paragraph as the Rust `describe_paragraph` prints it. */
    private fun describeParagraph(locale: String, maxWidth: Float, maxLines: Int, paragraph: Int): String {
        val width = if (maxWidth == TextWire.UNBOUNDED) "none" else "%.3f".format(Locale.ROOT, maxWidth)
        val lines = if (maxLines == TextWire.NO_LINE_LIMIT) "none" else maxLines.toString()
        return "paragraph locale=$locale maxWidth=$width maxLines=$lines" +
            " align=${paragraph and TextWire.ALIGN_MASK}" +
            " rtl=${paragraph and TextWire.RIGHT_TO_LEFT != 0}" +
            " ellipsis=${paragraph and TextWire.ELLIPSIS != 0}" +
            " strict=${paragraph and TextWire.STRICT_FAMILIES != 0}"
    }

    /** Run `run` of `spans` as the Rust `describe_run` prints it. */
    private fun describeRun(spans: IntArray, run: Int, families: Array<String>): String {
        val at = run * TextWire.SPAN_WORDS
        val flags = spans[at + TextWire.FLAGS]
        val family =
            spans[at + TextWire.FAMILY].let {
                if (it == TextWire.DEFAULT_FAMILY) "default" else families[it].replace(TextWire.FAMILY_SEPARATOR, '|')
            }
        fun colour(flag: Int, word: Int) =
            if (flags and flag != 0) "%016x".format(Locale.ROOT, TextWire.colorLong(spans, at + word)) else "none"
        val lineHeight =
            if (flags and TextWire.HAS_LINE_HEIGHT != 0) {
                "%.3f".format(Locale.ROOT, Float.fromBits(spans[at + TextWire.LINE_HEIGHT]))
            } else {
                "none"
            }
        return "run ${spans[at + TextWire.START]}..${spans[at + TextWire.END]} family=$family" +
            " size=${"%.3f".format(Locale.ROOT, Float.fromBits(spans[at + TextWire.SIZE]))}" +
            " weight=${spans[at + TextWire.WEIGHT]}" +
            " italic=${flags and TextWire.ITALIC != 0}" +
            " underline=${flags and TextWire.UNDERLINE != 0}" +
            " strikethrough=${flags and TextWire.STRIKETHROUGH != 0}" +
            " foreground=${colour(TextWire.HAS_FOREGROUND, TextWire.FOREGROUND)}" +
            " background=${colour(TextWire.HAS_BACKGROUND, TextWire.BACKGROUND)}" +
            " lineHeight=$lineHeight" +
            " letterSpacingEm=${"%.3f".format(Locale.ROOT, Float.fromBits(spans[at + TextWire.LETTER_SPACING]))}"
    }
}
