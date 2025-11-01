//
//  Core.swift
//  waterui-swift
//
//  Created by Lexo Liu on 9/30/25.
//

import CWaterUI

extension WuiId: @retroactive Equatable, @retroactive Hashable {
    public static func == (lhs: WuiId, rhs: WuiId) -> Bool {
        return lhs.inner == rhs.inner
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(inner)
    }
}
