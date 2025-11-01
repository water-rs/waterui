//
//  Progress.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import CWaterUI
import Foundation
import SwiftUI

extension WuiProgressStyle: SwiftUI.ProgressViewStyle {
    public func makeBody(configuration: Configuration) -> some View {
        let view = SwiftUI.ProgressView(
            value: configuration.fractionCompleted, label: { configuration.label },
            currentValueLabel: { configuration.currentValueLabel })
        switch self {
        case WuiProgressStyle_Circular:
            view.progressViewStyle(.circular)
        case WuiProgressStyle_Linear:
            view.progressViewStyle(.linear)
        default:
            view.progressViewStyle(.automatic)
        }
    }
}

struct WuiProgress: View, WuiComponent {
    static let id: String = decodeViewIdentifier(waterui_progress_id())
    var label: WuiAnyView
    @State var value: WuiComputed<Double>
    var style: WuiProgressStyle

    init(progress: CWaterUI.WuiProgress, env: WuiEnvironment) {
        label = WuiAnyView(anyview: progress.label, env: env)
        style = progress.style
        value = WuiComputed(progress.value)
    }

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(progress: waterui_force_as_progress(anyview), env: env)
    }

    var body: some View {
        VStack {
            if value.value.isNaN {
                SwiftUI.ProgressView {
                    label
                }
            } else {
                SwiftUI.ProgressView(
                    value: value.value,
                    label: {
                        label
                    }
                )
                .animation(.default, value: value.value)

            }
        }.progressViewStyle(style)

    }
}
