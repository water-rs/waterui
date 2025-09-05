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

enum StackMode{
    case Default
    case Vertical
    case Horizontal
    case Layered
}

extension StackMode{
    init(_ mode:waterui_stack_mode){
        switch mode {
        case waterui_stack_mode_HORIZONTAL:
            self = .Horizontal
        case waterui_stack_mode_LAYERED:
            self = .Layered
        default:
            self = .Default
        }
    }
}

@MainActor
struct Stack: View {
    private var views:[WithId<AnyView,Int>]
    private var mode:StackMode
    init(stack: waterui_stack, env: Environment) {
        let array = Array(UnsafeBufferPointer<OpaquePointer?>(start: stack.contents.head, count: Int(stack.contents.len)))

        print(array.count)

        views = array.enumerated().map { index, view in
            WithId(data: AnyView(view: view!, env: env), id: index)
        }
        mode = StackMode(stack.mode)

        
    }

    var body: some View {
        let content=ForEach(views){value in
            value.data
        }
        
        VStack{
            switch mode{
            case .Horizontal:
                    HStack{
                        content
                    }
            case .Layered:
                ZStack{content}
                    
                
                default:
                VStack{content}
                
                
            }
        }.padding()
    }
}
