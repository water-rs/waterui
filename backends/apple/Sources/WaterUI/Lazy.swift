//
//  Lazy.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//
import CWaterUI
import Combine
import SwiftUI

struct WuiLazy: WuiComponent, View {
    static var id: WuiTypeId { waterui_lazy_id() }

    let views: WuiAnyViews
    let env: WuiEnvironment

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(views: waterui_force_as_lazy(anyview), env: env)
    }

    init(views: CWaterUI.WuiLazy, env: WuiEnvironment) {
        self.views = WuiAnyViews(views.contents)
        self.env = env
    }

    var body: some View {
        ScrollView {
            LazyVStack {
                ForEach(WuiAnyViewCollection(views).enumerated(), id: \.element) { index, id in
                    views.getView(at: index, env: env).id(id)
                }
            }
        }
    }
}
