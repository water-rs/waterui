package dev.waterui.hydrolysis.hwui

/** Writes the event log the Rust `CommandBuffer` writes for the same frame. */
class LoggingSink : CommandSink {
    val lines = mutableListOf<String>()
    private val line = StringBuilder()

    private fun start(op: Int): LoggingSink {
        line.setLength(0)
        line.append(Protocol.name(op))
        return this
    }

    private fun end() {
        lines += line.toString()
    }

    private fun u(name: String, value: Int) = apply { line.append(' ').append(name).append('=').append(Integer.toUnsignedString(value)) }

    private fun i(name: String, value: Int) = apply { line.append(' ').append(name).append('=').append(value) }

    private fun f(name: String, value: Float) = apply { line.append(' ').append(name).append('=').append(bits(value)) }

    private fun c(name: String, value: Long) = apply { line.append(' ').append(name).append('=').append(String.format("%016x", value)) }

    private fun flag(name: String, value: Boolean) = u(name, if (value) 1 else 0)

    private fun us(name: String, count: Int, values: IntArray) = list(name, count) { Integer.toUnsignedString(values[it]) }

    private fun fs(name: String, count: Int, values: FloatArray) = list(name, count) { bits(values[it]) }

    private fun cs(name: String, count: Int, values: LongArray) = list(name, count) { String.format("%016x", values[it]) }

    private fun list(name: String, count: Int, each: (Int) -> String) = apply {
        line.append(' ').append(name).append("=[")
        for (index in 0 until count) {
            if (index > 0) line.append(',')
            line.append(each(index))
        }
        line.append(']')
    }

    private fun bits(value: Float) = String.format("%08x", java.lang.Float.floatToRawIntBits(value))

    private fun shape(shape: WireShape) = apply {
        u("shape", shape.kind)
        when (shape.kind) {
            ShapeKind.PATH -> u("path", shape.path)
            ShapeKind.LINE -> f("x0", shape.left).f("y0", shape.top).f("x1", shape.right).f("y1", shape.bottom)
            else -> {
                f("left", shape.left).f("top", shape.top).f("right", shape.right).f("bottom", shape.bottom)
                if (shape.kind == ShapeKind.ROUND_RECT) f("rx", shape.rx).f("ry", shape.ry)
            }
        }
    }

    private fun paint(paint: WirePaint) = apply {
        u("paint", paint.kind)
        if (paint.kind == PaintKind.COLOR) c("color", paint.color) else u("shader", paint.shader)
        f("alpha", paint.alpha)
    }

    private fun strokeStyle(stroke: WireStroke) =
        f("width", stroke.width).u("cap", stroke.cap).u("join", stroke.join).f("miter", stroke.miter).f("phase", stroke.phase).u("dashes", stroke.dashCount)

    override fun beginFrame(sequence: Long) {
        lines += "Frame sequence=$sequence"
    }

    override fun endFrame() = Unit

    override fun createNode(node: Int) = start(Protocol.CREATE_NODE).u("node", node).end()

    override fun releaseNode(node: Int) = start(Protocol.RELEASE_NODE).u("node", node).end()

    override fun setPosition(node: Int, left: Int, top: Int, right: Int, bottom: Int, clipToBounds: Boolean) =
        start(Protocol.SET_POSITION).u("node", node).i("left", left).i("top", top).i("right", right).i("bottom", bottom).flag("clip", clipToBounds).end()

    override fun setTransform(node: Int, values: FloatArray) {
        start(Protocol.SET_TRANSFORM).u("node", node)
        TRANSFORM_NAMES.forEachIndexed { index, name -> f(name, values[index]) }
        end()
    }

    override fun setAlpha(node: Int, alpha: Float) = start(Protocol.SET_ALPHA).u("node", node).f("alpha", alpha).end()

    override fun setClip(node: Int, kind: Int, left: Int, top: Int, right: Int, bottom: Int, radius: Float) =
        start(Protocol.SET_CLIP).u("node", node).u("kind", kind).i("left", left).i("top", top).i("right", right).i("bottom", bottom).f("radius", radius).end()

