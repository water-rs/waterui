package dev.waterui.android.reference

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// Twin of examples/form: three sections of labelled form controls plus live
// value previews, inside a padded scroll stack. Initial state only.
//
// Control translations mirror the labelled layouts the backend builds:
//   TextField        -> label over a filled MD3 TextField (TextInputLayout)
//   Toggle (switch)  -> label (weight 1) + trailing Switch (MaterialSwitch)
//   Stepper          -> label + value + connected filled −/+ pair
//   Slider (labelled)-> label over a full-width MD3 Slider (SeekBar)
//   Progress(0.5)    -> LinearProgressIndicator
//
// The example installs a theme body font of 17pt with no explicit line
// height (`ResolvedFont::new(17.0, Normal)` leaves `line_height=None`), so
// body text gets the face's natural metrics — mirrored here with an
// unspecified line height rather than bodyLarge's fixed 24sp.

private val formBodyStyle
    @Composable get() = MaterialTheme.typography.bodyLarge.copy(
        fontSize = 17.sp,
        lineHeight = TextUnit.Unspecified,
    )

@Composable
private fun FormBodyText(text: String, modifier: Modifier = Modifier) {
    Text(text, style = formBodyStyle, modifier = modifier)
}

@Composable
private fun FormBoldText(text: String, modifier: Modifier = Modifier) {
    Text(text, style = formBodyStyle, fontWeight = FontWeight.Bold, modifier = modifier)
}

@Composable
private fun FormTextField(label: String, prompt: String) {
    Column(horizontalAlignment = Alignment.Start, modifier = Modifier.fillMaxWidth()) {
        FormBodyText(label)
        Spacer(Modifier.height(4.dp))
        TextField(
            value = "",
            onValueChange = {},
            placeholder = { Text(prompt) },
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun FormToggle(label: String, isOn: Boolean) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        FormBodyText(label, modifier = Modifier.weight(1f))
        Switch(checked = isOn, onCheckedChange = {})
    }
}

@Composable
private fun FormStepper(label: String) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        FormBodyText(label, modifier = Modifier.weight(1f))
        // The WaterUI stepper only renders an inline value when the app sets
        // `value_formatter`; the example leaves it unset, so no value text.
        // The backend uses MaterialButtonGroup's connected silhouette; the
        // closest on this material3 version is a zero-gap pair.
        Button(onClick = {}) { Text("−") }
        Button(onClick = {}) { Text("+") }
    }
}

@Composable
private fun FormSlider(label: String, value: Float) {
    Column(horizontalAlignment = Alignment.Start, modifier = Modifier.fillMaxWidth()) {
        FormBodyText(label)
        Spacer(Modifier.height(4.dp))
        Slider(value = value, onValueChange = {}, modifier = Modifier.fillMaxWidth())
    }
}

@Composable
fun FormTwin() {
    Column(
        modifier = Modifier
            .verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp)
            .fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        VStack {
            TitleText("WaterUI Form Examples")
            FormBodyText("Demonstrating form building with reactive data binding")
            HorizontalDivider()
            Spacer(Modifier.height(0.dp))

            // Registration form (#[form] derive output)
            VStack {
                SubheadlineText("Registration Form")
                FormBodyText("Using #[form] derive macro")
                VStack {
                    FormTextField("Full Name", "Full name of the user")
                    FormTextField("Email", "Email address for account")
                    FormStepper("Age")
                    FormToggle("Newsletter", false)
                    FormSlider("Volume", 0f)
                }
                HorizontalDivider()
                FormBoldText("Live Preview:")
                FormBodyText("Name: ")
                FormBodyText("Email: ")
                FormBodyText("Age: 0")
                FormBodyText("Newsletter: false")
                FormBodyText("Volume: 0")
            }
            Spacer(Modifier.height(0.dp))

            // Settings form
            VStack {
                SubheadlineText("App Settings")
                FormBodyText("Another form with different field types")
                VStack {
                    FormSlider("Brightness", 0f)
                    FormToggle("Dark Mode", false)
                    FormSlider("Font Scale", 0f)
                    FormStepper("Auto Save Minutes")
                    FormToggle("Notifications Enabled", false)
                }
                HorizontalDivider()
                FormBoldText("Current Settings:")
                HStack {
                    FormBodyText("Dark Mode: ")
                    FormBodyText("false")
                }
                HStack {
                    FormBodyText("Brightness: ")
                    FormBodyText("0.0000")
                }
            }
            Spacer(Modifier.height(0.dp))

            // Manual controls
            VStack {
                SubheadlineText("Manual Form Controls")
                FormBodyText("Building forms manually with individual controls")
                FormTextField("Username", "Enter your username")
                FormToggle("Enable Feature", false)
                FormStepper("Item Count")
                FormSlider("Progress", 0.5f)
                LinearProgressIndicator(
                    progress = { 0.5f },
                    modifier = Modifier.fillMaxWidth(),
                )
                HorizontalDivider()
                FormBoldText("Manual Controls Preview:")
                FormBodyText("Username: ")
                FormBodyText("Feature Enabled: false")
                FormBodyText("Count: 5")
                FormBodyText("Progress: 0.5")
            }
            Spacer(Modifier.height(0.dp))
            HorizontalDivider()
            FormBodyText("Built with WaterUI Form Components")
        }
    }
}
