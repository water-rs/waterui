use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use cherenkov::WorkingColor;
use cherenkov::kurbo::{Affine, Rect};
use hydrolysis::{
    HydrolysisRenderer, OffscreenGpuContext, OffscreenWindow, Style, SurfaceProvider as _,
    WidgetTheme,
};
use waterui_core::{AnyView, Environment, View};

use crate::artifacts::{CapturedSnapshot, TestArtifacts};

/// RGBA8 frame captured from a headless hydrolysis render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel data in RGBA8 row-major order.
    pub rgba8: Vec<u8>,
}

impl Snapshot {
    /// Saves the snapshot to a PNG file.
    ///
    /// # Errors
    ///
    /// Returns image I/O or encoding errors from the underlying PNG writer.
    ///
    /// # Panics
    ///
    /// Panics if the stored RGBA buffer length does not match the snapshot dimensions.
    pub fn save_png(&self, path: impl AsRef<Path>) -> image::ImageResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(image::ImageError::IoError)?;
        }
        let image = image::RgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
            .expect("Snapshot::save_png: rgba buffer shape must match dimensions");
        image.save(path)
    }
}

/// Headless host that renders `WaterUI` views into an offscreen texture.
///
/// The host renders with a style — the same `hydrolysis::Style` a rendered
/// mount takes — because a frame is a product of the view tree *and* the
/// style package's widget theme; a style-free render exists only on the
/// semantic pipeline, which produces no pixels.
pub struct TestHost {
    env: Environment,
    /// Requested once and shared by every render this host performs. A wgpu
    /// device is expensive to request and expensive to hold on a runner whose
    /// only adapter is a software rasterizer; a host that renders ten views
    /// should ask for one device, not ten.
    gpu: OffscreenGpuContext,
    width: u32,
    height: u32,
    theme: Rc<dyn WidgetTheme>,
}

impl core::fmt::Debug for TestHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TestHost")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl TestHost {
    /// Creates a test host with a fixed render size, installing the style's
    /// tokens into `env` the way a rendered runtime does.
    #[must_use]
    pub fn new(env: Environment, width: u32, height: u32, style: impl Style) -> Self {
        let mut env = env;
        hydrolysis::theme::install_default_tokens(&mut env);
        style.install_tokens(&mut env);
        Self {
            env,
            gpu: OffscreenGpuContext::new_for_tests_blocking(),
            width,
            height,
            theme: Rc::new(style),
        }
    }

    /// Renders a view and returns the captured RGBA8 snapshot.
    ///
    /// # Panics
    ///
    /// Panics if the offscreen Hydrolysis surface cannot render or be read back.
    pub fn render<V: View>(&self, view: V) -> Snapshot {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let platform = OffscreenWindow::on_context(self.gpu.clone(), width, height);
        let mut renderer =
            HydrolysisRenderer::new(Rc::clone(self.gpu.engine()), Rc::clone(&self.theme));
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));

        renderer.reset_scene();
        renderer.begin_rebuild_frame();
        renderer.capture_window_tree(
            AnyView::new(view),
            &self.env,
            bounds,
            Affine::IDENTITY,
            Affine::IDENTITY,
        );
        renderer.finish_rebuild_frame();

        let surface = platform.surface_ref();
        renderer
            .present_frame(surface.surface(), WorkingColor::TRANSPARENT, Instant::now())
            .expect("waterui-testing failed to render the offscreen frame");
        let rgba8 = surface.readback_rgba8();
        drop(renderer);
        drop(platform);
        // Both owners of this render's GPU resources are gone; let the device
        // release them before the next render allocates its own.
        self.gpu.reclaim();

        Snapshot {
            width,
            height,
            rgba8,
        }
    }

    /// Renders a view, stores the PNG in the canonical artifact layout, and returns both.
    pub fn capture_snapshot_with<V: View>(
        &self,
        view: V,
        artifacts: &TestArtifacts,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        artifacts.capture_snapshot(case, stage, self.render(view))
    }
    /// Creates a canonical artifact helper rooted at the provided suite.
    #[must_use]
    pub fn artifacts(&self, suite: impl AsRef<str>) -> TestArtifacts {
        TestArtifacts::new(suite.as_ref())
    }

    /// Renders a view and stores the resulting snapshot in `WaterUI`'s canonical artifact layout.
    pub fn capture_snapshot<V: View>(
        &self,
        view: V,
        suite: impl AsRef<str>,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        let artifacts = self.artifacts(suite);
        artifacts.capture_snapshot(case, stage, self.render(view))
    }
}
