use std::cell::{Cell, RefCell};
#[cfg(feature = "chromium")]
use std::future::Future;
#[cfg(feature = "chromium")]
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

#[cfg(feature = "chromium")]
use base64::Engine as _;
#[cfg(feature = "webview")]
use cef::ImplRequest as _;
use cef::rc::Rc as _;
use cef::{
    AcceleratedPaintInfo, Browser, BrowserHost, BrowserSettings, CefString, Client,
    CompositionUnderline, DisplayHandler, Frame, ImplBrowser, ImplBrowserHost, ImplClient,
    ImplDictionaryValue, ImplDisplayHandler, ImplFrame, ImplLifeSpanHandler, ImplLoadHandler,
    ImplPreferenceManager, ImplRenderHandler, ImplRequestHandler, ImplValue, KeyEvent,
    KeyEventType, LifeSpanHandler, LoadHandler, MouseButtonType, MouseEvent, PaintElementType,
    Range, Rect, RenderHandler, Request, RequestContext, RequestContextSettings, RequestHandler,
    ScreenInfo, TerminationStatus, WindowInfo, WrapClient, WrapDisplayHandler, WrapLifeSpanHandler,
    WrapLoadHandler, WrapRenderHandler, WrapRequestHandler, browser_host_create_browser_sync,
    dictionary_value_create, request_context_create_context, value_create,
};
#[cfg(feature = "chromium")]
use futures::channel::oneshot;
use num_traits::ToPrimitive as _;
#[cfg(feature = "chromium")]
use waterui_chromium::{
    CdpError, ChromiumConfiguration, ChromiumEvent, ChromiumPageHandle, ChromiumProfile,
    CustomChromiumController, PageMode, ScreenshotFormat,
};
#[cfg(feature = "webview")]
use waterui_core::{Computed, Signal};
use waterui_url::Url;
#[cfg(feature = "webview")]
use waterui_webview::{BackendEvent, WatcherSet, WebViewError, WebViewEvent};

#[cfg(feature = "chromium")]
use crate::cdp::CefCdpError;
use crate::cdp::CefCdpSession;
use crate::runtime::CefRuntime;

const EVENTFLAG_SHIFT_DOWN: u32 = 1 << 1;
const EVENTFLAG_CONTROL_DOWN: u32 = 1 << 2;
const EVENTFLAG_ALT_DOWN: u32 = 1 << 3;
const EVENTFLAG_LEFT_MOUSE_BUTTON: u32 = 1 << 4;
const EVENTFLAG_MIDDLE_MOUSE_BUTTON: u32 = 1 << 5;
const EVENTFLAG_RIGHT_MOUSE_BUTTON: u32 = 1 << 6;
const EVENTFLAG_COMMAND_DOWN: u32 = 1 << 7;

/// Modifier snapshot forwarded into CEF windowless input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "CEF models keyboard and pointer modifier flags independently"
)]
pub struct CefInputModifiers {
    /// Shift key.
    pub shift: bool,
    /// Control key.
    pub control: bool,
    /// Alt or Option key.
    pub alt: bool,
    /// Command or Super key.
    pub command: bool,
    /// Primary mouse button.
    pub primary_button: bool,
    /// Middle mouse button.
    pub middle_button: bool,
    /// Secondary mouse button.
    pub secondary_button: bool,
}

impl CefInputModifiers {
    const fn bits(self) -> u32 {
        let mut bits = 0;
        if self.shift {
            bits |= EVENTFLAG_SHIFT_DOWN;
        }
        if self.control {
            bits |= EVENTFLAG_CONTROL_DOWN;
        }
        if self.alt {
            bits |= EVENTFLAG_ALT_DOWN;
        }
        if self.command {
            bits |= EVENTFLAG_COMMAND_DOWN;
        }
        if self.primary_button {
            bits |= EVENTFLAG_LEFT_MOUSE_BUTTON;
        }
        if self.middle_button {
            bits |= EVENTFLAG_MIDDLE_MOUSE_BUTTON;
        }
        if self.secondary_button {
            bits |= EVENTFLAG_RIGHT_MOUSE_BUTTON;
        }
        bits
    }
}

/// Pointer buttons represented by CEF's OSR input ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CefPointerButton {
    /// Primary button.
    Primary,
    /// Middle button.
    Middle,
    /// Secondary button.
    Secondary,
}

impl CefPointerButton {
    const fn native(self) -> MouseButtonType {
        match self {
            Self::Primary => MouseButtonType::LEFT,
            Self::Middle => MouseButtonType::MIDDLE,
            Self::Secondary => MouseButtonType::RIGHT,
        }
    }
}

/// Native keyboard metadata sent to CEF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CefKeyInput {
    /// Platform hardware keycode, or zero when unavailable.
    pub native_keycode: u32,
    /// Platform keysym or virtual key.
    pub keyval: u32,
    /// Textual character associated with this key.
    pub character: Option<char>,
}

/// A half-open UTF-16 range used by CEF text editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CefTextRange {
    /// Inclusive UTF-16 start offset.
    pub start: u32,
    /// Exclusive UTF-16 end offset.
    pub end: u32,
}

impl CefTextRange {
    /// Creates a validated half-open text range.
    ///
    /// # Panics
    ///
    /// Panics when `start` exceeds `end`.
    #[must_use]
    pub fn new(start: u32, end: u32) -> Self {
        assert!(
            start <= end,
            "CEF text range start {start} exceeds end {end}"
        );
        Self { start, end }
    }

    const fn into_cef(self) -> Range {
        Range {
            from: self.start,
            to: self.end,
        }
    }
}

