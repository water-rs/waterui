import CWaterUI
import SwiftUI

struct Spacer: View, Component {
    static var id = waterui_spacer_id()

    var body: some View {
        SwiftUI.Spacer()
    }

    init(anyview: OpaquePointer, env: Environment) {

    }
}
