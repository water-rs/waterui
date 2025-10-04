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
    @State var content: ComputedAttributedText
    @State var font: ComputedFont

    init(text: CWaterUI.WuiText) {
        self.content = ComputedAttributedText(inner: text.content)
        self.font = ComputedFont(inner: text.font)
    }

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(text: waterui_force_as_text(anyview))
    }
    
    func text() -> ObservableText{
        .init(content: content, font: font)
    }


    var body: some View {
        text().text
    }
}

@Observable
@MainActor
class ObservableText{
    private var content:ComputedAttributedText
    private var font:ComputedFont
    
    var text:Text{
        Text(makeAttributedString())
    }
    
    init(content: ComputedAttributedText, font: ComputedFont) {
        self.content = content
        self.font = font
    }

    private func makeAttributedString() -> AttributedString {
        var baseFont = font.value
        let baseStyle = TextBaseStyle(wuiFont: &baseFont)
        return ObservableText.compose(spans: content.value, base: baseStyle)
    }

    private static func compose(spans: [AttributedTextSpan], base: TextBaseStyle) -> AttributedString {
        var result = AttributedString()
        for span in spans {
            var substring = AttributedString(span.text)
            var container = AttributeContainer()

            var font = span.style.font ?? base.font
            if span.style.bold {
                font = font.weight(.bold)
            }
            if span.style.italic {
                font = font.italic()
            }
            container.font = font

            let foregroundColor = span.style.foreground
            if let foregroundColor {
                container.foregroundColor = foregroundColor
            }

            if let backgroundColor = span.style.background {
                container.backgroundColor = backgroundColor
            }

            if span.style.underline || base.underline.enabled {
                let underlineColor = span.style.underline ? (foregroundColor ?? base.underline.color) : base.underline.color
                container.underlineStyle = Text.LineStyle(pattern: .solid, color: underlineColor)
            }

            if span.style.strikethrough || base.strikethrough.enabled {
                let strikeColor = span.style.strikethrough ? (foregroundColor ?? base.strikethrough.color) : base.strikethrough.color
                container.strikethroughStyle = Text.LineStyle(pattern: .solid, color: strikeColor)
            }

            substring.setAttributes(container, range: substring.startIndex..<substring.endIndex)
            result += substring
        }
        return result
    }
}

struct TextBaseStyle {
    var font: Font
    var underline: (enabled: Bool, color: Color?)
    var strikethrough: (enabled: Bool, color: Color?)

    init(wuiFont: inout WuiFont) {
        font = Font(wuiFont: wuiFont)

        if let underlinePtr = wuiFont.underlined {
            underline = (true, underlinePtr.pointee.toSwiftUIColor())
            waterui_drop_color(underlinePtr)
            wuiFont.underlined = nil
        } else {
            underline = (false, nil)
        }

        if let strikePtr = wuiFont.strikethrough {
            strikethrough = (true, strikePtr.pointee.toSwiftUIColor())
            waterui_drop_color(strikePtr)
            wuiFont.strikethrough = nil
        } else {
            strikethrough = (false, nil)
        }
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
