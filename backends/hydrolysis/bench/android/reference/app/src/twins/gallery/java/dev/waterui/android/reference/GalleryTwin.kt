package dev.waterui.android.reference

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

private data class Control(val title: String, val section: Int)

private val SECTIONS = listOf("Actions", "Inputs", "Selection", "Display")

private val CONTROLS = listOf(
    Control("Buttons", 0),
    Control("Text Field", 1),
    Control("Slider", 1),
    Control("Stepper", 1),
    Control("Toggle", 2),
    Control("Picker", 2),
    Control("Label", 3),
    Control("Progress", 3),
)

private class DemoState {
    var taps by mutableIntStateOf(0)
    var name by mutableStateOf("")
    var volume by mutableFloatStateOf(50f)
    var quantity by mutableIntStateOf(0)
    var wifi by mutableStateOf(false)
    var bluetooth by mutableStateOf(false)
    var size by mutableStateOf("Medium")
}

/**
 * Compose twin of the gallery dogfood app: the collapsible grouped control
 * drawer (Actions / Inputs / Selection / Display) plus the detail pane —
 * every control demo live (the button counter counts, toggles stay toggled).
 */
@Composable
fun GalleryTwin() {
    val state = remember { DemoState() }
    var selected by remember { mutableStateOf("Buttons") }
    var expanded by remember { mutableStateOf(setOf(0, 1, 2, 3)) }

    Row(Modifier.fillMaxSize()) {
        // Grouped drawer.
        Column(
            Modifier.width(200.dp).fillMaxHeight()
                .verticalScroll(rememberScrollState())
                .padding(8.dp)
                .testTag("gallery:drawer"),
        ) {
            SECTIONS.forEachIndexed { s, name ->
                Row(
                    Modifier.fillMaxWidth()
                        .clickable {
                            BenchMarkers.tap()
                            expanded = if (s in expanded) expanded - s else expanded + s
                        }
                        .padding(vertical = 8.dp)
                        .testTag("gallery:group-$name"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    SubheadlineText(name)
                    Spacer(Modifier.weight(1f))
                    Icon(
                        if (s in expanded) Icons.Filled.KeyboardArrowDown
                        else Icons.Filled.KeyboardArrowRight,
                        contentDescription = if (s in expanded) "collapse" else "expand",
                    )
                }
                if (s in expanded) {
                    for (control in CONTROLS.filter { it.section == s }) {
                        Text(
                            control.title,
                            modifier = Modifier.fillMaxWidth()
                                .clickable {
                                    BenchMarkers.tap()
                                    selected = control.title
                                }
                                .padding(horizontal = 8.dp, vertical = 6.dp)
                                .testTag("gallery:item-${control.title.lowercase().replace(' ', '-')}"),
                            color = if (selected == control.title) {
                                MaterialTheme.colorScheme.primary
                            } else {
                                MaterialTheme.colorScheme.onSurface
                            },
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
        }

        HorizontalDivider(modifier = Modifier.fillMaxHeight().width(1.dp))

        Column(
            Modifier.weight(1f).fillMaxHeight()
                .verticalScroll(rememberScrollState())
                .padding(16.dp)
                .testTag("gallery:detail"),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            TitleText(selected)
            when (selected) {
                "Buttons" -> ButtonsDemo(state)
                "Text Field" -> TextFieldDemo(state)
                "Slider" -> SliderDemo(state)
                "Stepper" -> StepperDemo(state)
                "Toggle" -> ToggleDemo(state)
                "Picker" -> PickerDemo(state)
                "Label" -> LabelDemo()
                "Progress" -> ProgressDemo()
            }
        }
    }
}

@Composable
private fun ButtonsDemo(s: DemoState) {
    FootnoteText("Every button shares one action. ButtonStyle controls the appearance.")
    BodyText("Taps: ${s.taps}")
    Button(onClick = { BenchMarkers.tap(); s.taps++ }, modifier = Modifier.testTag("gallery:auto")) { Text("Automatic") }
    OutlinedButton(onClick = { BenchMarkers.tap(); s.taps++ }, modifier = Modifier.testTag("gallery:bordered")) { Text("Bordered") }
    Button(onClick = { BenchMarkers.tap(); s.taps++ }, modifier = Modifier.testTag("gallery:prominent")) { Text("Bordered Prominent") }
    TextButton(onClick = { BenchMarkers.tap(); s.taps++ }, modifier = Modifier.testTag("gallery:plain")) { Text("Plain") }
    TextButton(onClick = { BenchMarkers.tap(); s.taps++ }, modifier = Modifier.testTag("gallery:borderless")) { Text("Borderless") }
    TextButton(onClick = { BenchMarkers.tap(); s.taps++ }, modifier = Modifier.testTag("gallery:link")) { Text("Link") }
}

@Composable
private fun TextFieldDemo(s: DemoState) {
    FootnoteText("TextField binds to reactive text and echoes it live.")
    OutlinedTextField(
        value = s.name,
        onValueChange = { BenchMarkers.edit(); s.name = it },
        modifier = Modifier.fillMaxWidth().testTag("gallery:name"),
        label = { Text("Name") },
        placeholder = { Text("Type your name") },
        singleLine = true,
    )
    BodyText("Echo: ${s.name}")
}

@Composable
private fun SliderDemo(s: DemoState) {
    FootnoteText("Drag a slider; the value indicator follows the thumb and the progress bar reflects the value.")
    for (size in listOf("Extra small", "Small", "Medium", "Large", "Extra large")) {
        Slider(
            value = s.volume,
            onValueChange = { s.volume = it },
            valueRange = 0f..100f,
            modifier = Modifier.testTag("gallery:slider-$size"),
        )
    }
    BodyText("Value: ${s.volume.toInt()}")
    LinearProgressIndicator(
        progress = { s.volume / 100f },
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun StepperDemo(s: DemoState) {
    FootnoteText("Stepper adjusts an integer within a range.")
    Row(verticalAlignment = Alignment.CenterVertically) {
        Button(onClick = {
            BenchMarkers.tap()
            if (s.quantity > 0) s.quantity--
        }, modifier = Modifier.testTag("gallery:step-minus")) { Text("-") }
        BodyText("  ${s.quantity}  ")
        Button(onClick = {
            BenchMarkers.tap()
            if (s.quantity < 10) s.quantity++
        }, modifier = Modifier.testTag("gallery:step-plus")) { Text("+") }
    }
    BodyText("Quantity: ${s.quantity}")
}

@Composable
private fun ToggleDemo(s: DemoState) {
    FootnoteText("ToggleStyle switches between a switch and a checkbox.")
    Row(verticalAlignment = Alignment.CenterVertically) {
        Switch(
            checked = s.wifi,
            onCheckedChange = { BenchMarkers.tap(); s.wifi = it },
            modifier = Modifier.testTag("gallery:wifi"),
        )
        BodyText("Wi-Fi")
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
            checked = s.bluetooth,
            onCheckedChange = { BenchMarkers.tap(); s.bluetooth = it == true },
            modifier = Modifier.testTag("gallery:bluetooth"),
        )
        BodyText("Bluetooth")
    }
    BodyText("Wi-Fi ${s.wifi} · Bluetooth ${s.bluetooth}")
}

@Composable
private fun PickerDemo(s: DemoState) {
    FootnoteText("Picker chooses between mutually exclusive options.")
    val sizes = listOf("Small", "Medium", "Large")
    SingleChoiceSegmentedButtonRow {
        sizes.forEachIndexed { i, label ->
            SegmentedButton(
                selected = s.size == label,
                onClick = { BenchMarkers.tap(); s.size = label },
                shape = SegmentedButtonDefaults.itemShape(index = i, count = sizes.size),
                modifier = Modifier.testTag("gallery:size-$label"),
            ) { Text(label) }
        }
    }
    BodyText("Size: ${s.size}")
}

@Composable
private fun LabelDemo() {
    FootnoteText("Labels pair an icon with text in multiple styles.")
    TitleText("Title Label")
    HeadlineText("Headline Label")
    BodyText("Body Label")
    FootnoteText("Footnote Label")
    CaptionText("Caption Label")
}

@Composable
private fun ProgressDemo() {
    FootnoteText("Progress indicators report determinate and indeterminate work.")
    LinearProgressIndicator(progress = { 0.65f }, modifier = Modifier.fillMaxWidth())
    Spacer(Modifier.height(8.dp))
    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
}
