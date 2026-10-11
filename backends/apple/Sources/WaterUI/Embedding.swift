import Foundation
import SwiftUI

#if canImport(UIKit)
import UIKit

@_extern(c, "waterui_apple_mount_scene_id")
private func mountSceneID(_ mount: UnsafeRawPointer) -> UInt64
@_extern(c, "waterui_apple_scene_connect")
private func sceneConnect(
  _ runtime: UnsafeRawPointer, _ mountID: UInt64, _ scene: UnsafeMutableRawPointer
) -> UnsafeMutableRawPointer?
@_extern(c, "waterui_apple_scene_disconnect")
private func sceneDisconnect(
  _ runtime: UnsafeRawPointer, _ mountID: UInt64, _ scene: UnsafeMutableRawPointer
)
#else
import AppKit
#endif

@_extern(c, "waterui_apple_runtime_create")
private func runtimeCreate(
  _ _: UnsafeMutableRawPointer,
  _ _: @convention(c) (UnsafeMutableRawPointer, UnsafeMutableRawPointer) -> Void
)
@_extern(c, "waterui_apple_runtime_drop")
private func runtimeDrop(_ _: UnsafeMutableRawPointer)
@_extern(c, "waterui_apple_mount")
private func mountCreate(
  _ _: UnsafeRawPointer, _ _: UnsafeMutableRawPointer,
  _ _: UnsafePointer<CChar>, _ _: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer
@_extern(c, "waterui_apple_mount_drop")
private func mountDrop(_ _: UnsafeMutableRawPointer)

/// Resources supplied by the native application or the embedding package.
public struct WaterUIResourceContext: Sendable {
  public let assets: URL
  public let fonts: URL

  public init(assets: URL, fonts: URL) {
    precondition(assets.isFileURL && fonts.isFileURL, "WaterUI resources require file URLs")
    self.assets = assets
    self.fonts = fonts
  }

  public static var application: Self {
    guard let root = Bundle.main.resourceURL else {
      fatalError("The host application has no resource directory")
    }
    return Self(assets: root.appendingPathComponent("waterui_assets"),
                fonts: root.appendingPathComponent("fonts"))
  }
}

/// The process runtime, explicitly shared by every embedded WaterUI instance.
@MainActor
public final class WaterUIRuntime {
  fileprivate let pointer: UnsafeMutableRawPointer

  private init(_ pointer: UnsafeMutableRawPointer) { self.pointer = pointer }

  /// Call once per process, then pass the result to each host controller.
  public static func create() async -> WaterUIRuntime {
    await withCheckedContinuation { continuation in
      let pending = PendingRuntime(continuation)
      runtimeCreate(Unmanaged.passRetained(pending).toOpaque()) { context, runtime in
        MainActor.assumeIsolated {
          let pending = Unmanaged<PendingRuntime>.fromOpaque(context).takeRetainedValue()
          pending.continuation.resume(returning: WaterUIRuntime(runtime))
        }
      }
    }
  }

  @MainActor deinit { runtimeDrop(pointer) }
}

@MainActor
private final class PendingRuntime {
  let continuation: CheckedContinuation<WaterUIRuntime, Never>
  init(_ continuation: CheckedContinuation<WaterUIRuntime, Never>) {
    self.continuation = continuation
  }
}

@MainActor
private final class WaterUIMount {
  private let runtime: WaterUIRuntime
  private let pointer: UnsafeMutableRawPointer

  init(runtime: WaterUIRuntime, host: AnyObject, resources: WaterUIResourceContext) {
    self.runtime = runtime
    self.pointer = resources.assets.path(percentEncoded: false).withCString { assets in
      resources.fonts.path(percentEncoded: false).withCString { fonts in
        mountCreate(UnsafeRawPointer(runtime.pointer), Unmanaged.passUnretained(host).toOpaque(), assets, fonts)
      }
    }
  }

  @MainActor deinit { mountDrop(pointer) }

  #if canImport(UIKit)
  var sceneOwner: WaterUISceneOwner {
    WaterUISceneOwner(runtime: runtime, id: mountSceneID(UnsafeRawPointer(pointer)))
  }
  #endif
}

#if canImport(UIKit)
/// A scene route belonging to one mounted app. It retains the runtime, not the mount.
/// Keep this route with the host's scene session and forward its lifecycle callbacks.
/// Connections arriving after the owning controller is destroyed return nil.
@MainActor
public final class WaterUISceneOwner {
  public let id: UInt64
  private let runtime: WaterUIRuntime

  fileprivate init(runtime: WaterUIRuntime, id: UInt64) {
    self.runtime = runtime
    self.id = id
  }

  public func connect(_ scene: UIWindowScene) -> UIWindow? {
    guard let pointer = sceneConnect(
      UnsafeRawPointer(runtime.pointer), id, Unmanaged.passUnretained(scene).toOpaque()
    ) else { return nil }
    return Unmanaged<UIWindow>.fromOpaque(pointer).takeRetainedValue()
  }

  public func disconnect(_ scene: UIWindowScene) {
    sceneDisconnect(UnsafeRawPointer(runtime.pointer), id, Unmanaged.passUnretained(scene).toOpaque())
  }
}

@MainActor
public final class WaterUIHostController: UIViewController {
  private let runtime: WaterUIRuntime
  private let resources: WaterUIResourceContext
  private var mount: WaterUIMount?

  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("Use init(runtime:resources:)") }

  public override func loadView() { view = UIView(frame: .zero) }
  public override func viewDidLoad() {
    super.viewDidLoad()
    mount = WaterUIMount(runtime: runtime, host: view, resources: resources)
  }

  public var sceneOwner: WaterUISceneOwner {
    loadViewIfNeeded()
    return mount!.sceneOwner
  }
}

public struct WaterUIHost: UIViewControllerRepresentable {
  public let runtime: WaterUIRuntime
  public let resources: WaterUIResourceContext
  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
  }
  public func makeUIViewController(context: Context) -> WaterUIHostController {
    WaterUIHostController(runtime: runtime, resources: resources)
  }
  public func updateUIViewController(_ controller: WaterUIHostController, context: Context) {}
}
#else
@MainActor
public final class WaterUIHostController: NSViewController {
  private let runtime: WaterUIRuntime
  private let resources: WaterUIResourceContext
  private var mount: WaterUIMount?

  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("Use init(runtime:resources:)") }

  public override func loadView() { view = NSView(frame: .zero) }
  public override func viewDidLoad() {
    super.viewDidLoad()
    mount = WaterUIMount(runtime: runtime, host: view, resources: resources)
  }
}

public struct WaterUIHost: NSViewControllerRepresentable {
  public let runtime: WaterUIRuntime
  public let resources: WaterUIResourceContext
  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
  }
  public func makeNSViewController(context: Context) -> WaterUIHostController {
    WaterUIHostController(runtime: runtime, resources: resources)
  }
  public func updateNSViewController(_ controller: WaterUIHostController, context: Context) {}
}
#endif
