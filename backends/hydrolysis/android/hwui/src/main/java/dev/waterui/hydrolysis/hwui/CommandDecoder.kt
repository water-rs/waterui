package dev.waterui.hydrolysis.hwui

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import java.nio.IntBuffer

/** A command buffer the decoder refuses: the frame is not replayed further. */
class CommandBufferException(message: String) : IllegalStateException(message)

/**
 * Reads one HWUI frame and drives a [CommandSink]. Pure Kotlin, so the
 * contract test runs it on the JVM. Every record's payload length is checked
 * against its opcode's layout (or what its counts imply), an unknown opcode
 * or kind fails, recording ops must sit between `Record` and `EndRecord` and
 * node and host ops outside them, and saves must balance per recording.
 * Scratch arrays grow and are reused, and operand arrays are bulk-read
 * through `int` and `float` views of the buffer, made once per
 * buffer generation (a new buffer when the Rust side grows its storage), so
 * a steady frame allocates nothing.
 */
class CommandDecoder(private val sink: CommandSink) {
    private var buffer: ByteBuffer = ByteBuffer.allocate(0)
    private var viewed: ByteBuffer? = null
    private var intView: IntBuffer = IntBuffer.allocate(0)
    private var floatView: FloatBuffer = FloatBuffer.allocate(0)
    private var position = 0
    private var at = 0
    private var end = 0
    private var op = 0
    private var sequence = 0L
    private var recording = Protocol.NONE
    private var saves = 0

    private val shape = WireShape()
    private val paint = WirePaint()
    private val stroke = WireStroke()
    private val matrix = FloatArray(Protocol.MATRIX_WORDS)
    private val transform = FloatArray(Protocol.TRANSFORM_VALUES)
    private var ints = IntArray(64)
    private var floats = FloatArray(128)
    private var floats2 = FloatArray(128)
    private var longs = LongArray(16)

    /** The sequence of the last frame decoded, 0 before the first. */
    val lastSequence: Long
        get() = sequence

    /**
     * Decodes the first `length` bytes of `source`, which this call sets to
     * little-endian order.
     *
     * @throws CommandBufferException naming the frame, the op and the fault.
     */
    fun decode(source: ByteBuffer, length: Int) {
        position = 0
        op = 0
        if (length < Protocol.HEADER_BYTES || length % 4 != 0 || length > source.capacity()) {
            fail("a frame of $length bytes in a buffer of ${source.capacity()} is not whole words with a header")
        }
        source.order(ByteOrder.LITTLE_ENDIAN)
        buffer = source
        if (source !== viewed) {
            viewed = source
            val whole = source.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            whole.clear()
            intView = whole.asIntBuffer()
            floatView = whole.asFloatBuffer()
        }
        val magic = source.getInt(0)
        if (magic != Protocol.MAGIC) fail(String.format("magic 0x%08x is not HWUI", magic))
        val next = source.getInt(4).toLong() and 0xFFFF_FFFFL
        if (next <= sequence) fail("frame sequence $next does not follow $sequence")
        val declared = source.getInt(8)
        if (declared != length) fail("the header declares $declared bytes; the frame is $length")
        sequence = next
        recording = Protocol.NONE
        saves = 0
        sink.beginFrame(next)
        position = Protocol.HEADER_BYTES
        while (position < length) {
            val header = source.getInt(position)
            op = header and 0xFFFF
            val words = header ushr 16
            at = position + 4
            end = at + words * 4
            if (end > length) fail("its payload of $words words runs past the frame's end")
            val fixed = Protocol.fixedWords(op)
            if (fixed == Protocol.UNKNOWN) fail("the opcode is unknown")
            if (fixed >= 0 && fixed != words) fail("its payload is $words words; the layout is $fixed")
            checkPlacement()
            dispatch()
            if (at != end) {
                fail("its payload is $words words; its counts imply ${(at - position - 4) / 4}")
            }
            position = end
        }
        op = 0
        if (recording != Protocol.NONE) fail("node ${unsigned(recording)}'s recording is still open at the frame's end")
        sink.endFrame()
    }

    private fun checkPlacement() {
        val group = op ushr 8
        val open = recording != Protocol.NONE
        when {
            op == Protocol.RECORD && open ->
                fail("node ${unsigned(recording)}'s recording is still open")
            op == Protocol.END_RECORD -> if (!open) fail("no recording is open")
            group == 0x02 && !open -> fail("it draws outside a recording")
            (group == 0x01 || group == 0x04) && open ->
                fail("node ${unsigned(recording)}'s recording is open")
        }
    }

