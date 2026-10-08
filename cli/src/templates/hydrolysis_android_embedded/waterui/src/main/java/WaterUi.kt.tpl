package {{ ctx.android_package_name() }}.waterui

import android.content.Context
import android.view.View
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedDispatcher
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ViewModelStoreOwner
import dev.waterui.hydrolysis.HydrolysisEmbedding
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.HydrolysisSession
{%- if ctx.hydrolysis_android_has_painter_band() %}
import {{ ctx.hydrolysis_android_painter_band_import() }}
{%- endif %}

/** The entry point a host app mounts this WaterUI library through. */
object WaterUi {

    /** The `System.loadLibrary` name of the app's Hydrolysis cdylib. */
    const val NATIVE_LIBRARY: String = "{{ ctx.hydrolysis_android_embedded().app.native_library_name }}"

    /** Mounts the library's app as a View owned by [activity]. */
    fun createView(
        activity: ComponentActivity,
        onCloseRequested: () -> Unit,
        key: String = NATIVE_LIBRARY,
    ): View =
        createView(
            activity,
            activity,
            activity,
            activity.onBackPressedDispatcher,
            onCloseRequested,
            key,
        )

    /**
     * Mounts the library's app for any owner set (a Fragment passes its
     * viewLifecycleOwner). Two mounts under the same owner need distinct
     * [key]s — each key retains its own session.
     */
    fun createView(
        context: Context,
        lifecycleOwner: LifecycleOwner,
        viewModelStoreOwner: ViewModelStoreOwner,
        onBackPressedDispatcher: OnBackPressedDispatcher,
        onCloseRequested: () -> Unit,
        key: String = NATIVE_LIBRARY,
    ): View = HydrolysisEmbedding.createView(
        context,
        lifecycleOwner,
        viewModelStoreOwner,
        onBackPressedDispatcher,
        NATIVE_LIBRARY,
        onCloseRequested,
        key = key,
        createContentView = { session ->
            {% include "partials/hydrolysis_android_content_view.kt.tpl" %}
        },
    )
}
