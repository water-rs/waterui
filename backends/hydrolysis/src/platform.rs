use waterui::cursor::CursorStyle;
use waterui::window::{Window as WuiWindow, WindowState};

#[cfg(any(feature = "winit", all(target_arch = "wasm32", feature = "web")))]
use waterui_graphics::gpu_surface::preferred_surface_format;

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

/// Physical pointer source reported by the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Mouse,
    Touch,
    Pen,
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
    Character(String),
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

/// IME purpose for the focused text input target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputPurpose {
    Normal,
    Password,
}

pub use waterui_backend_core::input::TouchPhase;

/// Focused text-input area used for IME activation and candidate-window placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextInputState {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub purpose: TextInputPurpose,
}

/// Input events emitted by a windowing backend.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    PointerDown {
        id: u64,
        kind: PointerKind,
        x: f32,
        y: f32,
        button: PointerButton,
    },
    PointerUp {
        id: u64,
        kind: PointerKind,
        x: f32,
        y: f32,
        button: PointerButton,
    },
    PointerMove {
        id: u64,
        kind: PointerKind,
        x: f32,
        y: f32,
    },
    PointerCancel {
        id: u64,
        kind: PointerKind,
    },
    Moved {
        x: f32,
        y: f32,
    },
    Scroll {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        is_line_delta: bool,
    },
    Magnification {
        x: f32,
        y: f32,
        delta: f32,
        phase: TouchPhase,
    },
    Rotation {
        x: f32,
        y: f32,
        delta: f32,
        phase: TouchPhase,
    },
    TextInput {
        text: String,
    },
    Key {
        key: KeyCode,
        state: KeyState,
        modifiers: Modifiers,
    },
    ImePreedit {
        text: String,
    },
    ImeCommit {
        text: String,
    },
    ImeDisabled,
    Resize {
        width: u32,
        height: u32,
    },
    CloseRequested,
}

/// Errors raised by surface acquisition/presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    Timeout,
    Occluded,
    Outdated,
    Lost,
    Validation,
}

impl core::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Timeout => "surface acquisition timed out",
            Self::Occluded => "surface is occluded",
            Self::Outdated => "surface configuration is outdated",
            Self::Lost => "surface was lost",
            Self::Validation => "surface acquisition failed validation",
        })
    }
}

impl std::error::Error for SurfaceError {}

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
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    Browser {
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
            #[cfg(all(target_arch = "wasm32", feature = "web"))]
            Self::Browser { output, .. } => &output.texture,
        }
    }

    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Offscreen { view, .. } => view,
            #[cfg(feature = "winit")]
            Self::Window { view, .. } => view,
            #[cfg(all(target_arch = "wasm32", feature = "web"))]
            Self::Browser { view, .. } => view,
        }
    }
}

#[cfg(any(feature = "winit", all(target_arch = "wasm32", feature = "web")))]
fn select_hydrolysis_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    let preferred = preferred_surface_format(caps);
    if supports_hydrolysis_surface_format(preferred) {
        return normalize_surface_format(caps, preferred);
    }

    if let Some(format) = caps
        .formats
        .iter()
        .copied()
        .find(|format| supports_hydrolysis_surface_format(*format))
    {
        return normalize_surface_format(caps, format);
    }

    panic!(
        "hydrolysis surface: requires one of Rgba16Float/Rgba32Float/Rgba8/Bgra8 surface formats, got {:?}",
        caps.formats
    );
}

#[cfg(any(feature = "winit", all(target_arch = "wasm32", feature = "web")))]
fn supports_hydrolysis_surface_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format.remove_srgb_suffix(),
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
    ) || matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

#[cfg(any(feature = "winit", all(target_arch = "wasm32", feature = "web")))]
fn normalize_surface_format(
    caps: &wgpu::SurfaceCapabilities,
    format: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    if format.is_srgb() {
        let linear = format.remove_srgb_suffix();
        if caps.formats.contains(&linear) {
            return linear;
        }
    }
    format
}

#[cfg(any(feature = "winit", all(target_arch = "wasm32", feature = "web")))]
fn acquire_surface_texture(
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::SurfaceTexture, SurfaceError> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(output)
        | wgpu::CurrentSurfaceTexture::Suboptimal(output) => Ok(output),
        wgpu::CurrentSurfaceTexture::Timeout => Err(SurfaceError::Timeout),
        wgpu::CurrentSurfaceTexture::Occluded => Err(SurfaceError::Occluded),
        wgpu::CurrentSurfaceTexture::Outdated => Err(SurfaceError::Outdated),
        wgpu::CurrentSurfaceTexture::Lost => Err(SurfaceError::Lost),
        wgpu::CurrentSurfaceTexture::Validation => Err(SurfaceError::Validation),
    }
}

