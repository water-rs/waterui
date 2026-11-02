package dev.waterui.hydrolysis

import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.internal.DoNotInstrument

/**
 * The `uiMode` → colour-scheme channel (water-rs/waterui#3544): the session
 * create carries the real night flag, every later `onConfigurationChanged`
 * reports the new one, and a retained session rebinding under a changed
 * configuration pushes the current scheme.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    shadows = [ShadowNativeBridge::class],
    // The shadow can only replace `NativeBridge`'s natives on an
    // instrumented class.
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class ColorSchemeReportTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    @Before
    fun reset() = ShadowNativeBridge.reset()

    @Test
    fun theNightMaskMapsToTheScheme() {
        fun config(uiMode: Int) = Configuration().apply { this.uiMode = uiMode }
        assertTrue(config(Configuration.UI_MODE_NIGHT_YES).isNightMode)
        assertFalse(config(Configuration.UI_MODE_NIGHT_NO).isNightMode)
        assertFalse(config(Configuration.UI_MODE_NIGHT_UNDEFINED).isNightMode)
        // The type half of `uiMode` never disturbs the night flag.
        assertTrue(
            config(Configuration.UI_MODE_TYPE_WATCH or Configuration.UI_MODE_NIGHT_YES).isNightMode,
        )
    }

    @Test
    fun theSessionCreateCarriesTheRealNightMode() {
        HydrolysisSession(context, onCloseRequested = {})
        RuntimeEnvironment.setQualifiers("night")
        HydrolysisSession(context, onCloseRequested = {})
        assertEquals(listOf(false, true), ShadowNativeBridge.createNightModes)
    }

    @Test
    fun aConfigurationChangeReportsTheNewNightMode() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val session = HydrolysisSession(activity, onCloseRequested = {})
        val host = HydrolysisHostView(activity, session)
        activity.setContentView(host)
        assertTrue(host.isAttachedToWindow)
        // The bind itself pushed the create-time scheme once.
        assertEquals(listOf(false), ShadowNativeBridge.colorSchemes)

        val night =
            Configuration(activity.resources.configuration).apply {
                uiMode =
                    (uiMode and Configuration.UI_MODE_NIGHT_MASK.inv()) or
                    Configuration.UI_MODE_NIGHT_YES
            }
        // The framework entry a configuration change reaches the view by:
        // `Activity` dispatches it to the decor view, which lands on each
        // child's `onConfigurationChanged`.
        host.dispatchConfigurationChanged(night)

        assertEquals(listOf(false, true), ShadowNativeBridge.colorSchemes)
        session.destroy()
    }

    /**
     * A session retained across recreation — the host declared no `uiMode`
     * config change — binds the new view under the new configuration, so the
     * scheme re-publishes at bind rather than waiting for a dispatch that
     * never comes.
     */
    @Test
    fun aReboundSessionReportsTheNewConfiguration() {
        val first = Robolectric.buildActivity(Activity::class.java).setup().get()
        val session = HydrolysisSession(first, onCloseRequested = {})
        val firstView = HydrolysisHostView(first, session)
        first.setContentView(firstView)
        (firstView.parent as android.view.ViewGroup).removeView(firstView)
        assertEquals(listOf(false), ShadowNativeBridge.colorSchemes)

        // The recreated activity's configuration carries night; the new
        // view's bind pushes it even though no `onConfigurationChanged`
        // ever ran.
        RuntimeEnvironment.setQualifiers("night")
        val second = Robolectric.buildActivity(Activity::class.java).setup().get()
        val secondView = HydrolysisHostView(second, session)
        second.setContentView(secondView)

        assertEquals(listOf(false, true), ShadowNativeBridge.colorSchemes)
        session.destroy()
    }
}
