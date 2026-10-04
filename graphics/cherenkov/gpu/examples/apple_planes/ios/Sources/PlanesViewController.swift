import QuartzCore
import UIKit

@_cdecl("cherenkov_planes_wake")
func cherenkovPlanesWake() {
    DispatchQueue.main.async {
        NotificationCenter.default.post(name: Notification.Name("CherenkovPlanesWake"), object: nil)
    }
}

/// The view the engine presents into. Its backing layer is a
/// `CAMetalLayer` so the layer hierarchy it hosts — the engine's metal
/// parts and the video's `AVSampleBufferDisplayLayer` plane — sits on
/// the same kind the window server composites natively.
final class PlanesView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
}

/// Owns the display link and forwards each tick into the Rust harness.
/// Production, presentation and the per-second heartbeat all happen in
/// Rust; this class only paces them at the panel's refresh rate.
final class PlanesViewController: UIViewController {
    private var link: CADisplayLink?
    private var started = false
    private var lastSize = CGSize.zero
    private var coolingDeadline: DispatchWorkItem?

    override func loadView() {
        view = PlanesView()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let size = view.bounds.size
        guard let window = view.window, size.width > 0, size.height > 0 else { return }
        let scale = window.screen.nativeScale
        if !started {
            guard readyToStart() else { return }
            started = true
            lastSize = size
            cherenkov_planes_start(
                UnsafeRawPointer(Unmanaged.passUnretained(view).toOpaque()),
                Double(size.width),
                Double(size.height),
                Double(scale)
            )
            let link = CADisplayLink(target: self, selector: #selector(step))
            link.add(to: .main, forMode: .common)
            self.link = link
        } else if size != lastSize {
            lastSize = size
            cherenkov_planes_resize(
                Double(size.width),
                Double(size.height),
                Double(scale)
            )
        }
    }

    /// Observe real thermal recovery before creating the engine. The deadline
    /// only aborts a failed cool-down; it never starts a measurement.
    private func readyToStart() -> Bool {
        if ProcessInfo.processInfo.thermalState != .nominal {
            if coolingDeadline == nil {
                NSLog("Waiting for nominal thermal state before starting the harness")
                let deadline = DispatchWorkItem {
                    NSLog("Thermal recovery timed out after fifteen minutes")
                    exit(EXIT_FAILURE)
                }
                coolingDeadline = deadline
                DispatchQueue.main.asyncAfter(deadline: .now() + 900, execute: deadline)
            }
            return false
        }
        coolingDeadline?.cancel()
        coolingDeadline = nil
        return true
    }

    @objc private func thermalChanged() {
        DispatchQueue.main.async {
            if !self.started { self.view.setNeedsLayout() }
        }
    }

    @objc private func step() {
        link?.isPaused = cherenkov_planes_tick()
        if cherenkov_planes_finished() {
            link?.invalidate()
            // A finite benchmark run: devicectl --console observes process
            // completion; its report was fsynced before this signal.
            exit(EXIT_SUCCESS)
        }
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(thermalChanged),
            name: ProcessInfo.thermalStateDidChangeNotification,
            object: nil
        )
        NotificationCenter.default.addObserver(self, selector: #selector(resume), name: Notification.Name("CherenkovPlanesWake"), object: nil)
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(pause),
            name: UIScene.willDeactivateNotification,
            object: nil
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(resume),
            name: UIScene.didActivateNotification,
            object: nil
        )
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        NotificationCenter.default.removeObserver(self)
    }

    /// Backgrounded apps may not touch the GPU — stop the link while
    /// the app is inactive.
    @objc private func pause() {
        link?.isPaused = true
    }

    @objc private func resume() {
        link?.isPaused = false
    }
}
