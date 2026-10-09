package dev.waterui.hydrolysis.hwui

import android.graphics.BlendMode
import android.graphics.BlurMaskFilter
import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorSpace
import android.graphics.DashPathEffect
import android.graphics.LinearGradient
import android.graphics.Matrix
import android.graphics.Mesh
import android.graphics.MeshSpecification
import android.graphics.Outline
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RadialGradient
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.RecordingCanvas
import android.graphics.RenderEffect
import android.graphics.RenderNode
import android.graphics.RuntimeShader
import android.graphics.Shader
import android.graphics.SweepGradient
import android.graphics.fonts.Font
import android.graphics.fonts.FontVariationAxis
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.core.graphics.withMatrix
import androidx.core.graphics.withTranslation
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * An AGSL program registered for [Protocol.DEFINE_SHADER]'s runtime kind:
 * its source and its float uniforms in the order the wire carries them.
 */
class RuntimeShaderProgram(val source: String, val uniformNames: Array<String>, val uniformSizes: IntArray) {
    init {
        require(uniformNames.size == uniformSizes.size) { "a runtime shader names ${uniformNames.size} uniforms and sizes ${uniformSizes.size}" }
    }

    val floats: Int = uniformSizes.sum()

    /** Per-uniform scratch, so setting a uniform allocates nothing. */
    internal val scratch: Array<FloatArray> = Array(uniformSizes.size) { FloatArray(uniformSizes[it]) }

    /**
     * Compiled shaders of this program no definition holds: a released
     * definition returns its shader here and the next one takes it, so a
     * re-record compiles nothing. A display list recorded earlier keeps the
     * native shader it drew with; setting uniforms or a local matrix makes
     * a new one.
     */
    internal val idle = ArrayList<Any>()
}

/** A dense id → resource table; ids are the Rust side's free-list ids. */
internal class Table<T : Any>(private val kind: String) {
    private var items = arrayOfNulls<Any>(16)

    fun put(id: Int, item: T) {
        if (id < 0) throw CommandBufferException("HWUI replay: $kind id ${Integer.toUnsignedString(id)} is out of range")
        if (id >= items.size) items = items.copyOf(maxOf(items.size * 2, id + 1))
        if (items[id] != null) throw CommandBufferException("HWUI replay: $kind $id is already live")
        items[id] = item
    }

    @Suppress("UNCHECKED_CAST")
    operator fun get(id: Int): T =
        (items.getOrNull(id) ?: throw CommandBufferException("HWUI replay: $kind ${Integer.toUnsignedString(id)} is not live")) as T

    fun contains(id: Int): Boolean = items.getOrNull(id) != null

    fun remove(id: Int): T {
        val item = get(id)
        items[id] = null
        return item
    }

    fun forEach(each: (T) -> Unit) {
        for (item in items) {
            @Suppress("UNCHECKED_CAST")
            if (item != null) each(item as T)
        }
    }

    fun clear() {
        items.fill(null)
    }
}

/**
 * Replays decoded frames into retained [RenderNode]s. Node properties and
 * display lists live in the nodes; paths, shaders, fonts, bitmaps, runtime
 * programs, effects and text layouts in dense tables. A steady frame
 * allocates nothing: paints, matrices, outlines and rectangles are reused,
 * and the only allocations are new resources (a node, path or shader, a
 * dash pattern or blur radius that changed, a mesh).
 */
class RenderNodeSink(private val text: TextLayouts) : CommandSink {
    private val nodes = Table<RenderNode>("node")
    private val paths = Table<Path>("path")
    private val shaders = Table<Shader>("shader")
    /** Each shader's [SamplingKind] below API 33, or [NO_SAMPLING]. */
    private var shaderSampling = IntArray(16) { NO_SAMPLING }
    /** The program each runtime shader definition draws from. */
    private val runtimeOwners = Table<RuntimeShaderProgram>("runtime shader definition")
    private val fonts = Table<Font>("font")
    private val bitmaps = Table<Bitmap>("bitmap")
    private val programs = Table<RuntimeShaderProgram>("runtime shader")
    private val effects = Table<RenderEffect>("effect")

