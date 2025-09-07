//
//  Stack.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI

enum StackMode{
    case Default
    case Vertical
    case Horizontal
    case Layered
}

extension StackMode{
    init(_ mode:WuiStackMode){
        switch mode {
        case WuiStackMode_Horizonal:
            self = .Horizontal
        case WuiStackMode_Layered:
            self = .Layered
        default:
            self = .Default
        }
    }
}

extension WuiArray_____WuiAnyView{
    func toArray() -> Array<OpaquePointer?>{
        let array = Array(UnsafeBufferPointer<OpaquePointer?>(start: self.head, count: Int(self.len)))
        let array2 = array
        //waterui_free_anyview_array_without_free_elements(self) TODO: Free memory here
        return array2
    }
}

@MainActor
struct Stack: View,Component {
    static var id=waterui_stack_id()
    private var contents:[AnyView]
    private var mode:StackMode
    init(stack: WuiStack, env: Environment) {
        let array = stack.contents.toArray()

        contents = array.map { view in
            AnyView(anyview: view!, env: env)
        }
        mode = StackMode(stack.mode)
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(stack: waterui_force_as_stack(anyview), env: env)
    }

    var body: some View {
        let each=SwiftUI.ForEach(contents, id: \.id, content: {content in
            content
        })
       
        VStack{
            switch mode{
            case .Horizontal:
                HStack{each}
            case .Layered:
                ZStack{each}
                    
            default:
                VStack{each}
                
                
            }
        }.padding()
    }
}
