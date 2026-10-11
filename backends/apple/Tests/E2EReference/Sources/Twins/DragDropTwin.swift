// Twin of examples/drag_drop: fruit cards over a bordered drop basket. The
// basket's initial state is the empty "🧺 / Drop fruits here!" presentation.

import SwiftUI

private let fruitCardWidth: CGFloat = 160

struct DragDropTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("Fruit Basket").font(.title)
        Text("Drag fruits into the basket!")
        wuiDivider()
        VStack(spacing: 12) {
          Text("Drag these fruits").font(.system(size: 14))
          HStack(spacing: 12) {
            fruitCard("🍎", "Apple", srgbHex(0xEF4444))
            fruitCard("🍊", "Orange", srgbHex(0xF97316))
          }
          HStack(spacing: 12) {
            fruitCard("🍋", "Lemon", srgbHex(0xEAB308))
            fruitCard("🍇", "Grape", srgbHex(0x8B5CF6))
          }
          HStack(spacing: 12) {
            fruitCard("🍓", "Strawberry", srgbHex(0xEC4899))
            fruitCard("🥝", "Kiwi", srgbHex(0x22C55E))
          }
        }
        .padding(14)
        Spacer(minLength: 0).frame(height: 32)
        basket
        Spacer(minLength: 0).frame(height: 16)
        Button("Clear Basket") {}
        Spacer(minLength: 0)
      }
      .padding(14)
    }
  }

  private var basket: some View {
    VStack(spacing: 12) {
      Text("🧺").font(.system(size: 40))
      Text("Drop fruits here!").font(.system(size: 14))
    }
    .padding(24)
    .frame(minWidth: 280, minHeight: 120)
    .background(srgbHex(0x10B981).opacity(0.2))
    .border(srgbHex(0x10B981), width: 3)
  }

  private func fruitCard(_ emoji: String, _ label: String, _ color: Color) -> some View {
    HStack(spacing: 8) {
      Text(emoji).font(.system(size: 28))
      Text(label).font(.system(size: 16))
    }
    .padding(14)
    .frame(width: fruitCardWidth)
    .background(color.opacity(0.9))
  }
}