    private fun dispatch() {
        when (op) {
            Protocol.CREATE_NODE -> sink.createNode(word())
            Protocol.RELEASE_NODE -> sink.releaseNode(word())
            Protocol.SET_POSITION ->
                sink.setPosition(word(), word(), word(), word(), word(), flag())
            Protocol.SET_TRANSFORM -> {
                val node = word()
                readFloats(transform, Protocol.TRANSFORM_VALUES)
                sink.setTransform(node, transform)
            }
            Protocol.SET_ALPHA -> sink.setAlpha(word(), float())
            Protocol.SET_CLIP -> {
                val node = word()
                val kind = word()
                if (kind !in ClipKind.NONE..ClipKind.ROUND_RECT) fail("clip kind $kind is unknown")
                sink.setClip(node, kind, word(), word(), word(), word(), float())
            }
            Protocol.SET_COMPOSITE -> {
                val node = word()
                val blend = word()
                if (blend !in BlendCode.CLEAR..BlendCode.LUMINOSITY || blend == 13) {
                    fail("blend code $blend is unknown")
                }
                sink.setComposite(node, blend, flag())
            }
            Protocol.SET_EFFECT -> sink.setEffect(word(), word())
            Protocol.RECORD -> {
                val node = word()
                val width = word()
                val height = word()
                if (width < 0 || height < 0) fail("node ${unsigned(node)} records a ${width}x$height list")
                recording = node
                saves = 0
                sink.record(node, width, height)
            }
            Protocol.END_RECORD -> {
                if (saves != 0) fail("node ${unsigned(recording)}'s recording ends with $saves unrestored saves")
                recording = Protocol.NONE
                sink.endRecord()
            }
            Protocol.SAVE -> {
                saves++
                sink.save()
            }
            Protocol.RESTORE -> {
                if (saves == 0) fail("it restores more than the recording saved")
                saves--
                sink.restore()
            }
            Protocol.CONCAT -> {
                readMatrix()
                sink.concat(matrix)
            }
            Protocol.CLIP_RECT -> sink.clipRect(float(), float(), float(), float())
            Protocol.CLIP_PATH -> sink.clipPath(word())
            Protocol.FILL -> {
                readShape()
                readPaint()
                sink.fill(shape, paint)
            }
            Protocol.STROKE -> {
                readShape()
                readPaint()
                readStroke()
                sink.stroke(shape, paint, stroke)
            }
            Protocol.SHADOW -> {
                readShape()
                sink.shadow(shape, color(), float(), float(), float(), float())
            }
            Protocol.GLYPHS -> {
                val font = word()
                val size = float()
                readPaint()
                val style = word()
                if (style != GlyphStyle.FILL && style != GlyphStyle.STROKE) fail("glyph style $style is unknown")
                readStroke()
                val count = count()
                val ids = ints(count)
                readInts(ids, count)
                val positions = floats(2 * count)
                readFloats(positions, 2 * count)
                sink.glyphs(font, size, paint, style, stroke, count, ids, positions)
            }
            Protocol.TEXT -> {
                val layout = word()
                readMatrix()
                sink.text(layout, matrix)
            }
            Protocol.IMAGE -> {
                val bitmap = word()
                val left = float()
                val top = float()
                val right = float()
                val bottom = float()
                sink.image(bitmap, left, top, right, bottom, sampling())
            }
            Protocol.MESH -> readMesh()
            Protocol.DRAW_NODE -> sink.drawNode(word())
            Protocol.DEFINE_PATH -> readPath()
            Protocol.RELEASE_PATH -> sink.releasePath(word())
            Protocol.DEFINE_SHADER -> readShader()
            Protocol.RELEASE_SHADER -> sink.releaseShader(word())
            Protocol.RELEASE_FONT -> sink.releaseFont(word())
            Protocol.RELEASE_BITMAP -> sink.releaseBitmap(word())
            Protocol.RELEASE_RUNTIME_SHADER -> sink.releaseRuntimeShader(word())
            Protocol.RELEASE_EFFECT -> sink.releaseEffect(word())
            Protocol.RELEASE_TEXT_LAYOUT -> sink.releaseTextLayout(word())
            Protocol.DERIVE_FONT -> {
                val font = word()
                val base = word()
                val count = count()
                val tags = ints(count)
                readInts(tags, count)
                val values = floats(count)
                readFloats(values, count)
                sink.deriveFont(font, base, count, tags, values)
            }
            Protocol.HOST_ORDER -> {
                val count = count()
                val kinds = ints(count)
                val ids = longs(count)
                for (i in 0 until count) {
                    val kind = word()
                    if (kind != HostKind.NODE && kind != HostKind.PLATFORM_VIEW) fail("host entry kind $kind is unknown")
                    kinds[i] = kind
                    ids[i] = (word().toLong() and 0xFFFF_FFFFL) or (word().toLong() shl 32)
                }
                sink.hostOrder(count, kinds, ids)
            }
            else -> fail("the opcode is unknown")
        }
    }