/// Receives a CEF shared GPU frame while the platform handle is valid.
///
/// Implementations must reopen the platform resource and complete a GPU copy
/// into an application-owned texture before returning. CEF returns the resource
/// to its pool immediately after the callback; duplicating a file descriptor or
/// handle, or retaining an `IOSurface`, does not extend the resource lifetime.
pub trait AcceleratedFrameSink: 'static {
    /// Copies one accelerated browser frame while its native resource is valid.
    fn import(&self, element: PaintElementType, dirty_rects: &[Rect], frame: &AcceleratedPaintInfo);

    /// Updates the popup widget rectangle in view coordinates.
    fn set_popup_rect(&self, rect: Option<CefPopupRect>);
}

/// CEF popup widget bounds in view coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CefPopupRect {
    /// Horizontal origin.
    pub x: i32,
    /// Vertical origin.
    pub y: i32,
    /// Popup width.
    pub width: u32,
    /// Popup height.
    pub height: u32,
}

impl CefPopupRect {
    fn from_cef(rect: &Rect) -> Self {
        assert!(
            rect.width > 0 && rect.height > 0,
            "CEF popup dimensions must be positive"
        );
        Self {
            x: rect.x,
            y: rect.y,
            width: u32::try_from(rect.width).expect("CEF popup width must be positive"),
            height: u32::try_from(rect.height).expect("CEF popup height must be positive"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CefPageMode {
    Visible,
    #[cfg(feature = "chromium")]
    Headless,
}

#[derive(Debug, Clone, Default)]
pub struct CefPageProfile {
    pub name: Option<String>,
    pub data_directory: Option<PathBuf>,
    pub incognito: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CefPageConfiguration {
    pub url: Option<Url>,
    pub profile: CefPageProfile,
    pub proxy: Option<String>,
    pub language: Option<String>,
    pub user_agent: Option<String>,
}

#[cfg(feature = "chromium")]
#[derive(Debug, Clone, PartialEq)]
pub enum CefPageEvent {
    Ready,
    NavigationStarted(Url),
    Loading(f32),
    Loaded(Url),
    RendererTerminated(String),
    Closed,
}

#[cfg(feature = "chromium")]
type PageWatcher = Box<dyn Fn(CefPageEvent)>;

struct PageState {
    mode: CefPageMode,
    size: Cell<(i32, i32)>,
    device_scale_factor: Cell<f32>,
    frame_sink: RefCell<Option<Rc<dyn AcceleratedFrameSink>>>,
    accelerated_paint_received: Cell<bool>,
    #[cfg(feature = "chromium")]
    watchers: RefCell<Vec<PageWatcher>>,
    #[cfg(feature = "webview")]
    webview_watchers: WatcherSet<BackendEvent>,
    #[cfg(feature = "webview")]
    redirects_enabled: RefCell<Computed<bool>>,
    #[cfg(feature = "webview")]
    current_url: RefCell<Option<Url>>,
    #[cfg(feature = "chromium")]
    close_waiters: RefCell<Vec<oneshot::Sender<Result<(), CefCdpError>>>>,
    #[cfg(feature = "chromium")]
    closed: Cell<bool>,
    #[cfg(feature = "chromium")]
    ready: Cell<bool>,
}

impl PageState {
    fn new(mode: CefPageMode) -> Self {
        Self {
            mode,
            size: Cell::new((1, 1)),
            device_scale_factor: Cell::new(1.0),
            frame_sink: RefCell::new(None),
            accelerated_paint_received: Cell::new(false),
            #[cfg(feature = "chromium")]
            watchers: RefCell::new(Vec::new()),
            #[cfg(feature = "webview")]
            webview_watchers: WatcherSet::new(),
            #[cfg(feature = "webview")]
            redirects_enabled: RefCell::new(Computed::constant(true)),
            #[cfg(feature = "webview")]
            current_url: RefCell::new(None),
            #[cfg(feature = "chromium")]
            close_waiters: RefCell::new(Vec::new()),
            #[cfg(feature = "chromium")]
            closed: Cell::new(false),
            #[cfg(feature = "chromium")]
            ready: Cell::new(false),
        }
    }

    #[cfg(feature = "chromium")]
    fn emit(&self, event: &CefPageEvent) {
        for watcher in self.watchers.borrow().iter() {
            watcher(event.clone());
        }
    }

    #[cfg(feature = "chromium")]
    fn ready(&self) {
        if self.ready.replace(true) {
            return;
        }
        self.emit(&CefPageEvent::Ready);
    }

    #[cfg(feature = "chromium")]
    fn watch(&self, watcher: impl Fn(CefPageEvent) + 'static) {
        if self.ready.get() {
            watcher(CefPageEvent::Ready);
        }
        if self.closed.get() {
            watcher(CefPageEvent::Closed);
            return;
        }
        self.watchers.borrow_mut().push(Box::new(watcher));
    }

    #[cfg(feature = "chromium")]
    fn closed(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.emit(&CefPageEvent::Closed);
        for waiter in self.close_waiters.borrow_mut().drain(..) {
            let _ = waiter.send(Ok(()));
        }
    }

    #[cfg(feature = "webview")]
    fn emit_webview(&self, event: impl Into<BackendEvent>) {
        self.webview_watchers.emit(&event.into());
    }
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_render_handler(state: Rc<PageState>) -> RenderHandler {
    cef::wrap_render_handler! {
        struct WaterRenderHandler {
            state: Rc<PageState>,
        }

        impl RenderHandler {
            fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
                let rect = rect.expect("CEF must provide an output rectangle");
                let (width, height) = self.state.size.get();
                *rect = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
            }

            fn screen_info(
                &self,
                _browser: Option<&mut Browser>,
                screen_info: Option<&mut ScreenInfo>,
            ) -> std::os::raw::c_int {
                let screen_info = screen_info.expect("CEF must provide screen information");
                screen_info.device_scale_factor = self.state.device_scale_factor.get();
                screen_info.depth = 24;
                screen_info.depth_per_component = 8;
                screen_info.is_monochrome = 0;
                1
            }

            fn on_popup_show(&self, _browser: Option<&mut Browser>, show: std::os::raw::c_int) {
                assert!(
                    show == 0 || show == 1,
                    "CEF popup visibility must be zero or one"
                );
                if show == 0
                    && let Some(sink) = self.state.frame_sink.borrow().as_ref()
                {
                    sink.set_popup_rect(None);
                }
            }

            fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
                let rect = rect.expect("CEF popup size callback must contain a rectangle");
                if let Some(sink) = self.state.frame_sink.borrow().as_ref() {
                    sink.set_popup_rect(Some(CefPopupRect::from_cef(rect)));
                }
            }

            fn on_paint(
                &self,
                _browser: Option<&mut Browser>,
                _element: PaintElementType,
                _dirty_rects: Option<&[Rect]>,
                _buffer: *const u8,
                _width: std::os::raw::c_int,
                _height: std::os::raw::c_int,
            ) {
                panic!(
                    "CEF delivered a CPU paint buffer; WaterUI requires shared-texture accelerated paint"
                );
            }

            fn on_accelerated_paint(
                &self,
                _browser: Option<&mut Browser>,
                element: PaintElementType,
                dirty_rects: Option<&[Rect]>,
                info: Option<&AcceleratedPaintInfo>,
            ) {
                let info = info.expect("CEF accelerated paint callback must contain frame metadata");
                if !self.state.accelerated_paint_received.replace(true) {
                    tracing::debug!(
                        has_frame_sink = self.state.frame_sink.borrow().is_some(),
                        "Received the first accelerated CEF paint"
                    );
                }
                if let Some(sink) = self.state.frame_sink.borrow().as_ref() {
                    sink.import(element, dirty_rects.unwrap_or_default(), info);
                }
            }
        }
    }
    WaterRenderHandler::new(state)
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_load_handler(state: Rc<PageState>) -> LoadHandler {
    cef::wrap_load_handler! {
        struct WaterLoadHandler {
            state: Rc<PageState>,
        }

        impl LoadHandler {
            fn on_loading_state_change(
                &self,
                _browser: Option<&mut Browser>,
                is_loading: std::os::raw::c_int,
                _can_go_back: std::os::raw::c_int,
                _can_go_forward: std::os::raw::c_int,
            ) {
                // Progress itself comes from the display handler, which reports
                // Chromium's real fraction rather than just "started"/"finished".
                let _ = is_loading;
                #[cfg(feature = "webview")]
                let browser = _browser.expect("CEF loading state must identify the browser");
                #[cfg(feature = "webview")]
                self.state.emit_webview(BackendEvent::NavigationState {
                    can_go_back: browser.can_go_back() == 1,
                    can_go_forward: browser.can_go_forward() == 1,
                });
            }

            fn on_load_start(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                _transition_type: cef::TransitionType,
            ) {
                let frame = frame.expect("CEF load start must identify a frame");
                if frame.is_main() == 1 {
                    #[cfg(any(feature = "chromium", feature = "webview"))]
                    {
                    let url = frame_url(frame);
                    #[cfg(feature = "webview")]
                    self.state.current_url.replace(Some(url.clone()));
                    #[cfg(all(feature = "chromium", feature = "webview"))]
                    self.state
                        .emit(&CefPageEvent::NavigationStarted(url.clone()));
                    #[cfg(all(feature = "chromium", not(feature = "webview")))]
                    self.state.emit(&CefPageEvent::NavigationStarted(url));
                    #[cfg(feature = "webview")]
                    self.state
                        .emit_webview(WebViewEvent::WillNavigate { url });
                    }
                }
            }

            fn on_load_end(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                _http_status_code: std::os::raw::c_int,
            ) {
                let frame = frame.expect("CEF load end must identify a frame");
                if frame.is_main() == 1 {
                    #[cfg(feature = "chromium")]
                    self.state.emit(&CefPageEvent::Loaded(frame_url(frame)));
                    #[cfg(feature = "webview")]
                    self.state.emit_webview(WebViewEvent::Loaded);
                }
            }

            fn on_load_error(
                &self,
                _browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _error_code: cef::Errorcode,
                _error_text: Option<&CefString>,
                _failed_url: Option<&CefString>,
            ) {
                #[cfg(feature = "webview")]
                {
                    let frame = _frame.expect("CEF load error must identify a frame");
                    if frame.is_main() == 1 {
                        self.state.emit_webview(WebViewEvent::Error(
                            WebViewError::LoadFailed(
                                _error_text
                                    .map_or_else(String::new, ToString::to_string)
                                    .into(),
                            ),
                        ));
                    }
                }
            }
        }
    }
    WaterLoadHandler::new(state)
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_life_span_handler(state: Rc<PageState>) -> LifeSpanHandler {
    cef::wrap_life_span_handler! {
        struct WaterLifeSpanHandler {
            state: Rc<PageState>,
        }

        impl LifeSpanHandler {
            fn on_before_popup(
                &self,
                browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _popup_id: std::os::raw::c_int,
                target_url: Option<&CefString>,
                _target_frame_name: Option<&CefString>,
                _target_disposition: cef::WindowOpenDisposition,
                user_gesture: std::os::raw::c_int,
                _popup_features: Option<&cef::PopupFeatures>,
                _window_info: Option<&mut WindowInfo>,
                _client: Option<&mut Option<Client>>,
                _settings: Option<&mut BrowserSettings>,
                _extra_info: Option<&mut Option<cef::DictionaryValue>>,
                _no_javascript_access: Option<&mut std::os::raw::c_int>,
            ) -> std::os::raw::c_int {
                let target_url = target_url.map(ToString::to_string);
                if let Some(target_url) =
                    popup_navigation_target(user_gesture, target_url.as_deref())
                {
                    browser
                        .expect("CEF popup request must identify its opener browser")
                        .main_frame()
                        .expect("CEF popup opener must expose its main frame")
                        .load_url(Some(&target_url.into()));
                }

                // ChromiumView has no independent native-window surface. Allowing
                // CEF's default popup would escape the WaterUI view hierarchy and
                // create an unmanaged top-level Chrome window.
                1
            }

            fn on_before_close(&self, _browser: Option<&mut Browser>) {
                #[cfg(feature = "chromium")]
                self.state.closed();
            }
        }
    }
    WaterLifeSpanHandler::new(state)
}

fn popup_navigation_target(
    user_gesture: std::os::raw::c_int,
    target_url: Option<&str>,
) -> Option<&str> {
    assert!(
        user_gesture == 0 || user_gesture == 1,
        "CEF popup user gesture must be zero or one"
    );
    if user_gesture == 0 {
        return None;
    }
    target_url.filter(|target_url| !target_url.is_empty())
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_request_handler(state: Rc<PageState>) -> RequestHandler {
    cef::wrap_request_handler! {
        struct WaterRequestHandler {
            state: Rc<PageState>,
        }

        impl RequestHandler {
            fn on_before_browse(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                request: Option<&mut Request>,
                _user_gesture: std::os::raw::c_int,
                is_redirect: std::os::raw::c_int,
            ) -> std::os::raw::c_int {
                #[cfg(not(feature = "webview"))]
                {
                    let _ = (frame, request, is_redirect);
                    return 0;
                }
                #[cfg(feature = "webview")]
                {
                let frame = frame.expect("CEF navigation must identify a frame");
                if frame.is_main() != 1 {
                    return 0;
                }
                let request = request.expect("CEF navigation must contain a request");
                let destination: Url = CefString::from(&request.url())
                    .to_string()
                    .parse()
                    .expect("CEF navigation URL must be a valid WaterUI URL");
                if is_redirect == 1 {
                    if let Some(source) = self.state.current_url.borrow().clone() {
                        self.state.emit_webview(WebViewEvent::Redirect {
                            from: source,
                            to: destination.clone(),
                        });
                    }
                    if !self.state.redirects_enabled.borrow().get() {
                        return 1;
                    }
                }
                self.state.current_url.replace(Some(destination));
                0
                }
            }

            fn on_certificate_error(
                &self,
                _browser: Option<&mut Browser>,
                _cert_error: cef::Errorcode,
                _request_url: Option<&CefString>,
                _ssl_info: Option<&mut cef::Sslinfo>,
                _callback: Option<&mut cef::Callback>,
            ) -> std::os::raw::c_int {
                #[cfg(feature = "webview")]
                {
                    let url = _request_url
                        .map(ToString::to_string)
                        .and_then(|url| url.parse().ok());
                    if let Some(url) = url {
                        self.state.emit_webview(WebViewEvent::Error(WebViewError::Ssl {
                            url,
                            message: alloc_error_text(_cert_error),
                        }));
                    }
                }
                // Returning 0 rejects the certificate. WaterUI does not offer a
                // "continue anyway" path: overriding a certificate error is a
                // decision for the application, not a silent default.
                0
            }

            fn on_render_process_terminated(
                &self,
                _browser: Option<&mut Browser>,
                _status: TerminationStatus,
                _error_code: std::os::raw::c_int,
                _error_string: Option<&CefString>,
            ) {
                #[cfg(feature = "chromium")]
                let error_string = _error_string.map_or_else(String::new, ToString::to_string);
                #[cfg(feature = "chromium")]
                self.state
                    .emit(&CefPageEvent::RendererTerminated(format!(
                    "{_status:?} ({_error_code}): {error_string}"
                )));
            }
        }
    }
    WaterRequestHandler::new(state)
}

/// Reports Chromium's real load progress.
///
/// The load handler only knows "loading" and "not loading", which is why progress
/// used to be reported as 0.0 or 1.0 and nothing in between. Chromium computes a
/// fractional value and delivers it here.
#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_display_handler(state: Rc<PageState>) -> DisplayHandler {
    cef::wrap_display_handler! {
        struct WaterDisplayHandler {
            state: Rc<PageState>,
        }

        impl DisplayHandler {
            fn on_loading_progress_change(&self, _browser: Option<&mut Browser>, progress: f64) {
                #[allow(clippy::cast_possible_truncation, reason = "progress is a 0..1 fraction")]
                let progress = progress as f32;
                let _ = progress;
                #[cfg(feature = "chromium")]
                self.state.emit(&CefPageEvent::Loading(progress));
                #[cfg(feature = "webview")]
                self.state.emit_webview(WebViewEvent::Loading { progress });
            }
        }
    }
    WaterDisplayHandler::new(state)
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_client(
    render: RenderHandler,
    load: LoadHandler,
    life_span: LifeSpanHandler,
    request: RequestHandler,
    display: DisplayHandler,
) -> Client {
    cef::wrap_client! {
        struct WaterClient {
            render: RenderHandler,
            load: LoadHandler,
            life_span: LifeSpanHandler,
            request: RequestHandler,
            display: DisplayHandler,
        }

        impl Client {
            fn display_handler(&self) -> Option<DisplayHandler> {
                Some(self.display.clone())
            }

            fn render_handler(&self) -> Option<RenderHandler> {
                Some(self.render.clone())
            }

            fn load_handler(&self) -> Option<LoadHandler> {
                Some(self.load.clone())
            }

            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                Some(self.life_span.clone())
            }

            fn request_handler(&self) -> Option<RequestHandler> {
                Some(self.request.clone())
            }
        }
    }
    WaterClient::new(render, load, life_span, request, display)
}

/// CEF implementation of one visible or headless Chromium page.
#[derive(Clone)]
pub struct CefPageHandle {
    runtime: CefRuntime,
    browser: Browser,
    host: BrowserHost,
    _request_context: RequestContext,
    cdp: CefCdpSession,
    state: Rc<PageState>,
}

impl core::fmt::Debug for CefPageHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CefPageHandle")
            .field("mode", &self.state.mode)
            .finish_non_exhaustive()
    }
}

impl CefPageHandle {
    fn create(runtime: CefRuntime, configuration: CefPageConfiguration, mode: CefPageMode) -> Self {
        let state = Rc::new(PageState::new(mode));
        let mut client = new_client(
            new_render_handler(Rc::clone(&state)),
            new_load_handler(Rc::clone(&state)),
            new_life_span_handler(Rc::clone(&state)),
            new_request_handler(Rc::clone(&state)),
            new_display_handler(Rc::clone(&state)),
        );
        let mut request_context = create_request_context(&runtime, &configuration);
        let initial_url = configuration
            .url
            .as_ref()
            .map_or("about:blank", Url::as_str);
        let browser = browser_host_create_browser_sync(
            Some(&WindowInfo {
                windowless_rendering_enabled: 1,
                shared_texture_enabled: 1,
                external_begin_frame_enabled: 1,
                ..Default::default()
            }),
            Some(&mut client),
            Some(&initial_url.into()),
            Some(&BrowserSettings {
                windowless_frame_rate: 60,
                ..Default::default()
            }),
            None,
            Some(&mut request_context),
        )
        .expect("CEF failed to create a shared-texture windowless browser");
        let host = browser.host().expect("CEF browser must expose its host");
        let cdp = CefCdpSession::attach(host.clone());
        #[cfg(feature = "chromium")]
        cdp.watch_attachment({
            let state = Rc::downgrade(&state);
            move || {
                if let Some(state) = state.upgrade() {
                    state.ready();
                }
            }
        });
        let handle = Self {
            runtime,
            browser,
            host,
            _request_context: request_context,
            cdp,
            state,
        };
        if let Some(user_agent) = configuration.user_agent {
            handle.execute_without_result(&crate::cdp::protocol::SetUserAgentOverride {
                user_agent: &user_agent,
            });
        }
        handle
    }

    fn execute_without_result<C: crate::cdp::protocol::CdpCommand>(&self, command: &C) {
        drop(self.cdp.execute(command));
    }

    /// Installs the backend GPU importer for a visible page.
    ///
    /// # Panics
    ///
    /// Panics when the page is headless.
    pub fn set_frame_sink(&self, sink: impl AcceleratedFrameSink) {
        assert_eq!(
            self.state.mode,
            CefPageMode::Visible,
            "headless CEF pages cannot install a frame presenter"
        );
        self.state.frame_sink.replace(Some(Rc::new(sink)));
        self.host.was_resized();
        self.host.invalidate(PaintElementType::VIEW);
    }

    /// Executes due work for the process-owned CEF external message pump.
    pub fn pump(&self) {
        let _ = self.runtime.pump();
    }

    /// Requests one compositor frame for this windowless browser.
    pub fn request_frame(&self) {
        self.host.send_external_begin_frame();
    }

    /// Updates keyboard focus for the windowless browser.
    pub fn set_focus(&self, focused: bool) {
        self.host.set_focus(i32::from(focused));
    }

    /// Navigates backward.
    pub fn go_back(&self) {
        self.browser.go_back();
    }

    /// Navigates forward.
    pub fn go_forward(&self) {
        self.browser.go_forward();
    }

    /// Undoes the last edit in the focused Chromium frame.
    pub fn undo(&self) {
        self.focused_frame_for_editing().undo();
    }

    /// Redoes the last edit in the focused Chromium frame.
    pub fn redo(&self) {
        self.focused_frame_for_editing().redo();
    }

    /// Cuts the current selection in the focused Chromium frame.
    pub fn cut(&self) {
        self.focused_frame_for_editing().cut();
    }

    /// Copies the current selection in the focused Chromium frame.
    pub fn copy(&self) {
        self.focused_frame_for_editing().copy();
    }

    /// Pastes clipboard contents into the focused Chromium frame.
    pub fn paste(&self) {
        self.focused_frame_for_editing().paste();
    }

    /// Selects all content in the focused Chromium frame.
    pub fn select_all(&self) {
        self.focused_frame_for_editing().select_all();
    }

    /// Sends pointer movement in browser-local logical coordinates.
    ///
    /// # Panics
    ///
    /// Panics when either coordinate does not fit CEF's integer input ABI.
    pub fn pointer_move(&self, x: f64, y: f64, modifiers: CefInputModifiers) {
        self.host.send_mouse_move_event(
            Some(&MouseEvent {
                x: x.round().to_i32().expect("CEF pointer x exceeds i32"),
                y: y.round().to_i32().expect("CEF pointer y exceeds i32"),
                modifiers: modifiers.bits(),
            }),
            0,
        );
    }

    /// Sends one pointer button transition.
    ///
    /// # Panics
    ///
    /// Panics when either coordinate does not fit CEF's integer input ABI.
    pub fn pointer_button(
        &self,
        pressed: bool,
        button: CefPointerButton,
        x: f64,
        y: f64,
        modifiers: CefInputModifiers,
    ) {
        self.host.send_mouse_click_event(
            Some(&MouseEvent {
                x: x.round().to_i32().expect("CEF pointer x exceeds i32"),
                y: y.round().to_i32().expect("CEF pointer y exceeds i32"),
                modifiers: modifiers.bits(),
            }),
            button.native(),
            i32::from(!pressed),
            1,
        );
    }

    /// Sends a wheel event in CEF wheel units.
    ///
    /// # Panics
    ///
    /// Panics when a coordinate or wheel delta does not fit CEF's integer
    /// input ABI.
    pub fn scroll(&self, x: f64, y: f64, delta_x: f64, delta_y: f64, modifiers: CefInputModifiers) {
        self.host.send_mouse_wheel_event(
            Some(&MouseEvent {
                x: x.round().to_i32().expect("CEF scroll x exceeds i32"),
                y: y.round().to_i32().expect("CEF scroll y exceeds i32"),
                modifiers: modifiers.bits(),
            }),
            delta_x.round().to_i32().expect("CEF wheel x exceeds i32"),
            delta_y.round().to_i32().expect("CEF wheel y exceeds i32"),
        );
    }

    /// Sends one native key transition and its character event.
    ///
    /// # Panics
    ///
    /// Panics when a single-unit UTF-16 character cannot be encoded or a
    /// native key code does not fit CEF's integer input ABI.
    pub fn key(&self, pressed: bool, key: CefKeyInput, modifiers: CefInputModifiers) {
        let character = key
            .character
            .filter(|character| character.len_utf16() == 1)
            .map_or(0, |character| {
                character
                    .encode_utf16(&mut [0; 2])
                    .first()
                    .copied()
                    .expect("one UTF-16 code unit must exist")
            });
        let event = KeyEvent {
            type_: key_event_type(pressed),
            modifiers: modifiers.bits(),
            windows_key_code: windows_key_code(key.keyval),
            native_key_code: i32::try_from(key.native_keycode)
                .expect("CEF native key code exceeds i32"),
            is_system_key: i32::from(modifiers.alt),
            character,
            unmodified_character: character,
            ..Default::default()
        };
        self.host.send_key_event(Some(&event));
        if pressed && character != 0 {
            self.host.send_key_event(Some(&KeyEvent {
                type_: KeyEventType::CHAR,
                ..event
            }));
        }
    }

    /// Commits IME or composed text into the focused browser editor.
    pub fn commit_text(&self, text: &str, replacement: Option<CefTextRange>) {
        let replacement = replacement.map(CefTextRange::into_cef);
        self.host
            .ime_commit_text(Some(&text.into()), replacement.as_ref(), 0);
    }

    /// Updates active IME composition and its UTF-16 selection.
    ///
    /// # Panics
    ///
    /// Panics when the composition exceeds `u32` UTF-16 code units or the
    /// selection is outside the composition.
    pub fn set_composition(
        &self,
        text: &str,
        selection_start: u32,
        selection_end: u32,
        replacement: Option<CefTextRange>,
    ) {
        let utf16_len =
            u32::try_from(text.encode_utf16().count()).expect("CEF composition length exceeds u32");
        assert!(
            selection_start <= selection_end && selection_end <= utf16_len,
            "CEF composition selection {selection_start}..{selection_end} exceeds UTF-16 length {utf16_len}"
        );
        let selection = Range {
            from: selection_start,
            to: selection_end,
        };
        let replacement = replacement.map(CefTextRange::into_cef);
        let underline = CompositionUnderline {
            range: Range {
                from: 0,
                to: utf16_len,
            },
            thick: 1,
            ..Default::default()
        };
        self.host.ime_set_composition(
            Some(&text.into()),
            Some(&[underline]),
            replacement.as_ref(),
            Some(&selection),
        );
    }

    /// Finishes active IME composition.
    pub fn finish_composition(&self, keep_selection: bool) {
        self.host
            .ime_finish_composing_text(i32::from(keep_selection));
    }

    /// Cancels active IME composition.
    pub fn cancel_composition(&self) {
        self.host.ime_cancel_composition();
    }

    /// Updates the OSR viewport in physical-independent pixels.
    ///
    /// # Panics
    ///
    /// Panics when dimensions are zero, exceed CEF's integer ABI, or the scale
    /// is not positive and finite.
    pub fn set_viewport(&self, width: u32, height: u32, device_scale_factor: f32) {
        assert!(
            width > 0 && height > 0,
            "CEF viewport dimensions must be non-zero"
        );
        assert!(
            device_scale_factor.is_finite() && device_scale_factor > 0.0,
            "CEF device scale factor must be finite and positive"
        );
        let width = i32::try_from(width).expect("CEF viewport width exceeds i32");
        let height = i32::try_from(height).expect("CEF viewport height exceeds i32");
        if self.state.size.get() == (width, height)
            && self.state.device_scale_factor.get().to_bits() == device_scale_factor.to_bits()
        {
            return;
        }
        self.state.size.set((width, height));
        self.state.device_scale_factor.set(device_scale_factor);
        self.host.notify_screen_info_changed();
        self.host.was_resized();
    }

    /// Returns the CEF host for backend-native pointer, keyboard, wheel, and IME routing.
    #[must_use]
    pub const fn host(&self) -> &BrowserHost {
        &self.host
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn mode(&self) -> CefPageMode {
        self.state.mode
    }

    pub(crate) fn cdp(&self) -> CefCdpSession {
        self.cdp.clone()
    }

    pub(crate) fn navigate(&self, url: &Url) {
        self.browser
            .main_frame()
            .expect("CEF browser must expose its main frame")
            .load_url(Some(&url.as_str().into()));
    }

    pub(crate) fn reload(&self) {
        self.browser.reload();
    }

    pub(crate) fn stop(&self) {
        self.browser.stop_load();
    }

    fn focused_frame_for_editing(&self) -> Frame {
        self.browser
            .focused_frame()
            .expect("CEF browser must expose a focused frame for editing commands")
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn watch_page(&self, watcher: impl Fn(CefPageEvent) + 'static) {
        self.state.watch(watcher);
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn close_page(&self) -> impl Future<Output = Result<(), CefCdpError>> + 'static {
        let (sender, receiver) = oneshot::channel();
        if self.state.closed.get() {
            let _ = sender.send(Ok(()));
        } else {
            self.state.close_waiters.borrow_mut().push(sender);
            self.host.close_browser(1);
        }
        async move { receiver.await.unwrap_or(Err(CefCdpError::Detached)) }
    }

    #[cfg(feature = "webview")]
    pub(crate) fn watch_webview(
        &self,
        watcher: impl Fn(BackendEvent) + 'static,
    ) -> waterui_webview::WatcherGuard {
        self.state.webview_watchers.insert(watcher)
    }

    #[cfg(feature = "webview")]
    pub(crate) fn set_redirects_enabled(&self, enabled: Computed<bool>) {
        self.state.redirects_enabled.replace(enabled);
    }
}

const fn key_event_type(pressed: bool) -> KeyEventType {
    if !pressed {
        return KeyEventType::KEYUP;
    }
    #[cfg(target_os = "macos")]
    {
        KeyEventType::KEYDOWN
    }
    #[cfg(not(target_os = "macos"))]
    {
        KeyEventType::RAWKEYDOWN
    }
}

#[cfg_attr(
    target_os = "macos",
    expect(
        clippy::missing_const_for_fn,
        reason = "non-macOS uses a checked conversion that is unavailable in coverage const builds"
    )
)]
fn windows_key_code(keyval: u32) -> i32 {
    #[cfg(target_os = "macos")]
    {
        let _ = keyval;
        0
    }
    #[cfg(not(target_os = "macos"))]
    {
        i32::try_from(keyval).expect("CEF Windows key code exceeds i32")
    }
}

#[cfg(feature = "chromium")]
impl ChromiumPageHandle for CefPageHandle {
    type Cdp = CefCdpSession;

    fn mode(&self) -> PageMode {
        match Self::mode(self) {
            CefPageMode::Visible => PageMode::Visible,
            CefPageMode::Headless => PageMode::Headless,
        }
    }

    fn cdp(&self) -> Self::Cdp {
        Self::cdp(self)
    }

    fn navigate(&self, url: &Url) {
        Self::navigate(self, url);
    }

    fn go_back(&self) {
        Self::go_back(self);
    }

    fn go_forward(&self) {
        Self::go_forward(self);
    }

    fn reload(&self) {
        Self::reload(self);
    }

    fn stop(&self) {
        Self::stop(self);
    }

    fn set_download_directory(&self, directory: &Path) {
        assert!(
            directory.is_absolute(),
            "Chromium download directory must be absolute: {}",
            directory.display()
        );
        self.execute_without_result(&crate::cdp::protocol::SetDownloadBehavior {
            behavior: "allow",
            download_path: &directory.to_string_lossy(),
        });
    }

    fn watch(&self, watcher: impl Fn(ChromiumEvent) + 'static) {
        self.watch_page(move |event| {
            watcher(match event {
                CefPageEvent::Ready => ChromiumEvent::Ready,
                CefPageEvent::NavigationStarted(url) => ChromiumEvent::NavigationStarted(url),
                CefPageEvent::Loading(progress) => ChromiumEvent::Loading(progress),
                CefPageEvent::Loaded(url) => ChromiumEvent::Loaded(url),
                CefPageEvent::RendererTerminated(message) => {
                    ChromiumEvent::RendererTerminated(message)
                }
                CefPageEvent::Closed => ChromiumEvent::Closed,
            });
        });
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF DevTools sessions are confined to the UI thread"
    )]
    fn screenshot(
        &self,
        format: ScreenshotFormat,
    ) -> impl Future<Output = Result<Vec<u8>, CdpError>> + 'static {
        let command = screenshot_command(format);
        let session = self.cdp.clone();
        async move {
            let response = session.execute(&command).await.map_err(CdpError::from)?;
            base64::engine::general_purpose::STANDARD
                .decode(response.data)
                .map_err(|error| CdpError::InvalidPayload {
                    method: "Page.captureScreenshot".to_string(),
                    message: error.to_string(),
                })
        }
    }

    fn close(&self) -> impl Future<Output = Result<(), CdpError>> + 'static {
        let future = self.close_page();
        async move { future.await.map_err(Into::into) }
    }
}

#[cfg(feature = "chromium")]
const fn screenshot_command(format: ScreenshotFormat) -> crate::cdp::protocol::CaptureScreenshot {
    let (format, quality) = match format {
        ScreenshotFormat::Png => ("png", None),
        ScreenshotFormat::Jpeg(quality) => ("jpeg", Some(quality)),
        ScreenshotFormat::WebP(quality) => ("webp", Some(quality)),
    };
    crate::cdp::protocol::CaptureScreenshot {
        format,
        quality,
        from_surface: true,
        capture_beyond_viewport: false,
    }
}

/// Controller shared by standard `WebView` and full Chromium components.
#[derive(Clone)]
pub struct CefController {
    runtime: CefRuntime,
}

impl CefController {
    pub const fn new(runtime: CefRuntime) -> Self {
        Self { runtime }
    }

