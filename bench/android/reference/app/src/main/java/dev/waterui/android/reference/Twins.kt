package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

// Twin registry and shared translation helpers.
//
// Each twin is a hand-written Compose + Material 3 reproduction of a waterui
// example's first screen. Layout follows WaterUI's SwiftUI-derived semantics —
// it is intentionally NOT Compose's; only component behavior and styling must
// match MD3:
//
//   vstack/hstack bare            -> Column/Row spacedBy 10.dp (WaterUI default)
//   .padding()                     -> padding(14.dp) (WaterUI default padding)
//   .padding_with(all(n))          -> padding(n.dp)
//   .padding_with(symmetric(v,h))  -> padding(vertical=v.dp, horizontal=h.dp)
//   .width/.height                 -> width/height(n.dp)
//   scroll(...)                    -> verticalScroll + inner Column
//   Divider                        -> HorizontalDivider()
//   spacer()                       -> Spacer(weight) — bare spacer expands
//
// Text styles map SwiftUI's semantic slots onto the nearest MD3 typography
// role (divergences from the backend's own mapping are findings, not things
// to paper over):
//
//   .title()       -> headlineMedium  (SwiftUI 28pt ≈ 28sp)
//   .headline()    -> titleMedium     (semibold ≈ medium 16sp)
//   .subheadline() -> titleSmall      (SwiftUI 15pt ≈ 14sp medium)
//   body           -> bodyLarge       (SwiftUI 17pt ≈ 16sp)
//   .footnote()    -> bodySmall       (SwiftUI 13pt ≈ 12sp)
//   .caption()     -> labelSmall      (SwiftUI 12pt ≈ 11sp)
//   .bold()        -> FontWeight.Bold
//
//   Srgb::from_hex("#RRGGBB")      -> Color(0xFFRRGGBB)
//   .with_opacity(x)               -> copy(alpha = x)

/** The twin for `example`, or null when none is registered. */
fun twinFor(example: String): (@Composable () -> Unit)? =
    when (example) {
        "form" -> ({ FormTwin() })
        "gesture" -> ({ GestureTwin() })
        "hover" -> ({ HoverTwin() })
        "list" -> ({ ListTwin() })
        "menu" -> ({ MenuTwin() })
        "picker" -> ({ PickerTwin() })
        "typography-rtl" -> ({ TypographyRtlTwin() })
        else -> null
    }

const val WATERUI_SPACING = 10
const val WATERUI_PADDING = 14

/** `vstack` — WaterUI's bare stack spacing, centered like the backend lays out. */
@Composable
fun VStack(
    modifier: Modifier = Modifier,
    horizontalAlignment: Alignment.Horizontal = Alignment.CenterHorizontally,
    content: @Composable () -> Unit,
) {
    Column(
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
        horizontalAlignment = horizontalAlignment,
        modifier = modifier.fillMaxWidth(),
    ) { content() }
}

/** `hstack` — WaterUI's bare stack spacing, vertically centered. */
@Composable
fun HStack(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
        verticalAlignment = Alignment.CenterVertically,
        modifier = modifier,
    ) { content() }
}

@Composable
fun TitleText(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(text, style = MaterialTheme.typography.headlineMedium, modifier = modifier, color = color)
}

@Composable
fun HeadlineText(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(text, style = MaterialTheme.typography.titleMedium, modifier = modifier, color = color)
}

@Composable
fun SubheadlineText(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(text, style = MaterialTheme.typography.titleSmall, modifier = modifier, color = color)
}

@Composable
fun BodyText(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(text, style = MaterialTheme.typography.bodyLarge, modifier = modifier, color = color)
}

@Composable
fun FootnoteText(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(text, style = MaterialTheme.typography.bodySmall, modifier = modifier, color = color)
}

@Composable
fun CaptionText(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) {
    Text(text, style = MaterialTheme.typography.labelSmall, modifier = modifier, color = color)
}

@Composable
fun BoldText(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge,
        fontWeight = FontWeight.Bold,
        modifier = modifier,
    )
}
