// Twin of examples/edge_layout: 8-deep alternating stack nesting, a 16x10
// eager grid, and frame-constraint edges inside a leading scroll stack.

import SwiftUI

struct EdgeLayoutTwin: View {
  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 10) {
        Text("Edge Layout").font(.title)
        Text("Deep nesting, dense children, constraint edges")
          .font(.subheadline)
          .foregroundStyle(.secondary)
        wuiDivider()
        Text("Deep nesting (8 levels)").font(.subheadline)
        deepNest(8)
        wuiDivider()
        Text("Dense grid (160 eager children)").font(.subheadline)
        denseGrid
        wuiDivider()
        Text("Frame constraints").font(.subheadline)
        constraintEdges
        Spacer(minLength: 16)
      }
      .padding(16)
    }
  }

  private func deepNest(_ depth: Int) -> AnyView {
    if depth == 0 {
      return AnyView(
        Text("depth 0").font(.caption).foregroundStyle(.secondary)
      )
    }
    let inner = deepNest(depth - 1)
    if depth.isMultiple(of: 2) {
      return AnyView(
        HStack(spacing: 2) {
          Text("·").font(.caption)
          inner
        }
        .padding(.horizontal, 2)
      )
    }
    return AnyView(
      VStack(alignment: .leading, spacing: 2) {
        Text("·").font(.caption)
        inner
      }
    )
  }

  private var denseGrid: some View {
    VStack(alignment: .leading, spacing: 4) {
      ForEach(0 ..< 16, id: \.self) { row in
        HStack(spacing: 4) {
          ForEach(0 ..< 10, id: \.self) { col in
            let hue = Double(row * 10 + col) / 160.0
            let (r, g, b) = hsl(hue, 0.7, 0.55)
            RoundedRectangle(cornerRadius: 0.2 * 18)
              .fill(Color(.sRGB, red: r, green: g, blue: b))
              .frame(width: 28, height: 18)
          }
        }
      }
    }
  }

  private var constraintEdges: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 8) {
        Text("zero:").font(.caption)
        RoundedRectangle(cornerRadius: 0)
          .fill(srgbHex(0xEF4444))
          .frame(width: 0, height: 0)
        Text("after zero").font(.caption).foregroundStyle(.secondary)
      }
      HStack(spacing: 8) {
        Text("min 200:").font(.caption)
        Text("x")
          .frame(minWidth: 200, minHeight: 24)
          .background(srgbHex(0xDBEAFE))
      }
      HStack(spacing: 8) {
        Text("max 60:").font(.caption)
        Text("this label is far too long to fit")
          .frame(maxWidth: 60)
      }
    }
  }

  private func hsl(_ h: Double, _ s: Double, _ l: Double) -> (Double, Double, Double) {
    let c = (1 - abs(2 * l - 1)) * s
    let hp = h * 6
    let x = c * (1 - abs(hp.truncatingRemainder(dividingBy: 2) - 1))
    let (r1, g1, b1): (Double, Double, Double)
    switch Int(hp) {
    case 0: (r1, g1, b1) = (c, x, 0)
    case 1: (r1, g1, b1) = (x, c, 0)
    case 2: (r1, g1, b1) = (0, c, x)
    case 3: (r1, g1, b1) = (0, x, c)
    case 4: (r1, g1, b1) = (x, 0, c)
    default: (r1, g1, b1) = (c, 0, x)
    }
    let m = l - c / 2
    return (r1 + m, g1 + m, b1 + m)
  }
}
