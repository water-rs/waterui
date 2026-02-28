use waterui::window::{Window as WuiWindow, WindowState};

/// Input button mapped from a platform pointer event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
    Other(u16),
}

/// Input key state mapped from a platform keyboard event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

/// Platform-agnostic key identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCode {
    Named(String),
    Unidentified,
}

/// Active key modifiers snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

/// Input events emitted by a windowing backend.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    PointerDown {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    PointerUp {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    PointerMove {
        x: f32,
        y: f32,
    },
    Scroll {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    Key {
        key: KeyCode,
        state: KeyState,
        modifiers: Modifiers,
    },
    Resize {
        width: u32,
        height: u32,
    },
    CloseRequested,
}

/// Errors raised by surface acquisition/presentation.
#[derive(Debug)]
pub enum SurfaceError {
    Surface(wgpu::SurfaceError),
}

impl core::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Surface(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SurfaceError {}

impl From<wgpu::SurfaceError> for SurfaceError {
    fn from(value: wgpu::SurfaceError) -> Self {
        Self::Surface(value)
    }
}

/// A frame acquired from a `SurfaceProvider`.
pub enum SurfaceFrame {
    Offscreen {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
    },
    #[cfg(feature = "winit")]
    Window {
        output: wgpu::SurfaceTexture,
        view: wgpu::TextureView,
    },
}

impl SurfaceFrame {
    #[must_use]
    pub fn texture(&self) -> &wgpu::Texture {
        match self {
            Self::Offscreen { texture, .. } => texture,
            #[cfg(feature = "winit")]
            Self::Window { output, .. } => &output.texture,
        }
    }

    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Offscreen { view, .. } => view,
            #[cfg(feature = "winit")]
            Self::Window { view, .. } => view,
        }
    }
}

/// Rendering surface abstraction consumed by hydrolysis runner/renderer.
pub trait SurfaceProvider {
    fn device(&self) -> &wgpu::Device;
    fn queue(&self) -> &wgpu::Queue;
    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError>;
    fn present(&mut self, frame: SurfaceFrame);
    fn size(&self) -> (u32, u32);
    fn format(&self) -> wgpu::TextureFormat;
    fn resize(&mut self, width: u32, height: u32);
}

/// Window abstraction consumed by hydrolysis runner.
pub trait PlatformWindow {
    fn surface(&mut self) -> &mut dyn SurfaceProvider;
    fn apply_properties(&mut self, window: &WuiWindow);
    fn drain_events(&mut self) -> Vec<InputEvent>;
    fn request_redraw(&self);
    fn scale_factor(&self) -> f64;
}

/// Headless offscreen rendering surface.
pub struct OffscreenSurface {
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    last_presented: Option<wgpu::Texture>,
}

impl core::fmt::Debug for OffscreenSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OffscreenSurface")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl OffscreenSurface {
    pub async fn new(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("hydrolysis offscreen surface: failed to find wgpu adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("hydrolysis-offscreen-device"),
                ..Default::default()
            })
            .await
            .expect("hydrolysis offscreen surface: failed to request wgpu device");

        Self {
            device,
            queue,
            width: width.max(1),
            height: height.max(1),
            format,
            last_presented: None,
        }
    }

    #[must_use]
    pub fn new_blocking(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        pollster::block_on(Self::new(width, height, format))
    }

    #[must_use]
    pub fn last_presented(&self) -> Option<&wgpu::Texture> {
        self.last_presented.as_ref()
    }
}

impl SurfaceProvider for OffscreenSurface {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis-offscreen-frame"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(SurfaceFrame::Offscreen { texture, view })
    }

    fn present(&mut self, frame: SurfaceFrame) {
        match frame {
            SurfaceFrame::Offscreen { texture, .. } => {
                self.last_presented = Some(texture);
            }
            #[cfg(feature = "winit")]
            SurfaceFrame::Window { .. } => {
                panic!("hydrolysis offscreen surface received a window frame");
            }
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }
}

/// Headless platform window backed by an offscreen texture.
#[derive(Debug)]
pub struct OffscreenWindow {
    surface: OffscreenSurface,
    scale_factor: f64,
}

