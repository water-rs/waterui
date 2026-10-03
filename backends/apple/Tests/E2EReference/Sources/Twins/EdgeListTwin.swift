// Twin of examples/edge_list: 200,000 rows whose heights vary 1-3 detail
// lines, each led by a colored chip. Initial state: scrolled to top.

import SwiftUI

private let rowCount = 200_000

struct EdgeListTwin: View {
  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      VStack(alignment: .leading, spacing: 10) {
        Text("200,000 variable-height rows").font(.title)
        Text("Rows cycle 1-3 detail lines; only viewport rows materialize.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
      }
      .padding(14)
      wuiDivider()
      List(0 ..< rowCount, id: \.self) { index in
        row(index)
      }
    }
  }

  private func chipColor(_ index: Int) -> Color {
    switch index % 4 {
    case 0: return srgbHex(0x3B82F6)
    case 1: return srgbHex(0x10B981)
    case 2: return srgbHex(0xF59E0B)
    default: return srgbHex(0xEF4444)
    }
  }

  private func detailLine(_ n: Int, _ label: String) -> some View {
    Text("detail line \(n) \(label)")
      .font(.caption)
      .foregroundStyle(.secondary)
  }

  private func row(_ index: Int) -> some View {
    let lines = index % 3 + 1
    return VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 8) {
        Circle().fill(chipColor(index)).frame(width: 10, height: 10)
        Text("Row #\(index) - \(lines) detail line(s)").font(.subheadline)
      }
      detailLine(1, "always present")
      if lines >= 2 {
        detailLine(2, "makes this row taller")
      }
      if lines >= 3 {
        detailLine(3, "tallest variant")
      }
    }
    .padding(.vertical, 8)
    .padding(.horizontal, 16)
  }
}
