package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp

/**
 * The section-6 editing suite (a suite-only screen, not a frozen fixture):
 * single-line and multiline fields, a password field with visibility
 * toggle, a selection/readout pair, a CJK composition target and a
 * scrolling editor — all reporting the live window/IME insets so keyboard
 * transitions are observable on both sides.
 */
@Composable
fun EditingTwin() {
    var single by remember { mutableStateOf("") }
    var multi by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var showPassword by remember { mutableStateOf(false) }
    var cjk by remember { mutableStateOf("") }
    var editor by remember { mutableStateOf((1..24).joinToString("\n") { "Line $it of the scrolling editor." }) }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp).imePadding(),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Editing")

        Column(Modifier.testTag("edit:single")) {
            HeadlineText("Single line")
            OutlinedTextField(
                value = single,
                onValueChange = {
                    BenchMarkers.edit()
                    single = it
                },
                modifier = Modifier.fillMaxWidth().testTag("edit:single-field"),
                label = { Text("Name") },
                singleLine = true,
            )
        }

        Column(Modifier.testTag("edit:multi")) {
            HeadlineText("Multiline")
            OutlinedTextField(
                value = multi,
                onValueChange = {
                    BenchMarkers.edit()
                    multi = it
                },
                modifier = Modifier.fillMaxWidth().height(120.dp)
                    .testTag("edit:multi-field"),
                label = { Text("Notes") },
            )
        }

        Column(Modifier.testTag("edit:password")) {
            HeadlineText("Password")
            OutlinedTextField(
                value = password,
                onValueChange = {
                    BenchMarkers.edit()
                    password = it
                },
                modifier = Modifier.fillMaxWidth().testTag("edit:password-field"),
                label = { Text("Password") },
                singleLine = true,
                visualTransformation = if (showPassword) {
                    VisualTransformation.None
                } else {
                    PasswordVisualTransformation()
                },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                trailingIcon = {
                    IconButton(
                        onClick = {
                            BenchMarkers.tap()
                            showPassword = !showPassword
                        },
                        modifier = Modifier.testTag("edit:password-toggle"),
                    ) {
                        Icon(Icons.Filled.CheckCircle, "toggle visibility")
                    }
                },
            )
        }

        Column(Modifier.testTag("edit:selection")) {
            HeadlineText("Selection")
            SelectionContainer {
                BodyText("Selectable text: copy me from the field below, then paste.")
            }
            Row {
                OutlinedTextField(
                    value = multi,
                    onValueChange = {},
                    modifier = Modifier.weight(1f).testTag("edit:selection-readout"),
                    readOnly = true,
                    label = { Text("Multiline echo") },
                )
            }
        }

        Column(Modifier.testTag("edit:cjk")) {
            HeadlineText("CJK composition")
            OutlinedTextField(
                value = cjk,
                onValueChange = {
                    BenchMarkers.edit()
                    cjk = it
                },
                modifier = Modifier.fillMaxWidth().testTag("edit:cjk-field"),
                label = { Text("日本語で入力") },
                placeholder = { Text("IME composition target") },
            )
            FootnoteText("len: ${cjk.length} — last: ${cjk.takeLast(12)}")
        }

        Column(Modifier.testTag("edit:scrolling")) {
            HeadlineText("Scrolling editor")
            OutlinedTextField(
                value = editor,
                onValueChange = {
                    BenchMarkers.edit()
                    editor = it
                },
                modifier = Modifier.fillMaxWidth().height(220.dp)
                    .testTag("edit:editor-field"),
                label = { Text("Editor") },
            )
        }

        Spacer(Modifier.height(8.dp))
        CaptionText(
            "Insets readout — the twin reports the same snapshot the host "
                + "consumes so keyboard/inset transitions can be compared.",
        )
        val density = LocalDensity.current
        val imeInsets = WindowInsets.ime
        CaptionText("ime bottom px: ${imeInsets.getBottom(density)}")
    }
}