impl OffscreenWindow {
    #[must_use]
    pub fn new(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            surface: OffscreenSurface::new_blocking(width, height, format),
            scale_factor: 1.0,
        }
    }

    #[must_use]
    pub fn surface_ref(&self) -> &OffscreenSurface {
        &self.surface
    }
}

impl PlatformWindow for OffscreenWindow {
    fn surface(&mut self) -> &mut dyn SurfaceProvider {
        &mut self.surface
    }

    fn apply_properties(&mut self, window: &WuiWindow) {
        if window.state.get() == WindowState::Closed {
            return;
        }
        let frame = window.frame.get();
        self.surface.resize(
            frame.width().max(1.0) as u32,
            frame.height().max(1.0) as u32,
        );
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        Vec::new()
    }

    fn request_redraw(&self) {}

    fn scale_factor(&self) -> f64 {
        self.scale_factor
    }
}

#[cfg(feature = "winit")]
mod winit_impl {
    use std::sync::Arc;

    use nami::Signal;
    use waterui::window::WindowState;
    use waterui_graphics::gpu_surface::preferred_surface_format;
    use winit::{
        dpi::LogicalSize,
        event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
        keyboard::{Key, ModifiersState},
        window::{Fullscreen, Window as NativeWindow, WindowId},
    };

    use super::{
        InputEvent, KeyCode, KeyState, Modifiers, PlatformWindow, PointerButton, SurfaceError,
        SurfaceFrame, SurfaceProvider,
    };

    pub struct WinitSurface {
        _instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: wgpu::SurfaceConfiguration,
    }

