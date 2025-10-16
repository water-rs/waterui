//
//  Core.swift
//  waterui-swift
//
//  Created by Lexo Liu on 9/30/25.
//

import CWaterUI

extension WuiTypeId: @retroactive Equatable {
    public static func == (lhs: WuiTypeId, rhs: WuiTypeId) -> Bool {
        return lhs.inner.0 == rhs.inner.0 && lhs.inner.1 == rhs.inner.1
    }
}

extension WuiId: @retroactive Equatable, @retroactive Hashable {
    public static func == (lhs: WuiId, rhs: WuiId) -> Bool {
        return lhs.inner == rhs.inner
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(inner)
    }
}
