//
//  Lazy.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//
import CWaterUI
import Combine
import SwiftUI
struct LazyViewListView:View{
    var inner:LazyViewList
    var list:[WaterUI.AnyView]

    var body: some View{
        SwiftUI.ForEach(list, id: \.id, content: {view in
            view
        })
    }
}



@MainActor
class LazyViewList{
    var inner:OpaquePointer
    var env:WaterUI.Environment
    init(inner: OpaquePointer,env:WaterUI.Environment) {
        self.inner = inner
        self.env=env
    }
    
    func get(_ index: Int) async -> WaterUI.AnyView?{
        await withCheckedContinuation{continuation in
            waterui_lazy_view_list_get(inner, UInt(index), waterui_fnonce_____waterui_anyview(f:{view in
                continuation.resume(returning: view)
            },env:env))
        }
    }
    
    func get(_ range:Range<Int>)async -> [WaterUI.AnyView]{
        var result: [WaterUI.AnyView] = []
        for i in range{
            if let content = await get(i){
                result.append(content)
            }
            else{
                break
            }
            
        }
        return result

    }
    
    
    func len() -> UInt?{
        let len = waterui_lazy_list_len(inner)
        if len<0{
            return nil
        }
        else{
            return UInt(len)
        }
    }
    
    func iter() -> LazyViewIter{
        LazyViewIter(inner: waterui_lazy_list_iter(inner), env: env)
    }
    
    func revIter() -> LazyViewIter{
        LazyViewIter(inner: waterui_lazy_list_rev_iter(inner), env: env)
    }
    
    deinit{
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_lazy_view_list(this.inner)
            }
        }
        
    }
}


class LazyViewIter:AsyncIteratorProtocol{
    typealias Index = Int
    var inner:OpaquePointer
    var env:WaterUI.Environment

    init(inner: OpaquePointer, env: WaterUI.Environment) {
        self.inner = inner
        self.env = env
    }
    
    
    
    @MainActor
    func next() async -> WaterUI.AnyView?{
        await withCheckedContinuation{continuation in
            waterui_anyview_iter_next(inner, waterui_fnonce_____waterui_anyview(f:{view in
                continuation.resume(returning: view)
            },env:env))
        }
    }
}


@MainActor
extension waterui_fnonce_____waterui_anyview {
    init(f: @escaping (WaterUI.AnyView?) -> Void,env:WaterUI.Environment) {
        class Wrapper {
            var f: (WaterUI.AnyView?) -> Void
            var env:WaterUI.Environment
            init(f: @escaping (WaterUI.AnyView?) -> Void,env:WaterUI.Environment) {
                self.f = f
                self.env=env
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f:f,env:env)).toOpaque())

        self.init(data: data, call: { data,value in
            let wrapper = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()
            let f=wrapper.f
            if let value=value{
                f(WaterUI.AnyView(anyview: value, env: wrapper.env))
            }
            else{
                f(nil)
            }
        })
    }
}
