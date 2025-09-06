//
//  SwiftUIView.swift
//  
//
//  Created by Lexo Liu on 8/13/24.
//

import SwiftUI
import CWaterUI
@MainActor
struct Dynamic: View,Component {
    static var id=waterui_dynamic_id()
    @State var view:AnyView?
    var dynamic:OpaquePointer
    var env:Environment
    
    init(dynamic:OpaquePointer,env:Environment){
        self.dynamic=dynamic
        self.env=env
    }

    
    init(anyview: OpaquePointer,env:Environment) {
        self.init(dynamic: waterui_force_as_dynamic(anyview), env: env)
    }
    
    
    var body: some View {
        VStack{
            view
        }.onAppear{
            waterui_dynamic_connect(dynamic, WuiWatcher_____WuiAnyView({ new in
                view=new
            },env:env))
        }
    }
}


extension WuiWatcher_____WuiAnyView{
    @MainActor
    init(_ f:@escaping (WaterUI.AnyView)->Void, env:Environment) {
        class Wrapper {
            var inner: (WaterUI.AnyView) -> Void
            var env:Environment
            init(inner: @escaping (WaterUI.AnyView) -> Void,env:Environment) {
                self.inner = inner
                self.env=env
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(inner:f,env:env)).toOpaque())

        self.init(data: data, call: { data, value,ctx in
            let data = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue()
            (data.inner)(AnyView(anyview: value!, env: data.env))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()
        })
    }
}
