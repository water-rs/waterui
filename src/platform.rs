use nami::Signal;
use waterui::cursor::CursorStyle;
use waterui::window::{Window as WuiWindow, WindowState};
use waterui_graphics::RedrawHandle;

#[cfg(any(feature = "winit", all(target_arch = "wasm32", feature = "web")))]
use waterui_graphics::gpu_surface::preferred_surface_format;
use waterui_graphics::shared_context::reclaim_device;

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
///
/// This is the vocabulary `WaterUI`'s own widgets and the embedded browser
/// bridges match on. New code should read [`InputEvent::Key`]'s `logical_key`
/// and `physical_code` instead — the W3C UI Events pair, which is what GPU
/// surfaces receive and what the browser engines will move to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCode {
    Character(String),
    Named(String),
    Unidentified,
}

impl KeyCode {
    /// The W3C UI Events logical key this identifier denotes.
    ///
    /// A producer holding the platform's own key event maps that directly into
    /// [`InputEvent::Key`]'s `logical_key`, which is strictly better. This is
    /// for the synthetic keystrokes a test driver injects, where the
    /// identifier is the only thing there is.
    #[must_use]
    pub fn to_w3c_key(&self) -> keyboard_types::Key {
        let unidentified = keyboard_types::Key::Named(keyboard_types::NamedKey::Unidentified);
        match self {
            Self::Character(value) => keyboard_types::Key::Character(value.clone()),
            // The W3C vocabulary has no named "Space": it is the character the
            // key types.
            Self::Named(value) if value == "Space" => {
                keyboard_types::Key::Character(" ".to_owned())
            }
            Self::Named(value) => value.parse().unwrap_or(unidentified),
            Self::Unidentified => unidentified,
        }
    }
}

/// Active key modifiers snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl From<Modifiers> for keyboard_types::Modifiers {
    fn from(modifiers: Modifiers) -> Self {
        let mut result = Self::empty();
        result.set(Self::SHIFT, modifiers.shift);
        result.set(Self::CONTROL, modifiers.control);
        result.set(Self::ALT, modifiers.alt);
        result.set(Self::META, modifiers.super_key);
        result
    }
}

impl From<keyboard_types::Modifiers> for Modifiers {
    fn from(modifiers: keyboard_types::Modifiers) -> Self {
        Self {
            shift: modifiers.contains(keyboard_types::Modifiers::SHIFT),
            control: modifiers.contains(keyboard_types::Modifiers::CONTROL),
            alt: modifiers.contains(keyboard_types::Modifiers::ALT),
            super_key: modifiers.contains(keyboard_types::Modifiers::META),
        }
    }
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
    TrackpadPan {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        phase: TouchPhase,
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
        /// The logical key in the W3C UI Events vocabulary — what the layout
        /// and modifiers produce. Unlike `key`, this is never suppressed when
        /// the same keystroke also produces text: an embedded engine needs the
        /// real `keydown` alongside the insertion, exactly as the web does.
        logical_key: keyboard_types::Key,
        /// The physical key in the W3C UI Events vocabulary — where it sits on
        /// the keyboard, independent of layout.
        physical_code: keyboard_types::Code,
        /// Whether the platform generated this press by auto-repeat.
        repeat: bool,
        state: KeyState,
        modifiers: Modifiers,
    },
    /// A focus-change replay released a key that was held: winit resends
    /// every held key as a synthetic release when the window loses focus
    /// (on X11, at `XI_FocusOut`). The press it belonged to is aborted, not
    /// completed — the armed keyboard activation and its pressed affordance
    /// come down without firing an action, so a real release arriving later
    /// finds nothing stale left to activate.
    KeyboardCancel,
    ModifiersChanged(Modifiers),
    ImePreedit {
        text: String,
        /// Caret offset within `text`, in bytes, when the platform reports one.
        caret: Option<usize>,
    },
    ImeCommit {
        text: String,
    },
    ImeDisabled,
    Resize {
        width: u32,
        height: u32,
    },
    /// The OS window gained (`true`) or lost (`false`) focus.
    ///
    /// This is the window's own activation, not a focus move inside it:
    /// keyboard focus stays where it was, and the surface holding it is
    /// told focus left and returned so it can report the transition (a
    /// terminal's DECSET 1004 focus tracking, for one).
    Focused(bool),
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
    /// Reports this surface's device lost; taken when the device was opened.
    fn device_loss(&self) -> &waterui_graphics::DeviceLoss;
    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError>;
    fn present(&mut self, frame: SurfaceFrame);
    fn size(&self) -> (u32, u32);
    fn format(&self) -> wgpu::TextureFormat;
    fn resize(&mut self, width: u32, height: u32);
    /// Whether the pixels written into this surface's textures are consumed
    /// as premultiplied-alpha. True only for an OS surface configured
    /// `CompositeAlphaMode::PreMultiplied`; offscreen/readback targets keep
    /// their straight-alpha bytes and stay `false`.
    fn premultiply_alpha(&self) -> bool {
        false
    }
}

/// Asserts the app's `Window::frame` binding carries finite components on
/// all four fields. The binding is a trust boundary — a NaN or infinite
/// frame is a programming error, not something the runner silently repairs
/// deeper in the geometry path.
pub(crate) fn validated_window_frame(
    frame: waterui_core::layout::Rect,
) -> waterui_core::layout::Rect {
    for (field, value) in [
        ("x", frame.x()),
        ("y", frame.y()),
        ("width", frame.width()),
        ("height", frame.height()),
    ] {
        assert!(
            value.is_finite(),
            "hydrolysis runner: Window::frame.{field} must be finite, got {value}"
        );
    }
    frame
}

/// Window abstraction consumed by hydrolysis runner.
pub trait PlatformWindow: 'static {
    fn surface(&mut self) -> &mut dyn SurfaceProvider;
    fn apply_properties(&mut self, window: &WuiWindow);
    /// Applies the window's effective content-size limits (logical units).
    ///
    /// The minimum is the content's layout minimum, overridden by an explicit
    /// `Window::min_size`. The maximum stays `None` — resizable and
    /// maximizable — unless the app pins `Window::max_size`: content never
    /// contributes a maximum, since content that does not stretch on an axis
    /// is laid out inside a larger offer per the layout spec rather than
    /// capping the window. A `+∞` component inside an explicit `Some` max is
    /// the app's per-axis unbounded — treat it as "no bound on that axis".
    /// Targets without per-window runtime size limits
    /// (offscreen surfaces, web canvases, fixed embedded displays) keep this
    /// default no-op.
    fn set_size_limits(
        &mut self,
        min: Option<waterui_core::layout::Size>,
        max: Option<waterui_core::layout::Size>,
    ) {
        let _ = (min, max);
    }
    /// Whether this window acts on content-derived size limits.
    ///
    /// Deriving them costs four extra whole-tree measure passes per frame, so a
    /// surface that cannot resize to fit its content — offscreen capture, an
    /// embedded GPU host, a fixed-size shell — leaves this `false` and never pays
    /// for them. Defaults to `false` alongside the no-op [`Self::set_size_limits`].
    fn applies_size_limits(&self) -> bool {
        false
    }
    fn drain_events(&mut self) -> Vec<InputEvent>;
    fn request_redraw(&self);
    /// Returns a thread-safe wake bridge for nested GPU surfaces.
    ///
    /// Windowed platforms override this when their native window can be woken
    /// from a `RedrawHandle`. Offscreen and single-threaded hosts may keep the
    /// default and rely on their explicit render pump.
    fn gpu_surface_redraw_handle(&self) -> Option<RedrawHandle> {
        None
    }
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

/// The adapter, device and queue an [`OffscreenSurface`] renders on.
///
/// A wgpu device is a heavyweight, driver-allocated resource, and on a machine
/// whose only adapter is a software rasterizer it is heavyweight in *system*
/// memory too. A process that builds one offscreen surface — a snapshot, a
/// preview, a `waterui-testing` host — pays for exactly one and never notices.
/// A process that builds hundreds, because it measures a fresh runtime per
/// sample, pays hundreds of times and exhausts the machine.
///
/// Such a caller creates one context and hands a clone to every surface. The
/// device is shared; everything a measurement is actually about — the view
/// tree, the renderer, the retained scene — is still built fresh per surface.
///
/// This owns the device to the end of the last clone's life, so it drains it on
/// the way out — see `drain_device_before_teardown`. Every headless test builds
/// one of these, and on a runner without a GPU they were the ones dying on drop.
#[derive(Clone, Debug)]
pub struct OffscreenGpuContext {
    /// Shared so the device is drained once, when the last surface using it
    /// goes away. `drain_device_before_teardown` blocks until the device is
    /// idle with no timeout, so running it per clone would make every surface's
    /// drop wait out the work of every *other* surface still on that device.
    inner: std::sync::Arc<OffscreenGpuContextInner>,
}

#[derive(Debug)]
struct OffscreenGpuContextInner {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Reports this device lost; taken when the device was opened.
    device_loss: waterui_graphics::DeviceLoss,
}

impl Drop for OffscreenGpuContextInner {
    fn drop(&mut self) {
        waterui_graphics::shared_context::drain_device_before_teardown(&self.device);
    }
}

impl OffscreenGpuContext {
    /// Lets the device release everything dropped since the last call.
    ///
    /// Non-blocking: it processes the destruction queue rather than waiting for
    /// the device to go idle. Call it once the renderer and surface that used
    /// this device are both gone — a process that builds and drops many of them
    /// in sequence otherwise keeps every one of their allocations outstanding
    /// until the device itself is torn down.
    pub fn reclaim(&self) {
        reclaim_device(&self.inner.device);
    }

    /// Requests a context on the adapter WaterUI would render an application on.
    pub async fn new() -> Self {
        Self::new_with_adapter_selection(AdapterSelection::PRODUCTION).await
    }

