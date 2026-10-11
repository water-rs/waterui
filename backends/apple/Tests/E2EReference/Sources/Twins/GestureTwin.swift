// Twin of examples/gesture: seven gesture demo sections, each a labelled box
// with a tinted background and a counter label. Initial state only — all
// counters are zero, four tabs are open, and the chained status reads
// "Waiting for tap...".

import SwiftUI

private let tapColor = srgbHex(0x2196F3)
private let doubleTapColor = srgbHex(0x4CAF50)
private let longPressColor = srgbHex(0xFF9800)
private let dragColor = srgbHex(0x9C27B0)
private let chainedColor = srgbHex(0xF44336)
private let middleClickColor = srgbHex(0x607D8B)
private let onTapColor = srgbHex(0x00BCD4)

struct GestureTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Gesture Examples").font(.title)
        Text("Demonstrating gesture recognition and handling")
        wuiDivider()
        Spacer(minLength: 0)
        tapSection
        wuiDivider()
        doubleTapSection
        wuiDivider()
        longPressSection
        wuiDivider()
        dragSection
        wuiDivider()
        chainedSection
        wuiDivider()
        VStack(spacing: 10) {
          middleClickSection
          onTapSection
        }
      }
      .padding(16)
    }
  }

  private var tapSection: some View {
    GestureSection(
      title: "Tap Gesture",
      subtitle: "Tap the box below to increment the counter",
      counter: "Tap count: 0",
      label: "Tap Me!",
      color: tapColor
    )
  }

  private var doubleTapSection: some View {
    GestureSection(
      title: "Double Tap Gesture",
      subtitle: "Double-tap the box to increment",
      counter: "Double tap count: 0",
      label: "Double Tap Me!",
      color: doubleTapColor
    )
  }

  private var longPressSection: some View {
    GestureSection(
      title: "Long Press Gesture",
      subtitle: "Press and hold for 500ms",
      counter: "Long press count: 0",
      label: "Long Press Me!",
      color: longPressColor
    )
  }

  private var dragSection: some View {
    VStack(spacing: 10) {
      Text("Drag Gesture").font(.headline)
      Text("Drag within the box (min 5pt)")
      Text("Drag events: 0")
      Text("Drag Here")
        .padding(14)
        .frame(width: 200, height: 100)
        .background(dragColor.opacity(0.3))
    }
    .padding(14)
  }

  private var chainedSection: some View {
    GestureSection(
      title: "Chained Gesture",
      subtitle: "Tap first, then long press to complete",
      counter: "Waiting for tap...",
      label: "Tap then Long Press",
      color: chainedColor
    )
  }

  private var middleClickSection: some View {
    VStack(spacing: 10) {
      Text("Middle Click").font(.headline)
      Text("Middle-click a tab to close it")
      Text("4 open tabs")
      ScrollView(.horizontal) {
        HStack(spacing: 8) {
          ForEach(["Overview", "Details", "Activity", "Settings"], id: \.self) { title in
            Text(title)
              .padding(14)
              .background(middleClickColor.opacity(0.3))
          }
        }
      }
    }
    .padding(14)
  }

  private var onTapSection: some View {
    VStack(spacing: 10) {
      Text("on_tap Shorthand").font(.headline)
      Text("Convenient method for simple tap handlers")
      Text("This uses the same counter as Section 1")
      Text("Simple Tap")
        .padding(14)
        .background(onTapColor.opacity(0.3))
    }
    .padding(14)
  }
}

private struct GestureSection: View {
  let title: String
  let subtitle: String
  let counter: String
  let label: String
  let color: Color

  var body: some View {
    VStack(spacing: 10) {
      Text(title).font(.headline)
      Text(subtitle)
      Text(counter)
      Text(label)
        .padding(14)
        .background(color.opacity(0.3))
    }
    .padding(14)
  }
}
