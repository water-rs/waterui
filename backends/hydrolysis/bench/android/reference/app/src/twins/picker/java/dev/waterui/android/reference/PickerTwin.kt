package dev.waterui.android.reference

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MenuAnchorType
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

// Twin of examples/picker, first screen: the "Picker Styles" section.
//
// Control translations mirror the backend's own construction:
//   Picker automatic/menu -> read-only filled TextField + trailing chevron
//                            inside ExposedDropdownMenuBox (backend builds a
//                            textInputFilledExposedDropdownMenuStyle
//                            TextInputLayout + MaterialAutoCompleteTextView,
//                            hint disabled — the label is accessibility-only)
//   Picker radio          -> vertical RadioButton rows (backend: RadioGroup of
//                            MaterialRadioButton)
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PickerField(value: String) {
    ExposedDropdownMenuBox(expanded = false, onExpandedChange = {}) {
        TextField(
            value = value,
            onValueChange = {},
            readOnly = true,
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = false) },
            modifier = Modifier.menuAnchor(MenuAnchorType.PrimaryNotEditable),
        )
    }
}

@Composable
private fun RadioRow(label: String, selected: Boolean) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        RadioButton(selected = selected, onClick = {})
        Text(label)
    }
}

@Composable
fun PickerTwin() {
    Column(
        modifier = Modifier
            .verticalScroll(rememberScrollState())
            .padding(16.dp)
            .fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        VStack {
            VStack {
                TitleText("Picker Gallery")
                BodyText("Demonstrating WaterUI form and picker components")
            }
            HorizontalDivider()
            VStack(modifier = Modifier.padding(12.dp)) {
                HeadlineText("Picker Styles")
                BodyText("Choose from different picker presentation styles")
                Spacer(Modifier.height(0.dp))
                BoldText("Automatic (default)")
                PickerField("Apple")
                HStack {
                    BodyText("Selected: ")
                    BodyText("Apple")
                }
                Spacer(Modifier.height(0.dp))
                BoldText("Menu Style")
                PickerField("Banana")
                HStack {
                    BodyText("Selected: ")
                    BodyText("Banana")
                }
                Spacer(Modifier.height(0.dp))
                BoldText("Radio Style")
                Column {
                    RadioRow("Apple", selected = false)
                    RadioRow("Banana", selected = false)
                    RadioRow("Cherry", selected = true)
                    RadioRow("Date", selected = false)
                    RadioRow("Elderberry", selected = false)
                }
                HStack {
                    BodyText("Selected: ")
                    BodyText("Cherry")
                }
            }
            HorizontalDivider()
            VStack(modifier = Modifier.padding(12.dp)) {
                HeadlineText("DatePicker")
                BodyText("Select dates and times with platform-native pickers")
            }
        }
    }
}
