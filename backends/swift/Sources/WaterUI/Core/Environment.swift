//
//  Environment.swift
//
//
//  Created by Lexo Liu on 7/31/24.
//

import CWaterUI

@MainActor
public class WuiEnvironment {
    var inner: OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }
    
    @MainActor deinit{
        waterui_drop_env(inner)
    }
}
