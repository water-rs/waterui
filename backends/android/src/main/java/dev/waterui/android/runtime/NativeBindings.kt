package dev.waterui.android.runtime

/**
 * Centralised access to native WaterUI FFI functions. Actual JNI bindings live in the
 * native Rust/C layer; this object exposes type-safe Kotlin entry points.
 */
internal object NativeBindings {
    external fun waterui_init(): Long
    external fun waterui_main(envPtr: Long): Long
    external fun waterui_view_id(anyViewPtr: Long): LongArray
    external fun waterui_view_body(anyViewPtr: Long, envPtr: Long): Long

    // Type identifiers
    external fun waterui_empty_id(): LongArray
    external fun waterui_text_id(): LongArray
    external fun waterui_label_id(): LongArray
    external fun waterui_button_id(): LongArray
    external fun waterui_color_id(): LongArray
    external fun waterui_text_field_id(): LongArray
    external fun waterui_stepper_id(): LongArray
    external fun waterui_progress_id(): LongArray
    external fun waterui_dynamic_id(): LongArray
    external fun waterui_scroll_view_id(): LongArray
    external fun waterui_spacer_id(): LongArray
    external fun waterui_toggle_id(): LongArray
    external fun waterui_slider_id(): LongArray
    external fun waterui_renderer_view_id(): LongArray

    external fun waterui_env_clone(envPtr: Long): Long
    external fun waterui_env_drop(envPtr: Long)

    external fun waterui_anyview_drop(viewPtr: Long)

    // Layout bridging
    external fun waterui_container_id(): LongArray
    external fun waterui_force_as_container(viewPtr: Long): ContainerStruct
    external fun waterui_layout_propose(layoutPtr: Long, parent: ProposalStruct, children: Array<ChildMetadataStruct>): Array<ProposalStruct>
    external fun waterui_layout_size(layoutPtr: Long, parent: ProposalStruct, children: Array<ChildMetadataStruct>): SizeStruct
    external fun waterui_layout_place(layoutPtr: Long, bounds: RectStruct, proposal: ProposalStruct, children: Array<ChildMetadataStruct>): Array<RectStruct>

    external fun waterui_any_views_len(handle: Long): Int
    external fun waterui_any_views_get_view(handle: Long, index: Int): Long
    external fun waterui_any_views_get_id(handle: Long, index: Int): Int
    external fun waterui_drop_any_views(handle: Long)

    // Reactive bindings (skeleton only; full surface to be added incrementally)
    external fun waterui_watch_binding_bool(bindingPtr: Long, watcher: WatcherStruct): Long
    external fun waterui_watch_binding_int(bindingPtr: Long, watcher: WatcherStruct): Long
    external fun waterui_watch_binding_double(bindingPtr: Long, watcher: WatcherStruct): Long
    external fun waterui_watch_binding_str(bindingPtr: Long, watcher: WatcherStruct): Long

    external fun waterui_set_binding_bool(bindingPtr: Long, value: Boolean)
    external fun waterui_set_binding_int(bindingPtr: Long, value: Int)
    external fun waterui_set_binding_double(bindingPtr: Long, value: Double)
    external fun waterui_set_binding_str(bindingPtr: Long, bytes: ByteArray)

    external fun waterui_read_binding_bool(bindingPtr: Long): Boolean
    external fun waterui_read_binding_int(bindingPtr: Long): Int
    external fun waterui_read_binding_double(bindingPtr: Long): Double
    external fun waterui_read_binding_str(bindingPtr: Long): ByteArray

    external fun waterui_drop_binding_bool(bindingPtr: Long)
    external fun waterui_drop_binding_int(bindingPtr: Long)
    external fun waterui_drop_binding_double(bindingPtr: Long)
    external fun waterui_drop_binding_str(bindingPtr: Long)

    external fun waterui_drop_watcher_guard(guardPtr: Long)
    external fun waterui_get_animation(metadataPtr: Long): Int

