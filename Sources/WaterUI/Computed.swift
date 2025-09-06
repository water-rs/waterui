//
//  Computed.swift
//
//
//  Created by Lexo Liu on 5/13/24.
//

import CWaterUI
import Foundation
import Combine
import SwiftUI

@MainActor
class WatcherGuard{
    var inner:OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }
    deinit{
        
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_watcher_guard(this.inner)
            }
        }
       
    }
}

func useAnimation(animation:Animation?,publisher:ObservableObjectPublisher){
    if let animation = animation{
        withAnimation(animation){
            publisher.send()
        }
    }
    else{
        publisher.send()
    }
}

@MainActor
@Observable
class ComputedStr{
    @ObservationIgnored private var inner: OpaquePointer
    @ObservationIgnored private var watcher:WatcherGuard!

    var value:String = ""
    
    init(inner: OpaquePointer) {
        self.inner = inner
        value=self.compute()

        weak var this = self
        self.watcher=self.watch{new,animation in
            if let this = this{
                if let animation = animation{
                    SwiftUI.withAnimation(animation){
                        this.value=new
                    }
                }
                else{
                    this.value=new
                }
                
            }
        }
    }
    
    func compute()  -> String{
        waterui_read_computed_str(self.inner).toString()
    }
    
    
    func watch(_ f:@escaping (String,Animation?)->()) -> WatcherGuard{
        let g=waterui_watch_computed_str(self.inner, waterui_watcher_waterui_str({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }

    deinit {
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_computed_str(this.inner)
            }
        }
    }
}

@MainActor
class ComputedInt:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!

    var value:Int{
        self.compute()
    }
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher=self.watch{new,animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }
    
    func compute() -> Int{
        Int(waterui_read_computed_int(self.inner))
    }
    
    func watch(_ f:@escaping (Int,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_computed_int(self.inner, waterui_watcher_i32({value,animation in
            f(Int(value),animation)
        }))
        return WatcherGuard(g!)

    }

    deinit {
        let this=self
        Task{@MainActor in
            waterui_drop_computed_int(this.inner)
        }
        
    }
}

@MainActor
class ComputedDouble:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!
    var value:Double{
        self.compute()
    }
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher=self.watch{new,animation in
            self.objectWillChange.send()
        }
    }
    
    func compute() -> Double{
        waterui_read_computed_double(self.inner)
    }
    
    func watch(_ f:@escaping (Double,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_computed_double(self.inner, waterui_watcher_f64({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }

    deinit {
        let this=self
        Task{@MainActor in
            waterui_drop_computed_double(this.inner)
        }
        
    }
}

@MainActor
class ComputedData:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!
    var value:Data{
        self.compute()
    }
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher=self.watch{new,animation in
            self.objectWillChange.send()
        }
    }
    
    func compute() -> Data{
        Data(waterui_read_computed_data(self.inner))
    }
    
    func watch(_ f:@escaping (Data,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_computed_data(self.inner, waterui_watcher_waterui_data({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }

    deinit {
        let this=self
        Task{@MainActor in
            waterui_drop_computed_data(this.inner)
        }
        
    }
}

extension SwiftUI.Animation{
    init?(_ animation:waterui_animation){
        switch animation{
            case WATERUI_ANIMATION_DEFAULT:
                    self = .default
            default:
                return nil
        }
    }
}


extension waterui_watcher_waterui_str {
    init(_ f: @escaping (String,Animation?) -> Void) {
        class Wrapper {
            var inner: (String,Animation?) -> Void
            init(_ inner: @escaping (String,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value, metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value.toString(),Animation(waterui_get_animation(metadata)))
        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}

extension waterui_watcher_f64 {
    init(_ f: @escaping (Double,Animation?) -> Void) {
        class Wrapper {
            var inner: (Double,Animation?) -> Void
            init(_ inner: @escaping (Double,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value, metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value,Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}


extension waterui_watcher_i32 {
    init(_ f: @escaping (Int32,Animation?) -> Void) {
        class Wrapper {
            var inner: (Int32,Animation?) -> Void
            init(_ inner: @escaping (Int32,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value,metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value,Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}


extension waterui_watcher_waterui_data {
    init(_ f: @escaping (Data,Animation?) -> Void) {
        class Wrapper {
            var inner: (Data,Animation?) -> Void
            init(_ inner: @escaping (Data,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value,metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(Data(value),Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}

extension waterui_watcher_bool {
    init(_ f: @escaping (Bool,Animation?) -> Void) {
        class Wrapper {
            var inner: (Bool,Animation?) -> Void
            init(_ inner: @escaping (Bool,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value,metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value,Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}


