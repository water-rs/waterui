// Twin of examples/animation: transform stages, value animations and
// control rows. Initial state only — scale 1, rotation 0, offsets 0 for the
// transform stages, and each later section's starting binding — so every
// stage shows its box centered at natural size.
//
// The below-the-fold sections stay in the twin: the scroll knob on the first
// screen is sized by the whole document.

import SwiftUI

private let scaleBoxSide: CGFloat = 80
private let scaleStageSide = scaleBoxSide * 2.25
private let rotationBoxSide: CGFloat = 60
private let rotationStageSide = rotationBoxSide * 1.8
private let translationBoxSide: CGFloat = 50
private let translationStageSide = translationBoxSide * 3
private let combinedBoxSide: CGFloat = 60
private let combinedStageSide = combinedBoxSide * 3

struct AnimationTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Animation Examples").font(.title)
        Text("Visual demonstrations of the animation system")
        wuiDivider()
        VStack(spacing: 10) {
          scaleSection
          wuiDivider()
          rotationSection
          wuiDivider()
          translationSection
          wuiDivider()
          combinedSection
          wuiDivider()
          gpuAnimationSection
        }
        VStack(spacing: 10) {
          wuiDivider()
          progressSection
          wuiDivider()
          springSection
          wuiDivider()
          sizeIndicatorSection
        }
        VStack(spacing: 10) {
          wuiDivider()
          curvesSection
          wuiDivider()
          toggleSection
          wuiDivider()
          staggeredSection
        }
      }
      .padding(14)
    }
  }

  private var scaleSection: some View {
    VStack(spacing: 10) {
      Text("Scale Animation").font(.headline)
      Text("Click buttons to scale the box with spring physics").font(.body)
      srgbHex(0x2196F3)
        .frame(width: scaleBoxSide, height: scaleBoxSide)
        .frame(width: scaleStageSide, height: scaleStageSide)
      HStack(spacing: 10) {
        Button("0.5x") {}
        Button("1x") {}
        Button("1.5x") {}
        Button("2x") {}
      }
    }
    .padding(14)
  }

  private var rotationSection: some View {
    VStack(spacing: 10) {
      Text("Rotation Animation").font(.headline)
      Text("Rotate the box smoothly").font(.body)
      srgbHex(0x4CAF50)
        .frame(width: rotationBoxSide, height: rotationBoxSide)
        .frame(width: rotationStageSide, height: rotationStageSide)
      VStack(spacing: 10) {
        HStack(spacing: 10) {
          Button("-90°") {}
          Button("-45°") {}
          Button("Reset") {}
        }
        HStack(spacing: 10) {
          Button("+45°") {}
          Button("+90°") {}
        }
      }
    }
    .padding(14)
  }

  private var translationSection: some View {
    VStack(spacing: 10) {
      Text("Translation Animation").font(.headline)
      Text("Move the box with spring physics").font(.body)
      srgbHex(0x9C27B0)
        .frame(width: translationBoxSide, height: translationBoxSide)
        .frame(width: translationStageSide, height: translationStageSide)
      VStack(spacing: 10) {
        HStack(spacing: 10) {
          Button("Center") {}
          Button("Left") {}
          Button("Right") {}
        }
        HStack(spacing: 10) {
          Button("Up") {}
          Button("Down") {}
        }
      }
    }
    .padding(14)
  }

  private var combinedSection: some View {
    VStack(spacing: 10) {
      Text("Combined Transforms").font(.headline)
      Text("Scale and rotation together").font(.body)
      srgbHex(0xFF9800)
        .frame(width: combinedBoxSide, height: combinedBoxSide)
        .frame(width: combinedStageSide, height: combinedStageSide)
      HStack(spacing: 10) {
        Button("Reset") {}
        Button("Grow + Spin") {}
        Button("Pulse") {}
      }
    }
    .padding(14)
  }

  // `morph_to` starts from its source shape: a circle and a rectangle.
  private var gpuAnimationSection: some View {
    VStack(spacing: 10) {
      Text("Framework GPU Animation").font(.headline)
      Text("Shape geometry morphing (renderer-side, not plain Core Animation transform)")
        .font(.body)
      HStack(spacing: 16) {
        Circle().fill(srgbHex(0x3B82F6)).frame(width: 90, height: 90)
        Rectangle().fill(srgbHex(0x10B981)).frame(width: 128, height: 72)
      }
    }
    .padding(14)
  }

  /// `progress(value)`: `Progress::new`'s default label over the bar and its
  /// percentage value label under it.
  private func progressBar(_ value: Double) -> some View {
    ProgressView(value: value) {
      Text("Please wait...")
    } currentValueLabel: {
      Text("\(Int((value * 100).rounded()))%")
    }
  }

  private var progressSection: some View {
    VStack(spacing: 10) {
      Text("Progress Bar Animation").font(.headline)
      Text("Watch the bar smoothly transition between values").font(.body)
      progressBar(0.3)
      VStack(spacing: 10) {
        HStack(spacing: 10) {
          Button("0%") {}
          Button("25%") {}
          Button("50%") {}
        }
        HStack(spacing: 10) {
          Button("75%") {}
          Button("100%") {}
        }
      }
    }
    .padding(14)
  }

  private var springSection: some View {
    VStack(spacing: 10) {
      Text("Spring Physics Animation").font(.headline)
      Text("Notice the bouncy overshoot effect").font(.body)
      progressBar(0.2)
      HStack(spacing: 10) {
        Text("Low")
          .padding(14)
          .background(srgbHex(0x9C27B0).opacity(0.3))
        Spacer(minLength: 0)
        Button("Toggle") {}
      }
    }
    .padding(14)
  }

  private var sizeIndicatorSection: some View {
    VStack(spacing: 10) {
      Text("Size Indicator").font(.headline)
      Text("All elements respond to slider with different animations")
      HStack(spacing: 10) {
        srgbHex(0x2196F3)
          .frame(width: 30, height: 100)
          .scaleEffect(x: 1, y: 0.5)
          .frame(minWidth: 50, minHeight: 120)
        Spacer(minLength: 0)
        srgbHex(0xF44336)
          .frame(width: 50, height: 50)
          .rotationEffect(.degrees(180))
          .frame(minWidth: 80, minHeight: 80)
        Spacer(minLength: 0)
        srgbHex(0x4CAF50)
          .frame(width: 60, height: 60)
          .scaleEffect(0.6)
          .frame(minWidth: 100, minHeight: 100)
      }
      Slider(value: .constant(50), in: 0 ... 100).labelsHidden()
      VStack(spacing: 10) {
        HStack(spacing: 10) {
          Button("0") {}
          Button("25") {}
          Button("50") {}
        }
        HStack(spacing: 10) {
          Button("75") {}
          Button("100") {}
        }
      }
    }
    .padding(14)
  }

  private func curveRow(_ label: String, color: UInt32) -> some View {
    HStack(spacing: 10) {
      Text(label).frame(minWidth: 80)
      srgbHex(color)
        .frame(width: 200, height: 24)
        .scaleEffect(x: 0.7, y: 1)
        .frame(minWidth: 220)
    }
  }

  private var curvesSection: some View {
    VStack(spacing: 10) {
      Text("Animation Curves Comparison").font(.headline)
      Text("Watch how different curves animate at different speeds")
      VStack(spacing: 10) {
        curveRow("Linear", color: 0x00BCD4)
        curveRow("Ease-In", color: 0x4CAF50)
        curveRow("Ease-Out", color: 0xFF9800)
        curveRow("Spring", color: 0xF44336)
      }
      HStack(spacing: 10) {
        Button("Small (0.3)") {}
        Button("Medium (0.7)") {}
        Button("Large (1.0)") {}
      }
    }
    .padding(14)
  }

  private var toggleSection: some View {
    VStack(spacing: 10) {
      Text("Toggle Animation").font(.headline)
      Text("Watch the indicator scale and rotate with toggle").font(.body)
      HStack(spacing: 10) {
        Toggle("Power", isOn: .constant(false))
        Spacer(minLength: 0)
        srgbHex(0x4CAF50)
          .frame(width: 60, height: 60)
          .scaleEffect(0.3)
          .rotationEffect(.degrees(180))
          .frame(minWidth: 80, minHeight: 80)
      }
    }
    .padding(14)
  }

  private var staggeredSection: some View {
    VStack(spacing: 10) {
      Text("Staggered Animations").font(.headline)
      Text("Different spring stiffness creates cascading effect").font(.body)
      HStack(spacing: 10) {
        ForEach([0x4CAF50, 0x2196F3, 0x9C27B0, 0xFF9800] as [UInt32], id: \.self) { color in
          srgbHex(color)
            .frame(width: 50, height: 80)
            .frame(minWidth: 60, minHeight: 100)
        }
      }
      Button("Toggle Bars") {}
    }
    .padding(14)
  }
}
