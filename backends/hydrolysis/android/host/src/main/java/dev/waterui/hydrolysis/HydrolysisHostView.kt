package dev.waterui.hydrolysis

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Rect
import android.os.Build
import android.util.SparseArray
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.ViewStructure
import android.view.WindowInsets
import android.view.accessibility.AccessibilityNodeProvider
import android.view.autofill.AutofillValue
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.annotation.RequiresApi
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/**
 * The painter-independent Android host: a [ViewGroup] that owns the native
 * session's window services — metrics, input routing, the IME boundary, the
 * accessibility/autofill adapters and the platform-view overlay.
 *
 * The GPU presentation surface is deliberately *not* here: a painter's band
 * (the gpu module's `HydrolysisGpuBand`) is an ordinary child added at index 0
 * so it draws beneath every native child and the platform-view overlay. The
 * host never touches a wgpu device, a GPU surface, a render thread or a font
 * context — it would host an HWUI band just as happily.
 *
 * One coherent metrics snapshot ([pushMetrics]) carries size, density, font
 * scale, display refresh and system-bar insets to the session; the native side
 * never reads them piecemeal.
 */
@SuppressLint("ViewConstructor")
open class HydrolysisHostView
@JvmOverloads
constructor(context: Context, internal val session: HydrolysisSession? = null) :
    ViewGroup(context) {

    private val scheduler: FrameScheduler? = session?.let { FrameScheduler(it) }

    /**
     * The overlay native children (embedded platform views) are laid into,
     * always above the GPU band. Populated from the session's placement frames
     * by [platformViewRegistry].
     */
    val platformViewRegistry: PlatformViewRegistry = PlatformViewRegistry(context, session)

    private val accessibilityProvider: HydrolysisAccessibilityProvider =
        HydrolysisAccessibilityProvider(this, session)
    private val autofillBridge: AutofillBridge = AutofillBridge(accessibilityProvider)

    private var lastMetricsWidth = -1
    private var lastMetricsHeight = -1
    private var lastDensity = Float.NaN
    private var lastFontScale = Float.NaN
    private var lastRefreshHz = Float.NaN
    private var lastInsets = intArrayOf(0, 0, 0, 0)
    private var lastRootInsets: WindowInsetsCompat? = null

    /** The focused editor's rect in physical px plus purpose, mirrored to the IME. */
    private var imeRect = Rect()
    private var imePurpose = -1

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
        addView(platformViewRegistry.container)
        session?.bind(this)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        session?.bind(this)
        pushMetrics()
    }

    override fun onDetachedFromWindow() {
        session?.unbind(this)
        super.onDetachedFromWindow()
    }

    // ------------------------------------------------------------------
    // Metrics — one coherent snapshot per change, pushed to the session.

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        pushMetrics()
    }

    override fun onApplyWindowInsets(insets: WindowInsets): WindowInsets {
        // The dispatched insets *are* the new value — reading the root back
        // here would still see the pre-dispatch one.
        lastRootInsets = WindowInsetsCompat.toWindowInsetsCompat(insets)
        pushMetrics()
        return super.onApplyWindowInsets(insets)
    }

    private fun pushMetrics() {
        val session = session ?: return
        val metrics = resources.displayMetrics
        val configuration = resources.configuration
        val rootInsets = lastRootInsets ?: ViewCompat.getRootWindowInsets(this)
        val edges =
            if (rootInsets != null) {
                val bars =
                    rootInsets.getInsets(
                        WindowInsetsCompat.Type.systemBars() or
                            WindowInsetsCompat.Type.displayCutout()
                    )
                val ime =
                    if (rootInsets.isVisible(WindowInsetsCompat.Type.ime())) {
                        rootInsets.getInsets(WindowInsetsCompat.Type.ime())
                    } else {
                        Insets.NONE
                    }
                val combined = Insets.max(bars, ime)
                intArrayOf(combined.left, combined.top, combined.right, combined.bottom)
            } else {
                intArrayOf(0, 0, 0, 0)
            }
        val refreshHz = display?.refreshRate ?: 0f
        if (width == lastMetricsWidth &&
            height == lastMetricsHeight &&
            metrics.density == lastDensity &&
            configuration.fontScale == lastFontScale &&
            refreshHz == lastRefreshHz &&
            edges.contentEquals(lastInsets)
        ) {
            return
        }
        lastMetricsWidth = width
        lastMetricsHeight = height
        lastDensity = metrics.density
        lastFontScale = configuration.fontScale
        lastRefreshHz = refreshHz
        lastInsets = edges
        NativeBridge.nativeSetMetrics(
            session.nativePtr,
            width,
            height,
            metrics.density,
            configuration.fontScale,
            refreshHz,
            edges[0],
            edges[1],
            edges[2],
            edges[3],
        )
    }

    // ------------------------------------------------------------------
    // Layout — the GPU band (any painter SurfaceView/TextureView, child 0)
    // and the overlay fill the host bounds exactly; the band reports its own
    // size through its SurfaceHolder.

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val width = MeasureSpec.getSize(widthMeasureSpec)
        val height = MeasureSpec.getSize(heightMeasureSpec)
        measureChildren(
            MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
            MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY),
        )
        setMeasuredDimension(width, height)
    }

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        for (i in 0 until childCount) {
            getChildAt(i).layout(0, 0, r - l, b - t)
        }
        platformViewRegistry.publishIfPending()
    }

    // ------------------------------------------------------------------
    // Frame scheduling — native asks through the session bridge.

    internal fun requestFrame() {
        scheduler?.requestFrame("redraw-request")
    }

    internal fun closeRequested() {
        (context as? android.app.Activity)?.finish()
    }

    // ------------------------------------------------------------------
    // Input — decoded MotionEvents become session input events.

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        val session = session ?: return false
        val action = event.actionMasked
        val index = event.actionIndex
        val toolType = event.getToolType(index)
        val button =
            when {
                event.buttonState and MotionEvent.BUTTON_SECONDARY != 0 ||
                    event.actionButton == MotionEvent.BUTTON_SECONDARY -> 1
                event.buttonState and MotionEvent.BUTTON_TERTIARY != 0 ||
                    event.actionButton == MotionEvent.BUTTON_TERTIARY -> 2
                else -> 0
            }
        NativeBridge.nativePointerEvent(
            session.nativePtr,
            action,
            event.getPointerId(index),
            event.getX(index),
            event.getY(index),
            toolType,
            button,
        )
        when (action) {
            MotionEvent.ACTION_DOWN -> {
                requestFocus()
                scheduler?.setInteractionActive(true)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL ->
                scheduler?.setInteractionActive(false)
        }
        return true
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        val session = session ?: return super.onGenericMotionEvent(event)
        if (event.actionMasked == MotionEvent.ACTION_SCROLL) {
            val dx = -event.getAxisValue(MotionEvent.AXIS_HSCROLL)
            val dy = -event.getAxisValue(MotionEvent.AXIS_VSCROLL)
            NativeBridge.nativeScrollEvent(session.nativePtr, event.x, event.y, dx, dy)
            return true
        }
        return super.onGenericMotionEvent(event)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean =
        dispatchKey(event, pressed = true) || super.onKeyDown(keyCode, event)

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean =
        dispatchKey(event, pressed = false) || super.onKeyUp(keyCode, event)

    private fun dispatchKey(event: KeyEvent, pressed: Boolean): Boolean {
        val session = session ?: return false
        val key = w3cKey(event) ?: return false
        NativeBridge.nativeKeyEvent(
            session.nativePtr,
            key,
            pressed,
            event.isShiftPressed,
            event.isCtrlPressed,
            event.isAltPressed,
            event.isMetaPressed,
        )
        return true
    }

    /** Maps a hardware key onto the W3C `key` vocabulary the runner parses. */
    private fun w3cKey(event: KeyEvent): String? {
        if (event.unicodeChar != 0) return event.unicodeChar.toChar().toString()
        return when (event.keyCode) {
            KeyEvent.KEYCODE_ENTER -> "Enter"
            KeyEvent.KEYCODE_DEL -> "Backspace"
            KeyEvent.KEYCODE_FORWARD_DEL -> "Delete"
            KeyEvent.KEYCODE_TAB -> "Tab"
            KeyEvent.KEYCODE_ESCAPE -> "Escape"
            KeyEvent.KEYCODE_SPACE -> " "
            KeyEvent.KEYCODE_DPAD_LEFT -> "ArrowLeft"
            KeyEvent.KEYCODE_DPAD_RIGHT -> "ArrowRight"
            KeyEvent.KEYCODE_DPAD_UP -> "ArrowUp"
            KeyEvent.KEYCODE_DPAD_DOWN -> "ArrowDown"
            KeyEvent.KEYCODE_MOVE_HOME -> "Home"
            KeyEvent.KEYCODE_MOVE_END -> "End"
            KeyEvent.KEYCODE_PAGE_UP -> "PageUp"
            KeyEvent.KEYCODE_PAGE_DOWN -> "PageDown"
            KeyEvent.KEYCODE_SHIFT_LEFT, KeyEvent.KEYCODE_SHIFT_RIGHT -> "Shift"
            KeyEvent.KEYCODE_CTRL_LEFT, KeyEvent.KEYCODE_CTRL_RIGHT -> "Control"
            KeyEvent.KEYCODE_ALT_LEFT, KeyEvent.KEYCODE_ALT_RIGHT -> "Alt"
            KeyEvent.KEYCODE_META_LEFT, KeyEvent.KEYCODE_META_RIGHT -> "Meta"
            else -> null
        }
    }

    // ------------------------------------------------------------------
    // IME — the text-input boundary the session pushes through the bridge.

    override fun onCheckIsTextEditor(): Boolean = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType =
            if (imePurpose == 1) {
                EditorInfo.TYPE_CLASS_TEXT or EditorInfo.TYPE_TEXT_VARIATION_PASSWORD
            } else {
                EditorInfo.TYPE_CLASS_TEXT or EditorInfo.TYPE_TEXT_FLAG_MULTI_LINE
            }
        outAttrs.imeOptions = EditorInfo.IME_ACTION_NONE or EditorInfo.IME_FLAG_NO_FULLSCREEN
        outAttrs.initialSelStart = 0
        outAttrs.initialSelEnd = 0
        return HydrolysisInputConnection(this, session, outAttrs)
    }

    /**
     * The session's focused-editor state, physical px in host coordinates.
     * `purpose < 0` clears the target (the IME hides); otherwise the IME is
     * asked to show and the candidates window tracks the cursor rect.
     */
    internal fun updateTextInputTarget(x: Float, y: Float, w: Float, h: Float, purpose: Int) {
        val imm =
            context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        if (purpose < 0) {
            if (imePurpose >= 0) {
                imePurpose = -1
                imm.hideSoftInputFromWindow(windowToken, 0)
            }
            return
        }
        imePurpose = purpose
        imeRect.set(x.toInt(), y.toInt(), (x + w).toInt(), (y + h).toInt())
        if (!hasFocus()) requestFocus()
        if (Build.VERSION.SDK_INT >= 34) {
            imm.updateCursorAnchorInfo(this, cursorAnchorInfo(imeRect))
        }
        imm.updateCursor(this, imeRect.left, imeRect.top, imeRect.right, imeRect.bottom)
        imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
    }

    /**
     * The cursor in view-local coordinates. Positional parameters require the
     * view-to-screen matrix, which [transformMatrixToGlobal] supplies with
     * every ancestor's transform.
     */
    @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
    private fun cursorAnchorInfo(rect: Rect): android.view.inputmethod.CursorAnchorInfo =
        android.view.inputmethod.CursorAnchorInfo.Builder()
            .setMatrix(android.graphics.Matrix().also { transformMatrixToGlobal(it) })
            .setInsertionMarkerLocation(
                rect.left.toFloat(),
                rect.top.toFloat(),
                rect.bottom.toFloat(),
                rect.bottom.toFloat(),
                android.view.inputmethod.CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION,
            )
            .build()

    // ------------------------------------------------------------------
    // Accessibility + autofill — adapters backed by the session's snapshot.

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider =
        accessibilityProvider

    internal fun notifyAccessibilityTreeChanged() {
        accessibilityProvider.notifyTreeChanged()
    }

    override fun onProvideAutofillVirtualStructure(structure: ViewStructure, flags: Int) {
        super.onProvideAutofillVirtualStructure(structure, flags)
        autofillBridge.provideVirtualStructure(structure)
    }

    override fun autofill(values: SparseArray<AutofillValue>) {
        autofillBridge.autofill(values)
        super.autofill(values)
    }
}
