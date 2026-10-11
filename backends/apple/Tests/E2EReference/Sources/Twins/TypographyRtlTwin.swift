// Twin of examples/typography-rtl: semantic type styles, per-locale CJK
// specimens, and three localized reading-order panels (en LTR, ar RTL,
// he RTL).

import SwiftUI

struct TypographyRtlTwin: View {
  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 16) {
        Text("Typography & Bidirectional Layout").font(.headline)
        Text(
          "Semantic type styles, CJK fallback, mixed-script shaping, and logical RTL layout."
        )
        .font(.body)
        .foregroundStyle(.secondary)
        wuiDivider()
        typeScale
        wuiDivider()
        cjkSpecimens
        wuiDivider()
        Text("Logical reading order").font(.title)
        panel(
          locale: "en",
          direction: .leftToRight,
          title: "Left-to-right",
          body: "A logical HStack starts here",
          arrow: "→"
        )
        panel(
          locale: "ar",
          direction: .rightToLeft,
          title: "من اليمين إلى اليسار",
          body: "يبدأ الصف المنطقي من هنا",
          arrow: "←"
        )
        panel(
          locale: "he",
          direction: .rightToLeft,
          title: "מימין לשמאל",
          body: "השורה הלוגית מתחילה כאן",
          arrow: "←"
        )
      }
      .padding(20)
    }
  }

  private var typeScale: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Display / 展示 / عرض").font(.headline)
      Text("Title / 标题 / כותרת").font(.title)
      Text("Headline / 小标题 / عنوان").font(.subheadline)
      Text("Body — WaterUI shapes العربية、中文、日本語、한국어 in one paragraph.").font(.body)
      Text("Footnote — Mixed scripts keep punctuation and numbers stable: الإصدار 2.0 (2026).")
        .font(.footnote)
      Text("Caption — 字体 fallback follows locale and script.")
        .font(.caption)
        .foregroundStyle(.secondary)
    }
  }

  private var cjkSpecimens: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text("CJK locale-aware glyph selection").font(.subheadline)
      Text("简体中文：骨、直、门、关").font(.body)
        .environment(\.locale, Locale(identifier: "zh_CN"))
      Text("繁體中文：骨、直、門、關").font(.body)
        .environment(\.locale, Locale(identifier: "zh_TW"))
      Text("日本語：骨、直、門、関").font(.body)
        .environment(\.locale, Locale(identifier: "ja"))
      Text("한국어: 한글과 漢字").font(.body)
        .environment(\.locale, Locale(identifier: "ko"))
    }
  }

  private func panel(
    locale: String,
    direction: LayoutDirection,
    title: String,
    body: String,
    arrow: String
  ) -> some View {
    VStack(alignment: .leading, spacing: 12) {
      Text(title).font(.subheadline)
      HStack(spacing: 12) {
        Text("①").font(.headline)
        VStack(alignment: .leading, spacing: 10) {
          Text(body).font(.body)
          Text("WaterUI 2.0 · 2026")
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        Spacer(minLength: 0)
        Text(arrow).font(.headline)
      }
      VStack(alignment: .leading, spacing: 4) {
        Text("Name / الاسم / שם")
        TextField("Type here / اكتب هنا / הקלידו כאן", text: .constant(""))
      }
    }
    .padding(16)
    #if os(iOS)
      .background(Color(.secondarySystemBackground))
    #else
      .background(Color(.controlBackgroundColor))
    #endif
    .environment(\.locale, Locale(identifier: locale))
    .environment(\.layoutDirection, direction)
  }
}
