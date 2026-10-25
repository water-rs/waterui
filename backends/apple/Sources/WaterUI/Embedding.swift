import CoreText
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
  _ _: UnsafePointer<CChar>, _ _: UnsafePointer<CChar>,
  _ dark: Bool, _ rtl: Bool, _ locale: UnsafePointer<CChar>?,
  _ hostFonts: UnsafePointer<Double>?
) -> UnsafeMutableRawPointer
@_extern(c, "waterui_apple_mount_drop")
private func mountDrop(_ _: UnsafeMutableRawPointer)
@_extern(c, "waterui_apple_update_visibility")
private func updateVisibilityNative(_ _: UnsafeRawPointer)
@_extern(c, "waterui_apple_update_environment")
private func updateEnvironmentNative(
  _ _: UnsafeRawPointer, _ dark: Bool, _ rtl: Bool,
  _ locale: UnsafePointer<CChar>, _ hostFonts: UnsafePointer<Double>
)
@_extern(c, "waterui_apple_measure")
private func measureNative(
  _ _: UnsafeRawPointer, _ width: Double, _ height: Double,
  _ hasWidth: Bool, _ hasHeight: Bool, _ result: UnsafeMutablePointer<Double>
)

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

/// A host's declared environment for one mount: the snapshot a `SwiftUI`
/// representable hands the backend — scheme, direction and the six
/// resolved text styles as `(size, weight, lineHeight)` triples in Body,
/// Title1, Headline, Subheadline, Caption1, Footnote order.
private struct HostEnvironment: Equatable {
  var dark: Bool
  var rtl: Bool
  var locale: String
  var fonts: [Double]

  /// Resolves the environment's own text styles through `Font.resolve(in:)`
  /// against its `fontResolutionContext` — the host's content-size
  /// category included — and reads size, weight and face metrics off each
  /// resolved `CTFont`.
  init(_ environment: EnvironmentValues) {
    dark = environment.colorScheme == .dark
    rtl = environment.layoutDirection == .rightToLeft
    locale = environment.locale.identifier(.bcp47)
    let context = environment.fontResolutionContext
    fonts = [Font.body, .title, .headline, .subheadline, .caption, .footnote]
      .flatMap { style -> [Double] in
        let font = style.resolve(in: context).ctFont
        let traits = CTFontCopyTraits(font) as NSDictionary
        let weight = (traits[kCTFontWeightTrait] as? NSNumber)?.doubleValue ?? 0.0
        return [Double(CTFontGetSize(font)), weight,
                Double(CTFontGetAscent(font) + CTFontGetDescent(font) + CTFontGetLeading(font))]
      }
  }

  func withFontsPointer<R>(_ body: (UnsafePointer<Double>) -> R) -> R {
    fonts.withUnsafeBufferPointer { body($0.baseAddress!) }
  }
}

@MainActor
private final class WaterUIMount {
  private let runtime: WaterUIRuntime
  let pointer: UnsafeMutableRawPointer

  init(runtime: WaterUIRuntime, host: AnyObject, resources: WaterUIResourceContext,
       environment: HostEnvironment?) {
    self.runtime = runtime
    self.pointer = resources.assets.path(percentEncoded: false).withCString { assets in
      resources.fonts.path(percentEncoded: false).withCString { fonts in
        if let environment {
          return environment.withFontsPointer { hostFonts in
            environment.locale.withCString { locale in
              mountCreate(UnsafeRawPointer(runtime.pointer), Unmanaged.passUnretained(host).toOpaque(),
                          assets, fonts, environment.dark, environment.rtl, locale, hostFonts)
            }
          }
        }
        return mountCreate(UnsafeRawPointer(runtime.pointer), Unmanaged.passUnretained(host).toOpaque(),
                           assets, fonts, false, false, nil, nil)
      }
    }
  }

  /// Pushes a changed snapshot onto the mount in place.
  func updateEnvironment(_ environment: HostEnvironment) {
    environment.withFontsPointer { hostFonts in
      environment.locale.withCString { locale in
        updateEnvironmentNative(UnsafeRawPointer(pointer),
                                environment.dark, environment.rtl, locale, hostFonts)
      }
    }
  }

