package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.FavoriteBorder
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Star
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/**
 * Compose twin of the icons fixture: icon packs (the fixture's Material
 * Design and Lucide sets map to Compose's material-icons equivalents), plus
 * the tinted 32x32 row.
 */
@Composable
fun IconsTwin() {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Icons")

        HeadlineText("Material Design Icons")
        IconRow(
            listOf(
                Icons.Filled.Home to "mdi:home",
                Icons.Filled.AccountCircle to "mdi:account",
                Icons.Filled.Settings to "mdi:cog",
                Icons.Filled.Favorite to "mdi:heart",
                Icons.Filled.Star to "mdi:star",
            ),
        )

        HeadlineText("Lucide Icons")
        IconRow(
            listOf(
                Icons.Outlined.Home to "lucide:house",
                Icons.Outlined.Person to "lucide:user",
                Icons.Outlined.Settings to "lucide:settings",
                Icons.Outlined.FavoriteBorder to "lucide:heart",
                Icons.Outlined.Star to "lucide:star",
            ),
        )

        HeadlineText("Colored Icons")
        IconRow(
            listOf(
                Icons.Filled.Favorite to "color:heart",
                Icons.Filled.Star to "color:star",
                Icons.Filled.CheckCircle to "color:check",
                Icons.Filled.Info to "color:info",
            ),
            listOf(
                Color(0xFFEF4444), Color(0xFFF59E0B),
                Color(0xFF10B981), Color(0xFF3B82F6),
            ),
        )
    }
}

@Composable
private fun IconRow(icons: List<Pair<ImageVector, String>>, tints: List<Color>? = null) {
    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        icons.forEachIndexed { i, (icon, tag) ->
            Icon(
                icon,
                contentDescription = tag,
                modifier = Modifier
                    .size(if (tints != null) 32.dp else 24.dp)
                    .testTag("icon:$tag"),
                tint = tints?.get(i) ?: Color.Unspecified,
            )
        }
    }
}
