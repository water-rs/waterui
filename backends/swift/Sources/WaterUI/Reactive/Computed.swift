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
class Computed<T>{
    var inner: OpaquePointer
    var drop: (OpaquePointer)->Void
    
    init(inner: OpaquePointer, drop: @escaping (OpaquePointer)->Void){
        self.inner = inner
        self.drop = drop
    }
    
    func getValue() -> T {
        fatalError("Not implemented")
    }
    
    func watch(_ f:@escaping (T,Animation?)->()) -> WatcherGuard{
        fatalError("Not implemented")
    }
    
    deinit{
        let this=self
        Task{@MainActor in
            this.drop(this.inner)
        }
    }

}

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
                waterui_drop_box_watcher_guard(this.inner)
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
       WuiStr(waterui_read_computed_str(self.inner)).toString()
    }
    
    
    func watch(_ f:@escaping (String,Animation?)->()) -> WatcherGuard{
        let g=waterui_watch_computed_str(self.inner, WuiWatcher_WuiStr({value,animation in
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

struct TextStyleSnapshot {
    var font: Font?
    var bold: Bool
    var italic: Bool
    var underline: Bool
    var strikethrough: Bool
    var foreground: Color?
    var background: Color?

    init(style: inout CWaterUI.WuiTextStyle) {
        bold = style.bold
        italic = style.italic
        underline = style.underline
        strikethrough = style.strikethrough

        if style.has_font {
            let value = style.font
            font = Font(wuiFont: value)

            if let strike = value.strikethrough {
                waterui_drop_color(strike)
                style.font.strikethrough = nil
            }

            if let underlineColor = value.underlined {
                waterui_drop_color(underlineColor)
                style.font.underlined = nil
            }
        } else {
            font = nil
        }

        if let fg = style.foreground {
            foreground = fg.pointee.toSwiftUIColor()
            waterui_drop_color(fg)
            style.foreground = nil
        } else {
            foreground = nil
        }

        if let bg = style.background {
            background = bg.pointee.toSwiftUIColor()
            waterui_drop_color(bg)
            style.background = nil
        } else {
            background = nil
        }
    }

}

struct AttributedTextSpan {
    var text: String
    var style: TextStyleSnapshot
}

@MainActor
@Observable
class ComputedAttributedText {
    @ObservationIgnored private var inner: OpaquePointer
    @ObservationIgnored private var watcher: WatcherGuard!

    var value: [AttributedTextSpan] = []

    init(inner: OpaquePointer) {
        self.inner = inner
        value = ComputedAttributedText.readSnapshot(inner: inner)

        weak var this = self
        self.watcher = self.watch { spans, animation in
            guard let this else { return }
            if let animation {
                SwiftUI.withAnimation(animation) {
                    this.value = spans
                }
            } else {
                this.value = spans
            }
        }
    }

    func compute() -> [AttributedTextSpan] {
        ComputedAttributedText.readSnapshot(inner: inner)
    }

    func watch(_ f: @escaping ([AttributedTextSpan], Animation?) -> Void) -> WatcherGuard {
        let guardPtr = waterui_watch_computed_attributed_str(inner, WuiWatcher_WuiAttributedStr(f))
        return WatcherGuard(guardPtr!)
    }

    deinit {
        weak var this = self
        Task { @MainActor in
            if let this {
                waterui_drop_computed_attributed_str(this.inner)
            }
        }
    }

    fileprivate static func readSnapshot(inner: OpaquePointer) -> [AttributedTextSpan] {
        var attributed = waterui_read_computed_attributed_str(inner)
        return snapshotInPlace(&attributed)
    }

    fileprivate static func snapshotInPlace(_ attributed: inout CWaterUI.WuiAttributedStr) -> [AttributedTextSpan] {
        let raw = unsafeBitCast(attributed.chunks, to: CWaterUI.WuiArray.self)
        var chunks = WuiArray<CWaterUI.WuiAttributedChunk>(c: raw).toArray()
        var result: [AttributedTextSpan] = []
        result.reserveCapacity(chunks.count)
        for index in 0..<chunks.count {
            let chunk = chunks[index]
            var style = chunk.style
            let span = AttributedTextSpan(
                text: WuiStr(chunk.text).toString(),
                style: TextStyleSnapshot(style: &style)
            )
            result.append(span)
        }
        return result
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
        let g = waterui_watch_computed_int(self.inner, WuiWatcher_i32({value,animation in
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
        let g = waterui_watch_computed_double(self.inner, WuiWatcher_f64({value,animation in
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

// ComputedData is not currently supported in the FFI layer

extension SwiftUI.Animation{
    init?(_ animation: WuiAnimation){
        switch animation{
            case WuiAnimation_Default:
                    self = .default
            case WuiAnimation_None:
                return nil
            default:
                return nil
        }
    }
}


@MainActor
extension WuiWatcher_WuiStr {
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
            f(WuiStr(value).toString(),Animation(waterui_get_animation(metadata)))
        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}

@MainActor
extension WuiWatcher_WuiAttributedStr {
    init(_ f: @escaping ([AttributedTextSpan], Animation?) -> Void) {
        class Wrapper {
            var inner: ([AttributedTextSpan], Animation?) -> Void
            init(_ inner: @escaping ([AttributedTextSpan], Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(
            data: data,
            call: { data, value, metadata in
                let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
                var copy = value
                let spans = ComputedAttributedText.snapshotInPlace(&copy)
                f(spans, Animation(waterui_get_animation(metadata)))
            },
            drop: { data in
                _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()
            }
        )
    }
}

extension WuiWatcher_f64 {
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


extension WuiWatcher_i32 {
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



extension WuiWatcher_bool {
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
