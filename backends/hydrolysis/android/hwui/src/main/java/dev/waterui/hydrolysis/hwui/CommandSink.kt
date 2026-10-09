package dev.waterui.hydrolysis.hwui

/**
 * The events [CommandDecoder] reads out of one frame. Operand holders and
 * arrays are the decoder's reused scratch: they are valid only for the
 * duration of the call, and an array may be longer than its count.
 */
interface CommandSink {
    fun beginFrame(sequence: Long)

    fun endFrame()

    fun createNode(node: Int)

    fun releaseNode(node: Int)

    fun setPosition(node: Int, left: Int, top: Int, right: Int, bottom: Int, clipToBounds: Boolean)

    /**
     * `values`: translation x/y, scale x/y, rotation z, pivot x/y, rotation
     * x/y and camera distance; angles in degrees.
     */
    fun setTransform(node: Int, values: FloatArray)

    fun setAlpha(node: Int, alpha: Float)

    fun setClip(node: Int, kind: Int, left: Int, top: Int, right: Int, bottom: Int, radius: Float)

    fun setComposite(node: Int, blend: Int, forceLayer: Boolean)

    /** `effect` is [Protocol.NONE] to clear the node's effect. */
    fun setEffect(node: Int, effect: Int)

    fun record(node: Int, width: Int, height: Int)

    fun endRecord()

    fun save()

    fun restore()

    /** `matrix` in `Matrix.getValues` order. */
    fun concat(matrix: FloatArray)

    fun clipRect(left: Float, top: Float, right: Float, bottom: Float)

    fun clipPath(path: Int)

    fun fill(shape: WireShape, paint: WirePaint)

    fun stroke(shape: WireShape, paint: WirePaint, stroke: WireStroke)

    /**
     * A positive [spread] also strokes [shape] `2 × spread` wide with round
     * joins (`FILL_AND_STROKE`); a primitive arrives already grown.
     */
    fun shadow(shape: WireShape, color: Long, radius: Float, dx: Float, dy: Float, spread: Float)

    /** `style` is a [GlyphStyle]; `stroke` applies to [GlyphStyle.STROKE]. */
    fun glyphs(
        font: Int,
        size: Float,
        paint: WirePaint,
        style: Int,
        stroke: WireStroke,
        count: Int,
        ids: IntArray,
        positions: FloatArray,
    )

    fun text(layout: Int, matrix: FloatArray)

    fun image(bitmap: Int, left: Float, top: Float, right: Float, bottom: Float, sampling: Int)

    /**
     * `data` holds [MeshInterpolation.PATCH_FLOATS] floats per patch: its
     * corners 00, 10, 01, 11 as `x, y`, then their premultiplied linear
     * extended-sRGB colours. Patches draw with `BlendMode.SRC`, a later
     * one replacing an earlier one.
     */
    fun mesh(interpolation: Int, patches: Int, data: FloatArray)

    fun drawNode(node: Int)

    fun definePath(
        path: Int,
        fillType: Int,
        verbCount: Int,
        verbs: IntArray,
        pointCount: Int,
        points: FloatArray,
    )

    fun releasePath(path: Int)

    fun defineLinearGradient(
        shader: Int,
        matrix: FloatArray,
        x0: Float,
        y0: Float,
        x1: Float,
        y1: Float,
        tile: Int,
        stops: Int,
        colors: LongArray,
        positions: FloatArray,
    )

    fun defineRadialGradient(
        shader: Int,
        matrix: FloatArray,
        x0: Float,
        y0: Float,
        r0: Float,
        x1: Float,
        y1: Float,
        r1: Float,
        tile: Int,
        stops: Int,
        colors: LongArray,
        positions: FloatArray,
    )

    fun defineSweepGradient(
        shader: Int,
        matrix: FloatArray,
        cx: Float,
        cy: Float,
        stops: Int,
        colors: LongArray,
        positions: FloatArray,
    )

    fun defineBitmapShader(
        shader: Int,
        matrix: FloatArray,
        bitmap: Int,
        tileX: Int,
        tileY: Int,
        sampling: Int,
    )

    fun defineRuntimeShader(
        shader: Int,
        matrix: FloatArray,
        runtimeShader: Int,
        uniformCount: Int,
        uniforms: FloatArray,
    )

    fun releaseShader(shader: Int)

    fun releaseFont(font: Int)

    fun releaseBitmap(bitmap: Int)

    fun releaseRuntimeShader(runtimeShader: Int)

    fun releaseEffect(effect: Int)

    fun releaseTextLayout(layout: Int)

    /**
     * Defines font `font` as the variation instance of `base` at `values`
     * on the `axisCount` axes `tags` (OpenType tags, big-endian).
     */
    fun deriveFont(font: Int, base: Int, axisCount: Int, tags: IntArray, values: FloatArray)

    /** The session's top-level draw order, bottom first: `kinds` are [HostKind]s. */
    fun hostOrder(count: Int, kinds: IntArray, ids: LongArray)
}

/**
 * An inline shape. [ShapeKind.LINE] carries its end points in
 * `left, top` and `right, bottom`; [ShapeKind.PATH] names [path].
 */
class WireShape {
    var kind: Int = ShapeKind.RECT
    var left: Float = 0f
    var top: Float = 0f
    var right: Float = 0f
    var bottom: Float = 0f
    var rx: Float = 0f
    var ry: Float = 0f
    var path: Int = Protocol.NONE
}

/** An inline paint: a `ColorLong` or a shader, with an alpha multiplier. */
class WirePaint {
    var kind: Int = PaintKind.COLOR
    var color: Long = 0L
    var shader: Int = Protocol.NONE
    var alpha: Float = 1f
}

/** A stroke style; [intervals] holds [dashCount] dash intervals. */
class WireStroke {
    var width: Float = 0f
    var cap: Int = CapKind.BUTT
    var join: Int = JoinKind.MITER
    var miter: Float = 4f
    var phase: Float = 0f
    var dashCount: Int = 0
    var intervals: FloatArray = FloatArray(8)
}
