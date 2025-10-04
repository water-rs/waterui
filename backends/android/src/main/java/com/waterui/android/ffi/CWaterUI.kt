package com.waterui.android.ffi

import com.sun.jna.Callback
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.Structure

interface CWaterUI : Library {
    companion object {
        val INSTANCE: CWaterUI = Native.load("waterui", CWaterUI::class.java)
    }

    // Basic Types
    @Structure.FieldOrder("inner")
    open class WuiTypeId : Structure(), Structure.ByValue {
        @JvmField
        var inner: LongArray = LongArray(2)

        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (javaClass != other?.javaClass) return false
            other as WuiTypeId
            return inner.contentEquals(other.inner)
        }

        override fun hashCode(): Int {
            return inner.contentHashCode()
        }
    }

    // FFI Array Infrastructure
    interface DropCallback : Callback {
        fun invoke(ptr: Pointer?)
    }

    // Generic Array Boilerplate
    interface SliceCallback<T : Structure> : Callback {
        fun invoke(ptr: Pointer?): T
    }

    // --- u8 array ---
    @Structure.FieldOrder("drop", "slice")
    open class WuiArrayVTable_u8 : Structure() {
        @JvmField var drop: DropCallback? = null
        @JvmField var slice: SliceCallback<WuiArraySlice_u8>? = null
    }
    @Structure.FieldOrder("head", "len")
    open class WuiArraySlice_u8 : Structure(), Structure.ByValue { 
        @JvmField var head: Pointer? = null
        @JvmField var len: Long = 0
    }
    @Structure.FieldOrder("data", "vtable")
    open class WuiArray_u8 : Structure(), Structure.ByValue {
        @JvmField var data: Pointer? = null
        @JvmField var vtable: WuiArrayVTable_u8? = null
    }

    // --- AnyView array ---
    @Structure.FieldOrder("drop", "slice")
    open class WuiArrayVTable_AnyView : Structure() {
        @JvmField var drop: DropCallback? = null
        @JvmField var slice: SliceCallback<WuiArraySlice_AnyView>? = null
    }
    @Structure.FieldOrder("head", "len")
    open class WuiArraySlice_AnyView : Structure(), Structure.ByValue {
        @JvmField var head: Pointer? = null
        @JvmField var len: Long = 0
    }
    @Structure.FieldOrder("data", "vtable")
    open class WuiArray_AnyView : Structure(), Structure.ByValue {
        @JvmField var data: Pointer? = null
        @JvmField var vtable: WuiArrayVTable_AnyView? = null
    }

    // --- WuiChildMetadata array ---
    @Structure.FieldOrder("drop", "slice")
    open class WuiArrayVTable_ChildMetadata : Structure() {
        @JvmField var drop: DropCallback? = null
        @JvmField var slice: SliceCallback<WuiArraySlice_ChildMetadata>? = null
    }
    @Structure.FieldOrder("head", "len")
    open class WuiArraySlice_ChildMetadata : Structure(), Structure.ByValue {
        @JvmField var head: WuiChildMetadata.ByReference? = null
        @JvmField var len: Long = 0
    }
    @Structure.FieldOrder("data", "vtable")
    open class WuiArray_ChildMetadata : Structure(), Structure.ByValue {
        @JvmField var data: Pointer? = null
        @JvmField var vtable: WuiArrayVTable_ChildMetadata? = null
    }

    // --- WuiProposalSize array ---
    @Structure.FieldOrder("drop", "slice")
    open class WuiArrayVTable_ProposalSize : Structure() {
        @JvmField var drop: DropCallback? = null
        @JvmField var slice: SliceCallback<WuiArraySlice_ProposalSize>? = null
    }
    @Structure.FieldOrder("head", "len")
    open class WuiArraySlice_ProposalSize : Structure(), Structure.ByValue {
        @JvmField var head: WuiProposalSize.ByReference? = null
        @JvmField var len: Long = 0
    }
    @Structure.FieldOrder("data", "vtable")
    open class WuiArray_ProposalSize : Structure(), Structure.ByValue {
        @JvmField var data: Pointer? = null
        @JvmField var vtable: WuiArrayVTable_ProposalSize? = null
    }

    // --- WuiRect array ---
    @Structure.FieldOrder("drop", "slice")
    open class WuiArrayVTable_Rect : Structure() {
        @JvmField var drop: DropCallback? = null
        @JvmField var slice: SliceCallback<WuiArraySlice_Rect>? = null
    }
    @Structure.FieldOrder("head", "len")
    open class WuiArraySlice_Rect : Structure(), Structure.ByValue {
        @JvmField var head: WuiRect.ByReference? = null
        @JvmField var len: Long = 0
    }
    @Structure.FieldOrder("data", "vtable")
    open class WuiArray_Rect : Structure(), Structure.ByValue {
        @JvmField var data: Pointer? = null
        @JvmField var vtable: WuiArrayVTable_Rect? = null
    }


    // --- Component & Layout Structs ---

    @Structure.FieldOrder("_0")
    open class WuiStr : Structure(), Structure.ByValue {
        @JvmField var _0: WuiArray_u8? = null

        fun toKString(): String {
            if (_0 == null || _0!!.data == null || _0!!.vtable == null || _0!!.vtable!!.slice == null) {
                return ""
            }
            val slice = _0!!.vtable!!.slice!!.invoke(_0!!.data)
            if (slice.head == null || slice.len == 0L) {
                return ""
            }
            val bytes = slice.head!!.getByteArray(0, slice.len.toInt())
            _0!!.vtable!!.drop!!.invoke(_0!!.data)
            return String(bytes, Charsets.UTF_8)
        }

        companion object {
            fun fromString(s: String): WuiStr {
                val bytes = s.toByteArray(Charsets.UTF_8)
                return INSTANCE.waterui_string_from_bytes(bytes, bytes.size)
            }
        }
    }

    @Structure.FieldOrder("size", "italic", "strikethrough", "underlined", "bold")
    open class WuiFont : Structure(), Structure.ByValue {
        @JvmField var size: Double = Double.NaN
        @JvmField var italic: Boolean = false
        @JvmField var strikethrough: Pointer? = null
        @JvmField var underlined: Pointer? = null
        @JvmField var bold: Boolean = false
    }

    @Structure.FieldOrder("has_font", "font", "bold", "italic", "underline", "strikethrough", "foreground", "background")
    open class WuiTextStyle : Structure(), Structure.ByValue {
        @JvmField var has_font: Boolean = false
        @JvmField var font: WuiFont? = null
        @JvmField var bold: Boolean = false
        @JvmField var italic: Boolean = false
        @JvmField var underline: Boolean = false
        @JvmField var strikethrough: Boolean = false
        @JvmField var foreground: Pointer? = null
        @JvmField var background: Pointer? = null
    }

    @Structure.FieldOrder("text", "style")
    open class WuiAttributedChunk : Structure(), Structure.ByValue {
        @JvmField var text: WuiStr? = null
        @JvmField var style: WuiTextStyle? = null
    }

    @Structure.FieldOrder("drop", "slice")
    open class WuiArrayVTable_WuiAttributedChunk : Structure() {
        @JvmField var drop: DropCallback? = null
        @JvmField var slice: SliceCallback<WuiArraySlice_WuiAttributedChunk>? = null
    }

    @Structure.FieldOrder("head", "len")
    open class WuiArraySlice_WuiAttributedChunk : Structure(), Structure.ByValue {
        @JvmField var head: Pointer? = null
        @JvmField var len: Long = 0
    }

    @Structure.FieldOrder("data", "vtable")
    open class WuiArray_WuiAttributedChunk : Structure(), Structure.ByValue {
        @JvmField var data: Pointer? = null
        @JvmField var vtable: WuiArrayVTable_WuiAttributedChunk? = null
    }

    @Structure.FieldOrder("chunks")
    open class WuiAttributedStr : Structure(), Structure.ByValue {
        @JvmField var chunks: WuiArray_WuiAttributedChunk? = null

        fun toPlainString(): String {
            val array = chunks ?: return ""
            val vtable = array.vtable ?: return ""
            val slice = vtable.slice?.invoke(array.data) ?: return ""
            val head = slice.head ?: return ""
            if (slice.len <= 0L) {
                vtable.drop?.invoke(array.data)
                return ""
            }

            val chunk = WuiAttributedChunk()
            val chunkSize = chunk.size().toLong()
            val builder = StringBuilder()

            for (index in 0 until slice.len.toInt()) {
                val ptr = head.share(index.toLong() * chunkSize)
                val current = WuiAttributedChunk(ptr)
                current.read()
                builder.append(current.text?.toKString() ?: "")
            }

            vtable.drop?.invoke(array.data)
            return builder.toString()
        }
    }

    @Structure.FieldOrder("content", "font")
    open class WuiText : Structure(), Structure.ByValue {
        @JvmField var content: Pointer? = null // Computed_AttributedStr*
        @JvmField var font: Pointer? = null    // Computed_Font*
    }

    @Structure.FieldOrder("label", "action")
    open class WuiButton : Structure(), Structure.ByValue {
        @JvmField var label: Pointer? = null // WuiAnyView*
        @JvmField var action: Pointer? = null // WuiAction*
    }

    @Structure.FieldOrder("layout", "contents")
    open class WuiContainer : Structure(), Structure.ByValue {
        @JvmField var layout: Pointer? = null // WuiLayout*
        @JvmField var contents: WuiArray_AnyView? = null
    }

    @Structure.FieldOrder("axis", "content")
    open class WuiScrollView : Structure(), Structure.ByValue {
        @JvmField var axis: Int = 0
        @JvmField var content: Pointer? = null // WuiAnyView*
    }

    @Structure.FieldOrder("label", "value_label", "value", "style")
    open class WuiProgress : Structure(), Structure.ByValue {
        @JvmField var label: Pointer? = null
        @JvmField var value_label: Pointer? = null
        @JvmField var value: Pointer? = null // Computed_f64*
        @JvmField var style: Int = 0
    }

    @Structure.FieldOrder("value", "step", "label", "range")
    open class WuiStepper : Structure(), Structure.ByValue {
        @JvmField var value: Pointer? = null // Binding_i32*
        @JvmField var step: Pointer? = null // Computed_i32*
        @JvmField var label: Pointer? = null // WuiAnyView*
        @JvmField var range: WuiRange_i32? = null
    }

    @Structure.FieldOrder("label", "min_value_label", "max_value_label", "range", "value")
    open class WuiSlider : Structure(), Structure.ByValue {
        @JvmField var label: Pointer? = null
        @JvmField var min_value_label: Pointer? = null
        @JvmField var max_value_label: Pointer? = null
        @JvmField var range: WuiRange_f64? = null
        @JvmField var value: Pointer? = null // Binding_f64*
    }

    @Structure.FieldOrder("label", "value", "prompt", "keyboard")
    open class WuiTextField : Structure(), Structure.ByValue {
        @JvmField var label: Pointer? = null
        @JvmField var value: Pointer? = null // Binding_Str*
        @JvmField var prompt: WuiText? = null
        @JvmField var keyboard: Int = 0
    }

    @Structure.FieldOrder("label", "toggle")
    open class WuiToggle : Structure(), Structure.ByValue {
        @JvmField var label: Pointer? = null
        @JvmField var toggle: Pointer? = null // Binding_bool*
    }

    // --- Layout Structs ---
    @Structure.FieldOrder("width", "height")
    open class WuiProposalSize : Structure() {
        @JvmField var width: Double = 0.0
        @JvmField var height: Double = 0.0
        interface ByReference : Structure.ByReference
    }

    @Structure.FieldOrder("proposal", "priority", "stretch")
    open class WuiChildMetadata : Structure() {
        @JvmField var proposal: WuiProposalSize? = null
        @JvmField var priority: Byte = 0
        @JvmField var stretch: Boolean = false
        interface ByReference : Structure.ByReference
    }

    @Structure.FieldOrder("width", "height")
    open class WuiSize : Structure(), Structure.ByValue {
        @JvmField var width: Double = 0.0
        @JvmField var height: Double = 0.0
    }

    @Structure.FieldOrder("x", "y")
    open class WuiPoint : Structure() {
        @JvmField var x: Double = 0.0
        @JvmField var y: Double = 0.0
    }

    @Structure.FieldOrder("origin", "size")
    open class WuiRect : Structure() {
        @JvmField var origin: WuiPoint? = null
        @JvmField var size: WuiSize? = null
        interface ByReference : Structure.ByReference
    }


    // --- Watcher Infrastructure ---
    // Note: WuiWatcherMetadata is an opaque struct pointer

    interface WuiWatcherStrCallback : Callback {
        fun invoke(data: Pointer?, value: WuiStr, metadata: Pointer?)
    }

    @Structure.FieldOrder("data", "call", "drop")
    open class WuiWatcher_WuiStr : Structure() {
        @JvmField var data: Pointer? = null
        @JvmField var call: WuiWatcherStrCallback? = null
        @JvmField var drop: DropCallback? = null
    }

    interface WuiWatcherAttributedStrCallback : Callback {
        fun invoke(data: Pointer?, value: WuiAttributedStr, metadata: Pointer?)
    }

    @Structure.FieldOrder("data", "call", "drop")
    open class WuiWatcher_WuiAttributedStr : Structure() {
        @JvmField var data: Pointer? = null
        @JvmField var call: WuiWatcherAttributedStrCallback? = null
        @JvmField var drop: DropCallback? = null
    }

    interface WuiWatcherDoubleCallback : Callback {
        fun invoke(data: Pointer?, value: Double, metadata: Pointer?)
    }

    @Structure.FieldOrder("data", "call", "drop")
    open class WuiWatcher_f64 : Structure() {
        @JvmField var data: Pointer? = null
        @JvmField var call: WuiWatcherDoubleCallback? = null
        @JvmField var drop: DropCallback? = null
    }

    interface WuiWatcherIntCallback : Callback {
        fun invoke(data: Pointer?, value: Int, metadata: Pointer?)
    }

    @Structure.FieldOrder("data", "call", "drop")
    open class WuiWatcher_i32 : Structure() {
        @JvmField var data: Pointer? = null
        @JvmField var call: WuiWatcherIntCallback? = null
        @JvmField var drop: DropCallback? = null
    }

    interface WuiWatcherBoolCallback : Callback {
        fun invoke(data: Pointer?, value: Boolean, metadata: Pointer?)
    }

    @Structure.FieldOrder("data", "call", "drop")
    open class WuiWatcher_bool : Structure() {
        @JvmField var data: Pointer? = null
        @JvmField var call: WuiWatcherBoolCallback? = null
        @JvmField var drop: DropCallback? = null
    }

    // --- Core & Util Functions ---
    fun waterui_init(): Pointer
    fun waterui_main(): Pointer
    fun waterui_view_body(view: Pointer, env: Pointer): Pointer
    fun waterui_view_id(view: Pointer): WuiTypeId
    fun waterui_drop_env(env: Pointer)
    fun waterui_drop_any_view(view: Pointer)
    fun waterui_string_from_bytes(bytes: ByteArray, len: Int): WuiStr

    // --- Component & FFI Functions ---
    fun waterui_call_action(action: Pointer, env: Pointer)
    fun waterui_drop_action(action: Pointer?)

    fun waterui_layout_get_orientation(layout: Pointer): Int // 0 for horizontal, 1 for vertical
    fun waterui_drop_layout(layout: Pointer?)
    fun waterui_spacer_id(): WuiTypeId
    fun waterui_text_id(): WuiTypeId
    fun waterui_force_as_text(view: Pointer): WuiText
    fun waterui_button_id(): WuiTypeId
    fun waterui_force_as_button(view: Pointer): WuiButton
    fun waterui_container_id(): WuiTypeId
    fun waterui_force_as_container(view: Pointer): WuiContainer
    fun waterui_scroll_view_id(): WuiTypeId
    fun waterui_force_as_scroll_view(view: Pointer): WuiScrollView
    fun waterui_divider_id(): WuiTypeId
    fun waterui_progress_id(): WuiTypeId
    fun waterui_force_as_progress(view: Pointer): WuiProgress
    fun waterui_stepper_id(): WuiTypeId
    fun waterui_force_as_stepper(view: Pointer): WuiStepper
    fun waterui_slider_id(): WuiTypeId
    fun waterui_force_as_slider(view: Pointer): WuiSlider
    fun waterui_text_field_id(): WuiTypeId
    fun waterui_force_as_text_field(view: Pointer): WuiTextField
    fun waterui_toggle_id(): WuiTypeId
    fun waterui_force_as_toggle(view: Pointer): WuiToggle

    // --- Layout Functions ---
    fun waterui_layout_propose(layout: Pointer, parent: WuiProposalSize, children: WuiArray_ChildMetadata): WuiArray_ProposalSize
    fun waterui_layout_size(layout: Pointer, parent: WuiProposalSize, children: WuiArray_ChildMetadata): WuiSize
    fun waterui_layout_place(layout: Pointer, bound: WuiRect, proposal: WuiProposalSize, children: WuiArray_ChildMetadata): WuiArray_Rect

    // --- Reactive Functions ---
    fun waterui_read_computed_str(computed: Pointer): WuiStr
    fun waterui_drop_computed_str(computed: Pointer?)
    fun waterui_watch_computed_str(computed: Pointer, watcher: WuiWatcher_WuiStr): Pointer // Returns WuiWatcherGuard*

    fun waterui_read_computed_attributed_str(computed: Pointer): WuiAttributedStr
    fun waterui_drop_computed_attributed_str(computed: Pointer?)
    fun waterui_watch_computed_attributed_str(computed: Pointer, watcher: WuiWatcher_WuiAttributedStr): Pointer

    fun waterui_read_computed_double(computed: Pointer): Double
    fun waterui_drop_computed_double(computed: Pointer?)
    fun waterui_watch_computed_double(computed: Pointer, watcher: WuiWatcher_f64): Pointer // Returns WuiWatcherGuard*

    fun waterui_read_computed_int(computed: Pointer): Int
    fun waterui_drop_computed_int(computed: Pointer?)

    fun waterui_read_binding_int(binding: Pointer): Int
    fun waterui_set_binding_int(binding: Pointer, value: Int)
    fun waterui_watch_binding_int(binding: Pointer, watcher: WuiWatcher_i32): Pointer // Returns WuiWatcherGuard*
    fun waterui_drop_binding_int(binding: Pointer?)

    fun waterui_read_binding_double(binding: Pointer): Double
    fun waterui_set_binding_double(binding: Pointer, value: Double)
    fun waterui_watch_binding_double(binding: Pointer, watcher: WuiWatcher_f64): Pointer // Returns WuiWatcherGuard*
    fun waterui_drop_binding_double(binding: Pointer?)

    fun waterui_read_binding_str(binding: Pointer): WuiStr
    fun waterui_set_binding_str(binding: Pointer, value: WuiStr)
    fun waterui_watch_binding_str(binding: Pointer, watcher: WuiWatcher_WuiStr): Pointer // Returns WuiWatcherGuard*
    fun waterui_drop_binding_str(binding: Pointer?)

    fun waterui_read_binding_bool(binding: Pointer): Boolean
    fun waterui_set_binding_bool(binding: Pointer, value: Boolean)
    fun waterui_watch_binding_bool(binding: Pointer, watcher: WuiWatcher_bool): Pointer // Returns WuiWatcherGuard*
    fun waterui_drop_binding_bool(binding: Pointer?)

    fun waterui_drop_box_watcher_guard(guard: Pointer?)
}