  /// The mounted content's size under the host's proposal; a `nil` axis is
  /// unbounded while `0` and `.infinity` reach layout as themselves.
  func measure(_ proposal: ProposedViewSize) -> CGSize {
    var result = [Double](repeating: 0, count: 2)
    result.withUnsafeMutableBufferPointer { buffer in
      measureNative(UnsafeRawPointer(pointer),
                    proposal.width ?? 0, proposal.height ?? 0,
                    proposal.width != nil, proposal.height != nil, buffer.baseAddress!)
    }
    return CGSize(width: result[0], height: result[1])
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
  /// The latest declared snapshot — queued before the mount exists so the
  /// mount is created under the host's first environment, not ambient.
  fileprivate var latestEnvironment: HostEnvironment?

  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("Use init(runtime:resources:)") }

  /// Queues or applies the host's declared environment.
  ///
  /// A `SwiftUI` representable calls this before the view loads and on
  /// every environment update. Only a changed snapshot reaches the mount —
  /// an unchanged one is ignored rather than invalidating the measure
  /// cache and re-feeding every binding. The host's own view traits are
  /// applied first so live evaluators see them before the Rust theme
  /// update lands.
  fileprivate func updateEnvironment(_ environment: HostEnvironment) {
    guard environment != latestEnvironment else { return }
    latestEnvironment = environment
    let mounted = mount != nil
    applyEnvironment(environment)
    // A mount created by the view load above already carries this
    // snapshot — only an older mount needs the in-place update.
    if mounted { mount?.updateEnvironment(environment) }
  }

  /// The mounted content's size under the host's proposal; a `nil` axis is
  /// unbounded while `0` and `.infinity` reach layout as themselves.
  fileprivate func measure(_ proposal: ProposedViewSize) -> CGSize {
    loadViewIfNeeded()
    guard let mount else {
      preconditionFailure("measure reached a loaded view with no mount")
    }
    return mount.measure(proposal)
  }

  public override func loadView() { view = UIView(frame: .zero) }
  public override func viewDidLoad() {
    super.viewDidLoad()
    mount = WaterUIMount(runtime: runtime, host: view, resources: resources,
                         environment: latestEnvironment)
  }

  /// Scopes only this host's own view: `overrideUserInterfaceStyle` and
  /// `semanticContentAttribute` are inherited by the mounted subviews the
  /// way a parent view's traits would be — the app's ambient traits and
  /// the rest of the host window are untouched.
  private func applyEnvironment(_ environment: HostEnvironment) {
    overrideUserInterfaceStyle = environment.dark ? .dark : .light
    view.semanticContentAttribute =
      environment.rtl ? .forceRightToLeft : .forceLeftToRight
  }