    /// Requests a context for WaterUI test hosts.
    ///
    /// Unlike a production context, this allows compute-capable software
    /// adapters so CI can run Hydrolysis accessibility tests on llvmpipe
    /// without opting the runtime path into fallback adapters.
    #[cfg(any(test, feature = "testing"))]
    pub async fn new_for_tests() -> Self {
        Self::new_with_adapter_selection(AdapterSelection::TEST).await
    }

    /// Blocking [`Self::new_for_tests`], for synchronous test harnesses.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests_blocking() -> Self {
        pollster::block_on(Self::new_for_tests())
    }

    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::arc_with_non_send_sync,
            reason = "`OffscreenGpuContextInner` holds a wgpu adapter, device and queue, which the WebGPU backend makes neither `Send` nor `Sync` because they are JS objects. The context is shared by reference count on every target and is `Send + Sync` on all of them but this one, so the storage type is `Arc` everywhere rather than `Rc` here and `Arc` elsewhere."
        )
    )]
    async fn new_with_adapter_selection(selection: AdapterSelection) -> Self {
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
        #[cfg(not(feature = "frame-profile"))]
        let required_features =
            waterui_graphics::shared_context::required_media_features(adapter.features());
        // The frame profiler timestamps GPU work through timestamp queries
        // written between submits, which needs both timestamp features;
        // request them where the adapter has them and report absent where it
        // does not — the feature never fails a device request over this.
        #[cfg(feature = "frame-profile")]
        let required_features =
            waterui_graphics::shared_context::required_media_features(adapter.features())
                | (adapter.features()
                    & (wgpu::Features::TIMESTAMP_QUERY
                        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS));
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
        let device_loss = waterui_graphics::DeviceLoss::observe(&device);

        Self {
            inner: std::sync::Arc::new(OffscreenGpuContextInner {
                adapter,
                device,
                queue,
                device_loss,
            }),
        }
    }
}

/// Headless offscreen rendering surface.
///
/// Dropping one lets the device reclaim the textures it allocated. That is a
/// non-blocking maintain, not the full drain the device gets at teardown: a
/// process that builds surfaces in sequence must not leave every surface's
/// allocations outstanding until the last one goes away — on a software
/// rasterizer that runs the machine out of memory — but neither should each
/// drop wait out the queued work of the other surfaces sharing the device.
pub struct OffscreenSurface {
    gpu: OffscreenGpuContext,
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
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "WebGPU adapter selection cannot enumerate software adapters"
        )
    )]
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

    #[cfg(not(target_arch = "wasm32"))]
    fn allow_software_adapter(self) -> bool {
        self.allow_software_adapter || self.force_fallback_adapter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg(not(target_arch = "wasm32"))]
struct AdapterPreference {
    backend_rank: u8,
    device_type_rank: u8,
}

