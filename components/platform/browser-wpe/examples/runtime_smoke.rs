//! Native Linux validation for the staged WPE runtime and GPU DMA-BUF path.

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use base64::Engine as _;
    use waterui_browser_wpe::{
        DmaBufFrameSource, DmaBufGpuView, WPE_WEBKIT_VERSION, WpePage, WpeRuntime, WpeRuntimePaths,
    };
    use waterui_core::Environment;
    use waterui_graphics::gpu_surface::{GpuSurface, OffscreenRenderConfig, OffscreenSize};
    use waterui_graphics::shared_context::GpuRuntime;
    use waterui_webview::{BackendEvent, WebViewEvent};
    use wgpu_external_frame::dma_buf::DmaBufFrame;

    const WIDTH: u32 = 640;
    const HEIGHT: u32 = 360;

    struct SmokeFrameSource {
        frame: RefCell<Option<DmaBufFrame>>,
    }

    impl DmaBufFrameSource for SmokeFrameSource {
        fn pump(&self) {}

        fn resize(&self, _width: u32, _height: u32, _scale: f64) {}

        fn set_frame_waker(&self, _waker: Rc<dyn Fn()>) {}

        fn take_frame(&self) -> Option<DmaBufFrame> {
            self.frame.borrow_mut().take()
        }
    }

    pub fn run() {
        let mut arguments = std::env::args_os().skip(1);
        let runtime_root = arguments
            .next()
            .expect("runtime_smoke requires a staged WPE runtime root");
        let output_path = arguments
            .next()
            .expect("runtime_smoke requires an output PNG path");
        let timeout_seconds = arguments
            .next()
            .expect("runtime_smoke requires a timeout in seconds")
            .to_string_lossy()
            .parse::<u64>()
            .expect("runtime_smoke timeout must be an integer");
        assert!(
            arguments.next().is_none(),
            "runtime_smoke received unexpected arguments"
        );

        let gpu_runtime = pollster::block_on(GpuRuntime::new())
            .unwrap_or_else(|error| panic!("WPE smoke GPU runtime creation failed: {error}"));
        let paths = WpeRuntimePaths::new(runtime_root);
        let runtime = WpeRuntime::initialize(&paths);
        let page = WpePage::new(runtime);
        let loaded = Rc::new(Cell::new(false));
        let load_error = Rc::new(RefCell::new(None::<String>));
        // The guard has to outlive the pump loop below: dropping it
        // unsubscribes, and the smoke run would then wait for a `Loaded` it can
        // no longer observe until it times out.
        let _load_watcher = page.watch({
            let loaded = Rc::clone(&loaded);
            let load_error = Rc::clone(&load_error);
            move |event| match event {
                BackendEvent::Event(WebViewEvent::Loaded) => loaded.set(true),
                BackendEvent::Event(WebViewEvent::Error(error)) => {
                    load_error.replace(Some(format!("{error:?}")));
                }
                _ => {}
            }
        });
        let frame_ready = Rc::new(Cell::new(false));
        page.set_frame_waker({
            let frame_ready = Rc::clone(&frame_ready);
            move || frame_ready.set(true)
        });
        page.resize(WIDTH, HEIGHT, 1.0);
        let document =
            include_str!("runtime_smoke.html").replace("{{WPE_VERSION}}", WPE_WEBKIT_VERSION);
        let document = base64::engine::general_purpose::STANDARD.encode(document);
        page.load_uri(&format!("data:text/html;base64,{document}"));

        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        while !loaded.get() || !frame_ready.get() {
            page.pump();
            if let Some(error) = load_error.borrow().as_deref() {
                panic!("WPE smoke page load failed: {error}");
            }
            assert!(
                Instant::now() < deadline,
                "WPE smoke timed out before the page loaded and submitted a frame"
            );
            std::thread::yield_now();
        }
        let frame = page
            .take_frame()
            .expect("WPE signalled a frame without retaining it");
        while !frame.is_render_ready() {
            page.pump();
            assert!(
                Instant::now() < deadline,
                "WPE smoke timed out waiting for the rendering fence"
            );
            std::thread::yield_now();
        }

        let source = SmokeFrameSource {
            frame: RefCell::new(Some(frame)),
        };
        let surface = GpuSurface::new(DmaBufGpuView::new(source));
        let config = OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(WIDTH, HEIGHT)
                .expect("WPE smoke viewport must be non-zero"),
        )
        .format(wgpu::TextureFormat::Rgba8Unorm);
        let rendered = pollster::block_on(surface.render_offscreen(
            &gpu_runtime,
            config,
            &mut Environment::new(),
        ))
        .unwrap_or_else(|error| panic!("WPE smoke offscreen render failed: {error}"));
        rendered
            .save_png(output_path)
            .unwrap_or_else(|error| panic!("WPE smoke snapshot write failed: {error}"));
    }
}

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("runtime_smoke is supported only on Linux");
}
