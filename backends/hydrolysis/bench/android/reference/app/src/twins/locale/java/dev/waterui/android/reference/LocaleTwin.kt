package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * The fixture's i18n/<locale>.toml bundles, loaded verbatim from assets.
 * The format is deliberately tiny — ` "key" = "value"` lines plus
 * `{ one = "…", other = "…" }` plural tables — so the parser is a couple
 * of lines, matching what the fixture's build script extracts.
 */
private class StringTable(entries: Map<String, String>, private val plurals: Map<String, Pair<String, String>>) {
    private val entries = entries

    fun get(key: String, fallback: String): String = entries[key] ?: fallback

    fun plural(key: String, count: Int): String {
        val forms = plurals[key] ?: return key
        val template = if (count == 1) forms.first else forms.second
        return template.replace("{count}", count.toString())
    }
}

private fun parseToml(text: String): StringTable {
    val entries = mutableMapOf<String, String>()
    val plurals = mutableMapOf<String, Pair<String, String>>()
    val kv = Regex("""^"((?:[^"\\]|\\.)*)"\s*=\s*(.+?)\s*$""")
    val pluralRe = Regex("""\{[^}]*one\s*=\s*"((?:[^"\\]|\\.)*)"[^}]*other\s*=\s*"((?:[^"\\]|\\.)*)"[^}]*\}""")
    val strRe = Regex("""^"((?:[^"\\]|\\.)*)"\s*$""")
    for (line in text.lines()) {
        val m = kv.matchEntire(line.trim()) ?: continue
        val key = m.groupValues[1]
        val value = m.groupValues[2]
        val pm = pluralRe.matchEntire(value)
        when {
            pm != null -> plurals[key] = pm.groupValues[1] to pm.groupValues[2]
            else -> strRe.matchEntire(value)?.let { entries[key] = it.groupValues[1] }
        }
    }
    return StringTable(entries, plurals)
}

private val PICKER_LOCALES = listOf(
    "en-US" to "English (US)",
    "en-GB" to "English (UK)",
    "zh" to "中文 (简体)",
    "zh-TW" to "中文 (台灣)",
    "zh-HK" to "中文 (香港)",
    "ja" to "日本語",
    "ko" to "한국어",
    "de" to "Deutsch",
    "fr" to "Français",
    "es" to "Español",
    "ru" to "Русский",
)

private fun javaLocale(tag: String): Locale = Locale.forLanguageTag(tag)

/**
 * Compose twin of the locale fixture: the world-fair kiosk — Language Booth
 * picker, Welcome Desk greeting, UDHR article, Local Flavor, Passport Stamps
 * plurals, Festival Date and Distance Guide, all rendered from the same
 * bundled i18n toml strings through java.util.Locale formatting.
 */
@Composable
fun LocaleTwin() {
    val context = LocalContext.current
    val systemLocale = Locale.getDefault().toLanguageTag()

    var selected by remember { mutableStateOf("en-US") }
    var expanded by remember { mutableStateOf(false) }

    val table = remember(selected) {
        val candidates = buildList {
            add("i18n/$selected.toml")
            selected.substringBefore('-').takeIf { it != selected }
                ?.let { add("i18n/$it.toml") }
            add("i18n/en.toml")
        }
        candidates.firstNotNullOfOrNull { name ->
            runCatching {
                context.assets.open(name).bufferedReader().use { parseToml(it.readText()) }
            }.getOrNull()
        } ?: StringTable(emptyMap(), emptyMap())
    }
    val locale = javaLocale(selected)

    fun tr(key: String) = table.get(key, key)

    val kickoff = remember(selected) {
        val fmt = java.text.DateFormat.getDateTimeInstance(
            java.text.DateFormat.LONG, java.text.DateFormat.LONG, locale,
        )
        fmt.timeZone = TimeZone.getDefault()
        fmt.format(Date(1142830200000L)) // 2006-03-20T09:30 local
    }
    val dateShort = remember(selected) {
        java.text.DateFormat.getDateInstance(java.text.DateFormat.SHORT, locale)
            .format(Date(1142830200000L))
    }
    val dateLong = remember(selected) {
        java.text.DateFormat.getDateInstance(java.text.DateFormat.LONG, locale)
            .format(Date(1142830200000L))
    }
    val cityWalk = remember(selected) {
        java.text.NumberFormat.getNumberInstance(locale).format(1500) + " m"
    }
    val marathon = remember(selected) {
        java.text.NumberFormat.getNumberInstance(locale).format(42.195) + " km"
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        Column(Modifier.testTag("locale:booth")) {
            HeadlineText(tr("Language Booth"))
            Row {
                Text("${tr("Detected Locale:")} ")
                Text(systemLocale)
            }
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Text("${tr("Chosen Language:")} ")
                Button(
                    onClick = {
                        BenchMarkers.tap()
                        expanded = true
                    },
                    modifier = Modifier.testTag("locale:picker"),
                ) { Text(PICKER_LOCALES.first { it.first == selected }.second) }
                DropdownMenu(
                    expanded = expanded,
                    onDismissRequest = { expanded = false },
                ) {
                    for ((tag, label) in PICKER_LOCALES) {
                        DropdownMenuItem(
                            text = { Text(label) },
                            onClick = {
                                BenchMarkers.tap()
                                selected = tag
                                expanded = false
                            },
                            modifier = Modifier.testTag("locale:$tag"),
                        )
                    }
                }
            }
        }

        HorizontalDivider()

        Column(Modifier.testTag("locale:welcome")) {
            HeadlineText(tr("Welcome Desk"))
            Text(tr("Welcome to the World Fair!"), style = MaterialTheme.typography.headlineSmall)
        }

        HorizontalDivider()

        Column(Modifier.testTag("locale:udhr")) {
            HeadlineText(tr("Human Rights - Article 1"))
            BodyText(tr("\$udhr_article_1"))
        }

        HorizontalDivider()

        Column(Modifier.testTag("locale:flavor")) {
            HeadlineText(tr("Local Flavor"))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Spacer(Modifier.weight(1f)); Text(tr("Color")); Spacer(Modifier.weight(1f))
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Spacer(Modifier.weight(1f)); Text(tr("Favorite")); Spacer(Modifier.weight(1f))
            }
        }

        HorizontalDivider()

        Column(Modifier.testTag("locale:plurals")) {
            HeadlineText(tr("Passport Stamps"))
            for (count in listOf(0, 1, 2, 5)) {
                Text(table.plural("I have {#count} passport stamp", count))
            }
        }

        HorizontalDivider()

        Column(Modifier.testTag("locale:dates")) {
            HeadlineText(tr("Festival Date (2006-03-20)"))
            Row { Text("${tr("Short:")} "); Spacer(Modifier.weight(1f)); Text(dateShort) }
            Row { Text("${tr("Long:")} "); Spacer(Modifier.weight(1f)); Text(dateLong) }
            Row { Text("Timezone: "); Spacer(Modifier.weight(1f)); Text(TimeZone.getDefault().id) }
            Row { Text("${tr("Kickoff (TZ):")} "); Spacer(Modifier.weight(1f)); Text(kickoff) }
        }

        HorizontalDivider()

        Column(Modifier.testTag("locale:units")) {
            HeadlineText(tr("Distance Guide"))
            Row { Text("${tr("City Walk:")} "); Spacer(Modifier.weight(1f)); Text(cityWalk) }
            Row { Text("${tr("Marathon Route:")} "); Spacer(Modifier.weight(1f)); Text(marathon) }
        }
    }
}
