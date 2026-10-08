package dev.waterui.hydrolysis

import android.annotation.SuppressLint
import android.content.Context
import android.util.SparseArray
import android.view.KeyCharacterMap
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

    /**
     * The overlay native children (embedded platform views) are laid into,
     * always above the GPU band. Populated from the session's placement frames
     * by [platformViewRegistry].
     */
    val platformViewRegistry: PlatformViewRegistry =
        PlatformViewRegistry(context, session, this)

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
     * Wheel axis values are normalized (about ±1 per notch); Android's own
     * scrolling views multiply them by this configuration's scaled scroll
     * factors to get pixels, and so does the host.
     */
    private val wheelConfiguration = ViewConfiguration.get(context)

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

    /**
     * The deferred `restartInput` [applyEditingState] posts. A connection
     * created in the meantime already recorded the contract it was built
     * with; restarting again would only drop the IME's first keystrokes.
     */
    private val restartInput = Runnable {
        inputRestartPosted = false
        if (InputContract.from(editingState()) == inputContract) return@Runnable
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.restartInput(this)
    }

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
        // Nothing this view queued may reach the session after the detach:
        // a destroyed session tears down inside `unbind` below. The IMM
        // closes its connections only later, from its own queue — they
        // refuse once this view is off its window — and a posted restart
        // would pull editing state.
        inputConnection = null
        removeCallbacks(restartInput)
        inputRestartPosted = false
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
        session.withNativePtr(NativeBridge::nativeSetMetrics.name) { ptr ->
            NativeBridge.nativeSetMetrics(
                ptr,
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
    // Frame scheduling — the session's scheduler, which stops with it.

    internal open fun requestFrame() {
        session?.frameScheduler?.requestFrame("redraw-request")
    }

    // ------------------------------------------------------------------
    // Close — native asks through the session bridge.

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
        // One scope for the event and its refresh demand: both reach the
        // session the event did.
        session.withNativePtr(NativeBridge::nativePointerEvent.name) { ptr ->
            NativeBridge.nativePointerEvent(
                ptr,
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
                    session.frameScheduler.setInteractionActive(true)
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL ->
                    session.frameScheduler.setInteractionActive(false)
            }
        }
        return true
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        val session = session ?: return super.onGenericMotionEvent(event)
        if (event.actionMasked == MotionEvent.ACTION_SCROLL) {
            // Android AXIS_HSCROLL is positive when content moves left; Hydrolysis
            // input takes winit's opposite sign.
            val dx =
                -event.getAxisValue(MotionEvent.AXIS_HSCROLL) *
                    wheelConfiguration.scaledHorizontalScrollFactor
            // Android AXIS_VSCROLL is positive when content moves down, matching
            // Hydrolysis's winit sign.
            val dy =
                event.getAxisValue(MotionEvent.AXIS_VSCROLL) *
                    wheelConfiguration.scaledVerticalScrollFactor
            session.withNativePtr(NativeBridge::nativeScrollEvent.name) { ptr ->
                NativeBridge.nativeScrollEvent(ptr, event.x, event.y, dx, dy)
            }
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
        session.withNativePtr(NativeBridge::nativeKeyEvent.name) { ptr ->
            NativeBridge.nativeKeyEvent(
                ptr,
                key,
                pressed,
                event.isShiftPressed,
                event.isCtrlPressed,
                event.isAltPressed,
                event.isMetaPressed,
            )
        }
        return true
    }

    /** Maps a hardware key onto the W3C `key` vocabulary the runner parses. */
    private fun w3cKey(event: KeyEvent): String? {
        // Named keys first: Enter and Tab also carry `unicodeChar` ('\n', '\t'),
        // which is not their W3C value.
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
            KeyEvent.KEYCODE_F1 -> "F1"
            KeyEvent.KEYCODE_F2 -> "F2"
            KeyEvent.KEYCODE_F3 -> "F3"
            KeyEvent.KEYCODE_F4 -> "F4"
            KeyEvent.KEYCODE_F5 -> "F5"
            KeyEvent.KEYCODE_F6 -> "F6"
            KeyEvent.KEYCODE_F7 -> "F7"
            KeyEvent.KEYCODE_F8 -> "F8"
            KeyEvent.KEYCODE_F9 -> "F9"
            KeyEvent.KEYCODE_F10 -> "F10"
            KeyEvent.KEYCODE_F11 -> "F11"
            KeyEvent.KEYCODE_F12 -> "F12"
            KeyEvent.KEYCODE_INSERT -> "Insert"
            KeyEvent.KEYCODE_SYSRQ -> "PrintScreen"
            KeyEvent.KEYCODE_BREAK -> "Pause"
            KeyEvent.KEYCODE_SCROLL_LOCK -> "ScrollLock"
            KeyEvent.KEYCODE_MENU -> "ContextMenu"
            KeyEvent.KEYCODE_NUMPAD_ENTER -> "Enter"
            KeyEvent.KEYCODE_SHIFT_LEFT, KeyEvent.KEYCODE_SHIFT_RIGHT -> "Shift"
            KeyEvent.KEYCODE_CTRL_LEFT, KeyEvent.KEYCODE_CTRL_RIGHT -> "Control"
            KeyEvent.KEYCODE_ALT_LEFT, KeyEvent.KEYCODE_ALT_RIGHT -> "Alt"
            KeyEvent.KEYCODE_META_LEFT, KeyEvent.KEYCODE_META_RIGHT -> "Meta"
            else -> {
                // `unicodeChar` resolves the key with the full meta state,
                // and the stock Generic.kcm defines no `ctrl`/`meta`
                // behaviour for letters, so a Ctrl/Meta chord resolves to 0
                // and the press would be dropped. Take the character with
                // Ctrl and Meta masked off; drop Alt only when the layout
                // maps no Alt character for the key.
                val stripped =
                    event.metaState and (KeyEvent.META_CTRL_MASK or KeyEvent.META_META_MASK).inv()
                val unicode =
                    event.getUnicodeChar(stripped).takeIf { it != 0 }
                        ?: event.getUnicodeChar(stripped and KeyEvent.META_ALT_MASK.inv())
                when {
                    // A dead key reports its spacing accent with
                    // COMBINING_ACCENT set; dead-key composition on the
                    // view path is a separate missing feature, so the W3C
                    // name goes through instead of a lone combining mark.
                    unicode and KeyCharacterMap.COMBINING_ACCENT != 0 -> "Dead"
                    unicode != 0 -> unicode.toChar().toString()
                    else -> null
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // IME — the text-input boundary the session pushes through the bridge.

    override fun onCheckIsTextEditor(): Boolean = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        // Bind to the authoritative session state — the pulled `editorId`
        // becomes this connection's generation token, and the state seeds
        // the mirror so the IMM's first queries agree with the editor.
        val state = editingState()
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

    /** The session's authoritative editing state, pulled synchronously. */
    private fun editingState(): EditingStatePayload? {
        val session = session ?: return null
        return session.withNativePtr(NativeBridge::nativeEditingState.name) { ptr ->
            NativeBridge.nativeEditingState(ptr)
        }?.let(::EditingStatePayload)
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
                post(restartInput)
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
            // `hasFocus` is also true while a platform-view child (the
            // WebView) holds focus — `isFocused` asks for this view itself,
            // which is what pulls the IME back to the Hydrolysis field.
            if (!isFocused) requestFocus()
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
