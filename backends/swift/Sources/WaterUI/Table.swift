import SwiftUI
import CWaterUI

@MainActor
struct WuiTable: WuiComponent, View {
    static var id: WuiTypeId { waterui_table_id() }

    struct Column: Identifiable {
        let id: Int
        let rows: [WuiAnyView]
    }

    private var columns: [Column]

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let table = waterui_force_as_table(anyview)
        let columnArray = WuiArray<CWaterUI.WuiTableColumn>(table.columns)
        self.columns = columnArray.toArray().enumerated().map { index, column in
            let pointerArray = WuiArray<OpaquePointer>(column.rows)
            let rows = pointerArray.toArray().map { WuiAnyView(anyview: $0, env: env) }
            return Column(id: index, rows: rows)
        }
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(alignment: .top, spacing: 12) {
                ForEach(columns) { column in
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(column.rows) { row in
                            row
                        }
                    }
                }
            }
            .padding(.vertical, 8)
        }
    }
}