    private fun readShape() {
        val kind = word()
        shape.kind = kind
        when (kind) {
            ShapeKind.RECT, ShapeKind.OVAL, ShapeKind.LINE -> {
                readBox()
                pad(2)
            }
            ShapeKind.ROUND_RECT -> {
                readBox()
                shape.rx = float()
                shape.ry = float()
            }
            ShapeKind.PATH -> {
                shape.path = word()
                pad(5)
            }
            else -> fail("shape kind $kind is unknown")
        }
    }

    private fun readBox() {
        shape.left = float()
        shape.top = float()
        shape.right = float()
        shape.bottom = float()
    }

    private fun readPaint() {
        val kind = word()
        paint.kind = kind
        when (kind) {
            PaintKind.COLOR -> {
                paint.color = color()
                paint.shader = Protocol.NONE
            }
            PaintKind.SHADER -> {
                paint.shader = word()
                pad(1)
            }
            else -> fail("paint kind $kind is unknown")
        }
        paint.alpha = float()
    }

    private fun readStroke() {
        stroke.width = float()
        stroke.cap = word()
        if (stroke.cap !in CapKind.BUTT..CapKind.SQUARE) fail("cap ${stroke.cap} is unknown")
        stroke.join = word()
        if (stroke.join !in JoinKind.MITER..JoinKind.BEVEL) fail("join ${stroke.join} is unknown")
        stroke.miter = float()
        stroke.phase = float()
        val count = count()
        if (stroke.intervals.size < count) stroke.intervals = FloatArray(grow(count))
        readFloats(stroke.intervals, count)
        stroke.dashCount = count
    }

    private fun readMesh() {
        val interpolation = word()
        if (interpolation != MeshInterpolation.LINEAR && interpolation != MeshInterpolation.SMOOTHSTEP) {
            fail("mesh interpolation $interpolation is unknown")
        }
        val patches = word()
        if (patches < 0 || patches > MeshInterpolation.BAND_PATCHES) {
            fail("its ${unsigned(patches)} patches exceed a band of ${MeshInterpolation.BAND_PATCHES}")
        }
        val length = patches * MeshInterpolation.PATCH_FLOATS
        val data = floats(length)
        readFloats(data, length)
        sink.mesh(interpolation, patches, data)
    }

    private fun readPath() {
        val path = word()
        val fillType = word()
        if (fillType != FillType.WINDING && fillType != FillType.EVEN_ODD) fail("fill type $fillType is unknown")
        val verbCount = count()
        val pointCount = count()
        val verbs = ints(verbCount)
        readInts(verbs, verbCount)
        var consumed = 0
        for (i in 0 until verbCount) {
            val points = Verb.points(verbs[i])
            if (points < 0) fail("path verb ${verbs[i]} is unknown")
            consumed += points
        }
        if (consumed != pointCount) fail("its verbs consume $consumed points; it carries $pointCount")
        val points = floats(2 * pointCount)
        readFloats(points, 2 * pointCount)
        sink.definePath(path, fillType, verbCount, verbs, pointCount, points)
    }