    pub(crate) fn open_page(
        &self,
        configuration: CefPageConfiguration,
        mode: CefPageMode,
    ) -> CefPageHandle {
        CefPageHandle::create(self.runtime.clone(), configuration, mode)
    }
}

#[cfg(feature = "chromium")]
impl CustomChromiumController for CefController {
    type Page = CefPageHandle;

    fn open(&self, configuration: ChromiumConfiguration) -> Self::Page {
        self.open_page(configuration.into(), CefPageMode::Visible)
    }

    #[expect(
        clippy::future_not_send,
        reason = "CEF pages and DevTools sessions are confined to the UI thread"
    )]
    fn headless(
        &self,
        configuration: ChromiumConfiguration,
    ) -> impl Future<Output = Result<Self::Page, CdpError>> + 'static {
        let page = self.open_page(configuration.into(), CefPageMode::Headless);
        let cdp = page.cdp.clone();
        async move {
            cdp.wait_until_attached().await.map_err(CdpError::from)?;
            Ok(page)
        }
    }
}

#[cfg(feature = "chromium")]
impl From<ChromiumConfiguration> for CefPageConfiguration {
    fn from(configuration: ChromiumConfiguration) -> Self {
        let ChromiumConfiguration {
            url,
            profile:
                ChromiumProfile {
                    name,
                    data_directory,
                    incognito,
                },
            proxy,
            language,
            user_agent,
        } = configuration;
        Self {
            url,
            profile: CefPageProfile {
                name,
                data_directory,
                incognito,
            },
            proxy,
            language,
            user_agent,
        }
    }
}

