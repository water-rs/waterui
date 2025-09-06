//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI



@MainActor
struct Text: View,Component {
    static var id = waterui_view_text_id()
    @State var content: ComputedStr
    
    init(text: waterui_text) {
        
        self.content = ComputedStr(inner: text.content)
    }
    
    init(anyview: OpaquePointer,env: Environment) {
        self.init(text: waterui_view_force_as_text(anyview))
    }
    
    func toText() -> SwiftUI.Text{
        return SwiftUI.Text(content.value)
    }

    var body: some View {
        let lines=content.value.split(separator: "\n").map{line in
            (line,UUID())
        }
        LazyVStack{
            ForEach(lines,id:\.1){(line,_) in
                SwiftUI.Text(line)
            }
        }
        
    }
    
}


