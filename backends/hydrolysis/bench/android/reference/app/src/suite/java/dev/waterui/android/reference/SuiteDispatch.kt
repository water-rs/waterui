package dev.waterui.android.reference

import androidx.compose.runtime.Composable

// Suite dispatch — the rooted-suite reference application.
//
// The suite APK carries every twin (it is the whole-app package comparator),
// so its fixtureContent maps the E2EExample intent extra through twinFor.
// Per-fixture flavors instead ship a FixtureEntry.kt that statically selects
// their one twin, which is what lets R8 prune the others.

/** The twin for `example`, or null when none is registered. */
fun twinFor(example: String): (@Composable () -> Unit)? =
    when (example) {
        "anchored_overlay" -> ({ AnchoredOverlayTwin() })
        "animation" -> ({ AnimationTwin() })
        "drag_drop" -> ({ DragDropTwin() })
        "edge_layout" -> ({ EdgeLayoutTwin() })
        "edge_list" -> ({ EdgeListTwin() })
        "edge_text" -> ({ EdgeTextTwin() })
        "filter" -> ({ FilterTwin() })
        "flow_markdown" -> ({ FlowMarkdownTwin() })
        "form" -> ({ FormTwin() })
        "gallery" -> ({ GalleryTwin() })
        "gesture" -> ({ GestureTwin() })
        "gradient" -> ({ GradientTwin() })
        "hover" -> ({ HoverTwin() })
        "icons" -> ({ IconsTwin() })
        "list" -> ({ ListTwin() })
        "locale" -> ({ LocaleTwin() })
        "map" -> ({ MapTwin() })
        "markdown" -> ({ MarkdownTwin() })
        "media_picker" -> ({ MediaPickerTwin() })
        "menu" -> ({ MenuTwin() })
        "multi_window" -> ({ MultiWindowTwin() })
        "navigation" -> ({ NavigationTwin() })
        "picker" -> ({ PickerTwin() })
        "reminders" -> ({ RemindersTwin() })
        "reply" -> ({ ReplyTwin() })
        "shape" -> ({ ShapeTwin() })
        "snackbar" -> ({ SnackbarTwin() })
        "starfield" -> ({ StarfieldTwin() })
        "stress" -> ({ StressTwin() })
        "typography-rtl" -> ({ TypographyRtlTwin() })
        "video_player" -> ({ VideoPlayerTwin() })
        "waterkit_camera_filters" -> ({ WaterkitCameraFiltersTwin() })
        "webview" -> ({ WebviewTwin() })
        "editing" -> ({ EditingTwin() })
        "list-stress" -> ({ ListStressTwin() })
        else -> null
    }

// Section-6 coverage screens that are not frozen-inventory fixtures: the
// editing suite and the deterministic 1,000-row list stress case. They are
// reachable through the suite app (E2EExample) and the shared interaction
// specs, but carry no per-fixture variant of their own.
val SUITE_EXTRA_SCREENS = setOf("editing", "list-stress")

/** Suite flavor's entry: dispatch the example named by the launch intent. */
fun fixtureContent(example: String): (@Composable () -> Unit)? =
    twinFor(example)
