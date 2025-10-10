import SwiftUI
import CWaterUI

@MainActor
struct WuiList: WuiComponent, View {
    static var id: WuiTypeId { waterui_list_id() }

    private var items: [WuiAnyView]

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let list = waterui_force_as_list(anyview)
        let pointer = list.contents.map { UnsafeMutableRawPointer($0) }
        let collection = WuiAnyViewCollection(pointer, env: env)
        self.items = collection.toArray()
    }

    var body: some View {
        List {
            ForEach(items) { item in
                item
            }
        }
        .listStyle(.plain)
    }
}