/// Rendering surface abstraction consumed by hydrolysis runner/renderer.
pub trait SurfaceProvider {
    fn adapter(&self) -> &wgpu::Adapter;
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
    /// Applies the window's effective content-size limits (logical units).
    ///
    /// Explicit `Window::min_size`/`max_size` values take precedence; otherwise
    /// each limit comes from the content's layout negotiation. Targets without
    /// per-window runtime size limits (offscreen surfaces, web canvases, fixed
    /// embedded displays) keep this default no-op.
    fn set_size_limits(
        &mut self,
        min: Option<waterui_core::layout::Size>,
        max: Option<waterui_core::layout::Size>,
    ) {
        let _ = (min, max);
    }
    fn drain_events(&mut self) -> Vec<InputEvent>;
    fn request_redraw(&self);
    fn scale_factor(&self) -> f64;
    /// The refresh rate (Hz) of the display this window is on, if known.
    ///
    /// Drives the game-engine continuous-render frame budget and the diagnostics
    /// slow-frame threshold. Returns `None` on headless/offscreen/web paths with no
    /// monitor information, where the renderer falls back to its default pacing.
    fn refresh_rate_hz(&self) -> Option<f64> {
        None
    }
    fn sync_text_input_state(&mut self, state: Option<TextInputState>);
    fn set_cursor_style(&mut self, style: CursorStyle);
}

/// Headless offscreen rendering surface.
pub struct OffscreenSurface {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    last_presented: Option<wgpu::Texture>,
}

fn should_force_fallback_adapter() -> bool {
    std::env::var_os("WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER").is_some()
}

#[derive(Clone, Copy, Debug)]
struct AdapterSelection {
    allow_software_adapter: bool,
}

impl AdapterSelection {
    const PRODUCTION: Self = Self {
        allow_software_adapter: false,
    };

    #[cfg(any(test, feature = "testing"))]
    const TEST: Self = Self {
        allow_software_adapter: true,
    };

    fn force_fallback_adapter(self) -> bool {
        should_force_fallback_adapter()
    }

    fn allow_software_adapter(self) -> bool {
        self.allow_software_adapter || self.force_fallback_adapter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AdapterPreference {
    backend_rank: u8,
    device_type_rank: u8,
}

impl AdapterPreference {
    fn for_info(info: &wgpu::AdapterInfo) -> Self {
        Self {
            backend_rank: backend_rank(info.backend),
            device_type_rank: device_type_rank(info.device_type),
        }
    }
}

const fn backend_rank(backend: wgpu::Backend) -> u8 {
    if cfg!(target_os = "windows") {
        match backend {
            wgpu::Backend::Dx12 => 0,
            wgpu::Backend::Vulkan => 1,
            wgpu::Backend::Metal => 2,
            wgpu::Backend::Gl => 3,
            wgpu::Backend::BrowserWebGpu => 4,
            wgpu::Backend::Noop => 5,
        }
    } else if cfg!(target_os = "macos") {
        match backend {
            wgpu::Backend::Metal => 0,
            wgpu::Backend::Vulkan => 1,
            wgpu::Backend::Dx12 => 2,
            wgpu::Backend::Gl => 3,
            wgpu::Backend::BrowserWebGpu => 4,
            wgpu::Backend::Noop => 5,
        }
    } else {
        match backend {
            wgpu::Backend::Vulkan => 0,
            wgpu::Backend::Metal => 1,
            wgpu::Backend::Dx12 => 2,
            wgpu::Backend::Gl => 3,
            wgpu::Backend::BrowserWebGpu => 4,
            wgpu::Backend::Noop => 5,
        }
    }
}

const fn device_type_rank(device_type: wgpu::DeviceType) -> u8 {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

fn is_compute_capable_adapter(adapter: &wgpu::Adapter) -> bool {
    let downlevel_caps = adapter.get_downlevel_capabilities();
    let limits = adapter.limits();
    downlevel_caps
        .flags
        .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        && limits.max_compute_workgroups_per_dimension > 0
}

async fn request_hydrolysis_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
    context: &str,
    selection: AdapterSelection,
) -> wgpu::Adapter {
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface,
                force_fallback_adapter: selection.force_fallback_adapter(),
            })
            .await
            .expect("hydrolysis adapter selection: failed to find web adapter");
        log_selected_adapter(context, &adapter);
        return adapter;
    }

    #[cfg(not(all(target_arch = "wasm32", feature = "web")))]
    {
        if selection.force_fallback_adapter() {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface,
                    force_fallback_adapter: true,
                })
                .await
                .expect("hydrolysis adapter selection: failed to find fallback adapter");
            log_selected_adapter(context, &adapter);
            return adapter;
        }

        let backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all());
        let mut best_candidate: Option<(AdapterPreference, wgpu::Adapter)> = None;
        let mut inspected_adapters: Vec<String> = Vec::new();

        for adapter in instance.enumerate_adapters(backends).await {
            let info = adapter.get_info();
            let surface_supported = compatible_surface
                .as_ref()
                .is_none_or(|surface| adapter.is_surface_supported(surface));
            let limits = adapter.limits();
            let compute_capable = is_compute_capable_adapter(&adapter);

            tracing::info!(
                target: "hydrolysis::gpu",
                context,
                adapter = ?info,
                surface_supported,
                compute_capable,
                max_compute_workgroups_per_dimension = limits.max_compute_workgroups_per_dimension,
                "hydrolysis adapter candidate"
            );

            if !surface_supported {
                continue;
            }

            inspected_adapters.push(format!(
                "'{}' ({:?}, {:?}, compute={}, max_compute_workgroups_per_dimension={})",
                info.name,
                info.backend,
                info.device_type,
                compute_capable,
                limits.max_compute_workgroups_per_dimension
            ));

            if info.backend == wgpu::Backend::Noop
                || (info.device_type == wgpu::DeviceType::Cpu
                    && !selection.allow_software_adapter())
            {
                tracing::info!(
                    target: "hydrolysis::gpu",
                    context,
                    adapter = ?info,
                    "skipping software/noop adapter because fallback adapter was not requested"
                );
                continue;
            }

            if !compute_capable {
                continue;
            }

            let preference = AdapterPreference::for_info(&info);
            match &best_candidate {
                Some((best_preference, _)) if *best_preference <= preference => {}
                _ => best_candidate = Some((preference, adapter)),
            }
        }

        let (_, adapter) = best_candidate.unwrap_or_else(|| {
            if inspected_adapters.is_empty() {
                panic!(
                    "{context}: failed to find a surface-compatible wgpu adapter for requested backends {:?}. \
Set WGPU_BACKEND to an available backend or install/update the platform GPU driver.",
                    backends
                );
            }

            panic!(
                "{context}: failed to find a compute-capable modern adapter. \
Surface-compatible adapters inspected: {}. \
Set WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER=1 to explicitly allow software fallback adapters for diagnostics.",
                inspected_adapters.join("; ")
            );
        });

        log_selected_adapter(context, &adapter);
        adapter
    }
}

