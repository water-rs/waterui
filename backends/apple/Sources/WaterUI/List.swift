import CWaterUI
import SwiftUI

@MainActor
struct WuiList: WuiComponent, View {
    static let id: String = decodeViewIdentifier(waterui_list_id())

    let views: WuiAnyViews
    let env: WuiEnvironment

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(list: waterui_force_as_list(anyview), env: env)
    }

    init(list: CWaterUI.WuiList, env: WuiEnvironment) {
        self.views = WuiAnyViews(list.contents)
        self.env = env
    }

    var body: some View {
        List {
            views.intoForEach(env: env)
        }
    }
}

struct WuiListItem: WuiComponent, View {
    static let id: String = decodeViewIdentifier(waterui_list_item_id())

    let content: WuiAnyView

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.content = WuiAnyView(anyview: waterui_force_as_list_item(anyview).content, env: env)
    }

    init(content: WuiAnyView) {
        self.content = content
    }

    var body: some View {
        content
    }
}
