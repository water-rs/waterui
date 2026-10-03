// Twin of examples/edge_text: extreme text cases inside a leading-aligned
// scroll stack — unbreakable runs, combining marks, emoji ZWJ sequences,
// mixed scripts, empty text, size extremes, wrapping paragraph.

import SwiftUI

struct EdgeTextTwin: View {
  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 10) {
        Text("Edge Text").font(.title)
        Text("Extreme text measurement and line breaking")
          .font(.subheadline)
          .foregroundStyle(.secondary)
        wuiDivider()
        VStack(alignment: .leading, spacing: 10) {
          section("Unbroken 160-char string") {
            Text(
              "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789"
            )
          }
          wuiDivider()
          section("Combining marks") {
            Text("cafe\u{0301} nai\u{0308}ve a\u{0328} o\u{0323}\u{0302} Z\u{0350}")
          }
          wuiDivider()
          section("Emoji sequences") {
            Text("👨\u{200D}👩\u{200D}👧\u{200D}👦 🏳\u{FE0F}\u{200D}🌈 👍🏽 🇺🇳 1\u{FE0F}\u{20E3}")
          }
          wuiDivider()
          section("Mixed scripts") {
            Text("Latin 中文 العربية עברית 日本語 한국어 123")
          }
        }
        VStack(alignment: .leading, spacing: 10) {
          wuiDivider()
          section("Empty and whitespace") {
            VStack(alignment: .leading, spacing: 4) {
              HStack(spacing: 10) {
                Text("[before]")
                Text("")
                Text("[after]")
              }
              HStack(spacing: 10) {
                Text("[before]")
                Text("   ")
                Text("[after]")
              }
            }
          }
          wuiDivider()
          section("Size extremes") {
            HStack(alignment: .lastTextBaseline, spacing: 8) {
              Text("tiny 9").font(.system(size: 9))
              Text("huge 48").font(.system(size: 48))
              Text("body").font(.body)
            }
          }
          wuiDivider()
          section("Wrapping paragraph") {
            Text(
              "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump! Sphinx of black quartz, judge my vow. 敏捷的棕色狐狸跳过懒狗。素早い茶色のキツネが怠けた犬を飛び越える。"
            )
          }
          Spacer(minLength: 16)
        }
      }
      .padding(16)
    }
  }

  private func section<Content: View>(
    _ title: String, @ViewBuilder content: () -> Content
  ) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(title).font(.subheadline).foregroundStyle(.secondary)
      content()
    }
    .padding(12)
  }
}
