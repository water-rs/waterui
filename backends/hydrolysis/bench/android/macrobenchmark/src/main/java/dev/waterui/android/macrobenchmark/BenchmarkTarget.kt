package dev.waterui.android.macrobenchmark

import android.os.Bundle
import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.waterui.android.bench.SuiteRegistry

/** Explicit deployment and content readiness for either benchmark candidate. */
class BenchmarkTarget(args: Bundle, fixture: String) {
    private val externalPackage = args.getString("target_package")
    val packageName: String = externalPackage ?: SuiteRegistry.packageForFixture(fixture)
    private val readyText = args.getString("ready_text")
    val compilationMode: CompilationMode = when (val mode = args.getString("compilation")) {
        "baseline-profile" -> CompilationMode.Partial(BaselineProfileMode.Require)
        "none" -> CompilationMode.None()
        null -> {
            require(externalPackage == null) { "An explicit target requires compilation=baseline-profile or none" }
            CompilationMode.Partial(BaselineProfileMode.Require)
        }
        else -> error("Unknown compilation mode: $mode")
    }

    init {
        require(externalPackage == null || !readyText.isNullOrBlank()) {
            "An explicit target requires ready_text identifying rendered content"
        }
    }

    fun awaitContent(device: UiDevice) {
        val selector = if (readyText != null) By.pkg(packageName).text(readyText)
            else By.pkg(packageName).res("screen-root")
        check(device.wait(Until.hasObject(selector), 10_000)) {
            "Benchmark content did not become ready in $packageName"
        }
    }
}
