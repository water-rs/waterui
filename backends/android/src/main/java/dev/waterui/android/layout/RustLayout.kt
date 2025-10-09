package dev.waterui.android.layout

import androidx.compose.runtime.Composable
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.MeasurePolicy
import androidx.compose.ui.layout.MeasureScope
import androidx.compose.ui.layout.Placeable
import androidx.compose.ui.unit.Constraints
import dev.waterui.android.runtime.ChildMetadataStruct
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.ProposalStruct
import dev.waterui.android.runtime.RectStruct
import dev.waterui.android.runtime.WuiEnvironment
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId
import kotlin.math.roundToInt

/**
 * Compose layout that mirrors the Swift `RustLayout` pass-through. All measurement/placement logic is
 * delegated to the Rust layout engine via JNI.
 */
@Composable
fun RustLayout(
    layoutPtr: Long,
    descriptors: List<ChildDescriptor>,
    environment: WuiEnvironment,
    content: @Composable () -> Unit
) {
    Layout(
        content = content,
        measurePolicy = layoutMeasurePolicy(layoutPtr, descriptors, environment)
    )
}

private fun layoutMeasurePolicy(
    layoutPtr: Long,
    descriptors: List<ChildDescriptor>,
    @Suppress("UNUSED_PARAMETER") environment: WuiEnvironment
): MeasurePolicy = MeasurePolicy { measurables, constraints ->
    if (layoutPtr == 0L || measurables.isEmpty()) {
        fallbackMeasure(measurables, constraints)
    } else {
        val parentProposal = constraints.toProposalStruct()

        val initialMetadata = measurables.indices.map { index ->
            ChildMetadataStruct(
                proposal = UnspecifiedProposal,
                priority = descriptors.getPriority(index),
                stretch = descriptors.getOrNull(index)?.isSpacer == true
            )
        }.toTypedArray()

        val childProposals = NativeBindings.waterui_layout_propose(layoutPtr, parentProposal, initialMetadata)

        val placeables = Array(measurables.size) { index ->
            val proposal = childProposals.getOrNull(index) ?: UnspecifiedProposal
            val childConstraints = proposal.toConstraints(constraints)
            measurables[index].measure(childConstraints)
        }

        val finalMetadata = Array(measurables.size) { index ->
            val isSpacer = descriptors.getOrNull(index)?.isSpacer == true
            ChildMetadataStruct(
                proposal = placeables[index].toProposalStruct(isSpacer),
                priority = descriptors.getPriority(index),
                stretch = isSpacer
            )
        }

        val requestedSize = NativeBindings.waterui_layout_size(layoutPtr, parentProposal, finalMetadata)
        val layoutWidth = requestedSize.width.resolveDimension(constraints.minWidth, constraints.maxWidth)
        val layoutHeight = requestedSize.height.resolveDimension(constraints.minHeight, constraints.maxHeight)

        val rects = NativeBindings.waterui_layout_place(
            layoutPtr,
            RectStruct(originX = 0f, originY = 0f, width = layoutWidth.toFloat(), height = layoutHeight.toFloat()),
            parentProposal,
            finalMetadata
        )

        layout(layoutWidth, layoutHeight) {
            rects.forEachIndexed { index, rect ->
                if (index < placeables.size) {
                    val placeable = placeables[index]
                    placeable.place(
                        x = rect.originX.roundToInt(),
                        y = rect.originY.roundToInt()
                    )
                }
            }
        }
    }
}

private fun MeasureScope.fallbackMeasure(measurables: List<androidx.compose.ui.layout.Measurable>, constraints: Constraints): androidx.compose.ui.layout.MeasureResult {
    val placeables = measurables.map { it.measure(constraints) }
    val width = placeables.maxOfOrNull { it.width } ?: constraints.minWidth
    val height = placeables.sumOf { it.height }.coerceIn(constraints.minHeight, constraints.maxHeight.takeUnless { it == Constraints.Infinity } ?: Int.MAX_VALUE)
    return layout(width, height) {
        var y = 0
        placeables.forEach { placeable ->
            placeable.placeRelative(0, y)
            y += placeable.height
        }
    }
}

data class ChildDescriptor(
    val typeId: WuiTypeId,
    val isSpacer: Boolean
)

private val spacerTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_spacer_id().toTypeId()
}

private fun List<ChildDescriptor>.getPriority(@Suppress("UNUSED_PARAMETER") index: Int): Int = 0 // TODO: surface layout priorities when available.

fun List<Long>.toChildDescriptors(): List<ChildDescriptor> {
    if (isEmpty()) return emptyList()
    return map { pointer ->
        val typeId = NativeBindings.waterui_view_id(pointer).toTypeId()
        ChildDescriptor(typeId = typeId, isSpacer = typeId == spacerTypeId)
    }
}

private fun Constraints.toProposalStruct(): ProposalStruct = ProposalStruct(
    width = if (maxWidth != Constraints.Infinity) maxWidth.toFloat() else Float.NaN,
    height = if (maxHeight != Constraints.Infinity) maxHeight.toFloat() else Float.NaN
)

private fun ProposalStruct.toConstraints(parent: Constraints): Constraints {
    val widthSpecified = !width.isNaN()
    val heightSpecified = !height.isNaN()

    val resolvedMaxWidth = if (widthSpecified) width.roundToInt().coerceAtLeast(0) else parent.maxWidth
    val resolvedMinWidth = if (widthSpecified) resolvedMaxWidth else 0

    val resolvedMaxHeight = if (heightSpecified) height.roundToInt().coerceAtLeast(0) else parent.maxHeight
    val resolvedMinHeight = if (heightSpecified) resolvedMaxHeight else 0

    return Constraints(
        minWidth = resolvedMinWidth.coerceIn(0, resolvedMaxWidth.coerceAtLeast(resolvedMinWidth)),
        maxWidth = resolvedMaxWidth.coerceAtLeast(resolvedMinWidth),
        minHeight = resolvedMinHeight.coerceIn(0, resolvedMaxHeight.coerceAtLeast(resolvedMinHeight)),
        maxHeight = resolvedMaxHeight.coerceAtLeast(resolvedMinHeight)
    )
}

private fun Placeable.toProposalStruct(isSpacer: Boolean): ProposalStruct =
    if (isSpacer) UnspecifiedProposal else ProposalStruct(width.toFloat(), height.toFloat())

private fun Float.resolveDimension(min: Int, max: Int): Int {
    if (isNaN()) {
        return max.takeUnless { it == Constraints.Infinity } ?: min
    }
    val rounded = roundToInt().coerceAtLeast(0)
    if (max == Constraints.Infinity) return rounded.coerceAtLeast(min)
    return rounded.coerceIn(min, max)
}

private val UnspecifiedProposal = ProposalStruct(Float.NaN, Float.NaN)
