package dev.waterui.widgets

import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import java.io.File

/**
 * Android host bridge for WaterUI home-screen widgets.
 *
 * Supplies the shared persistent widget store directory inside the app's
 * private storage (`filesDir/waterui-widgets`) and dispatches timeline reload
 * updates to installed `AppWidgetProvider` and Jetpack Glance
 * `GlanceAppWidgetReceiver` components belonging to the application.
 */
object WaterUIWidgetHost {
    private const val STORE_DIR_NAME = "waterui-widgets"
    private const val META_WIDGET_KIND = "dev.waterui.widgets.kind"
    const val EXTRA_WIDGET_KIND = "dev.waterui.widgets.EXTRA_WIDGET_KIND"

    @JvmStatic
    fun sharedStorePath(context: Context): String {
        val dir = File(context.applicationContext.filesDir, STORE_DIR_NAME)
        if (!dir.exists()) {
            dir.mkdirs()
        }
        return dir.absolutePath
    }

    @JvmStatic
    fun reload(context: Context, kind: String) {
        if (kind.isEmpty()) {
            reloadAll(context)
            return
        }
        val appContext = context.applicationContext
        val manager = AppWidgetManager.getInstance(appContext)
        for (provider in matchingProviders(appContext, manager, kind)) {
            dispatchUpdate(appContext, manager, provider, kind)
        }
    }

    @JvmStatic
    fun reloadAll(context: Context) {
        val appContext = context.applicationContext
        val manager = AppWidgetManager.getInstance(appContext)
        for (provider in matchingProviders(appContext, manager, kind = null)) {
            dispatchUpdate(appContext, manager, provider, kind = null)
        }
    }

    private fun matchingProviders(
        context: Context,
        manager: AppWidgetManager,
        kind: String?,
    ): List<ComponentName> {
        val packageName = context.packageName
        val providers = manager.installedProviders.filter { info ->
            info.provider.packageName == packageName
        }
        if (kind == null) {
            return providers.map { it.provider }
        }
        val matched = providers.filter { info ->
            val declaredKind = receiverWidgetKind(context, info.provider)
            declaredKind == kind || info.provider.shortClassName.substringAfterLast('.') == kind
        }
        return matched.map { it.provider }
    }

    private fun receiverWidgetKind(context: Context, component: ComponentName): String? {
        return try {
            val info = context.packageManager.getReceiverInfo(
                component,
                PackageManager.GET_META_DATA,
            )
            info.metaData?.getString(META_WIDGET_KIND)
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }
    }

    private fun dispatchUpdate(
        context: Context,
        manager: AppWidgetManager,
        provider: ComponentName,
        kind: String?,
    ) {
        val ids = manager.getAppWidgetIds(provider)
        if (ids.isEmpty()) {
            return
        }
        val intent = Intent(AppWidgetManager.ACTION_APPWIDGET_UPDATE).apply {
            component = provider
            putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids)
            if (kind != null) {
                putExtra(EXTRA_WIDGET_KIND, kind)
            }
        }
        context.sendBroadcast(intent)
    }
}