    external fun waterui_drop_layout(layoutPtr: Long)

    external fun waterui_drop_action(actionPtr: Long)
    external fun waterui_call_action(actionPtr: Long, envPtr: Long)

    external fun waterui_force_as_button(anyViewPtr: Long): ButtonStruct
    external fun waterui_force_as_text(anyViewPtr: Long): TextStruct
    external fun waterui_force_as_label(anyViewPtr: Long): LabelStruct
    external fun waterui_force_as_color(anyViewPtr: Long): ColorStruct
    external fun waterui_force_as_text_field(anyViewPtr: Long): TextFieldStruct
    external fun waterui_force_as_toggle(anyViewPtr: Long): ToggleStruct
    external fun waterui_force_as_slider(anyViewPtr: Long): SliderStruct
    external fun waterui_force_as_stepper(anyViewPtr: Long): StepperStruct
    external fun waterui_force_as_progress(anyViewPtr: Long): ProgressStruct
    external fun waterui_force_as_scroll(anyViewPtr: Long): ScrollStruct
    external fun waterui_force_as_dynamic(anyViewPtr: Long): DynamicStruct
    external fun waterui_force_as_spacer(anyViewPtr: Long): SpacerStruct
    external fun waterui_force_as_renderer_view(anyViewPtr: Long): Long

    external fun waterui_renderer_view_width(handle: Long): Float
    external fun waterui_renderer_view_height(handle: Long): Float
    external fun waterui_renderer_view_preferred_format(handle: Long): Int
    external fun waterui_renderer_view_render_cpu(
        handle: Long,
        pixels: ByteArray,
        width: Int,
        height: Int,
        stride: Int,
        format: Int,
    ): Boolean
    external fun waterui_drop_renderer_view(handle: Long)
}

// --- Native struct mirrors (data-only skeletons) ---

const val RENDERER_BUFFER_FORMAT_RGBA8888: Int = 0

data class ContainerStruct(
    val layoutPtr: Long,
    val childrenPtr: Long
)

data class ProposalStruct(
    val width: Float,
    val height: Float
)

data class SizeStruct(
    val width: Float,
    val height: Float
)

data class RectStruct(
    val originX: Float,
    val originY: Float,
    val width: Float,
    val height: Float
)

data class ChildMetadataStruct(
    val proposal: ProposalStruct,
    val priority: Int,
    val stretch: Boolean
)

/**
 * Common watcher envelope for bindings/computed values. Concrete shapes will be marshalled via JNI.
 */
data class WatcherStruct(
    val dataPtr: Long,
    val callPtr: Long,
    val dropPtr: Long
)

data class ButtonStruct(
    val labelPtr: Long,
    val actionPtr: Long
)

data class TextStruct(
    val contentPtr: Long
)

data class LabelStruct(
    val textBytes: ByteArray
)

data class ColorStruct(
    val resolvedColorPtr: Long
)

data class TextFieldStruct(
    val labelPtr: Long,
    val valuePtr: Long,
    val promptPtr: Long,
    val keyboardType: Int
)

data class ToggleStruct(
    val labelPtr: Long,
    val bindingPtr: Long
)

data class SliderStruct(
    val labelPtr: Long,
    val minLabelPtr: Long,
    val maxLabelPtr: Long,
    val rangeStart: Double,
    val rangeEnd: Double,
    val bindingPtr: Long
)

data class StepperStruct(
    val bindingPtr: Long,
    val stepPtr: Long,
    val labelPtr: Long,
    val rangeStart: Int,
    val rangeEnd: Int
)

data class ProgressStruct(
    val labelPtr: Long,
    val valueLabelPtr: Long,
    val valuePtr: Long,
    val style: Int
)

data class ScrollStruct(
    val axis: Int,
    val contentPtr: Long
)

data class DynamicStruct(
    val dynamicPtr: Long
)

data class SpacerStruct(val ignored: Int = 0)
