package dev.waterui.android.bench

/**
 * The frozen-inventory naming contract shared by the harness and the test
 * modules.
 *
 * The suite flavor installs as `dev.waterui.android.reference`; every
 * per-fixture variant installs as `dev.waterui.android.reference.<flavor>`
 * where `<flavor>` is the suite.toml fixture name with '-' mapped to '_'.
 * run.py, the macrobenchmark module and the baselineprofile module all map
 * through this file so the three never disagree.
 */
object SuiteRegistry {
    const val SUITE_PACKAGE = "dev.waterui.android.reference"

    /**
     * Suite-hosted screens that are not frozen-inventory fixtures: the
     * section-6 editing suite and the deterministic 1,000-row list stress
     * case. They launch through the suite application via E2EExample.
     */
    val SUITE_SCREENS = setOf("suite", "editing", "list-stress")

    fun fixtureForPackage(pkg: String): String =
        if (pkg == SUITE_PACKAGE) {
            "suite"
        } else {
            pkg.removePrefix("$SUITE_PACKAGE.").replace('_', '-')
        }

    fun packageForFixture(fixture: String): String =
        if (fixture in SUITE_SCREENS) {
            SUITE_PACKAGE
        } else {
            SUITE_PACKAGE + "." + fixture.replace('-', '_')
        }

    /** The E2EExample value that selects the screen inside its host app. */
    fun exampleForFixture(fixture: String): String =
        if (fixture == "suite") "list" else fixture
}