    private var canvas: RecordingCanvas? = null
    private var recording: RenderNode? = null
    private val paint = Paint()
    private val layerPaint = Paint()
    private val matrix = Matrix()
    private val rect = Rect()
    private val rectF = RectF()
    private val outline = Outline()
    private val extendedSrgb = ColorSpace.get(ColorSpace.Named.EXTENDED_SRGB)

    private var dash: DashPathEffect? = null
    private var dashPhase = 0f
    private var dashIntervals = FloatArray(0)
    private var blur: BlurMaskFilter? = null
    private var blurRadius = 0f
    private var meshSpecification: Any? = null
    private var meshIndices: java.nio.ShortBuffer? = null

    /** The session's top-level draw order of the last frame. */
    var hostCount: Int = 0
        private set
    var hostKinds: IntArray = IntArray(8)
        private set
    var hostIds: LongArray = LongArray(8)
        private set

    /** The largest bitmap edge RenderThread samples: its maximum texture size. */
    fun maxBitmapDimension(): Int {
        val probe = RenderNode("HWUI bitmap limit")
        val canvas = probe.beginRecording()
        val dimension = minOf(canvas.maximumBitmapWidth, canvas.maximumBitmapHeight)
        probe.endRecording()
        return dimension
    }

    /** An app font from bytes Rust owns, copied into Java-owned memory. */
    fun registerFontBytes(id: Int, bytes: ByteBuffer, ttcIndex: Int) {
        val copy = ByteBuffer.allocateDirect(bytes.remaining())
        copy.put(bytes.duplicate())
        copy.flip()
        fonts.put(id, Font.Builder(copy).setTtcIndex(ttcIndex).build())
    }

    /**
     * The system font platform matching resolves `family` to; returns its
     * file data for the Rust side's metrics, the face index from [fontTtcIndex].
     */
    fun registerSystemFont(id: Int, family: String): ByteBuffer {
        val font = SystemFamilies.primaryFont(family)
        fonts.put(id, font)
        return font.buffer
    }

    fun fontTtcIndex(id: Int): Int = fonts[id].ttcIndex

    /** Uploads `pixels` once as a hardware bitmap. */
    fun registerBitmap(id: Int, width: Int, height: Int, format: Int, colorSpace: Int, premultiplied: Boolean, pixels: ByteBuffer) {
        val config = when (format) {
            BitmapFormat.ARGB_8888 -> Bitmap.Config.ARGB_8888
            BitmapFormat.RGBA_F16 -> Bitmap.Config.RGBA_F16
            else -> throw CommandBufferException("HWUI registration: bitmap format $format is unknown")
        }
        val space = bitmapColorSpace(format, colorSpace)
        val staging = Bitmap.createBitmap(width, height, config, true, space)
        // `copyPixelsFromBuffer` copies verbatim; an unpremultiplied source is
        // tagged so, then premultiplied by drawing it into a premultiplied target.
        if (!premultiplied) staging.isPremultiplied = false
        staging.copyPixelsFromBuffer(pixels.duplicate())
        val source = if (premultiplied) {
            staging
        } else {
            Bitmap.createBitmap(width, height, config, true, space).also { Canvas(it).drawBitmap(staging, 0f, 0f, null) }
        }
        val hardware = source.copy(Bitmap.Config.HARDWARE, false)
            ?: throw CommandBufferException("HWUI registration: the device refused a ${width}x$height hardware bitmap")
        bitmaps.put(id, hardware)
    }

    private fun bitmapColorSpace(format: Int, code: Int): ColorSpace {
        val f16 = format == BitmapFormat.RGBA_F16
        return when (code) {
            BitmapColorSpace.SRGB -> if (f16) extendedSrgb else ColorSpace.get(ColorSpace.Named.SRGB)
            BitmapColorSpace.DISPLAY_P3 -> ColorSpace.get(ColorSpace.Named.DISPLAY_P3)
            BitmapColorSpace.LINEAR_SRGB ->
                ColorSpace.get(if (f16) ColorSpace.Named.LINEAR_EXTENDED_SRGB else ColorSpace.Named.LINEAR_SRGB)
            BitmapColorSpace.LINEAR_P3 -> linearP3
            else -> throw CommandBufferException("HWUI registration: bitmap colour space $code is unknown")
        }
    }

