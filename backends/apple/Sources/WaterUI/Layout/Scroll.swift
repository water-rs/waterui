import CWaterUI
import SwiftUI

struct WuiScrollView: View, WuiComponent {
    static var id:WuiTypeId{
        waterui_scroll_view_id()
    }
    
    var content:WuiAnyView

    var body: some View {
        SwiftUI.ScrollView{
            content
        }
    }

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let scroll = waterui_force_as_scroll_view(anyview)
        self.content = .init(anyview: scroll.content, env: env)
    }
}

