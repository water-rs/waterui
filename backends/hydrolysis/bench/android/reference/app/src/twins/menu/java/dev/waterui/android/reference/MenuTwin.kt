package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

// Twin of examples/menu, first screen: menu trigger sections.
//
// The backend renders `Menu::new` as a FrameLayout wrapping the label view
// with a click -> PopupMenu trigger — the settled frame is just the plain
// label text (ripple only appears on press), so the twin draws the same
// unstyled labels. `MutedForeground` maps to onSurfaceVariant.
//
//   Menu("Choose an Option")      -> plain body Text (trigger surface)
//   Menu(text("Actions").bold())  -> bold Text trigger
//   context_menu box              -> Text + padding(24) + background/foreground

private val ORANGE_BG = Color(0xFFFFF3E0)
private val ORANGE_FG = Color(0xFFE65100)

@Composable
private fun MenuSection(content: @Composable () -> Unit) {
    VStack(modifier = Modifier.padding(WATERUI_PADDING.dp), content = content)
}

@Composable
fun MenuTwin() {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Column(
        modifier = Modifier
            .verticalScroll(rememberScrollState())
            .padding(16.dp)
            .fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        VStack {
            HeadlineText("WaterUI Menu Examples")
            BodyText(
                "Demonstrating popup menus, nested menus, and context menus",
                color = muted,
            )
            HorizontalDivider()
            Spacer(Modifier.height(8.dp))
            MenuSection {
                SubheadlineText("Menu Component")
                BodyText(
                    "Tap the menu button to see nested buttons, submenus, and separators",
                    color = muted,
                )
                Spacer(Modifier.height(12.dp))
                BodyText("Choose an Option")
                Spacer(Modifier.height(12.dp))
                HStack {
                    CaptionText("Selected: ", color = muted)
                    BodyText("None")
                }
            }
            HorizontalDivider()
            MenuSection {
                SubheadlineText("Styled Menu")
                BodyText(
                    "The popup label stays a normal label, and menu rows can now be plain buttons",
                    color = muted,
                )
                Spacer(Modifier.height(12.dp))
                BoldText("Actions")
                Spacer(Modifier.height(12.dp))
                CaptionText("No action yet", color = muted)
            }
            HorizontalDivider()
            MenuSection {
                SubheadlineText("Context Menu")
                BodyText("Long press the box below to see context menu", color = muted)
                Spacer(Modifier.height(12.dp))
                BodyText(
                    "Long Press Me",
                    color = ORANGE_FG,
                    modifier = Modifier
                        .background(ORANGE_BG)
                        .padding(24.dp),
                )
                Spacer(Modifier.height(12.dp))
                CaptionText("No action yet", color = muted)
            }
            HorizontalDivider()
            MenuSection {
                SubheadlineText("Context Menu on Views")
                BodyText("Long press any colored box", color = muted)
            }
        }
    }
}
