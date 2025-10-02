//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI

@MainActor
class ComputedFont: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    var value: WuiFont {
        self.compute()
    }

    init(inner: OpaquePointer) {
        self.inner = inner
        // Avoid strong self in the stored closure
        self.watcher = self.watch { [weak self] new, animation in
            guard let self else { return }
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }

    func compute() -> WuiFont {
        waterui_read_computed_font(self.inner)
    }

    func watch(_ f: @escaping (WuiFont, Animation?) -> Void) -> WatcherGuard {
        let g = waterui_watch_computed_font(
            self.inner,
            WuiWatcher_WuiFont({ value, animation in
                f(value, animation)
            }))
        return WatcherGuard(g!)

    }

    deinit {
        let this = self
        Task { @MainActor in
            //waterui_drop_computed_font(this.inner) // I don't know why it crashes here
        }

    }
}

extension WuiWatcher_WuiFont {
    init(_ f: @escaping (WuiFont, Animation?) -> Void) {
        class Wrapper {
            var inner: (WuiFont, Animation?) -> Void
            init(_ inner: @escaping (WuiFont, Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(
            data: data,
            call: { data, value, metadata in
                let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
                f(value, Animation(waterui_get_animation(metadata)))

            },
            drop: { data in
                _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

            })
    }
}

@MainActor
struct WuiText: View, WuiComponent {
    static var id:WuiTypeId{
        waterui_text_id()
    }
    @State var content: ComputedStr
    @State var font: ComputedFont

    init(text: CWaterUI.WuiText) {
        self.content = ComputedStr(inner: text.content)
        self.font = ComputedFont(inner: text.font)
    }

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(text: waterui_force_as_text(anyview))
    }
    
    func text() -> ObservableText{
        .init(content: content, font: font)
    }


    var body: some View {
        Text(content.value).font(Font.init(wuiFont: font.value))
    }
}

@Observable
@MainActor
class ObservableText{
    private var content:ComputedStr
    private var font:ComputedFont
    
    var text:Text{
        Text(content.value).font(Font.init(wuiFont: font.value))
    }
    
    init(content: ComputedStr, font: ComputedFont) {
        self.content = content
        self.font = font
    }
}

extension SwiftUI.Font {
    init(wuiFont: WuiFont) {
        if wuiFont.size.isNaN {
            self = .body
        } else {
            self = .system(size: wuiFont.size)
        }

        if wuiFont.bold {
            self = self.bold()
        }

        if wuiFont.italic {
            self = self.italic()
        }
    }
}

struct WuiLabel: View, WuiComponent {
    static var id:WuiTypeId{
        waterui_label_id()
    }
    var label: WuiStr
    init(label: WuiStr) {
        self.label = label
    }

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(label: WuiStr(waterui_force_as_label(anyview)))
    }

    var body: some View {
        SwiftUI.Text(label.toString())
    }

}
