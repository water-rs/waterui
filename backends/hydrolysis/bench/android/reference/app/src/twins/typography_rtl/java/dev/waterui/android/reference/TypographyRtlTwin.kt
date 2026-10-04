package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.intl.Locale
import androidx.compose.ui.text.intl.LocaleList
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp

// Twin of examples/typography-rtl: semantic type styles, per-locale CJK
// specimens, and three localized reading-order panels (en LTR, ar RTL,
// he RTL). Panel background follows the backend's secondary surface; text
// roles map per the table in Twins.kt.

@Composable
private fun LocaleBody(localeTag: String, text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge.copy(
            localeList = LocaleList(Locale(localeTag)),
        ),
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun ReadingPanel(
    direction: LayoutDirection,
    title: String,
    body: String,
    arrow: String,
) {
    CompositionLocalProvider(
        LocalLayoutDirection provides direction,
    ) {
        Column(
            horizontalAlignment = Alignment.Start,
            modifier = Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .padding(16.dp),
        ) {
            SubheadlineText(title)
            Row(verticalAlignment = Alignment.CenterVertically) {
                HeadlineText("①")
                Column(
                    horizontalAlignment = Alignment.Start,
                    modifier = Modifier
                        .weight(1f)
                        .padding(horizontal = 12.dp),
                ) {
                    BodyText(body)
                    CaptionText("WaterUI 2.0 · 2026", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                HeadlineText(arrow)
            }
            Column(horizontalAlignment = Alignment.Start, modifier = Modifier.padding(top = 12.dp)) {
                BodyText("Name / الاسم / שם")
                TextField(
                    value = "",
                    onValueChange = {},
                    placeholder = { Text("Type here / اكتب هنا / הקלידו כאן") },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
fun TypographyRtlTwin() {
    Column(
        modifier = Modifier
            .verticalScroll(rememberScrollState())
            .padding(20.dp)
            .fillMaxWidth(),
        horizontalAlignment = Alignment.Start,
    ) {
        HeadlineText("Typography & Bidirectional Layout")
        BodyText(
            "Semantic type styles, CJK fallback, mixed-script shaping, and logical RTL layout.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        HorizontalDivider(Modifier.padding(vertical = 8.dp))

        Column(horizontalAlignment = Alignment.Start) {
            HeadlineText("Display / 展示 / عرض")
            TitleText("Title / 标题 / כותרת")
            SubheadlineText("Headline / 小标题 / عنوان")
            BodyText("Body — WaterUI shapes العربية、中文、日本語、한국어 in one paragraph.")
            FootnoteText("Footnote — Mixed scripts keep punctuation and numbers stable: الإصدار 2.0 (2026).")
            CaptionText("Caption — 字体 fallback follows locale and script.", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        HorizontalDivider(Modifier.padding(vertical = 8.dp))

        Column(horizontalAlignment = Alignment.Start) {
            SubheadlineText("CJK locale-aware glyph selection")
            LocaleBody("zh-CN", "简体中文：骨、直、门、关")
            LocaleBody("zh-TW", "繁體中文：骨、直、門、關")
            LocaleBody("ja", "日本語：骨、直、門、関")
            LocaleBody("ko", "한국어: 한글과 漢字")
        }
        HorizontalDivider(Modifier.padding(vertical = 8.dp))

        TitleText("Logical reading order")
        ReadingPanel(LayoutDirection.Ltr, "Left-to-right",
            "A logical HStack starts here", "→")
        Spacer(Modifier.padding(6.dp))
        ReadingPanel(LayoutDirection.Rtl, "من اليمين إلى اليسار",
            "يبدأ الصف المنطقي من هنا", "←")
        Spacer(Modifier.padding(6.dp))
        ReadingPanel(LayoutDirection.Rtl, "מימין לשמאל",
            "השורה הלוגית מתחילה כאן", "←")
    }
}
