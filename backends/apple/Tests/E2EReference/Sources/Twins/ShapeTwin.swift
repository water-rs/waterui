// Twin of examples/shape: shape gallery inside a padded scroll stack.
//
// WaterUI normalized corner radii are fractions of the shorter side, so
// RoundedRectangle::new(0.1) at 80x60 is cornerRadius 6 in SwiftUI.
// WaterUI Path coordinates are normalized to the shape's bounds, reproduced
// here with a unit-space Shape that scales into `path(in:)`.
//
// morph_demo animates continuously; the twin renders the morph's base shape
// statically. The section sits below the fold of the 800x600 reference
// window, so it does not affect the compared capture.

import SwiftUI

private func pt(_ x: CGFloat, _ y: CGFloat, in rect: CGRect) -> CGPoint {
  CGPoint(x: rect.minX + x * rect.width, y: rect.minY + y * rect.height)
}

private struct UnitPath: Shape {
  let build: @Sendable (inout Path, CGRect) -> Void

  func path(in rect: CGRect) -> Path {
    var path = Path()
    build(&path, rect)
    return path
  }
}

private func starShape(points: Int, innerRatio: CGFloat) -> UnitPath {
  UnitPath { path, rect in
    let outer: CGFloat = 0.5
    let inner = outer * innerRatio
    for i in 0 ..< (points * 2) {
      let angle = CGFloat(i) * .pi / CGFloat(points) - .pi / 2
      let radius = i.isMultiple(of: 2) ? outer : inner
      let p = pt(0.5 + radius * cos(angle), 0.5 + radius * sin(angle), in: rect)
      if i == 0 { path.move(to: p) } else { path.addLine(to: p) }
    }
    path.closeSubpath()
  }
}

private func polygonShape(_ vertices: [(CGFloat, CGFloat)]) -> UnitPath {
  UnitPath { path, rect in
    for (i, v) in vertices.enumerated() {
      let p = pt(v.0, v.1, in: rect)
      if i == 0 { path.move(to: p) } else { path.addLine(to: p) }
    }
    path.closeSubpath()
  }
}

private var heartShape: UnitPath {
  UnitPath { path, rect in
    path.move(to: pt(0.5, 0.2, in: rect))
    path.addCurve(
      to: pt(0.0, 0.3, in: rect),
      control1: pt(0.5, 0.0, in: rect),
      control2: pt(0.0, 0.0, in: rect)
    )
    path.addCurve(
      to: pt(0.5, 1.0, in: rect),
      control1: pt(0.0, 0.6, in: rect),
      control2: pt(0.5, 0.8, in: rect)
    )
    path.addCurve(
      to: pt(1.0, 0.3, in: rect),
      control1: pt(0.5, 0.8, in: rect),
      control2: pt(1.0, 0.6, in: rect)
    )
    path.addCurve(
      to: pt(0.5, 0.2, in: rect),
      control1: pt(1.0, 0.0, in: rect),
      control2: pt(0.5, 0.0, in: rect)
    )
    path.closeSubpath()
  }
}

private func hexagonShape() -> UnitPath {
  polygonShape((0 ..< 6).map { i in
    let angle = CGFloat(i) * .pi / 3 - .pi / 2
    return (0.5 + 0.5 * cos(angle), 0.5 + 0.5 * sin(angle))
  })
}