    private val linearP3: ColorSpace by lazy {
        val p3 = ColorSpace.get(ColorSpace.Named.DISPLAY_P3) as ColorSpace.Rgb
        ColorSpace.Rgb("Linear Display P3", p3.primaries, p3.whitePoint, 1.0)
    }

    /**
     * An AGSL program and its float uniforms, compiled here once: a source
     * the platform rejects fails its registration, and the compiled shader
     * serves the first definition that draws from it.
     */
    fun registerRuntimeShader(id: Int, source: String, uniformNames: Array<String>, uniformSizes: IntArray) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            throw CommandBufferException("HWUI registration: runtime shaders need API 33; this device is API ${Build.VERSION.SDK_INT}")
        }
        val program = RuntimeShaderProgram(source, uniformNames, uniformSizes)
        program.idle.add(RuntimeShader(source))
        programs.put(id, program)
    }

    /** The node `id`, for the host to draw a top-level run. */
    fun node(id: Int): RenderNode = nodes[id]

    /** Drops every resource, for a painter that detaches; nothing is recycled explicitly. */
    fun clear() {
        nodes.clear()
        paths.clear()
        shaders.clear()
        shaderSampling.fill(NO_SAMPLING)
        runtimeOwners.clear()
        fonts.clear()
        bitmaps.clear()
        programs.clear()
        effects.clear()
        hostCount = 0
    }

    private fun canvas(): RecordingCanvas =
        canvas ?: throw CommandBufferException("HWUI replay: a draw reached the sink outside a recording")

    override fun beginFrame(sequence: Long) = Unit

    override fun endFrame() = Unit

    override fun createNode(node: Int) = nodes.put(node, RenderNode("hydrolysis#$node"))

    // Dropping the reference frees the node and its display list once no parent draws it.
    override fun releaseNode(node: Int) {
        nodes.remove(node)
    }

    override fun setPosition(node: Int, left: Int, top: Int, right: Int, bottom: Int, clipToBounds: Boolean) {
        val target = nodes[node]
        target.setPosition(left, top, right, bottom)
        target.clipToBounds = clipToBounds
    }

    override fun setTransform(node: Int, values: FloatArray) {
        val target = nodes[node]
        target.translationX = values[0]
        target.translationY = values[1]
        target.scaleX = values[2]
        target.scaleY = values[3]
        target.rotationZ = values[4]
        target.pivotX = values[5]
        target.pivotY = values[6]
        target.rotationX = values[7]
        target.rotationY = values[8]
        // 0 is the wire's "no perspective": the node has no x/y rotation for
        // a camera to see, and `setCameraDistance` takes positive distances.
        if (values[9] != 0f) target.cameraDistance = values[9]
    }

    override fun setAlpha(node: Int, alpha: Float) {
        nodes[node].alpha = alpha
    }

    override fun setClip(node: Int, kind: Int, left: Int, top: Int, right: Int, bottom: Int, radius: Float) {
        val target = nodes[node]
        when (kind) {
            ClipKind.NONE -> {
                target.setClipRect(null)
                target.clipToOutline = false
            }
            ClipKind.RECT -> {
                target.clipToOutline = false
                rect.set(left, top, right, bottom)
                target.setClipRect(rect)
            }
            else -> {
                target.setClipRect(null)
                outline.setRoundRect(left, top, right, bottom, radius)
                target.setOutline(outline)
                target.clipToOutline = true
            }
        }
    }

    override fun setComposite(node: Int, blend: Int, forceLayer: Boolean) {
        val target = nodes[node]
        if (blend == BlendCode.SRC_OVER) {
            target.setUseCompositingLayer(forceLayer, null)
        } else {
            layerPaint.blendMode = BlendCode.mode(blend)
            target.setUseCompositingLayer(forceLayer, layerPaint)
        }
    }

    override fun setEffect(node: Int, effect: Int) {
        nodes[node].setRenderEffect(if (effect == Protocol.NONE) null else effects[effect])
    }

    override fun record(node: Int, width: Int, height: Int) {
        val target = nodes[node]
        recording = target
        canvas = target.beginRecording(width, height)
    }

    override fun endRecord() {
        val target = recording ?: throw CommandBufferException("HWUI replay: EndRecord without a recording")
        target.endRecording()
        recording = null
        canvas = null
    }

    override fun save() {
        canvas().save()
    }

    override fun restore() = canvas().restore()

    override fun concat(matrix: FloatArray) {
        this.matrix.setValues(matrix)
        canvas().concat(this.matrix)
    }

    override fun clipRect(left: Float, top: Float, right: Float, bottom: Float) {
        canvas().clipRect(left, top, right, bottom)
    }

    override fun clipPath(path: Int) {
        canvas().clipPath(paths[path])
    }

    override fun fill(shape: WireShape, paint: WirePaint) {
        applyPaint(paint)
        this.paint.style = Paint.Style.FILL
        drawShape(shape)
    }

    override fun stroke(shape: WireShape, paint: WirePaint, stroke: WireStroke) {
        applyPaint(paint)
        applyStroke(stroke)
        if (stroke.dashCount > 0) this.paint.pathEffect = dashEffect(stroke)
        drawShape(shape)
    }

    override fun shadow(shape: WireShape, color: Long, radius: Float, dx: Float, dy: Float, spread: Float) {
        resetPaint()
        paint.setColor(color)
        if (spread > 0f) {
            paint.style = Paint.Style.FILL_AND_STROKE
            paint.strokeWidth = 2f * spread
            paint.strokeJoin = Paint.Join.ROUND
        } else {
            paint.style = Paint.Style.FILL
        }
        if (radius > 0f) paint.maskFilter = blurFilter(radius)
        canvas().withTranslation(dx, dy) { drawShape(shape) }
    }

    override fun glyphs(
        font: Int,
        size: Float,
        paint: WirePaint,
        style: Int,
        stroke: WireStroke,
        count: Int,
        ids: IntArray,
        positions: FloatArray,
    ) {
        applyPaint(paint)
        if (style == GlyphStyle.STROKE) {
            applyStroke(stroke)
            if (stroke.dashCount > 0) this.paint.pathEffect = dashEffect(stroke)
        } else {
            this.paint.style = Paint.Style.FILL
        }
        // drawGlyphs takes the size from the paint and the face from the font.
        this.paint.textSize = size
        canvas().drawGlyphs(ids, 0, positions, 0, count, fonts[font], this.paint)
    }

    override fun text(layout: Int, matrix: FloatArray) {
        this.matrix.setValues(matrix)
        val target = canvas()
        target.withMatrix(this.matrix) { drawRenderNode(text.node(layout)) }
    }

    override fun image(bitmap: Int, left: Float, top: Float, right: Float, bottom: Float, sampling: Int) {
        resetPaint()
        paint.isFilterBitmap = sampling == SamplingKind.LINEAR
        rectF.set(left, top, right, bottom)
        canvas().drawBitmap(bitmaps[bitmap], null, rectF, paint)
    }

    override fun mesh(interpolation: Int, patches: Int, data: FloatArray) {
        if (Build.VERSION.SDK_INT < 34) {
            throw CommandBufferException("HWUI replay: a mesh needs API 34; the device runs ${Build.VERSION.SDK_INT}")
        }
        drawMesh(interpolation, patches, data)
    }

    /**
     * Each patch is four vertices that all carry its corners and colours, so
     * the fragment shader inverts the bilinear patch at every pixel, as
     * Cherenkov's oracle does, rather than interpolating over triangles.
     */
    @RequiresApi(34)
    private fun drawMesh(interpolation: Int, patches: Int, data: FloatArray) {
        if (patches == 0) return
        val specification = meshSpecification as MeshSpecification? ?: createMeshSpecification().also { meshSpecification = it }
        // A Mesh keeps its vertex buffer, so each mesh draw is a new resource.
        val vertexCount = 4 * patches
        val vertices = ByteBuffer.allocateDirect(vertexCount * MESH_STRIDE).order(ByteOrder.nativeOrder())
        var minX = Float.POSITIVE_INFINITY
        var minY = Float.POSITIVE_INFINITY
        var maxX = Float.NEGATIVE_INFINITY
        var maxY = Float.NEGATIVE_INFINITY
        for (patch in 0 until patches) {
            val base = patch * MeshInterpolation.PATCH_FLOATS
            for (corner in 0 until 4) {
                val x = data[base + 2 * corner]
                val y = data[base + 2 * corner + 1]
                minX = minOf(minX, x)
                minY = minOf(minY, y)
                maxX = maxOf(maxX, x)
                maxY = maxOf(maxY, y)
                vertices.putFloat(x).putFloat(y)
                for (k in 0 until MeshInterpolation.PATCH_FLOATS) vertices.putFloat(data[base + k])
            }
        }
        vertices.flip()
        val indices = meshIndices().duplicate()
        indices.limit(6 * patches)
        val mesh = Mesh(specification, Mesh.TRIANGLES, vertices, vertexCount, indices, RectF(minX, minY, maxX, maxY))
        mesh.setFloatUniform("smoothing", if (interpolation == MeshInterpolation.SMOOTHSTEP) 1f else 0f)
        resetPaint()
        paint.color = Color.WHITE
        paint.blendMode = BlendMode.SRC
        canvas().drawMesh(mesh, BlendMode.DST, paint)
    }

    /** Two triangles per patch over its corners 00, 10, 01, 11, shared by every band. */
    private fun meshIndices(): java.nio.ShortBuffer =
        meshIndices ?: ByteBuffer.allocateDirect(MeshInterpolation.BAND_PATCHES * 12).order(ByteOrder.nativeOrder()).asShortBuffer().also { buffer ->
            for (patch in 0 until MeshInterpolation.BAND_PATCHES) {
                val corner = 4 * patch
                for (offset in MESH_TRIANGLES) buffer.put((corner + offset).toShort())
            }
            buffer.flip()
            meshIndices = buffer
        }

    @RequiresApi(34)
    private fun createMeshSpecification(): MeshSpecification =
        MeshSpecification.make(
            arrayOf(
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT2, 0, "position"),
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT4, 8, "corners01"),
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT4, 24, "corners23"),
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT4, 40, "color0"),
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT4, 56, "color1"),
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT4, 72, "color2"),
                MeshSpecification.Attribute(MeshSpecification.TYPE_FLOAT4, 88, "color3"),
            ),
            MESH_STRIDE,
            arrayOf(
                MeshSpecification.Varying(MeshSpecification.TYPE_FLOAT4, "corners01"),
                MeshSpecification.Varying(MeshSpecification.TYPE_FLOAT4, "corners23"),
                MeshSpecification.Varying(MeshSpecification.TYPE_FLOAT4, "color0"),
                MeshSpecification.Varying(MeshSpecification.TYPE_FLOAT4, "color1"),
                MeshSpecification.Varying(MeshSpecification.TYPE_FLOAT4, "color2"),
                MeshSpecification.Varying(MeshSpecification.TYPE_FLOAT4, "color3"),
            ),
            MESH_VERTEX_SHADER,
            MESH_FRAGMENT_SHADER,
            ColorSpace.get(ColorSpace.Named.LINEAR_EXTENDED_SRGB),
            MeshSpecification.ALPHA_TYPE_PREMULTIPLIED,
        )

    override fun drawNode(node: Int) = canvas().drawRenderNode(nodes[node])

    override fun definePath(path: Int, fillType: Int, verbCount: Int, verbs: IntArray, pointCount: Int, points: FloatArray) {
        val built = Path()
        built.fillType = if (fillType == FillType.EVEN_ODD) Path.FillType.EVEN_ODD else Path.FillType.WINDING
        var at = 0
        for (index in 0 until verbCount) {
            when (verbs[index]) {
                Verb.MOVE -> built.moveTo(points[at], points[at + 1])
                Verb.LINE -> built.lineTo(points[at], points[at + 1])
                Verb.QUAD -> built.quadTo(points[at], points[at + 1], points[at + 2], points[at + 3])
                Verb.CUBIC -> built.cubicTo(points[at], points[at + 1], points[at + 2], points[at + 3], points[at + 4], points[at + 5])
                else -> built.close()
            }
            at += 2 * Verb.points(verbs[index])
        }
        paths.put(path, built)
    }

    override fun releasePath(path: Int) {
        paths.remove(path)
    }

    override fun defineLinearGradient(
        shader: Int, matrix: FloatArray, x0: Float, y0: Float, x1: Float, y1: Float, tile: Int,
        stops: Int, colors: LongArray, positions: FloatArray,
    ) = putShader(shader, matrix, LinearGradient(x0, y0, x1, y1, colors.copyOf(stops), positions.copyOf(stops), tileMode(tile)))

    override fun defineRadialGradient(
        shader: Int, matrix: FloatArray, x0: Float, y0: Float, r0: Float, x1: Float, y1: Float, r1: Float,
        tile: Int, stops: Int, colors: LongArray, positions: FloatArray,
    ) = putShader(shader, matrix, RadialGradient(x0, y0, r0, x1, y1, r1, colors.copyOf(stops), positions.copyOf(stops), tileMode(tile)))

    override fun defineSweepGradient(
        shader: Int, matrix: FloatArray, cx: Float, cy: Float, stops: Int, colors: LongArray, positions: FloatArray,
    ) = putShader(shader, matrix, SweepGradient(cx, cy, colors.copyOf(stops), positions.copyOf(stops)))

    override fun defineBitmapShader(shader: Int, matrix: FloatArray, bitmap: Int, tileX: Int, tileY: Int, sampling: Int) {
        val built = BitmapShader(bitmaps[bitmap], tileMode(tileX), tileMode(tileY))
        if (Build.VERSION.SDK_INT >= 33) {
            built.filterMode =
                if (sampling == SamplingKind.LINEAR) BitmapShader.FILTER_MODE_LINEAR else BitmapShader.FILTER_MODE_NEAREST
        } else {
            // Below API 33 the paint's filter flag samples the shader's bitmap.
            if (shader >= shaderSampling.size) {
                val grown = IntArray(maxOf(shaderSampling.size * 2, shader + 1)) { NO_SAMPLING }
                shaderSampling.copyInto(grown)
                shaderSampling = grown
            }
            shaderSampling[shader] = sampling
        }
        putShader(shader, matrix, built)
    }

    override fun defineRuntimeShader(shader: Int, matrix: FloatArray, runtimeShader: Int, uniformCount: Int, uniforms: FloatArray) {
        if (Build.VERSION.SDK_INT < 33) {
            throw CommandBufferException("HWUI replay: a runtime shader needs API 33; the device runs ${Build.VERSION.SDK_INT}")
        }
        val program = programs[runtimeShader]
        putShader(shader, matrix, runtimeShader(program, uniformCount, uniforms))
        runtimeOwners.put(shader, program)
    }

    @RequiresApi(33)
    private fun runtimeShader(program: RuntimeShaderProgram, uniformCount: Int, uniforms: FloatArray): Shader {
        if (uniformCount != program.floats) {
            throw CommandBufferException("HWUI replay: a runtime shader carries $uniformCount uniform floats; its program declares ${program.floats}")
        }
        val built = program.idle.removeLastOrNull() as RuntimeShader? ?: RuntimeShader(program.source)
        var at = 0
        for (index in program.uniformNames.indices) {
            val values = program.scratch[index]
            uniforms.copyInto(values, 0, at, at + values.size)
            built.setFloatUniform(program.uniformNames[index], values)
            at += values.size
        }
        return built
    }

    private fun putShader(id: Int, matrix: FloatArray, shader: Shader) {
        this.matrix.setValues(matrix)
        shader.setLocalMatrix(this.matrix)
        shaders.put(id, shader)
    }

    override fun releaseShader(shader: Int) {
        val released = shaders.remove(shader)
        if (shader < shaderSampling.size) shaderSampling[shader] = NO_SAMPLING
        if (runtimeOwners.contains(shader)) runtimeOwners.remove(shader).idle.add(released)
    }

    private fun shaderSamplingOf(shader: Int): Int = if (shader < shaderSampling.size) shaderSampling[shader] else NO_SAMPLING

    override fun releaseFont(font: Int) {
        fonts.remove(font)
    }

    override fun releaseBitmap(bitmap: Int) {
        bitmaps.remove(bitmap)
    }

    override fun releaseRuntimeShader(runtimeShader: Int) {
        programs.remove(runtimeShader)
    }

    override fun releaseEffect(effect: Int) {
        effects.remove(effect)
    }

    override fun releaseTextLayout(layout: Int) = text.release(layout)

    override fun deriveFont(font: Int, base: Int, axisCount: Int, tags: IntArray, values: FloatArray) {
        val settings = Array(axisCount) { FontVariationAxis(tagString(tags[it]), values[it]) }
        fonts.put(font, Font.Builder(fonts[base]).setFontVariationSettings(settings).build())
    }

    private fun tagString(tag: Int): String =
        String(charArrayOf((tag ushr 24).toChar(), (tag ushr 16 and 0xFF).toChar(), (tag ushr 8 and 0xFF).toChar(), (tag and 0xFF).toChar()))

    override fun hostOrder(count: Int, kinds: IntArray, ids: LongArray) {
        if (hostKinds.size < count) {
            hostKinds = IntArray(count * 2)
            hostIds = LongArray(count * 2)
        }
        kinds.copyInto(hostKinds, 0, 0, count)
        ids.copyInto(hostIds, 0, 0, count)
        hostCount = count
    }

    private fun resetPaint() {
        paint.reset()
        paint.isAntiAlias = true
    }

    private fun applyPaint(wire: WirePaint) {
        resetPaint()
        if (wire.kind == PaintKind.COLOR) {
            paint.setColor(wire.color)
            return
        }
        paint.shader = shaders[wire.shader]
        val sampling = shaderSamplingOf(wire.shader)
        // API 33+ samples through the shader's own filter mode (`shaderOf`).
        if (sampling != NO_SAMPLING && Build.VERSION.SDK_INT < 33) {
            paint.isFilterBitmap = sampling == SamplingKind.LINEAR
        }
        paint.setColor(Color.pack(0f, 0f, 0f, wire.alpha, extendedSrgb))
    }

    private fun applyStroke(stroke: WireStroke) {
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = stroke.width
        paint.strokeCap =
            when (stroke.cap) {
                CapKind.ROUND -> Paint.Cap.ROUND
                CapKind.SQUARE -> Paint.Cap.SQUARE
                else -> Paint.Cap.BUTT
            }
        paint.strokeJoin =
            when (stroke.join) {
                JoinKind.ROUND -> Paint.Join.ROUND
                JoinKind.BEVEL -> Paint.Join.BEVEL
                else -> Paint.Join.MITER
            }
        paint.strokeMiter = stroke.miter
    }

    private fun drawShape(shape: WireShape) {
        val target = canvas()
        when (shape.kind) {
            ShapeKind.RECT -> target.drawRect(shape.left, shape.top, shape.right, shape.bottom, paint)
            ShapeKind.ROUND_RECT -> target.drawRoundRect(shape.left, shape.top, shape.right, shape.bottom, shape.rx, shape.ry, paint)
            ShapeKind.OVAL -> target.drawOval(shape.left, shape.top, shape.right, shape.bottom, paint)
            ShapeKind.PATH -> target.drawPath(paths[shape.path], paint)
            else -> target.drawLine(shape.left, shape.top, shape.right, shape.bottom, paint)
        }
    }

    private fun dashEffect(stroke: WireStroke): DashPathEffect {
        val cached = dash
        if (cached != null && dashPhase == stroke.phase && dashIntervals.size == stroke.dashCount &&
            (0 until stroke.dashCount).all { dashIntervals[it] == stroke.intervals[it] }
        ) {
            return cached
        }
        dashIntervals = stroke.intervals.copyOf(stroke.dashCount)
        dashPhase = stroke.phase
        return DashPathEffect(dashIntervals, stroke.phase).also { dash = it }
    }

    private fun blurFilter(radius: Float): BlurMaskFilter {
        val cached = blur
        if (cached != null && blurRadius == radius) return cached
        blurRadius = radius
        return BlurMaskFilter(radius, BlurMaskFilter.Blur.NORMAL).also { blur = it }
    }

    private companion object {
        const val NO_SAMPLING = -1
        /** Position, the patch's four corners and its four colours. */
        const val MESH_STRIDE = 104
        val MESH_TRIANGLES = intArrayOf(0, 1, 3, 0, 3, 2)
        const val MESH_VERTEX_SHADER =
            "Varyings main(const Attributes attributes) {\n" +
                "  Varyings varyings;\n" +
                "  varyings.position = attributes.position;\n" +
                "  varyings.corners01 = attributes.corners01;\n" +
                "  varyings.corners23 = attributes.corners23;\n" +
                "  varyings.color0 = attributes.color0;\n" +
                "  varyings.color1 = attributes.color1;\n" +
                "  varyings.color2 = attributes.color2;\n" +
                "  varyings.color3 = attributes.color3;\n" +
                "  return varyings;\n" +
                "}\n"

        /**
         * Inverts the bilinear patch as Cherenkov's CPU oracle does
         * (`cpu/src/render/mesh.rs`): of the two roots in range it takes
         * the greatest v, then the greatest u; rounding at the quad's edge
         * clamps into the patch.
         */
        const val MESH_FRAGMENT_SHADER =
            "uniform float smoothing;\n" +
                "float cross2(float2 a, float2 b) { return a.x * b.y - a.y * b.x; }\n" +
                "float uAt(float2 e, float2 f, float2 g, float2 q, float v) {\n" +
                "  float2 direction = e + g * v;\n" +
                "  float2 residual = q - f * v;\n" +
                "  return abs(direction.x) >= abs(direction.y) ? residual.x / direction.x : residual.y / direction.y;\n" +
                "}\n" +
                "float2 patchAt(float2 origin, float2 right, float2 bottom, float2 opposite, float2 point) {\n" +
                "  float2 e = right - origin;\n" +
                "  float2 f = bottom - origin;\n" +
                "  float2 g = (opposite - bottom) - e;\n" +
                "  float2 q = point - origin;\n" +
                "  float a = -cross2(f, g);\n" +
                "  float b = cross2(q, g) - cross2(f, e);\n" +
                "  float c = cross2(q, e);\n" +
                "  float2 roots = float2(0.0);\n" +
                "  if (a == 0.0) {\n" +
                "    roots = float2(b == 0.0 ? 0.0 : -c / b);\n" +
                "  } else {\n" +
                "    float t = -0.5 * (b + (b >= 0.0 ? 1.0 : -1.0) * sqrt(max(b * b - 4.0 * a * c, 0.0)));\n" +
                "    roots = t == 0.0 ? float2(-b / (2.0 * a)) : float2(t / a, c / t);\n" +
                "  }\n" +
                "  float2 best = float2(-1.0);\n" +
                "  for (int i = 0; i < 2; i++) {\n" +
                "    float v = i == 0 ? roots.x : roots.y;\n" +
                "    if (v >= 0.0 && v <= 1.0) {\n" +
                "      float u = uAt(e, f, g, q, v);\n" +
                "      if (u >= 0.0 && u <= 1.0 && (v > best.y || (v == best.y && u > best.x))) best = float2(u, v);\n" +
                "    }\n" +
                "  }\n" +
                "  if (best.y < 0.0) {\n" +
                "    float v = clamp(roots.y, 0.0, 1.0);\n" +
                "    best = float2(clamp(uAt(e, f, g, q, v), 0.0, 1.0), v);\n" +
                "  }\n" +
                "  return best;\n" +
                "}\n" +
                "float2 main(const Varyings varyings, out float4 color) {\n" +
                "  float2 uv = patchAt(varyings.corners01.xy, varyings.corners01.zw, varyings.corners23.xy, varyings.corners23.zw, varyings.position);\n" +
                "  uv = mix(uv, uv * uv * (3.0 - 2.0 * uv), smoothing);\n" +
                "  color = mix(mix(varyings.color0, varyings.color1, uv.x), mix(varyings.color2, varyings.color3, uv.x), uv.y);\n" +
                "  return varyings.position;\n" +
                "}\n"

        fun tileMode(tile: Int): Shader.TileMode =
            when (tile) {
                Tile.REPEAT -> Shader.TileMode.REPEAT
                Tile.MIRROR -> Shader.TileMode.MIRROR
                Tile.DECAL -> Shader.TileMode.DECAL
                else -> Shader.TileMode.CLAMP
            }
    }
}
