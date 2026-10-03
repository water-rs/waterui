// Twin of examples/animation: transform stages and control rows. Initial
// state only — scale 1, rotation 0, offsets 0, progress 0 — so every stage
// shows its box centered at natural size and every bar is empty.
//
// The below-the-fold sections after the combined-transform block are omitted:
// only the settled first screen is compared.

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
}