fn log_selected_adapter(context: &str, adapter: &wgpu::Adapter) {
    let info = adapter.get_info();
    tracing::info!(
        target: "hydrolysis::gpu",
        context,
        force_fallback_adapter = should_force_fallback_adapter(),
        adapter = ?info,
        "selected wgpu adapter"
    );
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
        Self::new_with_adapter_selection(width, height, format, AdapterSelection::PRODUCTION).await
    }

    /// Creates an offscreen surface for WaterUI test hosts.
    ///
    /// Unlike production surfaces, this constructor allows compute-capable
    /// software adapters so CI can run Hydrolysis accessibility tests on
    /// llvmpipe without opting the runtime path into fallback adapters.
    #[cfg(any(test, feature = "testing"))]
    pub async fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self::new_with_adapter_selection(width, height, format, AdapterSelection::TEST).await
    }

    async fn new_with_adapter_selection(
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        selection: AdapterSelection,
    ) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            request_hydrolysis_adapter(&instance, None, "hydrolysis offscreen surface", selection)
                .await;

        ensure_compute_capable_adapter(
            &adapter,
            "hydrolysis offscreen surface",
            "failed to find compute-capable wgpu adapter",
        );
        let required_limits = required_device_limits(&adapter);
        let required_features =
            waterui_graphics::shared_context::required_media_features(adapter.features());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("hydrolysis-offscreen-device"),
                required_features,
                required_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            })
            .await
            .expect("hydrolysis offscreen surface: failed to request wgpu device");

        Self {
            adapter,
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

fn required_device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let adapter_limits = adapter.limits();
    let downlevel_caps = adapter.get_downlevel_capabilities();
    let base_limits = if downlevel_caps.is_webgpu_compliant()
        || downlevel_caps
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
    {
        wgpu::Limits::default()
    } else {
        wgpu::Limits::downlevel_webgl2_defaults()
    };

    base_limits
        .using_resolution(adapter_limits.clone())
        .using_alignment(adapter_limits)
}

fn ensure_compute_capable_adapter(
    adapter: &wgpu::Adapter,
    context: &str,
    no_compute_message: &str,
) {
    let limits = adapter.limits();
    if is_compute_capable_adapter(adapter) {
        return;
    }

    let info = adapter.get_info();
    let fallback_hint = if cfg!(target_os = "windows") {
        " On Windows, try forcing DX12 WARP: set WGPU_BACKEND=dx12 and WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER=1."
    } else {
        ""
    };
    panic!(
        "{context}: {no_compute_message}. Selected adapter '{}' ({:?}) reports max_compute_workgroups_per_dimension = {}. \
Hydrolysis requires compute shader support. On virtual machines, enable hardware 3D acceleration and update VM graphics tools/driver, \
or run on a host with a compute-capable GPU.{fallback_hint}",
        info.name, info.backend, limits.max_compute_workgroups_per_dimension
    );
}

impl SurfaceProvider for OffscreenSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        let texture = self.last_presented.take().unwrap_or_else(|| {
            self.device.create_texture(&wgpu::TextureDescriptor {
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
            })
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
            #[cfg(all(target_arch = "wasm32", feature = "web"))]
            SurfaceFrame::Browser { .. } => {
                panic!("hydrolysis offscreen surface received a browser frame");
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
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.last_presented = None;
        }
    }
}