#[cfg(not(target_arch = "wasm32"))]
impl AdapterPreference {
    /// `prefer_software` is the diagnostics escape hatch: the flag is named
    /// `FORCE_FALLBACK_ADAPTER`, so with it set a software adapter must win
    /// over the real GPU beside it, which is the whole point of reproducing a
    /// software-adapter run on a machine that has a GPU.
    fn for_info(info: &wgpu::AdapterInfo, prefer_software: bool) -> Self {
        Self {
            backend_rank: backend_rank(info.backend),
            device_type_rank: if prefer_software {
                software_first_device_type_rank(info.device_type)
            } else {
                device_type_rank(info.device_type)
            },
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
const fn device_type_rank(device_type: wgpu::DeviceType) -> u8 {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

/// What a user on a host without a usable GPU is told, wherever the renderer
/// refuses to start.
///
/// Hydrolysis is GPU-required by design, so the honest answer is another host
/// or another renderer. The escape hatch is named as diagnostics and nothing
/// more: a software adapter reports compute support and still aborts the
/// process inside shader compilation (Microsoft Basic Render Driver on
/// Windows Server does exactly that), so recommending it as the remedy sends
/// the user somewhere worse than the refusal.
const GPU_REQUIRED_GUIDANCE: &str = "Hydrolysis renders through GPU compute pipelines and has no CPU path: \
run on a machine with a GPU, or on a virtual machine enable hardware 3D acceleration and install the vendor driver. \
For targets without a GPU, waterui-dew is the CPU renderer. \
WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER=1 widens adapter selection to software adapters for diagnostics only; \
a software adapter that cannot run the renderer's pipelines fails or aborts the process instead of drawing.";

#[cfg(all(test, not(target_arch = "wasm32")))]
mod adapter_selection_tests {
    use super::{AdapterPreference, GPU_REQUIRED_GUIDANCE};

    fn info(device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: String::new(),
            vendor: 0,
            device: 0,
            device_type,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: false,
        }
    }

    #[test]
    fn a_gpu_outranks_a_software_adapter_by_default() {
        let gpu = AdapterPreference::for_info(&info(wgpu::DeviceType::DiscreteGpu), false);
        let software = AdapterPreference::for_info(&info(wgpu::DeviceType::Cpu), false);
        assert!(gpu < software);
    }

    #[test]
    fn forcing_the_fallback_adapter_picks_software_over_a_gpu() {
        let gpu = AdapterPreference::for_info(&info(wgpu::DeviceType::DiscreteGpu), true);
        let software = AdapterPreference::for_info(&info(wgpu::DeviceType::Cpu), true);
        assert!(
            software < gpu,
            "the escape hatch is named `force`: it must select the software adapter even when a GPU is present"
        );
    }

    #[test]
    fn the_refusal_does_not_recommend_the_escape_hatch_as_a_remedy() {
        // A software adapter that reports compute support can still abort the
        // process in shader compilation, so the guidance must offer another
        // host or Dew and name the flag as diagnostics only.
        assert!(GPU_REQUIRED_GUIDANCE.contains("waterui-dew"));
        assert!(GPU_REQUIRED_GUIDANCE.contains("diagnostics only"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
const fn software_first_device_type_rank(device_type: wgpu::DeviceType) -> u8 {
    match device_type {
        wgpu::DeviceType::Cpu => 0,
        wgpu::DeviceType::DiscreteGpu => 1,
        wgpu::DeviceType::IntegratedGpu => 2,
        wgpu::DeviceType::VirtualGpu => 3,
        wgpu::DeviceType::Other => 4,
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
        adapter
    }

    #[cfg(not(all(target_arch = "wasm32", feature = "web")))]
    {
        // The diagnostics escape hatch widens which adapters are eligible
        // (`AdapterSelection::allow_software_adapter`); it does not hand the
        // renderer whatever `request_adapter` returns. Asking wgpu for a
        // fallback adapter directly skipped the compute-capability filter and
        // the ranking below, which is how a CPU adapter that cannot run the
        // compute pipelines reached vello's shader init.
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

            let preference = AdapterPreference::for_info(&info, selection.force_fallback_adapter());
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
                "{context}: this host has no GPU Hydrolysis can use. \
Surface-compatible adapters inspected: {}. \
{GPU_REQUIRED_GUIDANCE}",
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

impl Drop for OffscreenSurface {
    fn drop(&mut self) {
        self.last_presented = None;
        self.gpu.reclaim();
    }
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
        Self::on_context(OffscreenGpuContext::new().await, width, height, format)
    }

    /// Creates an offscreen surface for WaterUI test hosts.
    ///
    /// Unlike production surfaces, this constructor allows compute-capable
    /// software adapters so CI can run Hydrolysis accessibility tests on
    /// llvmpipe without opting the runtime path into fallback adapters.
    #[cfg(any(test, feature = "testing"))]
    pub async fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self::on_context(
            OffscreenGpuContext::new_for_tests().await,
            width,
            height,
            format,
        )
    }

    /// Creates a surface on an already-requested [`OffscreenGpuContext`].
    ///
    /// Every surface built on one context shares its device, so a process that
    /// needs many surfaces requests a device once instead of once per surface.
    #[must_use]
    pub fn on_context(
        gpu: OffscreenGpuContext,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            gpu,
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
    panic!(
        "{context}: {no_compute_message}. Selected adapter '{}' ({:?}) reports max_compute_workgroups_per_dimension = {}. \
{GPU_REQUIRED_GUIDANCE}",
        info.name, info.backend, limits.max_compute_workgroups_per_dimension
    );
}

impl SurfaceProvider for OffscreenSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.gpu.inner.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.gpu.inner.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.gpu.inner.queue
    }

    fn device_loss(&self) -> &waterui_graphics::DeviceLoss {
        &self.gpu.inner.device_loss
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        let texture = self.last_presented.take().unwrap_or_else(|| {
            self.gpu
                .inner
                .device
                .create_texture(&wgpu::TextureDescriptor {
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
    /// Requests a device of its own; a caller that builds several windows
    /// should request one [`OffscreenGpuContext`] and use [`Self::on_context`].
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self::on_context(
            OffscreenGpuContext::new_for_tests_blocking(),
            width,
            height,
            format,
        )
    }

    /// Creates a window on an already-requested [`OffscreenGpuContext`], so
    /// every window built on that context shares its device.
    #[must_use]
    pub fn on_context(
        gpu: OffscreenGpuContext,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            surface: OffscreenSurface::on_context(gpu, width, height, format),
            scale_factor: 1.0,
            size_limits: None,
        }
    }

    /// Renders at `scale_factor` physical pixels per logical pixel.
    ///
    /// Layout stays in logical units; only the surface allocation and the
    /// reported [`PlatformWindow::scale_factor`] change, so a 2x offscreen
    /// window produces a HiDPI-sharp image of the very same layout.
    /// Sets the physical-pixels-per-logical-pixel ratio, reallocating the
    /// surface to match. See [`Self::with_scale_factor`].
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        assert!(
            scale_factor.is_finite() && scale_factor > 0.0,
            "offscreen scale factor must be finite and positive, got {scale_factor}"
        );
        let logical_width = f64::from(self.surface.size().0) / self.scale_factor;
        let logical_height = f64::from(self.surface.size().1) / self.scale_factor;
        self.scale_factor = scale_factor;
        self.resize_to_logical(logical_width, logical_height);
    }

    #[must_use]
    pub fn with_scale_factor(mut self, scale_factor: f64) -> Self {
        assert!(
            scale_factor.is_finite() && scale_factor > 0.0,
            "offscreen scale factor must be finite and positive, got {scale_factor}"
        );
        self.set_scale_factor(scale_factor);
        self
    }

    fn resize_to_logical(&mut self, width: f64, height: f64) {
        let physical = |value: f64| (value * self.scale_factor).round().max(1.0) as u32;
        self.surface.resize(physical(width), physical(height));
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
        if window.state.snapshot() == WindowState::Closed {
            return;
        }
        let frame = validated_window_frame(window.frame.snapshot());
        // `frame` is in logical units; the surface is allocated in physical
        // pixels, so the scale factor has to be applied here or a HiDPI window
        // would rasterize at one physical pixel per logical pixel.
        self.resize_to_logical(
            f64::from(frame.width().max(1.0)),
            f64::from(frame.height().max(1.0)),
        );
    }

    fn set_size_limits(
        &mut self,
        min: Option<waterui_core::layout::Size>,
        max: Option<waterui_core::layout::Size>,
    ) {
        self.size_limits = Some((min, max));
    }

    fn applies_size_limits(&self) -> bool {
        true
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
    #[cfg(hydrolysis_macos_system_webview)]
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use nami::Signal;
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2::runtime::NSObjectProtocol;
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    };
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2_app_kit::NSView;
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2_core_graphics::CGPath;
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2_quartz_core::{CAMetalLayer, CAShapeLayer};
    #[cfg(hydrolysis_macos_system_webview)]
    use objc2_web_kit::WKWebView;
    use waterui::window::WindowState;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::{
        dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
        event::{
            ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase as WinitTouchPhase,
            WindowEvent,
        },
        keyboard::{Key, ModifiersState, PhysicalKey},
        window::{
            Cursor as WinitCursor, CursorIcon, Fullscreen, ImePurpose, Window as NativeWindow,
            WindowId,
        },
    };

    use super::{
        CursorStyle, InputEvent, KeyCode, KeyState, Modifiers, PlatformWindow, PointerButton,
        PointerKind, RedrawHandle, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose,
        TextInputState, TouchPhase, reclaim_device, validated_window_frame,
    };

    #[derive(Clone)]
    pub struct WinitGpuContext {
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        /// Reports this device lost; taken when the device was opened.
        device_loss: waterui_graphics::DeviceLoss,
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
        /// The composite alpha mode a surface is configured with.
        ///
        /// A window the compositor must see through needs a mode whose alpha
        /// channel reaches it — premultiplied, postmultiplied, or the
        /// platform-inherited mode compositors honour on alpha-capable
        /// visuals — in that preference order. An opaque window takes
        /// whatever the surface prefers first. An adapter offering a
        /// transparent window none of the transparency-capable modes cannot
        /// present it at all: under an opaque composite the pixels' alpha
        /// never reaches the compositor and the window reads as fully
        /// transparent, which is a window-creation error naming the adapter
        /// and the modes it reported, not a silently opaque window.
        fn select_alpha_mode(
            caps: &wgpu::SurfaceCapabilities,
            requires_transparency: bool,
            adapter_info: &wgpu::AdapterInfo,
        ) -> wgpu::CompositeAlphaMode {
            if !requires_transparency {
                // An opaque window's alpha channel never reaches the
                // compositor, so the surface's preferred mode is fine.
                return caps.alpha_modes[0];
            }
            const TRANSPARENT_MODES: [wgpu::CompositeAlphaMode; 3] = [
                wgpu::CompositeAlphaMode::PreMultiplied,
                wgpu::CompositeAlphaMode::PostMultiplied,
                wgpu::CompositeAlphaMode::Inherit,
            ];
            for wanted in TRANSPARENT_MODES {
                if caps.alpha_modes.contains(&wanted) {
                    return wanted;
                }
            }
            let info = adapter_info;
            panic!(
                "hydrolysis winit surface: a transparent window needs a \
                 transparency-capable composite alpha mode, but adapter {:?} \
                 ({:?}, driver {:?} {:?}) offers only {:?} — presented pixels \
                 would carry no alpha and the window would draw nothing. \
                 Transparent windows require a compositing window manager and \
                 an adapter that reports a non-opaque alpha mode.",
                info.name, info.backend, info.driver, info.driver_info, caps.alpha_modes
            );
        }

        /// The X11 presentation defect a transparent window hits on an old
        /// Mesa software rasterizer: the WSI's `x11_present_to_x11_sw` sent
        /// its `xcb_put_image` at a hardcoded depth of 24, which the X
        /// server rejects with BadMatch for a depth-32 window — and the
        /// driver discards the reply, so `present` reports success while
        /// the window never updates. Fixed by Mesa commit 1e849b12
        /// ("vk/wsi/x11/sw: use swapchain depth for putimage"), released in
        /// Mesa 24.1. Returns the failure message naming the cause when
        /// `adapter_info` is that stack; `None` otherwise.
        ///
        /// The gate is the version alone: only a software rasterizer
        /// (`DeviceType::Cpu`, which is how Vulkan reports llvmpipe and
        /// lavapipe) presents through the software X11 WSI path at all, and
        /// it takes it on every X11 server it drives — with DRI3 it would
        /// take the shared-pixmap path instead, but a hardware Mesa driver
        /// already answers `DeviceType` differently, so a version check on
        /// the software adapter cannot misfire against hardware.
        fn mesa_x11_transparency_blocker(adapter_info: &wgpu::AdapterInfo) -> Option<String> {
            if adapter_info.device_type != wgpu::DeviceType::Cpu {
                return None;
            }
            let (major, minor, patch) = Self::mesa_driver_version(&adapter_info.driver_info)?;
            ((major, minor) < (24, 1)).then(|| {
                format!(
                    "Mesa {major}.{minor}.{patch} software WSI presents \
                     depth-32 X11 windows at depth 24 and the server rejects \
                     every frame (fixed in Mesa 24.1, commit 1e849b12); \
                     upgrade Mesa"
                )
            })
        }

        /// The Mesa version `driver_info` announces, e.g. "Mesa
        /// 23.2.1-1ubuntu2". `None` for a non-Mesa driver or an
        /// unrecognizable string — an unversioned Mesa build is not proven
        /// broken, so it is not blocked.
        fn mesa_driver_version(driver_info: &str) -> Option<(u32, u32, u32)> {
            let version = &driver_info[driver_info.find("Mesa ")? + "Mesa ".len()..];
            let mut parts = version
                .split(|c: char| c != '.' && !c.is_ascii_digit())
                .next()?
                .split('.');
            let major = parts.next()?.parse().ok()?;
            let minor = parts.next().map_or(0, |p| p.parse().unwrap_or(0));
            let patch = parts.next().map_or(0, |p| p.parse().unwrap_or(0));
            Some((major, minor, patch))
        }

        /// Whether the realized winit window lives on an X11 connection —
        /// the only display path the Mesa software-WSI defect can hit. A
        /// window whose handle is not `Xcb`/`Xlib` (Wayland, AppKit,
        /// Windows, an unrecognized or missing handle) is not blocked.
        fn window_is_x11(window: &NativeWindow) -> bool {
            matches!(
                window.window_handle().map(|handle| handle.as_raw()),
                Ok(RawWindowHandle::Xcb(_) | RawWindowHandle::Xlib(_))
            )
        }

        fn from_surface(
            surface: wgpu::Surface<'static>,
            gpu: WinitGpuContext,
            width: u32,
            height: u32,
            requires_transparency: bool,
            on_x11: bool,
        ) -> Self {
            let caps = surface.get_capabilities(&gpu.adapter);
            let format = super::select_hydrolysis_surface_format(&caps);
            let adapter_info = gpu.adapter.get_info();
            if requires_transparency
                && on_x11
                && let Some(cause) = Self::mesa_x11_transparency_blocker(&adapter_info)
            {
                panic!("hydrolysis winit surface: {cause}");
            }
            let alpha_mode = Self::select_alpha_mode(&caps, requires_transparency, &adapter_info);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: width.max(1),
                height: height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&gpu.device, &config);
            Self {
                surface,
                gpu,
                config,
            }
        }

        pub async fn new(
            window: Arc<NativeWindow>,
            shared_gpu: Option<&WinitGpuContext>,
            requires_transparency: bool,
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
                    let device_loss = waterui_graphics::DeviceLoss::observe(&device);
                    (
                        WinitGpuContext {
                            instance,
                            adapter,
                            device,
                            queue,
                            device_loss,
                        },
                        surface,
                    )
                }
            };

            let size = window.inner_size();
            (
                Self::from_surface(
                    surface,
                    gpu.clone(),
                    size.width,
                    size.height,
                    requires_transparency,
                    Self::window_is_x11(&window),
                ),
                gpu,
            )
        }

        #[cfg(hydrolysis_macos_system_webview)]
        fn for_core_animation_layer(
            layer: &CAMetalLayer,
            gpu: &WinitGpuContext,
            width: u32,
            height: u32,
        ) -> Self {
            let target = wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                std::ptr::from_ref(layer).cast_mut().cast(),
            );
            // SAFETY: the layer handed to `create_surface_unsafe` is the window's own
            // `CAMetalLayer`, which the window keeps alive for at least as long as the
            // surface created from it.
            let surface = unsafe {
                gpu.instance
                    .create_surface_unsafe(target)
                    .expect("Hydrolysis failed to create a Metal overlay surface")
            };
            Self::from_surface(surface, gpu.clone(), width, height, true, false)
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

        fn device_loss(&self) -> &waterui_graphics::DeviceLoss {
            &self.gpu.device_loss
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
                SurfaceFrame::Window { output, .. } => {
                    output.present();
                    reclaim_device(&self.gpu.device);
                }
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

        fn premultiply_alpha(&self) -> bool {
            self.config.alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied
        }
    }

    #[cfg(hydrolysis_macos_system_webview)]
    struct MacOverlaySurface {
        layer: Retained<CAMetalLayer>,
        surface: WinitSurface,
    }

    /// Whether an AppKit rect contains a point, in the same coordinate space.
    #[cfg(hydrolysis_macos_system_webview)]
    fn ns_rect_contains(rect: NSRect, point: NSPoint) -> bool {
        point.x >= rect.origin.x
            && point.y >= rect.origin.y
            && point.x < rect.origin.x + rect.size.width
            && point.y < rect.origin.y + rect.size.height
    }

    #[cfg(hydrolysis_macos_system_webview)]
    struct NativeViewContainerIvars {
        /// Where `WaterUI` draws interactive content over the hosted native
        /// view, in this container's *superview* coordinate space — the space
        /// `hitTest:` is given its point in.
        occluded: core::cell::RefCell<Vec<NSRect>>,
    }

    #[cfg(hydrolysis_macos_system_webview)]
    define_class!(
        #[unsafe(super(NSView))]
        #[name = "WuiHydrolysisNativeViewContainer"]
        #[thread_kind = MainThreadOnly]
        #[ivars = NativeViewContainerIvars]
        struct NativeViewContainer;

        unsafe impl NSObjectProtocol for NativeViewContainer {}

        impl NativeViewContainer {
            /// Refuses hits where `WaterUI` painted interactive content on top.
            ///
            /// Raising the overlay's `zPosition` fixed only what the user sees:
            /// a `CALayer` is not in AppKit's hit-test chain, so a snackbar,
            /// dialog or menu drawn over a `WKWebView` rendered above it and
            /// still handed every click to the page underneath. Returning `nil`
            /// lets the event fall through to the winit content view, where
            /// Hydrolysis's own hit test finds the target that is visibly on
            /// top.
            ///
            /// The view is returned unowned, as `hitTest:` is defined to: the
            /// pointer travels straight through from the superclass, so it is a
            /// raw pointer rather than a `Retained` here.
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, point: NSPoint) -> *mut NSView {
                if self
                    .ivars()
                    .occluded
                    .borrow()
                    .iter()
                    .any(|rect| ns_rect_contains(*rect, point))
                {
                    return core::ptr::null_mut();
                }
                // SAFETY: main-thread call to `NSView`'s own implementation,
                // which is what this override defers to for every other point.
                unsafe { msg_send![super(self), hitTest: point] }
            }
        }
    );

    #[cfg(hydrolysis_macos_system_webview)]
    impl NativeViewContainer {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NativeViewContainerIvars {
                occluded: core::cell::RefCell::new(Vec::new()),
            });
            // SAFETY: `initWithFrame:` is `NSView`'s designated initializer, and
            // `-> Retained<Self>` is the signature objc2 expects here.
            unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
        }

        fn set_occluded(&self, rects: Vec<NSRect>) {
            self.ivars().occluded.replace(rects);
        }
    }

    #[cfg(hydrolysis_macos_system_webview)]
    struct MacNativeViewHost {
        web_view: Retained<WKWebView>,
        container: Retained<NativeViewContainer>,
        rounded_clip_views: Vec<Retained<NSView>>,
    }

    #[cfg(hydrolysis_macos_system_webview)]
    impl MacNativeViewHost {
        fn new(web_view: Retained<WKWebView>, root_view: &NSView) -> Self {
            let mtm = MainThreadMarker::new()
                .expect("Hydrolysis hybrid composition must run on the AppKit main thread");
            let container = NativeViewContainer::new(mtm);
            container.setWantsLayer(true);
            container
                .layer()
                .expect("Hydrolysis native WebView container must have a Core Animation layer")
                .setMasksToBounds(true);
            container.addSubview(&web_view);
            root_view.addSubview(&container);
            Self {
                web_view,
                container,
                rounded_clip_views: Vec::new(),
            }
        }

        fn set_rounded_clip_count(&mut self, count: usize) {
            if self.rounded_clip_views.len() == count {
                return;
            }
            self.web_view.removeFromSuperview();
            for clip_view in self.rounded_clip_views.drain(..) {
                clip_view.removeFromSuperview();
            }

            let mtm = MainThreadMarker::new()
                .expect("Hydrolysis hybrid composition must run on the AppKit main thread");
            for _ in 0..count {
                let clip_view = NSView::new(mtm);
                clip_view.setWantsLayer(true);
                clip_view
                    .layer()
                    .expect("Hydrolysis rounded clip view must have a Core Animation layer")
                    .setMasksToBounds(true);
                self.rounded_clip_views.push(clip_view);
            }

            let mut parent: &NSView = &self.container;
            for clip_view in &self.rounded_clip_views {
                parent.addSubview(clip_view);
                parent = clip_view;
            }
            parent.addSubview(&self.web_view);
        }
    }

    #[cfg(hydrolysis_macos_system_webview)]
    #[derive(Clone, Copy)]
    struct MacRoundedClip {
        rect: vello::kurbo::Rect,
        corner_width: f64,
        corner_height: f64,
    }

    #[cfg(hydrolysis_macos_system_webview)]
    fn assert_axis_aligned_positive(transform: vello::kurbo::Affine, operation: &str) -> [f64; 6] {
        let coefficients = transform.as_coeffs();
        let epsilon = f64::EPSILON * 64.0;
        assert!(
            coefficients[1].abs() <= epsilon && coefficients[2].abs() <= epsilon,
            "Hydrolysis native WebView {operation} requires an axis-aligned transform"
        );
        assert!(
            coefficients[0].is_finite()
                && coefficients[3].is_finite()
                && coefficients[0] > 0.0
                && coefficients[3] > 0.0,
            "Hydrolysis native WebView {operation} requires positive finite axis scales"
        );
        coefficients
    }

    #[cfg(hydrolysis_macos_system_webview)]
    fn appkit_root_rect(
        physical_rect: vello::kurbo::Rect,
        logical_height: f64,
        scale_factor: f64,
        flipped: bool,
    ) -> NSRect {
        let x = physical_rect.x0 / scale_factor;
        let y_from_top = physical_rect.y0 / scale_factor;
        let width = physical_rect.width() / scale_factor;
        let height = physical_rect.height() / scale_factor;
        let y = if flipped {
            y_from_top
        } else {
            logical_height - y_from_top - height
        };
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    #[cfg(hydrolysis_macos_system_webview)]
    struct MacHybridCompositor {
        gpu: WinitGpuContext,
        native_views: HashMap<usize, MacNativeViewHost>,
        overlays: Vec<MacOverlaySurface>,
    }

    #[cfg(hydrolysis_macos_system_webview)]
    impl core::fmt::Debug for MacHybridCompositor {
        fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter
                .debug_struct("MacHybridCompositor")
                .field("native_view_count", &self.native_views.len())
                .field("overlay_count", &self.overlays.len())
                .finish_non_exhaustive()
        }
    }

    #[cfg(hydrolysis_macos_system_webview)]
    impl MacHybridCompositor {
        fn new(gpu: WinitGpuContext) -> Self {
            Self {
                gpu,
                native_views: HashMap::new(),
                overlays: Vec::new(),
            }
        }

        fn root_view(window: &NativeWindow) -> &NSView {
            let handle = window
                .window_handle()
                .expect("Hydrolysis macOS window must expose an AppKit handle");
            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                panic!("Hydrolysis macOS window returned a non-AppKit handle");
            };
            // SAFETY: winit hands out the window's live `NSView` pointer, and the
            // borrow does not outlive the window handle it came from.
            unsafe { appkit.ns_view.cast::<NSView>().as_ref() }
        }

        fn sync(
            &mut self,
            window: &NativeWindow,
            native_views: &[crate::renderer::NativeViewLayer],
            physical_width: u32,
            physical_height: u32,
            scale_factor: f64,
        ) {
            assert!(
                scale_factor.is_finite() && scale_factor > 0.0,
                "Hydrolysis hybrid composition received invalid scale factor {scale_factor}"
            );
            let root_view = Self::root_view(window);
            root_view.setWantsLayer(true);
            let root_layer = root_view
                .layer()
                .expect("Hydrolysis macOS root view must have a Core Animation layer");
            let logical_height = f64::from(physical_height) / scale_factor;
            let mut active = HashSet::new();

            for (index, placement) in native_views.iter().enumerate() {
                let id = Retained::as_ptr(&placement.view) as usize;
                active.insert(id);
                let coefficients = placement.transform.as_coeffs();
                let epsilon = f64::EPSILON * 64.0;
                assert!(
                    coefficients[1].abs() <= epsilon && coefficients[2].abs() <= epsilon,
                    "Hydrolysis native WebView currently requires an axis-aligned transform"
                );
                assert!(
                    coefficients[0].is_finite()
                        && coefficients[3].is_finite()
                        && coefficients[0] > 0.0
                        && coefficients[3] > 0.0,
                    "Hydrolysis native WebView requires positive finite axis scales"
                );
                let transformed = placement.transform.transform_rect_bbox(placement.bounds);
                let mut opacity = 1.0f32;
                let mut visible = transformed;
                let mut rounded_clips = Vec::new();
                for active_layer in &placement.active_layers {
                    assert!(
                        active_layer.alpha.is_finite() && (0.0..=1.0).contains(&active_layer.alpha),
                        "Hydrolysis native WebView received invalid layer opacity {}",
                        active_layer.alpha
                    );
                    opacity *= active_layer.alpha;
                    match &active_layer.shape {
                        crate::renderer::LayerShape::Rect(rect) => {
                            assert_axis_aligned_positive(
                                active_layer.transform,
                                "rectangular clipping",
                            );
                            let clip = active_layer.transform.transform_rect_bbox(*rect);
                            visible = visible.intersect(clip);
                        }
                        crate::renderer::LayerShape::RoundedRect {
                            rect,
                            corner_width,
                            corner_height,
                            ..
                        } => {
                            let clip_transform = active_layer.transform;
                            let clip_coefficients =
                                assert_axis_aligned_positive(clip_transform, "rounded clipping");
                            let clip = clip_transform.transform_rect_bbox(*rect);
                            visible = visible.intersect(clip);
                            rounded_clips.push(MacRoundedClip {
                                rect: clip,
                                corner_width: corner_width * clip_coefficients[0],
                                corner_height: corner_height * clip_coefficients[3],
                            });
                        }
                        crate::renderer::LayerShape::Path(_) => {
                            panic!(
                                "Hydrolysis native WebView does not support non-rectangular path masks"
                            )
                        }
                    }
                }
                let host = self
                    .native_views
                    .entry(id)
                    .or_insert_with(|| MacNativeViewHost::new(placement.view.clone(), root_view));
                host.set_rounded_clip_count(rounded_clips.len());

                let container_frame =
                    appkit_root_rect(visible, logical_height, scale_factor, root_view.isFlipped());
                let web_view_frame = appkit_root_rect(
                    transformed,
                    logical_height,
                    scale_factor,
                    root_view.isFlipped(),
                );
                host.container.setFrame(container_frame);
                host.container
                    .setHidden(visible.is_zero_area() || opacity == 0.0);
                // The renderer republishes these every frame in window hit-test
                // space, which is logical points measured from the top-left, so
                // they convert with a scale factor of 1. `hitTest:` is given its
                // point in the root view's space, which is what this produces.
                host.container.set_occluded(
                    placement
                        .occlusion
                        .borrow()
                        .iter()
                        .map(|rect| {
                            appkit_root_rect(*rect, logical_height, 1.0, root_view.isFlipped())
                        })
                        .collect(),
                );
                let local_bounds = NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(container_frame.size.width, container_frame.size.height),
                );
                let web_view_local_frame = NSRect::new(
                    NSPoint::new(
                        web_view_frame.origin.x - container_frame.origin.x,
                        web_view_frame.origin.y - container_frame.origin.y,
                    ),
                    web_view_frame.size,
                );
                host.web_view.setFrame(web_view_local_frame);
                host.web_view.setWantsLayer(true);

                for (clip_view, rounded_clip) in host.rounded_clip_views.iter().zip(&rounded_clips)
                {
                    clip_view.setFrame(local_bounds);
                    let clip_layer = clip_view
                        .layer()
                        .expect("Hydrolysis rounded clip view must have a Core Animation layer");
                    let clip_root_frame = appkit_root_rect(
                        rounded_clip.rect,
                        logical_height,
                        scale_factor,
                        root_view.isFlipped(),
                    );
                    let clip_local_rect = NSRect::new(
                        NSPoint::new(
                            clip_root_frame.origin.x - container_frame.origin.x,
                            clip_root_frame.origin.y - container_frame.origin.y,
                        ),
                        clip_root_frame.size,
                    );
                    let mask = CAShapeLayer::layer();
                    mask.setFrame(local_bounds);
                    // SAFETY: main-thread Core Graphics call with a by-value rect and
                    // radii; the returned path is owned by this scope.
                    let path = unsafe {
                        CGPath::with_rounded_rect(
                            clip_local_rect,
                            rounded_clip.corner_width / scale_factor,
                            rounded_clip.corner_height / scale_factor,
                            core::ptr::null(),
                        )
                    };
                    mask.setPath(Some(&path));
                    // SAFETY: main-thread message send to layers this window owns;
                    // `mask` is retained by the layer for as long as it is set.
                    unsafe {
                        clip_layer.setMask(Some(&mask));
                    }
                }

                let container_layer = host
                    .container
                    .layer()
                    .expect("Hydrolysis native WebView container must have a Core Animation layer");
                container_layer.setOpacity(opacity);
                container_layer.setZPosition((index * 2 + 1) as f64);
            }

            self.native_views.retain(|id, host| {
                if active.contains(id) {
                    true
                } else {
                    host.container.removeFromSuperview();
                    false
                }
            });

            while self.overlays.len() < native_views.len() {
                let layer = CAMetalLayer::layer();
                layer.setOpaque(false);
                layer.setFramebufferOnly(false);
                root_layer.addSublayer(&layer);
                let surface = WinitSurface::for_core_animation_layer(
                    &layer,
                    &self.gpu,
                    physical_width,
                    physical_height,
                );
                self.overlays.push(MacOverlaySurface { layer, surface });
            }
            while self.overlays.len() > native_views.len() {
                let overlay = self
                    .overlays
                    .pop()
                    .expect("Hydrolysis overlay count changed during removal");
                overlay.layer.removeFromSuperlayer();
            }

            let logical_width = f64::from(physical_width) / scale_factor;
            for (index, overlay) in self.overlays.iter_mut().enumerate() {
                overlay.layer.setFrame(NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(logical_width, logical_height),
                ));
                overlay.layer.setContentsScale(scale_factor);
                overlay.layer.setDrawableSize(NSSize::new(
                    f64::from(physical_width),
                    f64::from(physical_height),
                ));
                overlay.layer.setZPosition((index * 2 + 2) as f64);
                overlay.surface.resize(physical_width, physical_height);
            }
        }

        fn clear(&mut self) {
            for (_, host) in self.native_views.drain() {
                host.container.removeFromSuperview();
            }
            for overlay in self.overlays.drain(..) {
                overlay.layer.removeFromSuperlayer();
            }
        }

        fn overlay_surface(&mut self, index: usize) -> &mut WinitSurface {
            &mut self
                .overlays
                .get_mut(index)
                .unwrap_or_else(|| {
                    panic!("Hydrolysis requested missing hybrid overlay surface {index}")
                })
                .surface
        }
    }