fn create_request_context(
    runtime: &CefRuntime,
    configuration: &CefPageConfiguration,
) -> RequestContext {
    let cache_path = if configuration.profile.incognito {
        assert!(
            configuration.profile.name.is_none() && configuration.profile.data_directory.is_none(),
            "incognito Chromium profiles cannot specify a name or data directory"
        );
        String::new()
    } else {
        let root = configuration
            .profile
            .data_directory
            .as_deref()
            .unwrap_or_else(|| runtime.cache_root());
        assert!(
            root.is_absolute(),
            "Chromium profile directory must be absolute: {}",
            root.display()
        );
        configuration
            .profile
            .name
            .as_ref()
            .map_or_else(
                || root.join("default"),
                |name| {
                    assert!(
                        !name.is_empty() && !name.contains(['/', '\\']),
                        "Chromium profile name must be one path component"
                    );
                    root.join(name)
                },
            )
            .to_string_lossy()
            .into_owned()
    };
    let context = request_context_create_context(
        Some(&RequestContextSettings {
            cache_path: cache_path.as_str().into(),
            persist_session_cookies: i32::from(!configuration.profile.incognito),
            accept_language_list: configuration.language.as_deref().unwrap_or_default().into(),
            ..Default::default()
        }),
        None,
    )
    .expect("CEF failed to create an isolated request context");
    if let Some(proxy) = configuration.proxy.as_deref() {
        let mut proxy_dictionary =
            dictionary_value_create().expect("CEF failed to allocate proxy preferences");
        assert_eq!(
            proxy_dictionary.set_string(Some(&"mode".into()), Some(&"fixed_servers".into())),
            1,
            "CEF rejected proxy mode"
        );
        assert_eq!(
            proxy_dictionary.set_string(Some(&"server".into()), Some(&proxy.into())),
            1,
            "CEF rejected proxy server"
        );
        let mut value = value_create().expect("CEF failed to allocate proxy preference value");
        assert_eq!(
            value.set_dictionary(Some(&mut proxy_dictionary)),
            1,
            "CEF rejected proxy preference dictionary"
        );
        let mut error = CefString::default();
        assert_eq!(
            context.set_preference(Some(&"proxy".into()), Some(&mut value), Some(&mut error)),
            1,
            "CEF rejected proxy preference: {error}"
        );
    }
    context
}