/// Headless platform window backed by an offscreen texture.
#[derive(Debug)]
pub struct OffscreenWindow {
    surface: OffscreenSurface,
    scale_factor: f64,
    /// Last applied (min, max) content-size limits, recorded so tests can
    /// assert what the runner derived; offscreen surfaces have no real window
    /// to constrain.
    size_limits: Option<(
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    )>,
}

impl OffscreenWindow {
    #[must_use]
    pub fn new(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            surface: OffscreenSurface::new_blocking(width, height, format),
            scale_factor: 1.0,
            size_limits: None,
        }
    }

    /// Creates an offscreen window for WaterUI test hosts.
    ///
    /// This keeps production adapter selection strict while allowing
    /// `waterui-testing` to run on compute-capable software adapters in CI.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            surface: pollster::block_on(OffscreenSurface::new_for_tests(width, height, format)),
            scale_factor: 1.0,
            size_limits: None,
        }
    }

    #[must_use]
    pub fn surface_ref(&self) -> &OffscreenSurface {
        &self.surface
    }

    /// The last (min, max) content-size limits the runner applied, for tests.
    #[must_use]
    pub fn applied_size_limits(
        &self,
    ) -> Option<(
        Option<waterui_core::layout::Size>,
        Option<waterui_core::layout::Size>,
    )> {
        self.size_limits
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

    fn set_size_limits(
        &mut self,
        min: Option<waterui_core::layout::Size>,
        max: Option<waterui_core::layout::Size>,
    ) {
        self.size_limits = Some((min, max));
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        Vec::new()
    }

    fn request_redraw(&self) {}

    fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    fn sync_text_input_state(&mut self, _state: Option<TextInputState>) {}

    fn set_cursor_style(&mut self, _style: CursorStyle) {}
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_impl;

#[cfg(all(feature = "winit", target_os = "macos"))]
mod macos_display_link;

#[cfg(feature = "winit")]
mod winit_impl {
    use std::sync::Arc;

    use nami::Signal;
    use waterui::window::WindowState;
    use winit::{
        dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
        event::{
            ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta,
            TouchPhase as WinitTouchPhase, WindowEvent,
        },
        keyboard::{Key, ModifiersState},
        window::{
            Cursor as WinitCursor, CursorIcon, Fullscreen, ImePurpose, Window as NativeWindow,
            WindowId,
        },
    };

    use super::{
        CursorStyle, InputEvent, KeyCode, KeyState, Modifiers, PlatformWindow, PointerButton,
        PointerKind, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose, TextInputState,
        TouchPhase,
    };

    #[derive(Clone)]
    pub struct WinitGpuContext {
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    }

    pub struct WinitSurface {
        surface: wgpu::Surface<'static>,
        gpu: WinitGpuContext,
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
        pub async fn new(
            window: Arc<NativeWindow>,
            shared_gpu: Option<&WinitGpuContext>,
        ) -> (Self, WinitGpuContext) {
            let (gpu, surface) = match shared_gpu {
                Some(gpu) => {
                    let surface = gpu
                        .instance
                        .create_surface(window.clone())
                        .expect("hydrolysis winit surface: failed to create shared surface");
                    (gpu.clone(), surface)
                }
                None => {
                    let instance =
                        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
                    let surface = instance
                        .create_surface(window.clone())
                        .expect("hydrolysis winit surface: failed to create surface");
                    let adapter = super::request_hydrolysis_adapter(
                        &instance,
                        Some(&surface),
                        "hydrolysis winit surface",
                        super::AdapterSelection::PRODUCTION,
                    )
                    .await;

                    super::ensure_compute_capable_adapter(
                        &adapter,
                        "hydrolysis winit surface",
                        "failed to find compute-capable wgpu adapter",
                    );
                    let required_limits = super::required_device_limits(&adapter);
                    let required_features =
                        waterui_graphics::shared_context::required_media_features(
                            adapter.features(),
                        );
                    let (device, queue) = adapter
                        .request_device(&wgpu::DeviceDescriptor {
                            label: Some("hydrolysis-winit-device"),
                            required_features,
                            required_limits,
                            memory_hints: wgpu::MemoryHints::Performance,
                            experimental_features: wgpu::ExperimentalFeatures::default(),
                            trace: wgpu::Trace::default(),
                        })
                        .await
                        .expect("hydrolysis winit surface: failed to request device");
                    (
                        WinitGpuContext {
                            instance,
                            adapter,
                            device,
                            queue,
                        },
                        surface,
                    )
                }
            };

            let caps = surface.get_capabilities(&gpu.adapter);
            let format = super::select_hydrolysis_surface_format(&caps);
            let alpha_mode = caps
                .alpha_modes
                .iter()
                .copied()
                .find(|mode| {
                    matches!(
                        mode,
                        wgpu::CompositeAlphaMode::PreMultiplied
                            | wgpu::CompositeAlphaMode::PostMultiplied
                    )
                })
                .unwrap_or(caps.alpha_modes[0]);
            let size = window.inner_size();
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&gpu.device, &config);

            (
                Self {
                    surface,
                    gpu: gpu.clone(),
                    config,
                },
                gpu,
            )
        }
    }

    impl SurfaceProvider for WinitSurface {
        fn adapter(&self) -> &wgpu::Adapter {
            &self.gpu.adapter
        }

        fn device(&self) -> &wgpu::Device {
            &self.gpu.device
        }

        fn queue(&self) -> &wgpu::Queue {
            &self.gpu.queue
        }

        fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
            let output = super::acquire_surface_texture(&self.surface)?;
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
            self.surface.configure(&self.gpu.device, &self.config);
        }
    }

    #[derive(Debug)]
    pub struct WinitWindow {
        window: Arc<NativeWindow>,
        surface: WinitSurface,
        pending_surface_size: Option<PhysicalSize<u32>>,
        pending_events: Vec<InputEvent>,
        pointer_position: (f32, f32),
        modifiers: Modifiers,
        applied_text_input_state: Option<TextInputState>,
        current_cursor_style: CursorStyle,
        /// Last applied (min, max) content-size limits, so per-frame application
        /// only reaches winit when the effective limits actually change.
        applied_size_limits: Option<(
            Option<waterui_core::layout::Size>,
            Option<waterui_core::layout::Size>,
        )>,
        /// Explicit ProMotion opt-in: declares the 120Hz frame-rate demand to
        /// the window server while redraws are being requested. `None` before
        /// macOS 14.
        #[cfg(target_os = "macos")]
        frame_rate_demand: Option<super::macos_display_link::FrameRateDemandLink>,
    }

    impl WinitWindow {
        pub async fn new(window: Arc<NativeWindow>) -> Self {
            Self::new_with_shared_gpu(window, None).await.0
        }

        pub async fn new_with_shared_gpu(
            window: Arc<NativeWindow>,
            shared_gpu: Option<&WinitGpuContext>,
        ) -> (Self, WinitGpuContext) {
            let (surface, gpu) = WinitSurface::new(window.clone(), shared_gpu).await;
            (
                Self {
                    #[cfg(target_os = "macos")]
                    frame_rate_demand: super::macos_display_link::FrameRateDemandLink::attach(
                        &window,
                    ),
                    window,
                    surface,
                    pending_surface_size: None,
                    pending_events: Vec::new(),
                    pointer_position: (0.0, 0.0),
                    modifiers: Modifiers::default(),
                    applied_text_input_state: None,
                    current_cursor_style: CursorStyle::Arrow,
                    applied_size_limits: None,
                },
                gpu,
            )
        }

        #[must_use]
        pub fn id(&self) -> WindowId {
            self.window.id()
        }

        #[must_use]
        pub fn native_window(&self) -> &NativeWindow {
            self.window.as_ref()
        }

        pub fn handle_window_event(&mut self, event: &WindowEvent) {
            match event {
                WindowEvent::CloseRequested => {
                    self.pending_events.push(InputEvent::CloseRequested);
                }
                WindowEvent::Resized(size) => {
                    self.pending_surface_size = Some(*size);
                    self.pending_events.push(InputEvent::Resize {
                        width: size.width.max(1),
                        height: size.height.max(1),
                    });
                }
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    assert!(
                        scale_factor.is_finite() && *scale_factor > 0.0,
                        "hydrolysis winit backend received invalid scale factor {scale_factor}"
                    );
                    let size = self.window.inner_size();
                    self.pending_surface_size = Some(size);
                    self.pending_events.push(InputEvent::Resize {
                        width: size.width.max(1),
                        height: size.height.max(1),
                    });
                }
                WindowEvent::Moved(position) => {
                    let logical = position.to_logical::<f64>(self.window.scale_factor());
                    self.pending_events.push(InputEvent::Moved {
                        x: logical.x as f32,
                        y: logical.y as f32,
                    });
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.pointer_position =
                        map_cursor_position(position, self.window.scale_factor());
                    tracing::trace!(
                        target: "waterui::hydrolysis::input_raw",
                        event = "cursor_moved",
                        x = self.pointer_position.0,
                        y = self.pointer_position.1,
                        "winit raw input event"
                    );
                    self.pending_events.push(InputEvent::PointerMove {
                        id: 0,
                        kind: PointerKind::Mouse,
                        x: self.pointer_position.0,
                        y: self.pointer_position.1,
                    });
                }
                WindowEvent::CursorLeft { .. } => {
                    self.pending_events.push(InputEvent::PointerCancel {
                        id: 0,
                        kind: PointerKind::Mouse,
                    });
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let mapped_button = map_button(*button);
                    let (x, y) = self.pointer_position;
                    tracing::trace!(
                        target: "waterui::hydrolysis::input_raw",
                        event = "mouse_input",
                        x,
                        y,
                        state = ?state,
                        button = ?mapped_button,
                        "winit raw input event"
                    );
                    match state {
                        ElementState::Pressed => {
                            self.pending_events.push(InputEvent::PointerDown {
                                id: 0,
                                kind: PointerKind::Mouse,
                                x,
                                y,
                                button: mapped_button,
                            });
                        }
                        ElementState::Released => {
                            self.pending_events.push(InputEvent::PointerUp {
                                id: 0,
                                kind: PointerKind::Mouse,
                                x,
                                y,
                                button: mapped_button,
                            });
                        }
                    }
                }
                WindowEvent::Touch(touch) => {
                    let position = map_cursor_position(&touch.location, self.window.scale_factor());
                    self.pointer_position = position;
                    let (x, y) = position;
                    let event = match touch.phase {
                        WinitTouchPhase::Started => InputEvent::PointerDown {
                            id: touch.id,
                            kind: PointerKind::Touch,
                            x,
                            y,
                            button: PointerButton::Primary,
                        },
                        WinitTouchPhase::Moved => InputEvent::PointerMove {
                            id: touch.id,
                            kind: PointerKind::Touch,
                            x,
                            y,
                        },
                        WinitTouchPhase::Ended => InputEvent::PointerUp {
                            id: touch.id,
                            kind: PointerKind::Touch,
                            x,
                            y,
                            button: PointerButton::Primary,
                        },
                        WinitTouchPhase::Cancelled => InputEvent::PointerCancel {
                            id: touch.id,
                            kind: PointerKind::Touch,
                        },
                    };
                    self.pending_events.push(event);
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let (dx, dy, is_line_delta) =
                        map_scroll_delta(delta, self.window.scale_factor());
                    self.pending_events.push(InputEvent::Scroll {
                        x: self.pointer_position.0,
                        y: self.pointer_position.1,
                        dx,
                        dy,
                        is_line_delta,
                    });
                }
                WindowEvent::PinchGesture { delta, phase, .. } => {
                    self.pending_events.push(InputEvent::Magnification {
                        x: self.pointer_position.0,
                        y: self.pointer_position.1,
                        delta: *delta as f32,
                        phase: map_touch_phase(*phase),
                    });
                }
                WindowEvent::RotationGesture { delta, phase, .. } => {
                    self.pending_events.push(InputEvent::Rotation {
                        x: self.pointer_position.0,
                        y: self.pointer_position.1,
                        delta: *delta,
                        phase: map_touch_phase(*phase),
                    });
                }
                WindowEvent::ModifiersChanged(modifiers) => {
                    self.modifiers = modifiers.state().into();
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed
                        && should_emit_keyboard_text(self.modifiers)
                        && let Some(text) = keyboard_text_payload(event)
                    {
                        tracing::trace!(
                            target: "waterui::hydrolysis::input_raw",
                            event = "keyboard_text",
                            text = text.as_str(),
                            "winit raw input event"
                        );
                        self.pending_events.push(InputEvent::TextInput { text });
                    }
                    tracing::trace!(
                        target: "waterui::hydrolysis::input_raw",
                        event = "keyboard_input",
                        state = ?event.state,
                        logical_key = ?event.logical_key,
                        modifiers = ?self.modifiers,
                        "winit raw input event"
                    );
                    self.pending_events.push(InputEvent::Key {
                        key: map_key_event(event, self.modifiers),
                        state: match event.state {
                            ElementState::Pressed => KeyState::Pressed,
                            ElementState::Released => KeyState::Released,
                        },
                        modifiers: self.modifiers,
                    });
                }
                WindowEvent::Ime(ime) => match ime {
                    Ime::Preedit(text, _) => {
                        tracing::trace!(
                            target: "waterui::hydrolysis::input_raw",
                            event = "ime_preedit",
                            text = text.as_str(),
                            "winit raw input event"
                        );
                        self.pending_events
                            .push(InputEvent::ImePreedit { text: text.clone() });
                    }
                    Ime::Commit(text) => {
                        tracing::trace!(
                            target: "waterui::hydrolysis::input_raw",
                            event = "ime_commit",
                            text = text.as_str(),
                            "winit raw input event"
                        );
                        self.pending_events
                            .push(InputEvent::ImeCommit { text: text.clone() });
                    }
                    Ime::Disabled => {
                        tracing::trace!(
                            target: "waterui::hydrolysis::input_raw",
                            event = "ime_disabled",
                            "winit raw input event"
                        );
                        self.pending_events.push(InputEvent::ImeDisabled);
                    }
                    Ime::Enabled => {}
                },
                _ => {}
            }
        }
    }

    fn map_cursor_position(position: &PhysicalPosition<f64>, scale_factor: f64) -> (f32, f32) {
        assert!(
            scale_factor.is_finite() && scale_factor > 0.0,
            "hydrolysis winit backend received invalid scale factor {scale_factor}"
        );
        let logical = position.to_logical::<f64>(scale_factor);
        (logical.x as f32, logical.y as f32)
    }

    fn map_scroll_delta(delta: &MouseScrollDelta, scale_factor: f64) -> (f32, f32, bool) {
        assert!(
            scale_factor.is_finite() && scale_factor > 0.0,
            "hydrolysis winit backend received invalid scale factor {scale_factor}"
        );
        match delta {
            MouseScrollDelta::LineDelta(dx, dy) => (*dx, *dy, true),
            MouseScrollDelta::PixelDelta(delta) => {
                let logical = delta.to_logical::<f64>(scale_factor);
                (logical.x as f32, logical.y as f32, false)
            }
        }
    }

    impl PlatformWindow for WinitWindow {
        fn surface(&mut self) -> &mut dyn SurfaceProvider {
            if let Some(size) = self.pending_surface_size.take() {
                self.surface.resize(size.width, size.height);
            }
            &mut self.surface
        }

        fn set_size_limits(
            &mut self,
            min: Option<waterui_core::layout::Size>,
            max: Option<waterui_core::layout::Size>,
        ) {
            if self.applied_size_limits == Some((min, max)) {
                return;
            }
            self.window.set_min_inner_size(
                min.map(|size| LogicalSize::new(f64::from(size.width), f64::from(size.height))),
            );
            self.window.set_max_inner_size(
                max.map(|size| LogicalSize::new(f64::from(size.width), f64::from(size.height))),
            );
            self.applied_size_limits = Some((min, max));
        }

        fn apply_properties(&mut self, window: &waterui::window::Window) {
            self.window.set_title(window.title.get().as_str());
            self.window.set_resizable(window.resizable);
            self.window.set_decorations(!matches!(
                window.style,
                waterui::window::WindowStyle::Borderless
            ));
            let frame = window.frame.get();
            let target_size = LogicalSize::new(frame.width() as f64, frame.height() as f64);
            let mut target_position = LogicalPosition::new(frame.x() as f64, frame.y() as f64);
            if let Some(monitor) = self.window.current_monitor() {
                let scale_factor = self.window.scale_factor();
                let monitor_position = monitor.position().to_logical::<f64>(scale_factor);
                let monitor_size = monitor.size().to_logical::<f64>(scale_factor);
                let max_x = (monitor_position.x + monitor_size.width - target_size.width)
                    .max(monitor_position.x);
                let max_y = (monitor_position.y + monitor_size.height - target_size.height)
                    .max(monitor_position.y);
                target_position.x = target_position.x.clamp(monitor_position.x, max_x);
                target_position.y = target_position.y.clamp(monitor_position.y, max_y);
            }
            let current_position = self
                .window
                .outer_position()
                .ok()
                .map(|value| value.to_logical::<f64>(self.window.scale_factor()));
            if current_position.is_none_or(|current| {
                (current.x - target_position.x).abs() > 0.5
                    || (current.y - target_position.y).abs() > 0.5
            }) {
                self.window.set_outer_position(target_position);
            }
            let current_size = self
                .window
                .inner_size()
                .to_logical::<f64>(self.window.scale_factor());
            if (current_size.width - target_size.width).abs() > 0.5
                || (current_size.height - target_size.height).abs() > 0.5
            {
                let _ = self.window.request_inner_size(target_size);
            }
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
            // Hold the ProMotion frame-rate demand while frames are being
            // requested, so animations run at 120Hz on high-refresh panels.
            #[cfg(target_os = "macos")]
            if let Some(demand) = &self.frame_rate_demand {
                demand.hold_demand();
            }
        }

        fn scale_factor(&self) -> f64 {
            self.window.scale_factor()
        }

        fn refresh_rate_hz(&self) -> Option<f64> {
            self.window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz())
                .map(|millihertz| f64::from(millihertz) / 1000.0)
        }

        fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
            if self.applied_text_input_state == state {
                return;
            }
            if self.applied_text_input_state.is_some() != state.is_some() {
                self.window.set_ime_allowed(state.is_some());
            }
            self.applied_text_input_state = state;

            let Some(state) = state else {
                return;
            };

            let purpose = match state.purpose {
                TextInputPurpose::Normal => ImePurpose::Normal,
                TextInputPurpose::Password => ImePurpose::Password,
            };
            self.window.set_ime_purpose(purpose);
            let scale_factor = self.window.scale_factor();
            assert!(
                scale_factor.is_finite() && scale_factor > 0.0,
                "hydrolysis winit backend received invalid scale factor {scale_factor}"
            );
            let cursor_origin =
                LogicalPosition::new(state.x, state.y).to_physical::<f64>(scale_factor);
            let cursor_size = LogicalSize::new(state.width.max(1.0), state.height.max(1.0))
                .to_physical::<f64>(scale_factor);
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(
                    cursor_origin.x.round() as i32,
                    cursor_origin.y.round() as i32,
                ),
                PhysicalSize::new(
                    cursor_size.width.ceil() as u32,
                    cursor_size.height.ceil() as u32,
                ),
            );
        }

        fn set_cursor_style(&mut self, style: CursorStyle) {
            if self.current_cursor_style == style {
                return;
            }
            self.current_cursor_style = style;
            self.window
                .set_cursor(WinitCursor::Icon(map_cursor_style(style)));
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

    fn map_touch_phase(phase: WinitTouchPhase) -> TouchPhase {
        match phase {
            WinitTouchPhase::Started => TouchPhase::Started,
            WinitTouchPhase::Moved => TouchPhase::Moved,
            WinitTouchPhase::Ended => TouchPhase::Ended,
            WinitTouchPhase::Cancelled => TouchPhase::Cancelled,
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
            Key::Character(value) => KeyCode::Character(value.to_string()),
            Key::Named(value) => KeyCode::Named(format!("{value:?}")),
            _ => KeyCode::Unidentified,
        }
    }

    fn should_emit_keyboard_text(modifiers: Modifiers) -> bool {
        !(modifiers.control || modifiers.alt || modifiers.super_key)
    }

    fn map_key_event(event: &KeyEvent, modifiers: Modifiers) -> KeyCode {
        if should_emit_keyboard_text(modifiers)
            && keyboard_text_payload(event).is_some()
            && matches!(event.logical_key, Key::Character(_))
        {
            return KeyCode::Unidentified;
        }
        map_key(&event.logical_key)
    }

    fn keyboard_text_payload(event: &KeyEvent) -> Option<String> {
        let text = event.text.as_ref()?;
        if text.is_empty() || text.chars().all(char::is_control) {
            return None;
        }
        Some(text.to_string())
    }

    fn map_cursor_style(style: CursorStyle) -> CursorIcon {
        match style {
            CursorStyle::Arrow => CursorIcon::Default,
            CursorStyle::PointingHand => CursorIcon::Pointer,
            CursorStyle::IBeam => CursorIcon::Text,
            CursorStyle::Crosshair => CursorIcon::Crosshair,
            CursorStyle::OpenHand => CursorIcon::Grab,
            CursorStyle::ClosedHand => CursorIcon::Grabbing,
            CursorStyle::NotAllowed => CursorIcon::NotAllowed,
            CursorStyle::ResizeLeft => CursorIcon::WResize,
            CursorStyle::ResizeRight => CursorIcon::EResize,
            CursorStyle::ResizeUp => CursorIcon::NResize,
            CursorStyle::ResizeDown => CursorIcon::SResize,
            CursorStyle::ResizeLeftRight => CursorIcon::EwResize,
            CursorStyle::ResizeUpDown => CursorIcon::NsResize,
            CursorStyle::Move => CursorIcon::Move,
            CursorStyle::Wait => CursorIcon::Wait,
            CursorStyle::Copy => CursorIcon::Copy,
            _ => panic!("unsupported CursorStyle variant in hydrolysis winit backend"),
        }
    }

    pub use WinitGpuContext as ExportedWinitGpuContext;
    pub use WinitWindow as ExportedWinitWindow;

    #[cfg(test)]
    mod tests {
        use winit::dpi::PhysicalPosition;
        use winit::event::MouseScrollDelta;

        use super::{map_cursor_position, map_scroll_delta, should_emit_keyboard_text};
        use crate::platform::Modifiers;

        #[test]
        fn cursor_position_is_converted_to_logical_coordinates() {
            let (x, y) = map_cursor_position(&PhysicalPosition::new(384.5, 216.25), 2.0);
            assert_eq!(x, 192.25);
            assert_eq!(y, 108.125);
        }

        #[test]
        fn pixel_scroll_delta_is_converted_to_logical_space() {
            let (dx, dy, is_line_delta) = map_scroll_delta(
                &MouseScrollDelta::PixelDelta(PhysicalPosition::new(120.0, -48.5)),
                2.0,
            );
            assert_eq!(dx, 60.0);
            assert_eq!(dy, -24.25);
            assert!(!is_line_delta);
        }

        #[test]
        fn line_scroll_delta_is_preserved() {
            let (dx, dy, is_line_delta) =
                map_scroll_delta(&MouseScrollDelta::LineDelta(-2.0, 3.5), 2.0);
            assert_eq!(dx, -2.0);
            assert_eq!(dy, 3.5);
            assert!(is_line_delta);
        }

        #[test]
        fn command_modified_characters_are_reserved_for_shortcuts() {
            assert!(should_emit_keyboard_text(Modifiers {
                shift: true,
                ..Modifiers::default()
            }));
            assert!(!should_emit_keyboard_text(Modifiers {
                control: true,
                ..Modifiers::default()
            }));
            assert!(!should_emit_keyboard_text(Modifiers {
                super_key: true,
                ..Modifiers::default()
            }));
            assert!(!should_emit_keyboard_text(Modifiers {
                alt: true,
                ..Modifiers::default()
            }));
        }

        #[test]
        fn cursor_position_panics_with_invalid_scale_factor() {
            let result = std::panic::catch_unwind(|| {
                let _ = map_cursor_position(&PhysicalPosition::new(120.0, 80.0), 0.0);
            });
            assert!(result.is_err());
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web_impl::ExportedBrowserWindow as BrowserWindow;

#[cfg(feature = "winit")]
pub(crate) use winit_impl::ExportedWinitGpuContext as WinitGpuContext;

#[cfg(feature = "winit")]
pub use winit_impl::ExportedWinitWindow as WinitWindow;