    /// The platform IME calls behind [`PlatformWindow::sync_text_input_state`],
    /// computed without a window: tracks the last state applied so a repeat
    /// sync is a no-op, `set_ime_allowed` fires only across a focus
    /// transition, and the cursor area is reported in physical pixels.
    #[derive(Debug, Default)]
    pub(crate) struct TextInputSync {
        applied: Option<TextInputState>,
    }

    /// One winit IME call [`TextInputSync::sync`] asks the window to make.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(crate) enum TextInputSyncOp {
        Allowed(bool),
        Purpose(TextInputPurpose),
        CursorArea {
            x: i32,
            y: i32,
            width: u32,
            height: u32,
        },
    }

    impl TextInputSync {
        /// The winit calls needed to bring the platform IME to `state`, in
        /// order. An unchanged state returns nothing — including when only
        /// the caret moved: a moved caret changes the state, so it is never
        /// swallowed by the equality early-return.
        pub(crate) fn sync(
            &mut self,
            state: Option<TextInputState>,
            scale_factor: f64,
        ) -> Vec<TextInputSyncOp> {
            if self.applied == state {
                return Vec::new();
            }
            let mut ops = Vec::with_capacity(3);
            if self.applied.is_some() != state.is_some() {
                ops.push(TextInputSyncOp::Allowed(state.is_some()));
            }
            self.applied = state;
            let Some(state) = state else {
                return ops;
            };
            ops.push(TextInputSyncOp::Purpose(state.purpose));
            assert!(
                scale_factor.is_finite() && scale_factor > 0.0,
                "hydrolysis winit backend received invalid scale factor {scale_factor}"
            );
            ops.push(TextInputSyncOp::CursorArea {
                x: (state.x * scale_factor).round() as i32,
                y: (state.y * scale_factor).round() as i32,
                width: (state.width.max(1.0) * scale_factor).ceil() as u32,
                height: (state.height.max(1.0) * scale_factor).ceil() as u32,
            });
            ops
        }
    }

    /// The window setup the app requested, held until the window has
    /// actually mapped.
    ///
    /// `apply_properties` pushes the requested frame and state while the
    /// window is still unmapped (`with_visible(false)`); on X11 a request
    /// made of an unmapped window is dropped and the window manager's own
    /// initial state wins the race — its placement for geometry, a normal
    /// window for `Fullscreen`/`Minimized`/`Closed`. The request is held
    /// for one re-delivery on the first event that only reaches a mapped
    /// window. Events that precede the map leave it armed.
    #[derive(Debug, Default)]
    struct MappedRequestRetry {
        pending: Option<PendingMappedRequest>,
        mapped: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct PendingMappedRequest {
        position: LogicalPosition<f64>,
        size: LogicalSize<f64>,
        state: WindowState,
    }

    impl MappedRequestRetry {
        /// Records the requested frame and state while the window is not
        /// yet known to be visible, overwriting any earlier pending one —
        /// the latest request wins. Once a mapped event has been seen this
        /// never re-arms: `is_visible` can lag the real map transition, and
        /// re-arming off it would re-deliver a request the live window has
        /// already overtaken.
        fn arm(
            &mut self,
            visible: Option<bool>,
            position: LogicalPosition<f64>,
            size: LogicalSize<f64>,
            state: WindowState,
        ) {
            if visible != Some(true) && !self.mapped {
                self.pending = Some(PendingMappedRequest {
                    position,
                    size,
                    state,
                });
            }
        }

        /// Consumes the held request on the first mapped-signal event —
        /// `Moved`, `Resized`, `Occluded(false)`, or `ScaleFactorChanged`,
        /// each of which only reaches a mapped window — returning it for
        /// re-application. Other events leave it armed.
        fn take_on_mapped_event(&mut self, event: &WindowEvent) -> Option<PendingMappedRequest> {
            let mapped = matches!(
                event,
                WindowEvent::Moved(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
                    | WindowEvent::Occluded(false)
            );
            if mapped {
                self.mapped = true;
                self.pending.take()
            } else {
                None
            }
        }
    }

    /// Snapshot of the window properties `apply_properties` last pushed to the
    /// native window, so unchanged syncs cost no platform calls.
    #[derive(Clone, Debug, PartialEq)]
    struct AppliedWindowProperties {
        title: waterui::Str,
        resizable: bool,
        decorations: bool,
        state: WindowState,
        frame: waterui_core::layout::Rect,
    }

    #[derive(Debug)]
    pub struct WinitWindow {
        window: Arc<NativeWindow>,
        surface: WinitSurface,
        pending_surface_size: Option<PhysicalSize<u32>>,
        pending_events: Vec<InputEvent>,
        pointer_position: (f32, f32),
        modifiers: Modifiers,
        text_input_sync: TextInputSync,
        current_cursor_style: CursorStyle,
        /// Last applied (min, max) content-size limits, so per-frame application
        /// only reaches winit when the effective limits actually change.
        applied_size_limits: Option<(
            Option<waterui_core::layout::Size>,
            Option<waterui_core::layout::Size>,
        )>,
        /// Last applied window properties. `apply_properties` runs every event
        /// cycle, and each unconditional winit setter emits X writes whose
        /// replies wake the loop again — dedupe keeps the loop idle when
        /// nothing changed. The frame binding in particular is enforced only
        /// when it changed since the previous pump: a user-driven resize or
        /// move lands in the window server before its `Resized`/`Moved` event
        /// updates the binding, and reading the live geometry against the
        /// stale binding would yank the window straight back to its old frame.
        applied_properties: Option<AppliedWindowProperties>,
        /// The requested window frame and state held until the window has
        /// actually mapped (`with_visible(false)`): on X11 a request made
        /// of an unmapped window is dropped, so the first mapped-signal
        /// event re-delivers it — the window manager's own initial state
        /// otherwise wins.
        pending_mapped_request: MappedRequestRetry,
        /// Explicit ProMotion opt-in: declares the 120Hz frame-rate demand to
        /// the window server while redraws are being requested. `None` before
        /// macOS 14.
        #[cfg(target_os = "macos")]
        frame_rate_demand: Option<super::macos_display_link::FrameRateDemandLink>,
        #[cfg(hydrolysis_macos_system_webview)]
        hybrid_compositor: MacHybridCompositor,
    }

    impl WinitWindow {
        pub async fn new(window: Arc<NativeWindow>, requires_transparency: bool) -> Self {
            Self::new_with_shared_gpu(window, None, requires_transparency)
                .await
                .0
        }

        pub async fn new_with_shared_gpu(
            window: Arc<NativeWindow>,
            shared_gpu: Option<&WinitGpuContext>,
            requires_transparency: bool,
        ) -> (Self, WinitGpuContext) {
            let (surface, gpu) =
                WinitSurface::new(window.clone(), shared_gpu, requires_transparency).await;
            (
                Self {
                    #[cfg(target_os = "macos")]
                    frame_rate_demand: super::macos_display_link::FrameRateDemandLink::attach(
                        &window,
                    ),
                    #[cfg(hydrolysis_macos_system_webview)]
                    hybrid_compositor: MacHybridCompositor::new(gpu.clone()),
                    window,
                    surface,
                    pending_surface_size: None,
                    pending_events: Vec::new(),
                    pointer_position: (0.0, 0.0),
                    modifiers: Modifiers::default(),
                    text_input_sync: TextInputSync::default(),
                    current_cursor_style: CursorStyle::Arrow,
                    applied_size_limits: None,
                    applied_properties: None,
                    pending_mapped_request: MappedRequestRetry::default(),
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

        #[cfg(hydrolysis_macos_system_webview)]
        pub(crate) fn sync_hybrid_composition(
            &mut self,
            native_views: &[crate::renderer::NativeViewLayer],
            physical_width: u32,
            physical_height: u32,
        ) {
            self.hybrid_compositor.sync(
                &self.window,
                native_views,
                physical_width,
                physical_height,
                self.window.scale_factor(),
            );
        }

        #[cfg(hydrolysis_macos_system_webview)]
        pub(crate) fn clear_hybrid_composition(&mut self) {
            self.hybrid_compositor.clear();
        }

        #[cfg(hydrolysis_macos_system_webview)]
        pub(crate) fn hybrid_overlay_surface(&mut self, index: usize) -> &mut dyn SurfaceProvider {
            self.hybrid_compositor.overlay_surface(index)
        }

        /// Pushes the requested `WindowState` to the window server. Shared
        /// by `apply_properties` and the first-mapped-event re-delivery: on
        /// X11 the same call made of an unmapped window is dropped.
        fn apply_window_state(&self, state: WindowState) {
            match state {
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

        pub fn handle_window_event(&mut self, event: &WindowEvent) {
            // The first mapped-signal event re-applies the frame and state
            // the app asked for: requests made of an unmapped window were
            // dropped by X11, and the window manager's own state won the
            // race.
            if let Some(request) = self.pending_mapped_request.take_on_mapped_event(event) {
                self.window.set_outer_position(request.position);
                let _ = self.window.request_inner_size(request.size);
                self.apply_window_state(request.state);
            }
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
                WindowEvent::Focused(focused) => {
                    self.pending_events.push(InputEvent::Focused(*focused));
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
                WindowEvent::MouseWheel { delta, phase, .. } => {
                    let (dx, dy, is_line_delta) =
                        map_scroll_delta(delta, self.window.scale_factor());
                    if is_line_delta {
                        self.pending_events.push(InputEvent::Scroll {
                            x: self.pointer_position.0,
                            y: self.pointer_position.1,
                            dx,
                            dy,
                            is_line_delta,
                        });
                    } else {
                        self.pending_events.push(InputEvent::TrackpadPan {
                            x: self.pointer_position.0,
                            y: self.pointer_position.1,
                            dx,
                            dy,
                            phase: map_touch_phase(*phase),
                        });
                    }
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
                    self.pending_events
                        .push(InputEvent::ModifiersChanged(self.modifiers));
                }
                WindowEvent::KeyboardInput {
                    event,
                    is_synthetic,
                    ..
                } => queue_keyboard_input(
                    &mut self.pending_events,
                    self.modifiers,
                    WinitKeyInput {
                        is_synthetic: *is_synthetic,
                        state: event.state,
                        repeat: event.repeat,
                        text: event.text.as_deref(),
                        logical_key: &event.logical_key,
                        physical_key: event.physical_key,
                    },
                ),
                WindowEvent::Ime(ime) => match ime {
                    Ime::Preedit(text, caret) => {
                        tracing::trace!(
                            target: "waterui::hydrolysis::input_raw",
                            event = "ime_preedit",
                            text = text.as_str(),
                            "winit raw input event"
                        );
                        self.pending_events.push(InputEvent::ImePreedit {
                            text: text.clone(),
                            // winit reports the pre-edit selection as a byte
                            // range; the caret sits at its start.
                            caret: caret.map(|(start, _)| start),
                        });
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

        fn applies_size_limits(&self) -> bool {
            true
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
            let title = window.display_title().snapshot();
            let decorations = !matches!(window.style, waterui::window::WindowStyle::Borderless);
            let state = window.state.snapshot();
            let frame = validated_window_frame(window.frame.snapshot());
            let properties = AppliedWindowProperties {
                title: title.clone(),
                resizable: window.resizable,
                decorations,
                state,
                frame,
            };
            let previous = self.applied_properties.replace(properties.clone());
            let applied = previous.as_ref();
            if applied.is_none_or(|p| p.title != properties.title) {
                self.window.set_title(properties.title.as_str());
            }
            if applied.is_none_or(|p| p.resizable != properties.resizable) {
                self.window.set_resizable(properties.resizable);
            }
            if applied.is_none_or(|p| p.decorations != properties.decorations) {
                self.window.set_decorations(properties.decorations);
            }
            // The frame binding is pushed to the window only when it changed
            // since the previous pump. A user-driven resize or move lands in
            // the window server before its `Resized`/`Moved` event reaches the
            // binding, so the live geometry legitimately disagrees with the
            // stale binding in that window — enforcing it then would yank the
            // window straight back and make user resizes impossible.
            // Arm the post-map re-apply while the window is still hidden:
            // the frame and state pushed above reach an unmapped X11 window
            // and are dropped, so the first mapped event must re-deliver
            // them — a window manager that stretched the window on map is
            // put back on the requested frame, and a Fullscreen/Minimized/
            // Closed request is restored after the window exists. Once the
            // window is visible the app-requested setup was already
            // enforced and further changes just flow through the ordinary
            // path.
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
            self.pending_mapped_request.arm(
                self.window.is_visible(),
                target_position,
                target_size,
                state,
            );
            let size_changed = applied.is_none_or(|p| *p.frame.size() != *frame.size());
            let origin_changed = applied.is_none_or(|p| p.frame.origin() != frame.origin());
            if size_changed || origin_changed {
                let current_position = self
                    .window
                    .outer_position()
                    .ok()
                    .map(|value| value.to_logical::<f64>(self.window.scale_factor()));
                if origin_changed
                    && current_position.is_none_or(|current| {
                        (current.x - target_position.x).abs() > 0.5
                            || (current.y - target_position.y).abs() > 0.5
                    })
                {
                    self.window.set_outer_position(target_position);
                }
                let current_size = self
                    .window
                    .inner_size()
                    .to_logical::<f64>(self.window.scale_factor());
                if size_changed
                    && ((current_size.width - target_size.width).abs() > 0.5
                        || (current_size.height - target_size.height).abs() > 0.5)
                {
                    let _ = self.window.request_inner_size(target_size);
                }
            }
            if applied.is_none_or(|p| p.state != state) {
                self.apply_window_state(state);
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

        fn gpu_surface_redraw_handle(&self) -> Option<RedrawHandle> {
            let handle = RedrawHandle::new();
            let window = Arc::clone(&self.window);
            handle.set_waker(Some(Arc::new(move || window.request_redraw())));
            Some(handle)
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
            let scale_factor = self.window.scale_factor();
            for op in self.text_input_sync.sync(state, scale_factor) {
                match op {
                    TextInputSyncOp::Allowed(allowed) => {
                        self.window.set_ime_allowed(allowed);
                    }
                    TextInputSyncOp::Purpose(purpose) => {
                        let purpose = match purpose {
                            TextInputPurpose::Normal => ImePurpose::Normal,
                            TextInputPurpose::Password => ImePurpose::Password,
                        };
                        self.window.set_ime_purpose(purpose);
                    }
                    TextInputSyncOp::CursorArea {
                        x,
                        y,
                        width,
                        height,
                    } => {
                        self.window.set_ime_cursor_area(
                            PhysicalPosition::new(x, y),
                            PhysicalSize::new(width, height),
                        );
                    }
                }
            }
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

    /// The fields of `WindowEvent::KeyboardInput` hydrolysis reads, decomposed
    /// at the match site: `winit::event::KeyEvent` cannot be constructed
    /// outside winit (its `platform_specific` field is private), and this
    /// translation is what the unit tests drive.
    #[derive(Clone, Copy)]
    struct WinitKeyInput<'a> {
        /// winit's focus-change replay (on X11, `XI_FocusIn` resends every
        /// held key as a synthetic press and `XI_FocusOut` as a synthetic
        /// release): state synchronisation, not a keystroke the user made.
        is_synthetic: bool,
        state: ElementState,
        repeat: bool,
        text: Option<&'a str>,
        logical_key: &'a Key,
        physical_key: PhysicalKey,
    }

    fn queue_keyboard_input(
        pending_events: &mut Vec<InputEvent>,
        modifiers: Modifiers,
        input: WinitKeyInput<'_>,
    ) {
        if input.is_synthetic {
            // A replayed press carries no user input: no text, key action or
            // gesture may observe it. The modifier side of the same sync
            // still arrives through `ModifiersChanged`.
            if input.state == ElementState::Released {
                // The focus-out replay of a held key's release aborts the
                // press it belonged to: nothing downstream may activate on
                // it, but the armed press state must come down so a real
                // release later cannot fire a stale target.
                pending_events.push(InputEvent::KeyboardCancel);
            }
            return;
        }
        if input.state == ElementState::Pressed
            && should_emit_keyboard_text(modifiers)
            && let Some(text) = keyboard_text_payload(input.text)
        {
            tracing::trace!(
                target: "waterui::hydrolysis::input_raw",
                event = "keyboard_text",
                text,
                "winit raw input event"
            );
            pending_events.push(InputEvent::TextInput {
                text: text.to_string(),
            });
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input_raw",
            event = "keyboard_input",
            state = ?input.state,
            logical_key = ?input.logical_key,
            modifiers = ?modifiers,
            "winit raw input event"
        );
        pending_events.push(InputEvent::Key {
            key: map_key_event(input.logical_key, input.text, modifiers),
            logical_key: ui_events_winit::keyboard::from_winit_key(input.logical_key.clone()),
            physical_code: ui_events_winit::keyboard::from_winit_code(input.physical_key),
            repeat: input.repeat,
            state: match input.state {
                ElementState::Pressed => KeyState::Pressed,
                ElementState::Released => KeyState::Released,
            },
            modifiers,
        });
    }

    fn map_key_event(logical_key: &Key, text: Option<&str>, modifiers: Modifiers) -> KeyCode {
        if should_emit_keyboard_text(modifiers)
            && keyboard_text_payload(text).is_some()
            && matches!(logical_key, Key::Character(_))
        {
            return KeyCode::Unidentified;
        }
        map_key(logical_key)
    }

    fn keyboard_text_payload(text: Option<&str>) -> Option<&str> {
        let text = text?;
        if text.is_empty() || text.chars().all(char::is_control) {
            return None;
        }
        Some(text)
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
        use winit::event::{ElementState, MouseScrollDelta};
        use winit::keyboard::{Key, PhysicalKey};

        use super::{
            TextInputSync, TextInputSyncOp, WinitKeyInput, map_cursor_position, map_scroll_delta,
            queue_keyboard_input, should_emit_keyboard_text,
        };
        use crate::platform::{InputEvent, KeyState, Modifiers, TextInputPurpose, TextInputState};

        fn input_state(x: f64, y: f64, purpose: TextInputPurpose) -> TextInputState {
            TextInputState {
                x,
                y,
                width: 2.0,
                height: 14.0,
                purpose,
            }
        }

        #[test]
        fn sync_reports_allowed_only_across_focus_transitions() {
            let mut sync = TextInputSync::default();
            let state = input_state(10.0, 20.0, TextInputPurpose::Normal);
            assert_eq!(
                sync.sync(Some(state), 1.0),
                vec![
                    TextInputSyncOp::Allowed(true),
                    TextInputSyncOp::Purpose(TextInputPurpose::Normal),
                    TextInputSyncOp::CursorArea {
                        x: 10,
                        y: 20,
                        width: 2,
                        height: 14,
                    },
                ]
            );
            // Same state again: nothing to do.
            assert_eq!(sync.sync(Some(state), 1.0), Vec::new());
            // Losing focus disables the IME once and reports nothing else.
            assert_eq!(sync.sync(None, 1.0), vec![TextInputSyncOp::Allowed(false)]);
            assert_eq!(sync.sync(None, 1.0), Vec::new());
            // Refocusing re-enables it.
            assert_eq!(
                sync.sync(Some(state), 1.0),
                vec![
                    TextInputSyncOp::Allowed(true),
                    TextInputSyncOp::Purpose(TextInputPurpose::Normal),
                    TextInputSyncOp::CursorArea {
                        x: 10,
                        y: 20,
                        width: 2,
                        height: 14,
                    },
                ]
            );
        }

        #[test]
        fn sync_reports_purpose_transitions() {
            let mut sync = TextInputSync::default();
            let normal = input_state(10.0, 20.0, TextInputPurpose::Normal);
            let password = input_state(10.0, 20.0, TextInputPurpose::Password);
            let _ = sync.sync(Some(normal), 1.0);
            // Purpose changed while staying allowed: no Allowed op, but the
            // new purpose and cursor area are reported.
            assert_eq!(
                sync.sync(Some(password), 1.0),
                vec![
                    TextInputSyncOp::Purpose(TextInputPurpose::Password),
                    TextInputSyncOp::CursorArea {
                        x: 10,
                        y: 20,
                        width: 2,
                        height: 14,
                    },
                ]
            );
        }

        #[test]
        fn sync_converts_logical_geometry_to_physical_pixels() {
            let mut sync = TextInputSync::default();
            let ops = sync.sync(Some(input_state(10.4, 20.5, TextInputPurpose::Normal)), 2.0);
            assert_eq!(
                ops.last(),
                Some(&TextInputSyncOp::CursorArea {
                    x: 21,      // 10.4 * 2.0 rounded
                    y: 41,      // 20.5 * 2.0 rounded
                    width: 4,   // max(2,1) * 2.0 ceiled
                    height: 28, // max(14,1) * 2.0 ceiled
                })
            );
        }

        #[test]
        fn sync_never_swallows_a_moving_caret() {
            let mut sync = TextInputSync::default();
            let _ = sync.sync(Some(input_state(10.0, 20.0, TextInputPurpose::Normal)), 1.0);
            // Only the caret x moves: the state differs, so the equality
            // early-return must not drop the update.
            let ops = sync.sync(Some(input_state(12.0, 20.0, TextInputPurpose::Normal)), 1.0);
            assert_eq!(
                ops,
                vec![
                    TextInputSyncOp::Purpose(TextInputPurpose::Normal),
                    TextInputSyncOp::CursorArea {
                        x: 12,
                        y: 20,
                        width: 2,
                        height: 14,
                    },
                ]
            );
        }

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

        /// water-rs/hydrolysis#211: on X11, `XI_FocusIn` replays every held
        /// key as a synthetic `KeyboardInput` press and `XI_FocusOut` as a
        /// synthetic release — state synchronisation, not keystrokes. The
        /// replayed press must not produce text or a key event; the replayed
        /// release surfaces only as the cancellation of the press it paired
        /// with. The real press that follows is the only one that types.
        #[test]
        fn a_synthetic_focus_replay_emits_no_input() {
            let logical_e = Key::Character("e".into());
            let held_key = WinitKeyInput {
                is_synthetic: true,
                state: ElementState::Pressed,
                repeat: false,
                text: Some("e"),
                logical_key: &logical_e,
                physical_key: PhysicalKey::Code(winit::keyboard::KeyCode::KeyE),
            };

            let mut events = Vec::new();
            // The focus-in replay of the held key, then the real press.
            queue_keyboard_input(&mut events, Modifiers::default(), held_key);
            queue_keyboard_input(
                &mut events,
                Modifiers::default(),
                WinitKeyInput {
                    state: ElementState::Released,
                    ..held_key
                },
            );
            queue_keyboard_input(
                &mut events,
                Modifiers::default(),
                WinitKeyInput {
                    is_synthetic: false,
                    ..held_key
                },
            );

            let text_inputs = events
                .iter()
                .filter(|event| matches!(event, InputEvent::TextInput { .. }))
                .count();
            let presses = events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        InputEvent::Key {
                            state: KeyState::Pressed,
                            ..
                        }
                    )
                })
                .count();
            let cancels = events
                .iter()
                .filter(|event| matches!(event, InputEvent::KeyboardCancel))
                .count();
            assert_eq!(text_inputs, 1, "only the real press may type text");
            assert_eq!(presses, 1, "only the real press may produce a key event");
            assert_eq!(
                cancels, 1,
                "the synthetic release cancels the press it belonged to"
            );
            assert_eq!(events.len(), 3, "the synthetic press emits nothing");
        }

        #[test]
        fn cursor_position_panics_with_invalid_scale_factor() {
            let result = std::panic::catch_unwind(|| {
                let _ = map_cursor_position(&PhysicalPosition::new(120.0, 80.0), 0.0);
            });
            assert!(result.is_err());
        }

        fn caps_with_alpha_modes(
            alpha_modes: &[wgpu::CompositeAlphaMode],
        ) -> wgpu::SurfaceCapabilities {
            wgpu::SurfaceCapabilities {
                alpha_modes: alpha_modes.to_vec(),
                ..wgpu::SurfaceCapabilities::default()
            }
        }

        fn fake_adapter_info() -> wgpu::AdapterInfo {
            wgpu::AdapterInfo {
                name: "fake adapter".to_string(),
                vendor: 0,
                device: 0,
                device_type: wgpu::DeviceType::Cpu,
                device_pci_bus_id: String::new(),
                driver: String::new(),
                driver_info: String::new(),
                backend: wgpu::Backend::Gl,
                subgroup_min_size: 4,
                subgroup_max_size: 128,
                transient_saves_memory: false,
            }
        }

        #[test]
        fn a_transparent_window_gets_the_first_transparent_alpha_mode_the_surface_offers() {
            use wgpu::CompositeAlphaMode as Mode;
            assert_eq!(
                super::WinitSurface::select_alpha_mode(
                    &caps_with_alpha_modes(&[Mode::Opaque, Mode::Inherit, Mode::PreMultiplied]),
                    true,
                    &fake_adapter_info(),
                ),
                Mode::PreMultiplied
            );
            // X11 compositing on a 32-bit visual reports exactly this pair:
            // with no explicit multiplied mode the inherited mode is the only
            // one whose alpha reaches the compositor.
            assert_eq!(
                super::WinitSurface::select_alpha_mode(
                    &caps_with_alpha_modes(&[Mode::Opaque, Mode::Inherit]),
                    true,
                    &fake_adapter_info(),
                ),
                Mode::Inherit
            );
        }

        #[test]
        fn an_opaque_window_keeps_the_surface_preferred_alpha_mode() {
            use wgpu::CompositeAlphaMode as Mode;
            assert_eq!(
                super::WinitSurface::select_alpha_mode(
                    &caps_with_alpha_modes(&[Mode::Opaque, Mode::PreMultiplied]),
                    false,
                    &fake_adapter_info(),
                ),
                Mode::Opaque
            );
        }

        /// water-rs/hydrolysis#118: a depth-32 X11 window on a Mesa
        /// software rasterizer below 24.1 is silently un-presentable — the
        /// version gate is the only signal presentation never had.
        #[test]
        fn a_mesa_software_adapter_below_24_1_is_blocked_for_transparency() {
            use super::WinitSurface;
            let adapter = |driver_info: &str, device_type: wgpu::DeviceType| {
                let mut info = fake_adapter_info();
                info.name = "llvmpipe (LLVM 15.0.7, 256 bits)".to_string();
                info.device_type = device_type;
                info.driver_info = driver_info.to_string();
                info
            };
            let blocked = WinitSurface::mesa_x11_transparency_blocker(&adapter(
                "Mesa 23.2.1-1ubuntu2",
                wgpu::DeviceType::Cpu,
            ));
            let cause = blocked.expect("Mesa 23.2.1 llvmpipe must be rejected");
            assert!(
                cause.contains("23.2.1")
                    && cause.contains("Mesa 24.1")
                    && cause.contains("1e849b12")
                    && cause.contains("upgrade Mesa"),
                "the failure must name the cause and the fix: {cause}"
            );
            for driver_info in [
                "Mesa 24.1.0",
                "Mesa 24.1.0-devel (git-c4b20fd)",
                "Mesa 25.0.7",
                "Mesa 26.2.3",
                // Not a Mesa driver — the defect is Mesa-specific.
                "SwiftShader driver",
                // No Mesa version to prove the defect against.
                "",
            ] {
                assert!(
                    WinitSurface::mesa_x11_transparency_blocker(&adapter(
                        driver_info,
                        wgpu::DeviceType::Cpu,
                    ))
                    .is_none(),
                    "{driver_info} must pass"
                );
            }
            // A hardware adapter presents through DRI3, never the software
            // X11 WSI path — even a Mesa one on an old version.
            for device_type in [
                wgpu::DeviceType::IntegratedGpu,
                wgpu::DeviceType::DiscreteGpu,
                wgpu::DeviceType::VirtualGpu,
                wgpu::DeviceType::Other,
            ] {
                assert!(
                    WinitSurface::mesa_x11_transparency_blocker(&adapter(
                        "Mesa 23.2.1",
                        device_type,
                    ))
                    .is_none(),
                    "{device_type:?} must pass"
                );
            }
        }

        #[test]
        #[should_panic(expected = "fake adapter")]
        fn a_transparent_window_on_an_opaque_only_adapter_fails_at_creation() {
            use wgpu::CompositeAlphaMode as Mode;
            let _ = super::WinitSurface::select_alpha_mode(
                &caps_with_alpha_modes(&[Mode::Opaque]),
                true,
                &fake_adapter_info(),
            );
        }

        #[test]
        fn the_requested_frame_is_reapplied_on_the_first_mapped_event() {
            use waterui::window::WindowState;
            use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
            use winit::event::WindowEvent;

            let mut retry = super::MappedRequestRetry::default();
            let position = LogicalPosition::new(12.0, 34.0);
            let size = LogicalSize::new(800.0, 300.0);
            let armed = |position, size, state| super::PendingMappedRequest {
                position,
                size,
                state,
            };

            // Armed while the window is unmapped: a pre-map event leaves the
            // frame held; the first mapped-signal event returns it once.
            retry.arm(Some(false), position, size, WindowState::Normal);
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Focused(true)),
                None,
                "a non-mapped event must not consume the held frame"
            );
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Moved(PhysicalPosition::new(0, 0))),
                Some(armed(position, size, WindowState::Normal)),
            );
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Resized(PhysicalSize::new(1, 1))),
                None,
                "the re-apply fires once only"
            );

            // A later request while still hidden replaces the earlier one —
            // frame and state alike.
            let mut retry = super::MappedRequestRetry::default();
            retry.arm(Some(false), position, size, WindowState::Normal);
            let newer = (
                LogicalPosition::new(56.0, 78.0),
                LogicalSize::new(1024.0, 640.0),
            );
            retry.arm(None, newer.0, newer.1, WindowState::Fullscreen);
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Occluded(false)),
                Some(armed(newer.0, newer.1, WindowState::Fullscreen)),
            );

            // Once the window is visible nothing is held: the ordinary
            // frame-binding path applies later frames. `is_visible` may lag
            // the real map transition — a mapped event latches `mapped`, so
            // arming must not resurrect a retry the live geometry overtook.
            let mut retry = super::MappedRequestRetry::default();
            retry.arm(Some(false), position, size, WindowState::Normal);
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Moved(PhysicalPosition::new(0, 0))),
                Some(armed(position, size, WindowState::Normal)),
            );
            retry.arm(None, newer.0, newer.1, WindowState::Normal);
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Moved(PhysicalPosition::new(0, 0))),
                None,
                "no re-arm is allowed once a mapped event has been seen"
            );
            let mut retry = super::MappedRequestRetry::default();
            retry.arm(Some(true), position, size, WindowState::Normal);
            assert_eq!(
                retry.take_on_mapped_event(&WindowEvent::Moved(PhysicalPosition::new(0, 0))),
                None,
                "a confirmed-visible window holds nothing"
            );
        }

        #[test]
        fn a_premap_state_is_reapplied_on_the_first_mapped_event() {
            use waterui::window::WindowState;
            use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
            use winit::event::WindowEvent;

            let position = LogicalPosition::new(12.0, 34.0);
            let size = LogicalSize::new(800.0, 300.0);

            // A Fullscreen request made while the window was still unmapped
            // is held and delivered once on the first mapped-signal event.
            for state in [
                WindowState::Fullscreen,
                WindowState::Minimized,
                WindowState::Closed,
            ] {
                let mut retry = super::MappedRequestRetry::default();
                retry.arm(Some(false), position, size, state);
                assert_eq!(
                    retry.take_on_mapped_event(&WindowEvent::Moved(PhysicalPosition::new(0, 0))),
                    Some(super::PendingMappedRequest {
                        position,
                        size,
                        state,
                    }),
                    "a pre-map {state:?} must be re-delivered on the first mapped event"
                );
                assert_eq!(
                    retry.take_on_mapped_event(&WindowEvent::Resized(PhysicalSize::new(1, 1))),
                    None,
                    "a pre-map {state:?} is delivered once only"
                );
            }
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web_impl::ExportedBrowserWindow as BrowserWindow;

#[cfg(feature = "winit")]
pub(crate) use winit_impl::ExportedWinitGpuContext as WinitGpuContext;

#[cfg(feature = "winit")]
pub use winit_impl::ExportedWinitWindow as WinitWindow;
