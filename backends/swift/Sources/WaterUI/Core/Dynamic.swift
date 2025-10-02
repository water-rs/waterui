//
//  SwiftUIView.swift
//  
//
//  Created by Lexo Liu on 8/13/24.
//

import SwiftUI
import CWaterUI
@MainActor
struct WuiDynamic: View,WuiComponent {
    static var id:WuiTypeId{
        waterui_dynamic_id()
    }
    @State var view:WuiAnyView?
    var dynamic:OpaquePointer
    var env:WuiEnvironment
    
    init(dynamic:OpaquePointer,env:WuiEnvironment){
        self.dynamic=dynamic
        self.env=env
    }

    
    init(anyview: OpaquePointer,env:WuiEnvironment) {
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
    init(_ f:@escaping (WuiAnyView)->Void, env:WuiEnvironment) {
        class Wrapper {
            var inner: (WuiAnyView) -> Void
            var env:WuiEnvironment
            init(inner: @escaping (WuiAnyView) -> Void,env:WuiEnvironment) {
                self.inner = inner
                self.env=env
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(inner:f,env:env)).toOpaque())

        self.init(data: data, call: { data, value,ctx in
            let data = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue()
            (data.inner)(WuiAnyView(anyview: value!, env: data.env))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()
        })
    }
}
