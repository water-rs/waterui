// Twin of examples/list: header controls over a 100,000-row lazy list.
// Initial state: 100000 rows, not editing, scrolled to top.

import SwiftUI

private let datasetSize = 100_000

struct ListTwin: View {
  var body: some View {
    VStack(spacing: 10) {
      VStack(alignment: .leading, spacing: 8) {
        Text("100,000-row lazy List").font(.title)
        Text("100,000 active rows")
          .font(.subheadline)
          .foregroundStyle(.secondary)
        HStack(spacing: 8) {
          Button("Top") {}.buttonStyle(.bordered)
          Button("Middle") {}.buttonStyle(.bordered)
          Button("Last") {}.buttonStyle(.bordered)
          Button("Edit") {}.buttonStyle(.borderedProminent)
        }
        Text("Animated jumps and the draggable scrollbar keep only viewport rows materialized.")
          .font(.caption)
          .foregroundStyle(.secondary)
      }
      .padding(14)
      wuiDivider()
      List(0 ..< datasetSize, id: \.self) { index in
        VStack(alignment: .leading, spacing: 10) {
          Text(String(format: "Record #%06d", index))
            .font(.subheadline)
            .foregroundStyle(.primary)
          Text("Materialized only while this row is visible")
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 16)
      }
    }
  }
}
