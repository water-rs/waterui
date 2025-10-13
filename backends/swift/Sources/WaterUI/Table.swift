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
        let collection = WuiTableColumnCollection(UnsafeMutableRawPointer(table.columns), env: env)
        self.columns = collection.toArray().map { snapshot in
            Column(id: snapshot.id, rows: snapshot.rows)
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