    impl core::fmt::Debug for WinitSurface {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("WinitSurface")
                .field("config", &self.config)
                .finish_non_exhaustive()
        }
    }

    impl WinitSurface {
        pub async fn new(window: Arc<NativeWindow>) -> Self {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let surface = instance
                .create_surface(window.clone())
                .expect("hydrolysis winit surface: failed to create surface");
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .expect("hydrolysis winit surface: failed to find adapter");

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("hydrolysis-winit-device"),
                    ..Default::default()
                })
                .await
                .expect("hydrolysis winit surface: failed to request device");

            let caps = surface.get_capabilities(&adapter);
            let format = preferred_surface_format(&caps);
            if !matches!(
                format,
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
            ) {
                panic!(
                    "hydrolysis winit surface: Vello requires Rgba8Unorm( sRGB ) target, got {format:?}"
                );
            }
            let size = window.inner_size();
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            Self {
                _instance: instance,
                surface,
                device,
                queue,
                config,
            }
        }
    }

    impl SurfaceProvider for WinitSurface {
        fn device(&self) -> &wgpu::Device {
            &self.device
        }

        fn queue(&self) -> &wgpu::Queue {
            &self.queue
        }

        fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
            let output = self.surface.get_current_texture()?;
            let view = output
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            Ok(SurfaceFrame::Window { output, view })
        }

        fn present(&mut self, frame: SurfaceFrame) {
            match frame {
                SurfaceFrame::Window { output, .. } => output.present(),
                SurfaceFrame::Offscreen { .. } => {
                    panic!("hydrolysis winit surface received an offscreen frame")
                }
            }
        }

        fn size(&self) -> (u32, u32) {
            (self.config.width, self.config.height)
        }

        fn format(&self) -> wgpu::TextureFormat {
            self.config.format
        }

        fn resize(&mut self, width: u32, height: u32) {
            self.config.width = width.max(1);
            self.config.height = height.max(1);
            self.surface.configure(&self.device, &self.config);
        }
    }

    #[derive(Debug)]
    pub struct WinitWindow {
        window: Arc<NativeWindow>,
        surface: WinitSurface,
        pending_events: Vec<InputEvent>,
        pointer_position: (f32, f32),
        modifiers: Modifiers,
    }

    impl WinitWindow {
        pub async fn new(window: Arc<NativeWindow>) -> Self {
            let surface = WinitSurface::new(window.clone()).await;
            Self {
                window,
                surface,
                pending_events: Vec::new(),
                pointer_position: (0.0, 0.0),
                modifiers: Modifiers::default(),
            }
        }

        #[must_use]
        pub fn id(&self) -> WindowId {
            self.window.id()
        }

        pub fn handle_window_event(&mut self, event: &WindowEvent) {
            match event {
                WindowEvent::CloseRequested => {
                    self.pending_events.push(InputEvent::CloseRequested);
                }
                WindowEvent::Resized(size) => {
                    self.surface.resize(size.width, size.height);
                    self.pending_events.push(InputEvent::Resize {
                        width: size.width.max(1),
                        height: size.height.max(1),
                    });
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.pointer_position = (position.x as f32, position.y as f32);
                    self.pending_events.push(InputEvent::PointerMove {
                        x: self.pointer_position.0,
                        y: self.pointer_position.1,
                    });
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let mapped_button = map_button(*button);
                    let (x, y) = self.pointer_position;
                    match state {
                        ElementState::Pressed => {
                            self.pending_events.push(InputEvent::PointerDown {
                                x,
                                y,
                                button: mapped_button,
                            });
                        }
                        ElementState::Released => {
                            self.pending_events.push(InputEvent::PointerUp {
                                x,
                                y,
                                button: mapped_button,
                            });
                        }
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let (dx, dy) = match delta {
                        MouseScrollDelta::LineDelta(dx, dy) => (*dx, *dy),
                        MouseScrollDelta::PixelDelta(delta) => (delta.x as f32, delta.y as f32),
                    };
                    self.pending_events.push(InputEvent::Scroll {
                        x: self.pointer_position.0,
                        y: self.pointer_position.1,
                        dx,
                        dy,
                    });
                }
                WindowEvent::ModifiersChanged(modifiers) => {
                    self.modifiers = modifiers.state().into();
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    self.pending_events.push(InputEvent::Key {
                        key: map_key(&event.logical_key),
                        state: match event.state {
                            ElementState::Pressed => KeyState::Pressed,
                            ElementState::Released => KeyState::Released,
                        },
                        modifiers: self.modifiers,
                    });
                }
                _ => {}
            }
        }
    }

    impl PlatformWindow for WinitWindow {
        fn surface(&mut self) -> &mut dyn SurfaceProvider {
            &mut self.surface
        }

        fn apply_properties(&mut self, window: &waterui::window::Window) {
            self.window.set_title(window.title.get().as_str());
            self.window.set_resizable(window.resizable);
            let frame = window.frame.get();
            let _ = self.window.request_inner_size(LogicalSize::new(
                frame.width() as f64,
                frame.height() as f64,
            ));
            match window.state.get() {
                WindowState::Normal => {
                    self.window.set_minimized(false);
                    self.window.set_fullscreen(None);
                }
                WindowState::Minimized => {
                    self.window.set_minimized(true);
                }
                WindowState::Fullscreen => {
                    self.window
                        .set_fullscreen(Some(Fullscreen::Borderless(None)));
                }
                WindowState::Closed => {
                    self.window.set_visible(false);
                }
            }
        }

        fn drain_events(&mut self) -> Vec<InputEvent> {
            core::mem::take(&mut self.pending_events)
        }

        fn request_redraw(&self) {
            self.window.request_redraw();
        }

        fn scale_factor(&self) -> f64 {
            self.window.scale_factor()
        }
    }

    impl From<ModifiersState> for Modifiers {
        fn from(value: ModifiersState) -> Self {
            Self {
                shift: value.shift_key(),
                control: value.control_key(),
                alt: value.alt_key(),
                super_key: value.super_key(),
            }
        }
    }

    fn map_button(button: MouseButton) -> PointerButton {
        match button {
            MouseButton::Left => PointerButton::Primary,
            MouseButton::Right => PointerButton::Secondary,
            MouseButton::Middle => PointerButton::Middle,
            MouseButton::Back => PointerButton::Back,
            MouseButton::Forward => PointerButton::Forward,
            MouseButton::Other(value) => PointerButton::Other(value),
        }
    }

    fn map_key(key: &Key) -> KeyCode {
        match key {
            Key::Character(value) => KeyCode::Named(value.to_string()),
            Key::Named(value) => KeyCode::Named(format!("{value:?}")),
            _ => KeyCode::Unidentified,
        }
    }

    pub use WinitWindow as ExportedWinitWindow;
}

#[cfg(feature = "winit")]
pub use winit_impl::ExportedWinitWindow as WinitWindow;
