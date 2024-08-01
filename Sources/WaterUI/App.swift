//
//  App.swift
//
//
//  Created by Lexo Liu on 7/31/24.
//

import CWaterUI
import Foundation
import Semaphore

public class App {
    var env: OpaquePointer
    var bridge: OpaquePointer

    private init(env: OpaquePointer, bridge: OpaquePointer) {
        self.env = env
        self.bridge = bridge
    }

    public init() async {
        let queue = DispatchQueue(label: "WaterUI", qos: .userInteractive)
        let app = queue.sync {
            waterui_init()
        }
        env = app.env
        bridge = app.bridge

        queue.async {
            waterui_launch_app(app)
        }
    }

    public func mainWidget() async -> WaterUI.AnyView {
        await task {
            AnyView(view: waterui_main()!, app: self)
        }
    }

    public func task<T>(_ f: @escaping () -> T) async -> T {
        let semaphore = AsyncSemaphore(value: 0)
        var result: T? = nil
        spawn {
            result = f()
            semaphore.signal()
        }

        await semaphore.wait()

        return result!
    }

    public func spawn(_ f: @escaping () -> Void) {
        waterui_bridge_send(bridge, waterui_bridge_closure(f))
    }

    deinit {
        Task {
            await self.task {
                waterui_drop_env(self.env)
            }

            waterui_drop_bridge(self.bridge)
        }
    }
}