/// Describes a Chromium certificate error code for the `WebViewError::Ssl` message.
#[cfg(feature = "webview")]
fn alloc_error_text(code: cef::Errorcode) -> waterui_str::Str {
    waterui_str::Str::from(format!("certificate error {code:?}"))
}

fn frame_url(frame: &Frame) -> Url {
    CefString::from(&frame.url())
        .to_string()
        .parse()
        .expect("CEF main-frame URL must be a valid WaterUI URL")
}

#[cfg(all(test, feature = "chromium"))]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use waterui_chromium::ScreenshotFormat;

    use super::{
        CefPageEvent, CefPageMode, PageState, key_event_type, popup_navigation_target,
        screenshot_command, windows_key_code,
    };

    #[test]
    fn late_lifecycle_watcher_receives_ready_and_closed_state() {
        let state = PageState::new(CefPageMode::Visible);
        state.ready();
        state.closed();

        let events = Rc::new(RefCell::new(Vec::new()));
        state.watch({
            let events = Rc::clone(&events);
            move |event| events.borrow_mut().push(event)
        });

        assert_eq!(
            *events.borrow(),
            [CefPageEvent::Ready, CefPageEvent::Closed]
        );
    }

    #[test]
    fn png_screenshot_omits_the_format_specific_quality_parameter() {
        let png = screenshot_command(ScreenshotFormat::Png);
        assert_eq!(png.format, "png");
        assert!(png.quality.is_none());

        let jpeg = screenshot_command(ScreenshotFormat::Jpeg(82));
        assert_eq!(jpeg.quality, Some(82));
    }

    #[test]
    fn user_initiated_popup_navigates_the_embedded_page() {
        let target = "https://crates.io/crates/waterui";

        assert_eq!(
            popup_navigation_target(1, Some(target)),
            Some("https://crates.io/crates/waterui")
        );
        assert_eq!(popup_navigation_target(0, Some(target)), None);
        assert_eq!(popup_navigation_target(1, None), None);
    }

    #[test]
    fn pressed_keys_use_the_platform_cef_event_type() {
        let expected = if cfg!(target_os = "macos") {
            cef::KeyEventType::KEYDOWN
        } else {
            cef::KeyEventType::RAWKEYDOWN
        };
        assert_eq!(key_event_type(true), expected);
        assert_eq!(key_event_type(false), cef::KeyEventType::KEYUP);
    }

    #[test]
    fn platform_windows_key_code_matches_cef_input_conventions() {
        if cfg!(target_os = "macos") {
            assert_eq!(windows_key_code(u32::from('A')), 0);
        } else {
            assert_eq!(windows_key_code(u32::from('A')), i32::from(b'A'));
        }
    }
}
