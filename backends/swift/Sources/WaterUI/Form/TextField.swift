//
//  TextField.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

public struct WuiTextField: View, WuiComponent {
    public static var id: CWaterUI.WuiTypeId {
        waterui_text_field_id()
    }
    
    private var label: WuiAnyView
    @ObservedObject private var prompt: WuiComputed<CWaterUI.WuiStyledStr>
    @ObservedObject private var value: WuiBindingStr

    public init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(field: waterui_force_as_text_field(anyview), env: env)
    }
    
    init(field: CWaterUI.WuiTextField, env: WuiEnvironment) {
        self.label = WuiAnyView(anyview: field.label, env: env)
        self.value = WuiBindingStr(inner: field.value)
        self.prompt = WuiComputed<CWaterUI.WuiStyledStr>(
            inner: field.prompt.content,
            read: waterui_read_computed_attributed_str,
            watch: { ptr, f in
                let watcher = CWaterUI.WuiWatcher_WuiStyledStr(f)
                let guardPtr = waterui_watch_computed_attributed_str(ptr, watcher)
                return WatcherGuard(guardPtr!)
            },
            drop: waterui_drop_computed_attributed_str
        )
    }

    public var body: some View {
        let promptText = plainString(from: prompt.value)
        
        SwiftUI.TextField(text: value.value, prompt: SwiftUI.Text(promptText)) {
            label
        }
    }
    
    private func plainString(from styledStr: CWaterUI.WuiStyledStr) -> String {
        let cChunks = styledStr.chunks
        let slice = cChunks.vtable.slice(cChunks.data)
        let buffer = UnsafeBufferPointer(start: slice.head, count: Int(slice.len))
        return buffer.map { WuiStr($0.text).toString() }.joined()
    }
}
