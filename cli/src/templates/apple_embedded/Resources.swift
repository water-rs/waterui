import Foundation

/// Pass `.module` to the host's `resources` argument for this package's assets and fonts.
extension WaterUIResourceContext {
    public static var module: Self {
        Self(assets: assetsURL, fonts: fontsURL)
    }

    private static var assetsURL: URL {
        guard let url = Bundle.module.url(forResource: "waterui_assets", withExtension: nil) else {
            fatalError("The WaterUI package is missing its asset bundle")
        }
        return url
    }

    private static var fontsURL: URL {
        guard let url = Bundle.module.url(forResource: "fonts", withExtension: nil) else {
            fatalError("The WaterUI package is missing its font bundle")
        }
        return url
    }
}
