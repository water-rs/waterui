//
//  Stack.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI

private struct WithId<T, Id: Hashable>: Identifiable {
    var data: T
    var id: Id
}

struct Stack: View {
    var main: any View

    init(stack: waterui_stack, app: App) {
        main = EmptyView()

        let array = Array(UnsafeBufferPointer<OpaquePointer?>(start: stack.contents.head, count: Int(stack.contents.len)))

        print(array.count)

        let contents = array.enumerated().map { index, view in
            WithId(data: AnyView(view: view!, app: app), id: index)
        }
        let mode = stack.mode
        let stack = SwiftUI.ForEach(contents, content: { value in
            value.data
        }).id(UUID())

        switch mode {
        case waterui_stack_mode_HORIZONTAL:
            main = SwiftUI.HStack { stack }
        case waterui_stack_mode_LAYERED:
            main = SwiftUI.ZStack { stack }
        default:
            main = stack
        }
    }

    var body: some View {
        VStack {
            SwiftUI.AnyView(main)
        }
    }
}
