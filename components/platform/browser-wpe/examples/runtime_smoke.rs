//! Native Linux validation for the staged WPE runtime and GPU DMA-BUF path.

#[cfg(target_os = "linux")]
#[path = "runtime_smoke/bridge_smoke.rs"]
mod bridge_smoke;

#[cfg(target_os = "linux")]
#[path = "runtime_smoke/executor.rs"]
mod executor;

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::{Cell, RefCell};
    use std::ffi::{OsStr, OsString};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use base64::Engine as _;
    use waterui_browser_wpe::{
        DmaBufFrameSource, DmaBufGpuView, WPE_WEBKIT_VERSION, WpeRuntime, WpeRuntimePaths,
    };
    use waterui_graphics::cherenkov::{Display, FrameTime};
    use waterui_graphics::gpu::{GpuContentRenderer, GpuRuntime};
    use waterui_graphics::{OffscreenImage, OffscreenSize, RedrawHandle};
    use waterui_url::Url;
    use waterui_webview::{BackendEvent, WebViewEvent};
    use waterui_webview::{BridgeOrigins, DOCUMENT_START_SCRIPT, JsReply, OriginPolicy};
    use wgpu_external_frame::dma_buf::DmaBufFrame;

    use super::executor::{SmokeExecutor, SmokePage};

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

    fn parse_args() -> (OsString, OsString, Duration) {
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
        (
            runtime_root,
            output_path,
            Duration::from_secs(timeout_seconds),
        )
    }

    fn await_loaded_page(page: &SmokePage, deadline: Instant) -> Rc<Cell<bool>> {
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

        while !loaded.get() {
            page.pump();
            if let Some(error) = load_error.borrow().as_deref() {
                panic!("WPE smoke page load failed: {error}");
            }
            assert!(
                Instant::now() < deadline,
                "WPE smoke timed out before the page loaded"
            );
            std::thread::yield_now();
        }
        frame_ready
    }

    fn await_rendered_frame(
        page: &SmokePage,
        frame_ready: &Cell<bool>,
        deadline: Instant,
    ) -> DmaBufFrame {
        while !frame_ready.get() {
            page.pump();
            assert!(
                Instant::now() < deadline,
                "WPE smoke timed out before the page submitted a frame"
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
        frame
    }

    // Presents the frame through the engine-content path and writes the GPU
    // readback out as a PNG.
    fn render_to_png(gpu_runtime: &GpuRuntime, frame: DmaBufFrame, output_path: &OsStr) {
        let source = SmokeFrameSource {
            frame: RefCell::new(Some(frame)),
        };
        let size = OffscreenSize::try_from_pixels(WIDTH, HEIGHT)
            .expect("WPE smoke viewport must be non-zero");
        let mut view = DmaBufGpuView::new(source).into_view();
        let engine_content = view.take_engine_content();
        let context = gpu_runtime.context();
        // One frame, read back at once: no host loop exists to wake.
        let mut renderer = GpuContentRenderer::new(
            gpu_runtime,
            context.clone(),
            engine_content,
            size,
            RedrawHandle::new(|| {}),
        )
        .expect("the smoke renderer's engine layer installs");
        // The UI hook feeds the content's mailbox; run it before presenting so
        // the smoke frame is queued for the render.
        view.frame();
        let device = context.device();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wpe_smoke_target"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        renderer
            .present(
                &target,
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
                FrameTime(Instant::now()),
            )
            .expect("the smoke frame presents");
        let bytes_per_row = WIDTH * 4;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wpe_smoke_readback"),
            size: u64::from(bytes_per_row) * u64::from(HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("wpe_smoke_readback"),
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        context.queue().submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("WPE smoke readback wait failed");
        let rgba8 = buffer
            .slice(..)
            .get_mapped_range()
            .expect("WPE smoke readback buffer must be mapped after a successful wait")
            .to_vec();
        buffer.unmap();
        let image = OffscreenImage {
            width: WIDTH,
            height: HEIGHT,
            rgba8,
        };
        image
            .save_png(output_path)
            .unwrap_or_else(|error| panic!("WPE smoke snapshot write failed: {error}"));
    }

    fn await_bridge_smoke(page: &SmokePage, deadline: Instant) {
        let result = Rc::new(RefCell::new(None));
        let result_slot = Rc::clone(&result);
        let page_for_future = page.clone();
        executor_core::spawn_local(async move {
            let result = page_for_future
                .call_async_javascript(
                    r#"
                    const nativeReply = globalThis.__wateruiNativeSend(
                      JSON.stringify({id: 0, name: "json", json: null})
                    );
                    nativeReply.catch(() => {});
                    const escape = nativeReply.constructor.constructor("return globalThis")();
                    const defaultWorld = escape === globalThis &&
                      !escape.webkit?.messageHandlers?.__waterui;
                    let rawHandlerRejected = false;
                    try {
                      const rawHandler = globalThis.webkit?.messageHandlers?.__waterui;
                      if (!rawHandler) {
                        rawHandlerRejected = true;
                      } else {
                        rawHandler.postMessage({origin: "https://wpe-smoke.invalid", envelope: "{}"});
                      }
                    } catch {
                      rawHandlerRejected = true;
                    }
                    const frame = document.createElement("iframe");
                    frame.srcdoc = "<!doctype html><title>frame</title>";
                    document.body.append(frame);
                    await new Promise((resolve) => frame.addEventListener("load", resolve, {once: true}));
                    const iframeHasTransport =
                      typeof frame.contentWindow.__wateruiNativeSend === "function";
                    location.hash = "same-document";
                    const json = await waterui.invoke("json", {});
                    const binary = await waterui.invoke("binary", {});
                    let failure = "";
                    try {
                      await waterui.invoke("failure", {});
                    } catch (error) {
                      failure = String(error.message || error);
                    }
                    const iframeHasHandler =
                      Boolean(frame.contentWindow.webkit?.messageHandlers?.__waterui);
                    return {defaultWorld, rawHandlerRejected, iframeHasTransport, iframeHasHandler,
                      json, binary: Array.from(binary), failure};
                    "#,
                )
                .await;
            result_slot.replace(Some(result));
        })
        .detach();

        while result.borrow().is_none() {
            page.pump();
            assert!(
                Instant::now() < deadline,
                "WPE bridge smoke timed out waiting for native replies"
            );
            std::thread::yield_now();
        }
        let result = result
            .borrow_mut()
            .take()
            .expect("WPE bridge smoke result was set");
        let result = result.unwrap_or_else(|error| panic!("WPE bridge smoke failed: {error}"));
        let result: serde_json::Value = serde_json::from_str(result.as_str())
            .unwrap_or_else(|error| panic!("WPE bridge smoke returned invalid JSON: {error}"));
        assert_eq!(result["defaultWorld"], true);
        assert_eq!(result["rawHandlerRejected"], true);
        assert_eq!(result["iframeHasTransport"], false);
        assert_eq!(result["iframeHasHandler"], false);
        assert_eq!(result["json"]["answer"], 42);
        assert_eq!(result["binary"], serde_json::json!([0, 1, 2]));
        assert_eq!(result["failure"], "smoke failure");
        eprintln!("PASS basic bridge: default-world isolation and raw-handler rejection");
        eprintln!("PASS basic bridge: iframe transport and handler isolation");
        eprintln!("PASS admitted-page bridge call and JSON reply after same-document hash change");
        eprintln!("PASS basic bridge: binary reply");
        eprintln!("PASS basic bridge: handler failure reply");
    }

    pub fn run() {
        let executor = SmokeExecutor::install();
        let (runtime_root, output_path, timeout) = parse_args();
        let paths = WpeRuntimePaths::new(runtime_root);
        let runtime = WpeRuntime::initialize(&paths);
        let page = SmokePage::new(runtime.clone(), &executor);
        page.set_bridge_origins(OriginPolicy::new(
            BridgeOrigins::Any,
            &Url::new("https://wpe-smoke.invalid/"),
        ));
        page.add_script(
            "waterui:wpe-transport",
            include_str!("../src/transport.js"),
            false,
        );
        page.add_script("waterui:bridge", DOCUMENT_START_SCRIPT, false);
        page.add_handler(
            "json",
            Box::new(|_| Box::pin(async { Ok(JsReply::Json(br#"{"answer":42}"#.to_vec())) })),
        );
        page.add_handler(
            "binary",
            Box::new(|_| Box::pin(async { Ok(JsReply::Bytes(vec![0, 1, 2])) })),
        );
        page.add_handler(
            "failure",
            Box::new(|_| Box::pin(async { Err(String::from("smoke failure")) })),
        );
        let deadline = Instant::now() + timeout;
        let frame_ready = await_loaded_page(&page, deadline);
        eprintln!("RUN basic bridge checks");
        await_bridge_smoke(&page, deadline);
        eprintln!("RUN navigation-barrier checks");
        super::bridge_smoke::run(&runtime, &executor, deadline);
        eprintln!("RUN frame import and snapshot");
        let frame = await_rendered_frame(&page, &frame_ready, deadline);
        let gpu_runtime = pollster::block_on(GpuRuntime::new())
            .unwrap_or_else(|error| panic!("WPE smoke GPU runtime creation failed: {error}"));
        render_to_png(&gpu_runtime, frame, &output_path);
        eprintln!("PASS frame import and snapshot");
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
