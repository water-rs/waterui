import CWaterUI
import SwiftUI

@MainActor
struct WuiStyledStr {
    var chunks: [WuiStyledChunk]
    
    init(_ inner: CWaterUI.WuiStyledStr) {
        self.chunks = []
        for chunk in WuiArray(inner.chunks).toArray() {
            chunks.append(WuiStyledChunk(chunk))
        }
    }
    
    func toAttributedString(env: WuiEnvironment) -> AttributedString {
        var result = AttributedString()
        for chunk in chunks{
            result.append(chunk.toAttributedString(env: env))
        }
        return result
    }
            

}

@MainActor
struct WuiStyledChunk {
    var text: WuiStr
    var style: WuiTextStyle
    init(_ inner: CWaterUI.WuiStyledChunk) {
        self.text = WuiStr(inner.text)
        self.style = WuiTextStyle(inner.style)
    }
    
    func toAttributedString(env: WuiEnvironment) -> AttributedString{
        var result = AttributedString(text.toString())
        
        let font = style.font.resolve(in:env).value.toSwiftUI()
        
        if style.underline {
            result.underlineStyle = .single
        }
        
        if style.strikethrough {
            result.strikethroughStyle = .single
        }
        
        result.font = font.italic(style.italic)
        
        return result
    }

}


@MainActor
struct WuiTextStyle{
    var font:WuiFont
    var foreground:WuiColor?
    var background:WuiColor?
    var underline:Bool
    var strikethrough:Bool
    var italic:Bool
    
    init(_ inner:CWaterUI.WuiTextStyle){
        self.font = WuiFont(inner.font)
        if inner.foreground != nil{
            self.foreground=WuiColor(inner.foreground)
        }
        
        if inner.background != nil{
            self.background=WuiColor(inner.background)
        }
        
        self.underline = inner.underline
        
        self.strikethrough = inner.strikethrough
        
        self.italic = inner.italic
    }
    
}


extension WuiResolvedFont{
    func toSwiftUI() -> SwiftUI.Font {
        let size = CGFloat(self.size)
        let weight = self.weight.toSwiftUI()
        
        return .system(size: size,weight: weight)
    }
            
}

extension CWaterUI.WuiFontWeight {
    func toSwiftUI() -> Font.Weight {
        switch self {
        case WuiFontWeight_Thin: return .thin
        case WuiFontWeight_UltraLight: return .ultraLight
        case WuiFontWeight_Light: return .light
        case WuiFontWeight_Normal: return .regular
        case WuiFontWeight_Medium: return .medium
        case WuiFontWeight_SemiBold: return .semibold
        case WuiFontWeight_Bold: return .bold
        case WuiFontWeight_UltraBold: return .heavy
        case WuiFontWeight_Black: return .black
        default: return .regular
        }
    }
}

@MainActor
class WuiFont {
    var inner: OpaquePointer

    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    func resolve(in env: WuiEnvironment) -> WuiComputed<CWaterUI.WuiResolvedFont> {
        let computedPtr = waterui_resolve_font(inner, env.inner)
        return WuiComputed<CWaterUI.WuiResolvedFont>(computedPtr!)
    }

    @MainActor deinit {
        waterui_drop_font(inner)
    }
}
