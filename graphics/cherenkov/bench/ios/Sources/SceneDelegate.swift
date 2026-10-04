import UIKit

/// iOS 26 requires scene lifecycle: an app with only a
/// `UIApplicationDelegate` is killed by UIKit's
/// no-scene-lifecycle-adoption runtime check. The scene owns the
/// window; the bench itself stays in `AppDelegate`.
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(
        _ scene: UIScene,
        willConnectTo _: UISceneSession,
        options _: UIScene.ConnectionOptions
    ) {
        guard let windowScene = scene as? UIWindowScene,
              let status = (UIApplication.shared.delegate as? AppDelegate)?.status
        else { return }
        let window = UIWindow(windowScene: windowScene)
        window.rootViewController = status
        window.makeKeyAndVisible()
        self.window = window
    }
}