  /// Re-evaluates presentation visibility across the mounted tree.
  ///
  /// WaterUI-owned layout, scroll and scene changes reach surfaces on
  /// their own. Mutations on containers between the window and this
  /// controller's `view` — `isHidden`, `alpha`, clipping bounds,
  /// transform, or reparenting it inside the same window — publish
  /// nothing a mounted surface can observe; call this so surfaces parked
  /// for visibility re-check and resume.
  public func updateVisibility() {
    guard let mount else { return }
    updateVisibilityNative(UnsafeRawPointer(mount.pointer))
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
    let controller = WaterUIHostController(runtime: runtime, resources: resources)
    controller.updateEnvironment(HostEnvironment(context.environment))
    return controller
  }
  public func updateUIViewController(_ controller: WaterUIHostController, context: Context) {
    controller.updateEnvironment(HostEnvironment(context.environment))
  }
  public func sizeThatFits(_ proposal: ProposedViewSize,
                           uiViewController controller: WaterUIHostController,
                           context: Context) -> CGSize? {
    // Sync before measuring: a stale snapshot would size under the wrong
    // fonts.
    controller.updateEnvironment(HostEnvironment(context.environment))
    return controller.measure(proposal)
  }
}
#else
@MainActor
public final class WaterUIHostController: NSViewController {
  private let runtime: WaterUIRuntime
  private let resources: WaterUIResourceContext
  private var mount: WaterUIMount?
  /// The latest declared snapshot — queued before the mount exists so the
  /// mount is created under the host's first environment, not ambient.
  fileprivate var latestEnvironment: HostEnvironment?

  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("Use init(runtime:resources:)") }

  /// Queues or applies the host's declared environment — the same contract
  /// as the UIKit counterpart: unchanged snapshots are ignored, the host's
  /// own view traits apply before the Rust theme update lands.
  fileprivate func updateEnvironment(_ environment: HostEnvironment) {
    guard environment != latestEnvironment else { return }
    latestEnvironment = environment
    let mounted = mount != nil
    applyEnvironment(environment)
    if mounted { mount?.updateEnvironment(environment) }
  }

  /// The mounted content's size under the host's proposal; a `nil` axis is
  /// unbounded while `0` and `.infinity` reach layout as themselves.
  fileprivate func measure(_ proposal: ProposedViewSize) -> CGSize {
    loadViewIfNeeded()
    guard let mount else {
      preconditionFailure("measure reached a loaded view with no mount")
    }
    return mount.measure(proposal)
  }

  public override func loadView() { view = NSView(frame: .zero) }
  public override func viewDidLoad() {
    super.viewDidLoad()
    mount = WaterUIMount(runtime: runtime, host: view, resources: resources,
                         environment: latestEnvironment)
  }

  /// Scopes only this host's own view: `appearance` and
  /// `userInterfaceLayoutDirection` are inherited by the mounted subviews
  /// the way a parent view's traits would be — the app's appearance and
  /// the rest of the host window are untouched.
  private func applyEnvironment(_ environment: HostEnvironment) {
    view.appearance = NSAppearance(named: environment.dark ? .darkAqua : .aqua)
    view.userInterfaceLayoutDirection =
      environment.rtl ? .rightToLeft : .leftToRight
  }

  /// Re-evaluates presentation visibility across the mounted tree.
  ///
  /// WaterUI-owned layout, scroll and window transitions reach surfaces
  /// on their own. Mutations on containers between the window and this
  /// controller's `view` — `isHidden`, `alphaValue`, clipping bounds,
  /// frame/transform changes, or reparenting it inside the same window —
  /// publish nothing a mounted surface can observe; foreign `NSView`
  /// ancestors never gain posting flags or subscriptions, so call this
  /// whenever the host mutates one of them and surfaces parked for
  /// visibility re-check and resume.
  public func updateVisibility() {
    guard let mount else { return }
    updateVisibilityNative(UnsafeRawPointer(mount.pointer))
  }
}

public struct WaterUIHost: NSViewRepresentable {
  public let runtime: WaterUIRuntime
  public let resources: WaterUIResourceContext
  public init(runtime: WaterUIRuntime, resources: WaterUIResourceContext = .application) {
    self.runtime = runtime
    self.resources = resources
  }
  /// The coordinator owns the controller — and through it the mount — for
  /// the representable's lifetime, so the native view never outlives its
  /// backend.
  public func makeCoordinator() -> WaterUIHostController {
    WaterUIHostController(runtime: runtime, resources: resources)
  }
  public func makeNSView(context: Context) -> NSView {
    let coordinator = context.coordinator
    coordinator.updateEnvironment(HostEnvironment(context.environment))
    // Reading `view` loads it and mounts under the applied environment —
    // calling `loadView()` here would re-run it and drop the mount's view.
    return coordinator.view
  }
  public func updateNSView(_ nsView: NSView, context: Context) {
    context.coordinator.updateEnvironment(HostEnvironment(context.environment))
  }
  public func sizeThatFits(_ proposal: ProposedViewSize,
                           nsView: NSView,
                           context: Context) -> CGSize? {
    context.coordinator.updateEnvironment(HostEnvironment(context.environment))
    return context.coordinator.measure(proposal)
  }
}
#endif
