//
//  Binding.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import Combine
import SwiftUI



@MainActor
class BindingStr :ObservableObject{
    private var inner: OpaquePointer
    private var rustWatcher=Set<OpaquePointer>()
    private var swiftWatcher:AnyCancellable!
    @Published var value:String = ""
    init(inner: OpaquePointer) {
        self.inner = inner
        value=self.compute()
        
        
        swiftWatcher=self.$value.removeDuplicates().sink{new in
            self.set(new)
        }
        

        self.watch{new in
            self.value=new
        }
    }
    
    
    func compute() -> String{
        waterui_read_binding_str(self.inner).toString()
    }
    

    
    
    func watch(_ f:@escaping (String)->()) {
        let g = waterui_watch_binding_str(self.inner, waterui_fn_waterui_str({value in
            f(value)
        }))
        rustWatcher.insert(g!)
    }
    
    func set(_ value:String){
        waterui_set_binding_str(self.inner, waterui_str(value))
    }



    deinit {
        waterui_drop_binding_str(self.inner)
            for watcher in self.rustWatcher{
                waterui_drop_watcher_guard(watcher)
            }
        
    }
}

struct Value<T>{
    var value:T
    var fromRust:Bool
}

@MainActor
class BindingInt:ObservableObject{
    private var inner: OpaquePointer
    private var rustWatcher=Set<OpaquePointer>()
    private var swiftWatcher=Set<AnyCancellable>()
    
    @Published var value=0
    
    init(inner: OpaquePointer) {
        self.inner = inner
        value=self.compute()
        
        
        self.$value.removeDuplicates().sink{new in
            self.set(new)
        }.store(in: &swiftWatcher)
        

        self.watch{new in
            self.value=new
        }
    }
    
    
    func compute()  -> Int{
        Int(waterui_read_binding_int(self.inner))
    }
    
    func watch(_ f:@escaping (Int)->()){
        let g = waterui_watch_binding_int(self.inner, waterui_fn_i32({value in
            f(Int(value))
        }))
        rustWatcher.insert(g!)
    }
    
    func set(_ value:Int){
        waterui_set_binding_int(self.inner, Int32(value))

    }



    deinit {
            waterui_drop_binding_int(self.inner)
            for watcher in self.rustWatcher{
                waterui_drop_watcher_guard(watcher)
            }
        
    }
}


@MainActor
class BindingBool:ObservableObject{
    private var inner: OpaquePointer
    private var rustWatcher=Set<OpaquePointer>()
    private var swiftWatcher:AnyCancellable!

    @Published var value = false
    init(inner: OpaquePointer) {
        self.inner = inner
        self.value=self.compute()
        self.swiftWatcher=_value.projectedValue.sink{new in
            self.set(new)
        }
        self.watch{new in
            if self.value != new{
                self.value=new
            }
        }
    }
    
    
    func compute()  -> Bool{
        waterui_read_binding_bool(self.inner)
    }
    
    func watch(_ f:@escaping (Bool)->()){
        let g = waterui_watch_binding_bool(self.inner, waterui_fn_bool({value in
            f(value)
        }))
        rustWatcher.insert(g!)
    }
    
    func set(_ value:Bool){
        waterui_set_binding_bool(self.inner, value)

    }



    deinit {
            waterui_drop_binding_bool(self.inner)
            for watcher in self.rustWatcher{
                waterui_drop_watcher_guard(watcher)
            }
        
    }
}