    override fun setComposite(node: Int, blend: Int, forceLayer: Boolean) =
        start(Protocol.SET_COMPOSITE).u("node", node).u("blend", blend).flag("layer", forceLayer).end()

    override fun setEffect(node: Int, effect: Int) = start(Protocol.SET_EFFECT).u("node", node).u("effect", effect).end()

    override fun record(node: Int, width: Int, height: Int) =
        start(Protocol.RECORD).u("node", node).i("width", width).i("height", height).end()

    override fun endRecord() = start(Protocol.END_RECORD).end()

    override fun save() = start(Protocol.SAVE).end()

    override fun restore() = start(Protocol.RESTORE).end()

    override fun concat(matrix: FloatArray) = start(Protocol.CONCAT).fs("matrix", Protocol.MATRIX_WORDS, matrix).end()

    override fun clipRect(left: Float, top: Float, right: Float, bottom: Float) =
        start(Protocol.CLIP_RECT).f("left", left).f("top", top).f("right", right).f("bottom", bottom).end()

    override fun clipPath(path: Int) = start(Protocol.CLIP_PATH).u("path", path).end()

    override fun fill(shape: WireShape, paint: WirePaint) = start(Protocol.FILL).shape(shape).paint(paint).end()

    override fun stroke(shape: WireShape, paint: WirePaint, stroke: WireStroke) =
        start(Protocol.STROKE).shape(shape).paint(paint).strokeStyle(stroke).fs("intervals", stroke.dashCount, stroke.intervals).end()

    override fun shadow(shape: WireShape, color: Long, radius: Float, dx: Float, dy: Float, spread: Float) =
        start(Protocol.SHADOW).shape(shape).c("color", color).f("radius", radius).f("dx", dx).f("dy", dy)
            .f("spread", spread).end()

    override fun glyphs(
        font: Int,
        size: Float,
        paint: WirePaint,
        style: Int,
        stroke: WireStroke,
        count: Int,
        ids: IntArray,
        positions: FloatArray,
    ) = start(Protocol.GLYPHS).u("font", font).f("size", size).paint(paint).u("style", style).strokeStyle(stroke)
        .fs("intervals", stroke.dashCount, stroke.intervals).u("count", count).us("ids", count, ids).fs("xy", 2 * count, positions).end()

    override fun text(layout: Int, matrix: FloatArray) =
        start(Protocol.TEXT).u("layout", layout).fs("matrix", Protocol.MATRIX_WORDS, matrix).end()

    override fun image(bitmap: Int, left: Float, top: Float, right: Float, bottom: Float, sampling: Int) =
        start(Protocol.IMAGE).u("bitmap", bitmap).f("left", left).f("top", top).f("right", right).f("bottom", bottom).u("sampling", sampling).end()

    override fun mesh(interpolation: Int, patches: Int, data: FloatArray) =
        start(Protocol.MESH).u("interpolation", interpolation).u("patches", patches)
            .fs("patch", patches * MeshInterpolation.PATCH_FLOATS, data).end()

    override fun drawNode(node: Int) = start(Protocol.DRAW_NODE).u("node", node).end()

    override fun definePath(path: Int, fillType: Int, verbCount: Int, verbs: IntArray, pointCount: Int, points: FloatArray) =
        start(Protocol.DEFINE_PATH).u("path", path).u("fill", fillType).u("verbs", verbCount).u("points", pointCount)
            .us("verb", verbCount, verbs).fs("xy", 2 * pointCount, points).end()

    override fun releasePath(path: Int) = start(Protocol.RELEASE_PATH).u("path", path).end()

    private fun shaderHead(shader: Int, kind: Int, matrix: FloatArray) =
        start(Protocol.DEFINE_SHADER).u("shader", shader).u("kind", kind).fs("matrix", Protocol.MATRIX_WORDS, matrix)

