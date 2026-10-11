// Twin of examples/form: three sections of labelled form controls plus live
// value previews, inside a padded scroll stack.
//
// The example installs a theme that overrides the body font to
// `ResolvedFont::new(16.0 + (1.0 + scale * 10.0))`; with the default
// font_scale of 0 that is 17pt regular — applied here as the environment font
// so unstyled text and control labels inherit it, exactly as WaterUI's theme
// body font does.

import SwiftUI

struct FormTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Form Examples").font(.title)
        Text("Demonstrating form building with reactive data binding")
        wuiDivider()
        Spacer(minLength: 0)
        registrationSection
        Spacer(minLength: 0)
        settingsSection
        Spacer(minLength: 0)
        manualSection
        Spacer(minLength: 0)
        wuiDivider()
        Text("Built with WaterUI Form Components")
      }
      .padding(14)
    }
    .font(.system(size: 17))
  }

  // MARK: - Registration form (#[form] derive output)

  private var registrationSection: some View {
    VStack(spacing: 10) {
      Text("Registration Form").font(.subheadline)
      Text("Using #[form] derive macro")
      generatedForm {
        textField("Full Name", prompt: "Full name of the user")
        textField("Email", prompt: "Email address for account")
        stepper("Age", value: 0)
        toggle("Newsletter", isOn: false)
        labeledSlider("Volume", value: 0)
      }
      wuiDivider()
      Text("Live Preview:").fontWeight(.bold)
      Text("Name: ")
      Text("Email: ")
      Text("Age: 0")
      Text("Newsletter: false")
      Text("Volume: 0")
    }
  }

  // MARK: - Settings form

  private var settingsSection: some View {
    VStack(spacing: 10) {
      Text("App Settings").font(.subheadline)
      Text("Another form with different field types")
      generatedForm {
        labeledSlider("Brightness", value: 0)
        toggle("Dark Mode", isOn: false)
        labeledSlider("Font Scale", value: 0)
        stepper("Auto Save Minutes", value: 0)
        toggle("Notifications Enabled", isOn: false)
      }
      wuiDivider()
      Text("Current Settings:").fontWeight(.bold)
      HStack(spacing: 10) {
        Text("Dark Mode: ")
        Text("false")
      }
      HStack(spacing: 10) {
        Text("Brightness: ")
        Text("0.0000")
      }
    }
  }

  // MARK: - Manual controls

  private var manualSection: some View {
    VStack(spacing: 10) {
      Text("Manual Form Controls").font(.subheadline)
      Text("Building forms manually with individual controls")
      textField("Username", prompt: "Enter your username")
      toggle("Enable Feature", isOn: false)
      stepper("Item Count", value: 5, range: 0 ... 100, step: 5)
      labeledSlider("Progress", value: 0.5)
      ProgressView(value: 0.5)
      wuiDivider()
      Text("Manual Controls Preview:").fontWeight(.bold)
      Text("Username: ")
      Text("Feature Enabled: false")
      Text("Count: 5")
      Text("Progress: 0.5")
    }
  }

  // MARK: - Control translations
  //
  // These mirror the labelled layouts the backend builds natively:
  //   TextField: label over field, 4pt vertical gap, leading aligned
  //   Toggle:    native control beside its label, 8pt gap
  //   Stepper:   label + 8pt + formatted value + 8pt + control, one row
  //   Slider:    label over a full-width track, 4pt vertical gap

  private func generatedForm(@ViewBuilder _ fields: () -> some View) -> some View {
    // #[form] emits a vstack of field views at the default spacing.
    VStack(spacing: 10) { fields() }
  }

  private func textField(_ label: String, prompt: String) -> some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(label)
      TextField(prompt, text: .constant(""))
    }
  }

  private func toggle(_ label: String, isOn: Bool) -> some View {
    Toggle(label, isOn: .constant(isOn))
  }

  private func stepper(
    _ label: String,
    value: Int,
    range: ClosedRange<Int> = Int(Int32.min) ... Int(Int32.max),
    step: Int = 1
  ) -> some View {
    // WaterUI lays a labelled stepper out as a full-width row: label leading,
    // control trailing (no value label without an explicit formatter).
    HStack {
      Text(label)
      Spacer(minLength: 0)
      Stepper("", value: .constant(value), in: range, step: step)
        .labelsHidden()
    }
  }

  private func labeledSlider(_ label: String, value: Double) -> some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(label)
      Slider(value: .constant(value))
    }
  }
}