    private fun readShader() {
        val shader = word()
        val kind = word()
        readMatrix()
        when (kind) {
            ShaderKind.LINEAR -> {
                val x0 = float()
                val y0 = float()
                val x1 = float()
                val y1 = float()
                val tile = tile()
                val stops = readStops()
                sink.defineLinearGradient(shader, matrix, x0, y0, x1, y1, tile, stops, longs, floats)
            }
            ShaderKind.RADIAL -> {
                val x0 = float()
                val y0 = float()
                val r0 = float()
                val x1 = float()
                val y1 = float()
                val r1 = float()
                val tile = tile()
                val stops = readStops()
                sink.defineRadialGradient(shader, matrix, x0, y0, r0, x1, y1, r1, tile, stops, longs, floats)
            }
            ShaderKind.SWEEP -> {
                val cx = float()
                val cy = float()
                val stops = readStops()
                sink.defineSweepGradient(shader, matrix, cx, cy, stops, longs, floats)
            }
            ShaderKind.BITMAP -> {
                val bitmap = word()
                val tileX = tile()
                val tileY = tile()
                sink.defineBitmapShader(shader, matrix, bitmap, tileX, tileY, sampling())
            }
            ShaderKind.RUNTIME -> {
                val runtime = word()
                val count = count()
                val uniforms = floats(count)
                readFloats(uniforms, count)
                sink.defineRuntimeShader(shader, matrix, runtime, count, uniforms)
            }
            else -> fail("shader kind $kind is unknown")
        }
    }

    /** Reads `stop_count`, the colours into [longs] and the positions into [floats]. */
    private fun readStops(): Int {
        val stops = count()
        if (stops < 2) fail("a gradient carries $stops stops; it needs two")
        val colors = longs(stops)
        for (i in 0 until stops) colors[i] = color()
        val positions = floats(stops)
        readFloats(positions, stops)
        return stops
    }

    private fun readMatrix() = readFloats(matrix, Protocol.MATRIX_WORDS)

    private fun tile(): Int {
        val tile = word()
        if (tile !in Tile.CLAMP..Tile.DECAL) fail("tile mode $tile is unknown")
        return tile
    }

    private fun sampling(): Int {
        val sampling = word()
        if (sampling != SamplingKind.NEAREST && sampling != SamplingKind.LINEAR) fail("sampling $sampling is unknown")
        return sampling
    }

    private fun flag(): Boolean =
        when (val value = word()) {
            0 -> false
            1 -> true
            else -> fail("flag word $value is neither 0 nor 1")
        }

    private fun pad(words: Int) {
        repeat(words) {
            val value = word()
            if (value != 0) fail("a padding word holds $value")
        }
    }

    /** A count word, bounded by the payload words left so a bad count cannot size a huge array. */
    private fun count(): Int {
        val value = word()
        if (value < 0 || value > (end - at) / 4 + 1) {
            fail("count ${unsigned(value)} exceeds its payload")
        }
        return value
    }

    private fun color(): Long {
        val low = word().toLong() and 0xFFFF_FFFFL
        val high = word().toLong() shl 32
        return high or low
    }

    private fun word(): Int {
        if (at + 4 > end) fail("its counts exceed its payload")
        val value = buffer.getInt(at)
        at += 4
        return value
    }

    private fun float(): Float = java.lang.Float.intBitsToFloat(word())

    /** Fails unless `words` more words fit the payload. */
    private fun need(words: Int) {
        if (words < 0 || at + words.toLong() * 4 > end) fail("its counts exceed its payload")
    }

    private fun readInts(dst: IntArray, count: Int) {
        need(count)
        intView.position(at ushr 2)
        intView.get(dst, 0, count)
        at += count * 4
    }

    private fun readFloats(dst: FloatArray, count: Int) {
        need(count)
        floatView.position(at ushr 2)
        floatView.get(dst, 0, count)
        at += count * 4
    }

    private fun ints(count: Int): IntArray {
        if (ints.size < count) ints = IntArray(grow(count))
        return ints
    }

    private fun floats(count: Int): FloatArray {
        if (floats.size < count) floats = FloatArray(grow(count))
        return floats
    }

    private fun floats2(count: Int): FloatArray {
        if (floats2.size < count) floats2 = FloatArray(grow(count))
        return floats2
    }

    private fun longs(count: Int): LongArray {
        if (longs.size < count) longs = LongArray(grow(count))
        return longs
    }

    private fun grow(count: Int): Int = Integer.highestOneBit(count) shl 1

    private fun fail(reason: String): Nothing {
        val where = if (op == 0) "" else ", ${Protocol.name(op)} at byte $position"
        throw CommandBufferException("HWUI command buffer: frame $sequence$where: $reason")
    }

    private fun unsigned(value: Int): String = Integer.toUnsignedString(value)
}