struct ShapeTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Shape Examples").font(.system(size: 28))
        Text("Shapes and clipping demonstrations")
        wuiDivider()
        VStack(spacing: 10) {
          circleDemo
          wuiDivider()
          ellipseDemo
          wuiDivider()
          capsuleDemo
          wuiDivider()
          rectangleDemo
        }
        VStack(spacing: 10) {
          wuiDivider()
          roundedRectangleDemo
          wuiDivider()
          unevenRoundedRectangleDemo
          wuiDivider()
          customPathDemo
          wuiDivider()
          hdrDemo
        }
        VStack(spacing: 10) {
          wuiDivider()
          clipShowcase
          wuiDivider()
          morphDemo
          wuiDivider()
          layoutDemo
          Spacer(minLength: 32)
        }
      }
      .padding(16)
    }
  }

  private func demo<Content: View>(
    _ title: String, _ subtitle: String, @ViewBuilder content: () -> Content
  ) -> some View {
    VStack(spacing: 10) {
      Text(title).font(.system(size: 18))
      Text(subtitle)
      content()
    }
    .padding(14)
  }

  private var circleDemo: some View {
    demo("Circle", "Inscribed in the view bounds") {
      HStack(spacing: 16) {
        Circle().fill(srgbHex(0x3B82F6)).frame(width: 80, height: 80)
        ZStack {
          srgbHex(0x10B981)
          Text("Clipped").foregroundStyle(.white)
        }
        .frame(width: 80, height: 80)
        .clipShape(Circle())
      }
    }
  }

  private var ellipseDemo: some View {
    demo("Ellipse", "Fills the view bounds as an ellipse") {
      HStack(spacing: 16) {
        Ellipse().fill(srgbHex(0x8B5CF6)).frame(width: 120, height: 60)
        ZStack {
          srgbHex(0xF59E0B)
          Text("Clipped").foregroundStyle(.white)
        }
        .frame(width: 120, height: 60)
        .clipShape(Ellipse())
      }
    }
  }

  private var capsuleDemo: some View {
    demo("Capsule", "Pill shape with fully rounded ends") {
      HStack(spacing: 16) {
        Capsule().fill(srgbHex(0xEC4899)).frame(width: 120, height: 50)
        Capsule().fill(srgbHex(0x06B6D4)).frame(width: 50, height: 100)
      }
      ZStack {
        Capsule().fill(srgbHex(0x3B82F6))
        Text("Capsule Button").foregroundStyle(.white).padding(14)
      }
      .frame(width: 150, height: 44)
    }
  }

  private var rectangleDemo: some View {
    demo("Rectangle", "Sharp corners") {
      HStack(spacing: 16) {
        Rectangle().fill(srgbHex(0xEF4444)).frame(width: 80, height: 60)
        ZStack {
          srgbHex(0x84CC16)
          Text("Rect").foregroundStyle(.white)
        }
        .frame(width: 80, height: 60)
        .clipShape(Rectangle())
      }
    }
  }

  private var roundedRectangleDemo: some View {
    demo("Rounded Rectangle", "Uniform corner radius (0.0-0.5)") {
      HStack(spacing: 12) {
        RoundedRectangle(cornerRadius: 0.1 * 60)
          .fill(srgbHex(0x14B8A6)).frame(width: 80, height: 60)
        RoundedRectangle(cornerRadius: 0.2 * 60)
          .fill(srgbHex(0xF97316)).frame(width: 80, height: 60)
        RoundedRectangle(cornerRadius: 0.4 * 60)
          .fill(srgbHex(0xA855F7)).frame(width: 80, height: 60)
      }
    }
  }

  private var unevenRoundedRectangleDemo: some View {
    demo("Uneven Rounded Rectangle", "Independent corner radii") {
      HStack(spacing: 12) {
        UnevenRoundedRectangle(
          topLeadingRadius: 0.3 * 60, bottomLeadingRadius: 0,
          bottomTrailingRadius: 0, topTrailingRadius: 0.3 * 60
        )
        .fill(srgbHex(0x0EA5E9)).frame(width: 80, height: 60)
        UnevenRoundedRectangle(
          topLeadingRadius: 0.3 * 60, bottomLeadingRadius: 0.3 * 60,
          bottomTrailingRadius: 0, topTrailingRadius: 0
        )
        .fill(srgbHex(0xF43F5E)).frame(width: 80, height: 60)
        UnevenRoundedRectangle(
          topLeadingRadius: 0, bottomLeadingRadius: 0.3 * 60,
          bottomTrailingRadius: 0.3 * 60, topTrailingRadius: 0
        )
        .fill(srgbHex(0x22C55E)).frame(width: 80, height: 60)
      }
    }
  }

  private var customPathDemo: some View {
    demo("Custom Paths", "Build arbitrary shapes with Path") {
      HStack(spacing: 12) {
        polygonShape([(0.5, 0), (1, 1), (0, 1)])
          .fill(srgbHex(0xFBBF24)).frame(width: 70, height: 70)
        starShape(points: 5, innerRatio: 0.4)
          .fill(srgbHex(0xF472B6)).frame(width: 70, height: 70)
        heartShape
          .fill(srgbHex(0xEF4444)).frame(width: 70, height: 70)
        polygonShape([(0.5, 0), (1, 0.4), (0.7, 0.4), (0.7, 1), (0.3, 1), (0.3, 0.4), (0, 0.4)])
          .fill(srgbHex(0x6366F1)).frame(width: 70, height: 70)
      }
    }
  }

  private var hdrShapes: some View {
    HStack(spacing: 12) {
      ZStack {
        RoundedRectangle(cornerRadius: 0.18 * 60).fill(srgb(0.9, 0.2, 0.35))
        Text("SDR").foregroundStyle(.white)
      }
      .frame(width: 100, height: 60)
      ZStack {
        RoundedRectangle(cornerRadius: 0.18 * 60).fill(srgb(0.9, 0.2, 0.35))
        Text("HDR").foregroundStyle(.white)
      }
      .frame(width: 100, height: 60)
      Circle().fill(srgb(0.2, 0.6, 1.0)).frame(width: 60, height: 60)
    }
  }

  private func srgb(_ r: Double, _ g: Double, _ b: Double) -> Color {
    Color(.sRGB, red: r, green: g, blue: b)
  }

  private var hdrDemo: some View {
    VStack(spacing: 10) {
      Text("HDR Shapes").font(.system(size: 18))
      Text("Extended range colors via headroom")
      Toggle("Show HDR", isOn: .constant(true))
      ZStack {
        hdrShapes.opacity(0)
        hdrShapes
      }
      .frame(width: 284, height: 60)
    }
    .padding(14)
  }

  private var clipShowcase: some View {
    demo("Clipping Views", "Apply shapes as masks to any view") {
      HStack(spacing: 12) {
        ZStack {
          srgbHex(0x818CF8)
          VStack(spacing: 10) {
            Text("Avatar").foregroundStyle(.white)
            Text("Image").foregroundStyle(srgb(200 / 255, 200 / 255, 200 / 255))
          }
        }
        .frame(width: 80, height: 80)
        .clipShape(Circle())
        ZStack {
          srgbHex(0x1E293B)
          VStack(spacing: 10) {
            Text("Card").foregroundStyle(.white)
            Text("Content").foregroundStyle(srgb(156 / 255, 163 / 255, 175 / 255))
          }
        }
        .frame(width: 100, height: 80)
        .clipShape(RoundedRectangle(cornerRadius: 0.15 * 80))
        ZStack {
          srgbHex(0x059669)
          Text("Hex").foregroundStyle(.white)
        }
        .frame(width: 80, height: 80)
        .clipShape(hexagonShape())
      }
    }
  }

  private var morphDemo: some View {
    demo("Morph Animation", "SDF-based shape morphing for built-in shapes") {
      HStack(spacing: 12) {
        Circle().fill(srgbHex(0x3B82F6)).frame(width: 90, height: 90)
        RoundedRectangle(cornerRadius: 0.08 * 70)
          .fill(srgbHex(0x10B981)).frame(width: 120, height: 70)
        Rectangle().fill(srgbHex(0xF97316)).frame(width: 110, height: 70)
      }
    }
  }

  private var layoutDemo: some View {
    demo("Layout Behavior", "Shapes fill available space like Color") {
      HStack(spacing: 16) {
        ZStack {
          srgbHex(0xE5E7EB)
          Circle().fill(srgbHex(0x3B82F6))
        }
        .frame(width: 100, height: 100)
        VStack(spacing: 10) {
          Text("Above").foregroundStyle(srgb(107 / 255, 114 / 255, 128 / 255))
          RoundedRectangle(cornerRadius: 0.2 * 100).fill(srgbHex(0x10B981))
          Text("Below").foregroundStyle(srgb(107 / 255, 114 / 255, 128 / 255))
        }
        .frame(width: 100, height: 120)
      }
    }
  }
}
