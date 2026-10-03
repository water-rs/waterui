// Twin of examples/multi_window: five window-style cards with Open/Close
// buttons. All window states start Closed, so no secondary windows render.

import SwiftUI

struct MultiWindowTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("Multi-Window Gallery").font(.title).fontWeight(.bold)
        Text("Explore different window styles and backgrounds").font(.body)
        Spacer(minLength: 0).frame(height: 20)
        wuiDivider()
        Spacer(minLength: 0).frame(height: 20)
        VStack(spacing: 10) {
          windowSection(
            "Standard Titled Window",
            "Classic window with title bar and opaque background"
          )
          Spacer(minLength: 0).frame(height: 16)
          windowSection(
            "Borderless Window",
            "Frameless window with colored semi-transparent background"
          )
          Spacer(minLength: 0).frame(height: 16)
          windowSection(
            "Frosted Glass Window",
            "Window with material blur effect (Regular thickness)"
          )
          Spacer(minLength: 0).frame(height: 16)
          windowSection(
            "Transparent Overlay",
            "Fully transparent window with FullSizeContentView style"
          )
          Spacer(minLength: 0).frame(height: 16)
          windowSection(
            "Ultra-Thin Material Window",
            "Subtle frosted effect with UltraThin material"
          )
        }
        .padding(12)
        Spacer(minLength: 0)
        wuiDivider()
        Spacer(minLength: 0).frame(height: 12)
        Text("Built with WaterUI Multi-Window Support").font(.caption)
        Spacer(minLength: 0).frame(height: 12)
      }
      .padding(20)
    }
  }

  private func windowSection(_ title: String, _ description: String) -> some View {
    VStack(spacing: 10) {
      Text(title).font(.headline).fontWeight(.bold)
      Text(description).font(.body)
      Spacer(minLength: 0).frame(height: 8)
      HStack(spacing: 10) {
        Button("Open Window") {}
        Spacer(minLength: 0).frame(width: 12)
        Button("Close Window") {}
      }
    }
    .padding(16)
    #if os(iOS)
      .background(Color(uiColor: .tertiarySystemFill))
    #else
      .background(Color(nsColor: .tertiarySystemFill))
    #endif
  }
}
