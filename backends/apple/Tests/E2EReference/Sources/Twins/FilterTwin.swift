// Twin of examples/filter: six color-swatch grids under blur, brightness,
// saturation, contrast, hue rotation, grayscale, opacity, and a combined
// section — each with a slider and preset buttons.
//
// All bindings start at identity values, so the settled first screen shows
// unfiltered swatches; the twin keeps the controls live for completeness.

import SwiftUI

struct FilterTwin: View {
  @State private var blurRadius = 0.0
  @State private var brightness = 0.0
  @State private var saturation = 1.0
  @State private var contrast = 1.0
  @State private var hue = 0.0
  @State private var grayscale = 0.0
  @State private var opacity = 1.0
  @State private var cBlur = 0.0
  @State private var cSaturation = 1.0
  @State private var cHue = 0.0

  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Filter Examples").font(.title)
        Text("Visual demonstrations of the filter system")
        wuiDivider()
        VStack(spacing: 10) {
          blurSection
          wuiDivider()
          brightnessSection
          wuiDivider()
          saturationSection
          wuiDivider()
          contrastSection
        }
        VStack(spacing: 10) {
          wuiDivider()
          hueSection
          wuiDivider()
          grayscaleSection
          wuiDivider()
          opacitySection
          wuiDivider()
          combinedSection
        }
      }
      .padding(16)
    }
  }

  // The swatches are waterui's named colors (`Red`, `Green`, ...), which
  // resolve to the material palette — not SwiftUI's system colors.
  private var sample: some View {
    VStack(spacing: 0) {
      HStack(spacing: 0) {
        srgbHex(0xF44336).frame(width: 40, height: 40)
        srgbHex(0x4CAF50).frame(width: 40, height: 40)
        srgbHex(0x2196F3).frame(width: 40, height: 40)
      }
      HStack(spacing: 0) {
        srgbHex(0xFFEB3B).frame(width: 40, height: 40)
        srgbHex(0x9C27B0).frame(width: 40, height: 40)
        srgbHex(0x00BCD4).frame(width: 40, height: 40)
      }
    }
    .frame(width: 120, height: 80)
  }

  private func buttons(_ binding: Binding<Double>, _ values: [(String, Double)]) -> some View {
    HStack(spacing: 10) {
      ForEach(values, id: \.0) { label, v in
        Button(label) { binding.wrappedValue = v }
      }
    }
  }

  private var blurSection: some View {
    VStack(spacing: 10) {
      Text("Blur").font(.headline)
      Text("Apply Gaussian blur to content")
      sample.blur(radius: blurRadius).frame(minHeight: 100)
      Slider(value: $blurRadius, in: 0 ... 20).labelsHidden()
      buttons($blurRadius, [("0", 0), ("5", 5), ("10", 10), ("20", 20)])
    }
    .padding(14)
  }

  private var brightnessSection: some View {
    VStack(spacing: 10) {
      Text("Brightness").font(.headline)
      Text("Adjust brightness (-1 to 1)")
      sample.brightness(brightness).frame(minHeight: 100)
      Slider(value: $brightness, in: -1 ... 1).labelsHidden()
      buttons($brightness, [("-1", -1), ("0", 0), ("0.5", 0.5), ("1", 1)])
    }
    .padding(14)
  }

  private var saturationSection: some View {
    VStack(spacing: 10) {
      Text("Saturation").font(.headline)
      Text("Adjust color saturation (0 = grayscale)")
      sample.saturation(saturation).frame(minHeight: 100)
      Slider(value: $saturation, in: 0 ... 2).labelsHidden()
      buttons($saturation, [("0", 0), ("1", 1), ("1.5", 1.5), ("2", 2)])
    }
    .padding(14)
  }

  private var contrastSection: some View {
    VStack(spacing: 10) {
      Text("Contrast").font(.headline)
      Text("Adjust color contrast")
      sample.contrast(contrast).frame(minHeight: 100)
      Slider(value: $contrast, in: 0 ... 2).labelsHidden()
      buttons($contrast, [("0", 0), ("1", 1), ("1.5", 1.5), ("2", 2)])
    }
    .padding(14)
  }

  private var hueSection: some View {
    VStack(spacing: 10) {
      Text("Hue Rotation").font(.headline)
      Text("Rotate colors around the color wheel (0-360 degrees)")
      sample.hueRotation(.degrees(hue)).frame(minHeight: 100)
      Slider(value: $hue, in: 0 ... 360).labelsHidden()
      buttons($hue, [("0", 0), ("90", 90), ("180", 180), ("270", 270)])
    }
    .padding(14)
  }

  private var grayscaleSection: some View {
    VStack(spacing: 10) {
      Text("Grayscale").font(.headline)
      Text("Convert to grayscale (0 = color, 1 = grayscale)")
      sample.grayscale(grayscale).frame(minHeight: 100)
      Slider(value: $grayscale).labelsHidden()
      buttons($grayscale, [("0", 0), ("0.5", 0.5), ("1", 1)])
    }
    .padding(14)
  }

  private var opacitySection: some View {
    VStack(spacing: 10) {
      Text("Opacity").font(.headline)
      Text("Adjust transparency (0 = invisible, 1 = opaque)")
      sample.opacity(opacity).frame(minHeight: 100)
      Slider(value: $opacity).labelsHidden()
      buttons($opacity, [("0", 0), ("0.5", 0.5), ("1", 1)])
    }
    .padding(14)
  }

  private var combinedSection: some View {
    VStack(spacing: 10) {
      Text("Combined Filters").font(.headline)
      Text("Apply multiple filters with spring animations")
      sample
        .blur(radius: cBlur)
        .saturation(cSaturation)
        .hueRotation(.degrees(cHue))
        .frame(minHeight: 100)
      HStack(spacing: 10) {
        Button("Reset") { cBlur = 0; cSaturation = 1; cHue = 0 }
        Button("Dreamy") { cBlur = 3; cSaturation = 0.7 }
        Button("Vibrant") { cHue = 180; cSaturation = 1.8 }
        Button("Vintage") { cBlur = 1; cSaturation = 0.5; cHue = 30 }
      }
    }
    .padding(14)
  }
}
