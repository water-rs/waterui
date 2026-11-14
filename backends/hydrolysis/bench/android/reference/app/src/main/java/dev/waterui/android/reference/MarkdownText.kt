package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp

/**
 * The shared markdown renderer the markdown and flow_markdown twins use.
 * Covers the block/inline constructs the fixture documents contain:
 * headings, bold/italic/bold-italic, `code`, fenced code blocks, unordered
 * and ordered lists, blockquotes, pipe tables, thematic breaks and links
 * (rendered with an underline — navigation is not part of the fixtures).
 */
@Composable
fun MarkdownText(source: String, modifier: Modifier = Modifier) {
    SelectionContainer {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            val lines = source.lines()
            var i = 0
            while (i < lines.size) {
                val line = lines[i]
                when {
                    line.startsWith("```") -> {
                        val buf = StringBuilder()
                        i += 1
                        while (i < lines.size && !lines[i].startsWith("```")) {
                            buf.append(lines[i]).append('\n')
                            i += 1
                        }
                        i += 1
                        Text(
                            buf.toString().trimEnd('\n'),
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                            modifier = Modifier
                                .fillMaxWidth()
                                .background(
                                    MaterialTheme.colorScheme.surfaceContainerHigh,
                                    RoundedCornerShape(6.dp),
                                )
                                .horizontalScroll(rememberScrollState())
                                .padding(8.dp),
                        )
                    }
                    line.startsWith("### ") ->
                        Text(inline(line.removePrefix("### ")), style = MaterialTheme.typography.titleSmall)
                    line.startsWith("## ") ->
                        Text(inline(line.removePrefix("## ")), style = MaterialTheme.typography.titleMedium)
                    line.startsWith("# ") ->
                        Text(inline(line.removePrefix("# ")), style = MaterialTheme.typography.headlineMedium)
                    line.startsWith("> ") -> {
                        Text(
                            inline(line.removePrefix("> ")),
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier
                                .fillMaxWidth()
                                .background(
                                    MaterialTheme.colorScheme.surfaceContainerHigh,
                                    RoundedCornerShape(4.dp),
                                )
                                .padding(8.dp),
                        )
                    }
                    line.startsWith("- ") || line.startsWith("* ") ->
                        Row {
                            Text("• ", style = MaterialTheme.typography.bodyLarge)
                            Text(inline(line.removePrefix("- ").removePrefix("* ")), style = MaterialTheme.typography.bodyLarge)
                        }
                    line.matches(Regex("^\\d+\\.\\s.*")) ->
                        Text(inline(line), style = MaterialTheme.typography.bodyLarge)
                    line.startsWith("|") && line.endsWith("|") -> {
                        // Pipe table row: cells separated by |; a separator
                        // row of dashes renders as a divider.
                        if (line.contains("---")) {
                            HorizontalDivider()
                        } else {
                            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                for (cell in line.trim('|').split('|')) {
                                    Text(
                                        inline(cell.trim()),
                                        style = MaterialTheme.typography.bodySmall,
                                    )
                                }
                            }
                        }
                    }
                    line.startsWith("---") -> HorizontalDivider()
                    line.isBlank() -> {}
                    else -> Text(inline(line), style = MaterialTheme.typography.bodyLarge)
                }
                i += 1
            }
        }
    }
}

private fun inline(text: String): AnnotatedString = buildAnnotatedString {
    var rest = text
    while (rest.isNotEmpty()) {
        when {
            rest.startsWith("***") -> {
                val end = rest.indexOf("***", 3)
                if (end > 0) {
                    pushStyle(SpanStyle(fontWeight = FontWeight.Bold, fontStyle = FontStyle.Italic))
                    append(rest.substring(3, end))
                    pop()
                    rest = rest.substring(end + 3)
                } else { append('*'); rest = rest.substring(1) }
            }
            rest.startsWith("**") -> {
                val end = rest.indexOf("**", 2)
                if (end > 0) {
                    pushStyle(SpanStyle(fontWeight = FontWeight.Bold))
                    append(rest.substring(2, end))
                    pop()
                    rest = rest.substring(end + 2)
                } else { append('*'); rest = rest.substring(1) }
            }
            rest.startsWith("*") -> {
                val end = rest.indexOf('*', 1)
                if (end > 0) {
                    pushStyle(SpanStyle(fontStyle = FontStyle.Italic))
                    append(rest.substring(1, end))
                    pop()
                    rest = rest.substring(end + 1)
                } else { append('*'); rest = rest.substring(1) }
            }
            rest.startsWith("`") -> {
                val end = rest.indexOf('`', 1)
                if (end > 0) {
                    pushStyle(SpanStyle(fontFamily = FontFamily.Monospace))
                    append(rest.substring(1, end))
                    pop()
                    rest = rest.substring(end + 1)
                } else { append('`'); rest = rest.substring(1) }
            }
            rest.startsWith("[") -> {
                val close = rest.indexOf("](")
                if (close > 0) {
                    val end = rest.indexOf(')', close)
                    if (end > 0) {
                        pushStyle(SpanStyle(textDecoration = TextDecoration.Underline))
                        append(rest.substring(1, close))
                        pop()
                        rest = rest.substring(end + 1)
                    } else { append('['); rest = rest.substring(1) }
                } else { append('['); rest = rest.substring(1) }
            }
            else -> {
                val next = listOfNotNull(
                    rest.indexOf("***").takeIf { it >= 0 },
                    rest.indexOf("**").takeIf { it >= 0 },
                    rest.indexOf('*').takeIf { it >= 0 },
                    rest.indexOf('`').takeIf { it >= 0 },
                    rest.indexOf('[').takeIf { it >= 0 },
                ).minOrNull() ?: rest.length
                append(rest.substring(0, next))
                rest = rest.substring(next)
            }
        }
    }
}
