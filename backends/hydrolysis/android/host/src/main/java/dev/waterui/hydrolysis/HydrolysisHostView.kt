package dev.waterui.hydrolysis

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.util.SparseArray
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.ViewStructure
import android.view.WindowInsets
import android.view.accessibility.AccessibilityNodeProvider
import android.view.autofill.AutofillValue
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsAnimationCompat
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
    private val autofillBridge: AutofillBridge =
        AutofillBridge(this, accessibilityProvider)

    private var lastMetricsWidth = -1
    private var lastMetricsHeight = -1
    private var lastDensity = Float.NaN
    private var lastFontScale = Float.NaN
    private var lastRefreshHz = Float.NaN
    private var lastInsets = intArrayOf(0, 0, 0, 0)
    private var lastKeyboardInsets = intArrayOf(0, 0, 0, 0)
    private var lastTouchSlop = Float.NaN
    private var lastMinFlingVelocity = Float.NaN
    private var lastMaxFlingVelocity = Float.NaN
    private var lastScrollFriction = Float.NaN
    private var lastRootInsets: WindowInsetsCompat? = null

    /**
     * An IME `WindowInsetsAnimation` is running. While it is, the insets
     * `onApplyWindowInsets` dispatches already carry the animation's *end*
     * state — pushing them would jump the layout to the full keyboard
     * height for one frame before `onProgress` pulls it back. The flag
     * defers the keyboard region to `onProgress` (and `onEnd` for the
     * settled value); the container region still tracks the dispatch.
     */
    private var imeAnimating = false

    /**
     * §7.1's keyboard-motion rule: the IME inset the session avoids by
     * follows the platform's keyboard animation frame by frame, so every
     * `onProgress` lands as its own metrics push instead of a single jump
     * when the animation settles. The deferral described on [imeAnimating]
     * covers the interactive swipe-dismiss path too — the same callbacks
     * fire for an `InsetsController`-driven animation.
     */
    private val insetsAnimationCallback =
        object : WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
            override fun onPrepare(animation: WindowInsetsAnimationCompat) {
                if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) {
                    imeAnimating = true
                }
            }

            override fun onProgress(
                insets: WindowInsetsCompat,
                runningAnimations: MutableList<WindowInsetsAnimationCompat>,
            ): WindowInsetsCompat {
                // Only the IME component is mid-animation state — the
                // container keeps coming from the persisted dispatches
                // (`lastRootInsets`), so a concurrent `pushMetrics` never
                // reads the animation frame's container insets back out.
                val imeRunning =
                    runningAnimations.any {
                        it.typeMask and WindowInsetsCompat.Type.ime() != 0
                    }
                if (imeAnimating && imeRunning) {
                    val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
                    pushMetrics(intArrayOf(ime.left, ime.top, ime.right, ime.bottom))
                } else {
                    // A non-IME animation (e.g. a system-bar hide/show)
                    // lands here too and pushes the persisted
                    // `lastRootInsets` on every progress frame, so its
                    // metrics jump at `onApplyWindowInsets` rather
                    // than following the animation — outside §7.1's scope.
                    pushMetrics()
                }
                return insets
            }

            override fun onEnd(animation: WindowInsetsAnimationCompat) {
                if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) {
                    imeAnimating = false
                    // The last progress frame is not guaranteed to carry
                    // fraction 1.0 — publish the settled insets.
                    lastRootInsets = ViewCompat.getRootWindowInsets(this@HydrolysisHostView)
                    pushMetrics()
                }
            }
        }

    /**
     * The live [HydrolysisInputConnection], if the IMM has bound one — the
     * target for the session's editing-state and cursor-anchor pushes.
     */
    internal var inputConnection: HydrolysisInputConnection? = null

    /**
     * The [EditorInfo] contract the live connection was built with. The view
     * is one editor for every field, so a focus move has to [InputMethodManager.restartInput]
     * before the IME will read a new input type — a password field reached
     * through a connection opened while nothing was focused otherwise keeps
     * the plain multiline type and shows suggestions.
     */
    private var inputContract: InputContract? = null
    private var inputRestartPosted = false

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
        importantForAutofill = IMPORTANT_FOR_AUTOFILL_YES
        addView(platformViewRegistry.container)
        ViewCompat.setWindowInsetsAnimationCallback(this, insetsAnimationCallback)
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

    private fun pushMetrics(keyboardEdgesOverride: IntArray? = null) {
        val session = session ?: return
        val metrics = resources.displayMetrics
        val configuration = resources.configuration
        val rootInsets = lastRootInsets ?: ViewCompat.getRootWindowInsets(this)
        val containerEdges: IntArray
        val keyboardEdges: IntArray
        if (rootInsets != null) {
            val bars =
                rootInsets.getInsets(
                    WindowInsetsCompat.Type.systemBars() or
                        WindowInsetsCompat.Type.displayCutout()
                )
            val ime = rootInsets.getInsets(WindowInsetsCompat.Type.ime())
            // §7.1's two regions travel apart: the container band is the
            // bars/cutout insets only, the keyboard band the IME insets
            // only — no region ever absorbs the other. While an IME
            // animation runs, the keyboard region comes only from
            // `onProgress`/`onEnd`; a dispatch carrying the end state
            // leaves the last pushed value in place.
            containerEdges = intArrayOf(bars.left, bars.top, bars.right, bars.bottom)
            keyboardEdges =
                keyboardEdgesOverride
                    ?: if (imeAnimating) {
                        lastKeyboardInsets
                    } else {
                        intArrayOf(ime.left, ime.top, ime.right, ime.bottom)
                    }
        } else {
            containerEdges = intArrayOf(0, 0, 0, 0)
            keyboardEdges = keyboardEdgesOverride ?: intArrayOf(0, 0, 0, 0)
        }
        val refreshHz = display?.refreshRate ?: 0f
        val viewConfiguration = ViewConfiguration.get(context)
        val touchSlop = viewConfiguration.scaledTouchSlop.toFloat()
        val minFlingVelocity = viewConfiguration.scaledMinimumFlingVelocity.toFloat()
        val maxFlingVelocity = viewConfiguration.scaledMaximumFlingVelocity.toFloat()
        val scrollFriction = ViewConfiguration.getScrollFriction()
        if (width == lastMetricsWidth &&
            height == lastMetricsHeight &&
            metrics.density == lastDensity &&
            configuration.fontScale == lastFontScale &&
            refreshHz == lastRefreshHz &&
            containerEdges.contentEquals(lastInsets) &&
            keyboardEdges.contentEquals(lastKeyboardInsets) &&
            touchSlop == lastTouchSlop &&
            minFlingVelocity == lastMinFlingVelocity &&
            maxFlingVelocity == lastMaxFlingVelocity &&
            scrollFriction == lastScrollFriction
        ) {
            return
        }
        lastMetricsWidth = width
        lastMetricsHeight = height
        lastDensity = metrics.density
        lastFontScale = configuration.fontScale
        lastRefreshHz = refreshHz
        lastInsets = containerEdges
        lastKeyboardInsets = keyboardEdges
        lastTouchSlop = touchSlop
        lastMinFlingVelocity = minFlingVelocity
        lastMaxFlingVelocity = maxFlingVelocity
        lastScrollFriction = scrollFriction
        NativeBridge.nativeSetMetrics(
            session.nativePtr,
            width,
            height,
            metrics.density,
            configuration.fontScale,
            refreshHz,
            containerEdges[0],
            containerEdges[1],
            containerEdges[2],
            containerEdges[3],
            keyboardEdges[0],
            keyboardEdges[1],
            keyboardEdges[2],
            keyboardEdges[3],
            touchSlop,
            minFlingVelocity,
            maxFlingVelocity,
            scrollFriction,
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
    // Ordered content — a painter that draws in the host's dispatchDraw
    // (HWUI) interleaves its node runs with the platform views; without one
    // the children draw as usual, a GPU band at index 0 beneath the overlay.

    private var orderedContent: OrderedContent? = null
    private var orderedTouchTarget: View? = null
    private var orderedTouchClaimed = false

    /** Installs (or, with null, removes) the painter drawing in session order. */
    fun setOrderedContent(content: OrderedContent?) {
        orderedContent = content
        orderedTouchTarget = null
        orderedTouchClaimed = false
        invalidate()
    }

    override fun dispatchDraw(canvas: Canvas) {
        val content = orderedContent ?: return super.dispatchDraw(canvas)
        content.drawBackground(canvas)
        val time = drawingTime
        val overlay = platformViewRegistry.container
        for (i in 0 until childCount) {
            val child = getChildAt(i)
            if (child !== overlay) drawChild(canvas, child, time)
        }
        for (index in 0 until content.entryCount) {
            if (content.isPlatformView(index)) {
                platformViewRegistry.drawSlot(canvas, content.platformViewId(index))
            } else {
                content.drawEntry(canvas, index)
            }
        }
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        val content = orderedContent ?: return super.dispatchTouchEvent(event)
        val action = event.actionMasked
        if (action == MotionEvent.ACTION_DOWN) {
            val x = event.x
            val y = event.y
            val entry =
                TouchOrder.topmost(content.entryCount) { index ->
                    if (content.isPlatformView(index)) {
                        platformViewRegistry.slotAt(content.platformViewId(index), x, y) != null
                    } else {
                        content.covers(index, x, y)
                    }
                }
            orderedTouchTarget =
                if (entry >= 0 && content.isPlatformView(entry)) {
                    platformViewRegistry.slotAt(content.platformViewId(entry), x, y)
                } else {
                    null
                }
            orderedTouchClaimed = true
        }
        if (!orderedTouchClaimed) return false
        val target = orderedTouchTarget
        val handled =
            if (target != null) {
                val local = MotionEvent.obtain(event)
                local.offsetLocation(-target.left.toFloat(), -target.top.toFloat())
                val result = target.dispatchTouchEvent(local)
                local.recycle()
                result
            } else {
                onTouchEvent(event)
            }
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
            orderedTouchTarget = null
            orderedTouchClaimed = false
        }
        return handled
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
        // Bind to the authoritative session state — the pulled `editorId`
        // becomes this connection's generation token, and the state seeds
        // the mirror so the IMM's first queries agree with the editor.
        val state =
            session
                ?.let { NativeBridge.nativeEditingState(it.nativePtr) }
                ?.let(::EditingStatePayload)
        inputContract = InputContract.from(state)
        val password = state?.password == true
        outAttrs.inputType =
            EditorInfo.TYPE_CLASS_TEXT or
                (if (password) {
                    EditorInfo.TYPE_TEXT_VARIATION_PASSWORD or EditorInfo.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                } else {
                    0
                }) or
                (if (!password && (state == null || !state.singleLine)) {
                    EditorInfo.TYPE_TEXT_FLAG_MULTI_LINE
                } else {
                    0
                })
        outAttrs.imeOptions =
            (if (state?.hasSubmit == true) EditorInfo.IME_ACTION_DONE else EditorInfo.IME_ACTION_NONE) or
                EditorInfo.IME_FLAG_NO_FULLSCREEN or
                (if (password) EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING else 0)
        outAttrs.initialSelStart = state?.selStart ?: 0
        outAttrs.initialSelEnd = state?.selEnd ?: 0
        outAttrs.initialCapsMode = 0
        val connection =
            HydrolysisInputConnection(
                this,
                session,
                editorId = state?.editorId ?: 0L,
                live = state?.focused ?: false,
            )
        if (state != null && state.focused) {
            connection.applyNativeState(state)
        }
        inputConnection = connection
        return connection
    }

    /** The session's authoritative editing push — the connection adopts it. */
    internal fun applyEditingState(json: String) {
        val state = EditingStatePayload(json)
        val contract = InputContract.from(state)
        if (contract != inputContract) {
            // Posted, not called here. This push runs inside the session
            // borrow; restartInput re-enters nativeEditingState on this
            // thread, which would alias that borrow.
            if (!inputRestartPosted) {
                inputRestartPosted = true
                post {
                    inputRestartPosted = false
                    // A connection created in the meantime already recorded
                    // the contract it was built with. Restarting again would
                    // only drop the IME's first keystrokes.
                    val latest =
                        session
                            ?.let { NativeBridge.nativeEditingState(it.nativePtr) }
                            ?.let(::EditingStatePayload)
                    if (InputContract.from(latest) == inputContract) return@post
                    val imm =
                        context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
                    imm.restartInput(this)
                }
            }
            return
        }
        inputConnection?.applyNativeState(state)
    }

    /** The session's subscribed cursor-anchor push. */
    internal fun applyCursorAnchorInfo(json: String) {
        inputConnection?.applyCursorAnchorInfo(AnchorInfoPayload(json))
    }

    /**
     * Shows or hides the soft keyboard. The session calls this when a field
     * gains or loses focus and when a press lands on the focused field, never
     * per frame: the input contract and candidate geometry travel on the
     * editing-state and cursor-anchor pushes.
     */
    internal fun setSoftInputVisible(visible: Boolean) {
        val imm =
            context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        if (visible) {
            if (!hasFocus()) requestFocus()
            imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
        } else {
            imm.hideSoftInputFromWindow(windowToken, 0)
        }
    }

    /**
     * The view-to-screen transform `CursorAnchorInfo` positions are mapped
     * through: every ancestor's transform down to the window
     * ([transformMatrixToGlobal]), then the window's offset on screen.
     */
    internal fun viewToScreenMatrix(): android.graphics.Matrix {
        val matrix = android.graphics.Matrix()
        transformMatrixToGlobal(matrix)
        val windowOrigin = IntArray(2)
        rootView.getLocationOnScreen(windowOrigin)
        matrix.postTranslate(windowOrigin[0].toFloat(), windowOrigin[1].toFloat())
        return matrix
    }

    // ------------------------------------------------------------------
    // Accessibility + autofill — adapters backed by the session's snapshot.

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider =
        accessibilityProvider

    /**
     * Explore-by-touch: TalkBack injects hover events to find the node under
     * the pointer; without a dispatch here they die in the ViewGroup and a
     * tap activates instead of focusing (#246). The platform overlay fills
     * the host, so super would claim every point — a mounted platform view
     * keeps the hover traffic only inside its own slot bounds, and
     * everywhere else the provider maps the point onto the served virtual
     * tree.
     */
    override fun dispatchHoverEvent(event: MotionEvent): Boolean {
        if (!platformViewRegistry.coversPixel(event.x, event.y)) {
            accessibilityProvider.dispatchHoverEvent(event)
            return true
        }
        val handled = super.dispatchHoverEvent(event)
        if (handled) accessibilityProvider.clearHovered()
        return handled
    }

    internal fun notifyAccessibilityTreeChanged(diffJson: String) {
        accessibilityProvider.notifyTreeChanged(diffJson)
    }

    /** The session pushed a new placement set — pull and apply it. */
    internal fun notifyPlatformViewsChanged() {
        platformViewRegistry.notifyChanged()
    }

    /** Each a11y publish also drives the autofill enter/exit diff. */
    internal fun autofillSnapshotChanged() {
        autofillBridge.snapshotChanged()
    }

    /** The activity finishing commits the pending autofill save. */
    internal fun autofillCommit() {
        autofillBridge.commit()
    }

    internal fun autofillCancel() {
        autofillBridge.cancel()
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

/**
 * The slice of an editing-state push that [EditorInfo] is built from.
 * Text and selection stay on the live connection; the IME reads these
 * only when the connection is created.
 */
private data class InputContract(
    val focused: Boolean,
    val editorId: Long,
    val password: Boolean,
    val singleLine: Boolean,
    val hasSubmit: Boolean,
) {
    companion object {
        fun from(state: EditingStatePayload?): InputContract =
            if (state == null) {
                InputContract(
                    focused = false,
                    editorId = 0L,
                    password = false,
                    singleLine = false,
                    hasSubmit = false,
                )
            } else {
                InputContract(
                    focused = state.focused,
                    editorId = state.editorId,
                    password = state.password,
                    singleLine = state.singleLine,
                    hasSubmit = state.hasSubmit,
                )
            }
    }
}
