package dev.waterui.android.macrobenchmark

import android.content.Intent
import android.os.Bundle
import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.ExperimentalMacrobenchmarkApi
import androidx.test.shell.Shell
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.waterui.android.bench.SuiteRegistry
import java.io.Closeable
import java.util.UUID
import org.json.JSONObject

/** Explicit deployment and content readiness for either benchmark candidate. */
class BenchmarkTarget(args: Bundle, private val fixture: String) : Closeable {
    private val externalPackage = args.getString("target_package")
    val packageName: String = externalPackage ?: SuiteRegistry.packageForFixture(fixture)
    private val readyText = args.getString("ready_text")
    private var environmentId: String? = null
    @OptIn(ExperimentalMacrobenchmarkApi::class)
    val compilationMode: CompilationMode =
        when (val mode = args.getString("compilation")) {
            "baseline-profile" -> CompilationMode.Partial(BaselineProfileMode.Require)
            "precompiled" -> CompilationMode.Ignore()
            "none" -> CompilationMode.None()
            null -> {
                require(externalPackage == null) {
                    "An explicit target requires compilation=baseline-profile, precompiled, or none"
                }
                CompilationMode.Partial(BaselineProfileMode.Require)
            }
            else -> error("Unknown compilation mode: $mode")
        }

    init {
        require(externalPackage == null || !readyText.isNullOrBlank()) {
            "An explicit target requires ready_text identifying rendered content"
        }
    }

    /** Provider writes can start the process, so this runs only in setupBlock. */
    fun prepareLaunch() {
        if (externalPackage == null) return
        clearEnvironment()
        val id = UUID.randomUUID().toString()
        environmentId = id
        val payload = JSONObject(mapOf("WATERUI_DISABLE_DYNAMIC_COLORS" to "1")).toString()
        val output =
            Shell.command(
                "printf %s ${shellQuote(payload)} | content write --uri ${shellQuote(environmentUri(id))}"
            )
        check(output.stdOut.isBlank() && output.stdErr.isBlank()) {
            "Could not stage the benchmark launch environment"
        }
    }

    fun configureLaunch(intent: Intent, markers: Boolean = false) {
        intent.putExtra("E2EExample", SuiteRegistry.exampleForFixture(fixture))
        intent.putExtra("waterui.bench.markers", markers)
        if (externalPackage == null) {
            // The Compose reference owns this explicit benchmark-only protocol.
            intent.putExtra("waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS", "1")
        } else {
            intent.putExtra(
                "waterui.launch.environment",
                checkNotNull(environmentId) {
                    "Prepare the launch environment in setupBlock before measuring"
                },
            )
        }
    }

    private fun shellQuote(value: String): String = "'" + value.replace("'", "'\"'\"'") + "'"

    private fun environmentUri(id: String): String =
        "content://$packageName.waterui.launch-environment/$id"

    override fun close() {
        try {
            clearEnvironment()
        } finally {
            if (externalPackage != null) {
                val output = Shell.command("am force-stop ${shellQuote(packageName)}")
                check(output.stdOut.isBlank() && output.stdErr.isBlank()) {
                    "Could not stop the benchmark target after environment cleanup"
                }
            }
        }
    }

    private fun clearEnvironment() {
        val id = environmentId ?: return
        val output = Shell.command("content delete --uri ${shellQuote(environmentUri(id))}")
        check(output.stdErr.isBlank() && output.stdOut.isBlank()) {
            "Could not remove the benchmark launch environment: stdout=${output.stdOut}, stderr=${output.stdErr}"
        }
        environmentId = null
    }

    fun awaitContent(device: UiDevice) {
        val selector =
            if (readyText != null) By.pkg(packageName).text(readyText)
            else By.pkg(packageName).res("screen-root")
        check(device.wait(Until.hasObject(selector), 10_000)) {
            "Benchmark content did not become ready in $packageName"
        }
    }
}
