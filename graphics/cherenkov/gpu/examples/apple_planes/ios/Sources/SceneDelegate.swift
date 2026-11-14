import UIKit

/// The iOS 27 host uses the scene lifecycle. An app that only implements
/// `UIApplicationDelegate` is killed by
/// `UIKitEvaluateRuntimeIssueForNoSceneLifecycleAdoption`. The scene
/// owns the window and the foreground brightness transitions.
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(
        _ scene: UIScene,
        willConnectTo _: UISceneSession,
        options _: UIScene.ConnectionOptions
    ) {
        guard let windowScene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: windowScene)
        window.rootViewController = PlanesViewController()
        window.makeKeyAndVisible()
        self.window = window
    }

    func sceneWillResignActive(_ scene: UIScene) {
        cherenkov_planes_brightness_restore()
    }

    func sceneDidBecomeActive(_ scene: UIScene) {
        cherenkov_planes_brightness_dim()
    }

    func sceneDidDisconnect(_ scene: UIScene) {
        cherenkov_planes_brightness_restore()
    }
}