    private fun stops(stops: Int, colors: LongArray, positions: FloatArray) =
        u("stops", stops).cs("colors", stops, colors).fs("positions", stops, positions)

    override fun defineLinearGradient(
        shader: Int, matrix: FloatArray, x0: Float, y0: Float, x1: Float, y1: Float, tile: Int,
        stops: Int, colors: LongArray, positions: FloatArray,
    ) = shaderHead(shader, ShaderKind.LINEAR, matrix).f("x0", x0).f("y0", y0).f("x1", x1).f("y1", y1).u("tile", tile)
        .stops(stops, colors, positions).end()

    override fun defineRadialGradient(
        shader: Int, matrix: FloatArray, x0: Float, y0: Float, r0: Float, x1: Float, y1: Float, r1: Float,
        tile: Int, stops: Int, colors: LongArray, positions: FloatArray,
    ) = shaderHead(shader, ShaderKind.RADIAL, matrix).f("x0", x0).f("y0", y0).f("r0", r0).f("x1", x1).f("y1", y1).f("r1", r1)
        .u("tile", tile).stops(stops, colors, positions).end()

    override fun defineSweepGradient(
        shader: Int, matrix: FloatArray, cx: Float, cy: Float, stops: Int, colors: LongArray, positions: FloatArray,
    ) = shaderHead(shader, ShaderKind.SWEEP, matrix).f("cx", cx).f("cy", cy).stops(stops, colors, positions).end()

    override fun defineBitmapShader(shader: Int, matrix: FloatArray, bitmap: Int, tileX: Int, tileY: Int, sampling: Int) =
        shaderHead(shader, ShaderKind.BITMAP, matrix).u("bitmap", bitmap).u("tile_x", tileX).u("tile_y", tileY).u("sampling", sampling).end()

    override fun defineRuntimeShader(shader: Int, matrix: FloatArray, runtimeShader: Int, uniformCount: Int, uniforms: FloatArray) =
        shaderHead(shader, ShaderKind.RUNTIME, matrix).u("runtime", runtimeShader).u("uniforms", uniformCount)
            .fs("values", uniformCount, uniforms).end()

    override fun releaseShader(shader: Int) = start(Protocol.RELEASE_SHADER).u("shader", shader).end()

    override fun releaseFont(font: Int) = start(Protocol.RELEASE_FONT).u("id", font).end()

    override fun releaseBitmap(bitmap: Int) = start(Protocol.RELEASE_BITMAP).u("id", bitmap).end()

    override fun releaseRuntimeShader(runtimeShader: Int) = start(Protocol.RELEASE_RUNTIME_SHADER).u("id", runtimeShader).end()

    override fun releaseEffect(effect: Int) = start(Protocol.RELEASE_EFFECT).u("id", effect).end()

    override fun releaseTextLayout(layout: Int) = start(Protocol.RELEASE_TEXT_LAYOUT).u("id", layout).end()

    override fun deriveFont(font: Int, base: Int, axisCount: Int, tags: IntArray, values: FloatArray) =
        start(Protocol.DERIVE_FONT).u("font", font).u("base", base).u("axes", axisCount)
            .us("tags", axisCount, tags).fs("values", axisCount, values).end()

    override fun hostOrder(count: Int, kinds: IntArray, ids: LongArray) {
        start(Protocol.HOST_ORDER).u("count", count)
        list("entries", 3 * count) {
            val entry = it / 3
            when (it % 3) {
                0 -> Integer.toUnsignedString(kinds[entry])
                1 -> (ids[entry] and 0xFFFF_FFFFL).toString()
                else -> (ids[entry] ushr 32).toString()
            }
        }
        end()
    }

    private companion object {
        val TRANSFORM_NAMES = listOf(
            "translation_x", "translation_y", "scale_x", "scale_y", "rotation",
            "pivot_x", "pivot_y", "rotation_x", "rotation_y", "camera_distance",
        )
    }
}
