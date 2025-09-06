//
//  Environment.swift
//
//
//  Created by Lexo Liu on 7/31/24.
//

import CWaterUI

@MainActor
public class Environment{
    var inner: OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }
    
    deinit{
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_env_drop(this.inner)
            }
        }
       
    }
}
