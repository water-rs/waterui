// Twin of examples/hover: hover/cursor demo sections. Initial state only —
// nothing is hovered or dragged, so counters read zero and both zstack colour
// layers resolve to their inactive opacity.

import SwiftUI

private let hoverActive = srgbHex(0x4CAF50)
private let hoverInactive = srgbHex(0x2196F3)
private let dragActive = srgbHex(0xFF5722)
private let dragInactive = srgbHex(0xFF9800)

private let cursorColors: [(String, UInt32)] = [
  ("Arrow", 0x9E9E9E),
  ("Hand", 0x2196F3),
  ("Text", 0x4CAF50),
  ("Cross", 0xFF9800),
  ("Grab", 0x9C27B0),
  ("Grabbing", 0x673AB7),
  ("No", 0xF44336),
  ("Wait", 0x795548),
  ("H-Resize", 0x00BCD4),
  ("V-Resize", 0x009688),
  ("Move", 0x607D8B),
  ("Copy", 0x8BC34A),
]

struct HoverTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Hover & Cursor Examples").font(.title)
        Text("Demonstrating hover events, cursor styles, and lifecycle hooks")
        wuiDivider()
        Spacer(minLength: 0)
        hoverEvents
        wuiDivider()
        cursorStyles
        wuiDivider()
        reactiveCursor
        wuiDivider()
        interactiveButtons
        Spacer(minLength: 0)
      }
      .padding(14)
    }
  }

  private var hoverEvents: some View {
    VStack(spacing: 10) {
      Text("Hover Events").font(.headline)
      Text("Move your pointer in and out of the box")
      HStack(spacing: 10) {
        Text("Hover events: ")
        Text("Count: 0")
      }
      HStack(spacing: 10) {
        Text("Currently hovered: ")
        Text("Status: false")
      }
      ZStack {
        hoverInactive.opacity(0.3)
        hoverActive.opacity(0.5).opacity(0)
        Text("Hover Me!").padding(14)
      }
      .frame(width: 200, height: 80)
    }
    .padding(14)
  }

  private var cursorStyles: some View {
    VStack(spacing: 10) {
      Text("Cursor Styles").font(.headline)
      Text("Hover over each box to see different cursor styles")
      HStack(spacing: 10) {
        ForEach(0 ..< 4, id: \.self) { cursorBox(cursorColors[$0]) }
      }
      HStack(spacing: 10) {
        ForEach(4 ..< 8, id: \.self) { cursorBox(cursorColors[$0]) }
      }
      HStack(spacing: 10) {
        ForEach(8 ..< 12, id: \.self) { cursorBox(cursorColors[$0]) }
      }
    }
    .padding(14)
  }

  private func cursorBox(_ entry: (String, UInt32)) -> some View {
    Text(entry.0)
      .font(.caption)
      .padding(14)
      .frame(width: 96, height: 44)
      .background(srgbHex(entry.1).opacity(0.3))
  }

  private var reactiveCursor: some View {
    VStack(spacing: 10) {
      Text("Reactive Cursor").font(.headline)
      Text("The cursor changes based on drag state")
      HStack(spacing: 10) {
        Text("State: ")
        Text("Dragging: false")
      }
      Text("(Hover to simulate drag state change)")
      ZStack {
        dragInactive.opacity(0.3)
        dragActive.opacity(0.5).opacity(0)
        Text("Drag Area").padding(14)
      }
      .frame(width: 200, height: 100)
    }
    .padding(14)
  }

  private var interactiveButtons: some View {
    VStack(spacing: 10) {
      Text("Interactive Buttons").font(.headline)
      Text("Buttons naturally have cursor changes")
      HStack(spacing: 10) {
        Button("Bordered") {}.buttonStyle(.bordered)
        Button("Plain") {}.buttonStyle(.plain)
        Button("Link Style") {}.buttonStyle(.plain).foregroundStyle(Color.accentColor).underline()
      }
      Text("Link buttons show pointer cursor by default")
    }
    .padding(14)
  }
}
