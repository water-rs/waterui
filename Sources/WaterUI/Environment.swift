//
//  Environment.swift
//
//
//  Created by Lexo Liu on 7/31/24.
//

import CWaterUI

public class Environment{
    var inner:OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }
    
    deinit{
        waterui_drop_env(inner)
    }
}
