package dev.waterui.hydrolysis.hwui

import android.graphics.RenderEffect
import android.graphics.RenderNode
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.annotation.RealObject
import org.robolectric.shadow.api.Shadow
import org.robolectric.shadows.ShadowNativeRenderNode

/**
 * Records every `setRenderEffect` argument on each node's own shadow, so the
 * sink's rebinding is proven against the real call surface — not bookkeeping.
 */
@Implements(RenderNode::class)
class RecordingRenderNodeShadow : ShadowNativeRenderNode() {
    @RealObject private lateinit var realNode: RenderNode

    /** Every argument `setRenderEffect` has received, in order. */
    val calls = mutableListOf<RenderEffect?>()

    @Implementation
    fun setRenderEffect(effect: RenderEffect?): Boolean {
        calls.add(effect)
        return Shadow.directlyOn(
            realNode,
            RenderNode::class.java,
            "setRenderEffect",
            org.robolectric.util.ReflectionHelpers.ClassParameter.from(
                RenderEffect::class.java,
                effect,
            ),
        )
    }
}

private fun calls(node: RenderNode): List<RenderEffect?> =
    Shadow.extract<RecordingRenderNodeShadow>(node).calls

private fun last(node: RenderNode): RenderEffect? = calls(node).last()

/**
 * A `DefineEffect` rebinding must reach every bound node in the same replay;
 * the shadow above is the only way to observe the native calls.
 */
@RunWith(RobolectricTestRunner::class)
@Config(shadows = [RecordingRenderNodeShadow::class], sdk = [34])
class RenderNodeSinkEffectTest {
    private lateinit var sink: RenderNodeSink
    private lateinit var nodes: List<RenderNode>

    @Before
    fun setUp() {
        sink = RenderNodeSink(HwuiTextProvider())
        nodes = (1..2).map {
            sink.createNode(it)
            sink.node(it)
        }
    }

    /** A one-stage blur definition; reused for every define below. */
    private fun define(effect: Int, radius: Float) =
        sink.defineEffect(
            effect,
            1,
            intArrayOf(EffectKind.BLUR),
            FloatArray(12) { if (it == 0) radius else 0f },
        )

    @Test
    fun two_bound_nodes_receive_the_redefinition_together() {
        define(7, 8f)
        sink.setEffect(1, 7)
        sink.setEffect(2, 7)
        val first = last(nodes[0])
        assertSame(first, last(nodes[1]))
        define(7, 12f)
        val second = last(nodes[0])
        assertTrue(second !== first)
        assertSame(second, last(nodes[1]))
        // One bound node releases its node; the other still updates.
        sink.releaseNode(1)
        define(7, 3f)
        assertTrue(
            "the released node received no call",
            calls(nodes[0]).size == 2,
        )
        val third = last(nodes[1])
        assertTrue(third !== second)
    }

    @Test
    fun a_clear_then_release_lifecycle_is_ordered() {
        define(7, 8f)
        sink.setEffect(1, 7)
        // Releasing a still-bound effect is a replay error.
        assertThrows(CommandBufferException::class.java) {
            sink.releaseEffect(7)
        }
        sink.setEffect(1, Protocol.NONE)
        assertTrue(last(nodes[0]) == null)
        sink.releaseEffect(7)
        // And a fresh definition can reuse the slot afterwards.
        define(7, 4f)
    }
}
