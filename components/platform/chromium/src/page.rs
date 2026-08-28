use std::any::{Any, type_name};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;

use waterui_core::env::use_env;
use waterui_core::extract::{ExtractionState, Extractor};
use waterui_core::layout::StretchAxis;
use waterui_core::view::{Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, Error, Native, NativeView, View, impl_debug};
use waterui_url::Url;

use crate::{CdpError, CdpSession, ChromiumController, CustomCdpSession};

/// Chromium profile and persistent data-store selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChromiumProfile {
    /// Optional profile name within the configured data directory.
    pub name: Option<String>,
    /// Persistent Chromium user-data directory.
    pub data_directory: Option<PathBuf>,
    /// Whether this page uses an in-memory off-the-record browser context.
    pub incognito: bool,
}

/// Configuration shared by visible and headless Chromium pages.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChromiumConfiguration {
    /// Initial page URL. A missing URL opens `about:blank`.
    pub url: Option<Url>,
    /// Browser profile and data-store configuration.
    pub profile: ChromiumProfile,
    /// Explicit proxy server accepted by Chromium.
    pub proxy: Option<String>,
    /// Preferred UI and `Accept-Language` locale.
    pub language: Option<String>,
    /// User-agent override.
    pub user_agent: Option<String>,
}

impl ChromiumConfiguration {
    /// Creates configuration for an initial URL.
    #[must_use]
    pub fn new(url: impl Into<Url>) -> Self {
        Self {
            url: Some(url.into()),
            ..Self::default()
        }
    }

    /// Selects a Chromium profile.
    #[must_use]
    pub fn profile(mut self, profile: ChromiumProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Sets a Chromium proxy server.
    #[must_use]
    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// Sets the preferred Chromium locale.
    #[must_use]
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Overrides the Chromium user agent.
    #[must_use]
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }
}

/// Presentation mode of a Chromium page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageMode {
    /// Page owns a visible GPU presenter.
    Visible,
    /// Page has no presenter and is controlled through CDP.
    Headless,
}

/// Chromium page lifecycle events.
#[derive(Debug, Clone, PartialEq)]
pub enum ChromiumEvent {
    /// Browser creation completed and CDP is attached.
    Ready,
    /// Main-frame navigation started.
    NavigationStarted(Url),
    /// Main-frame loading progress changed.
    Loading(f32),
    /// Main-frame navigation completed.
    Loaded(Url),
    /// The Chromium renderer process terminated.
    RendererTerminated(String),
    /// The page closed.
    Closed,
}

/// Encoded screenshot format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotFormat {
    /// Portable Network Graphics.
    Png,
    /// JPEG image with the specified quality.
    Jpeg(u8),
    /// WebP image with the specified quality.
    WebP(u8),
}

/// Concrete Chromium page implemented by the CEF runtime.
pub trait ChromiumPageHandle: Clone + 'static {
    /// CDP transport associated with this page.
    type Cdp: CustomCdpSession;

    /// Returns whether this page owns a visible presenter.
    fn mode(&self) -> PageMode;
    /// Returns the page CDP transport.
    fn cdp(&self) -> Self::Cdp;
    /// Navigates the main frame.
    fn navigate(&self, url: &Url);
    /// Navigates backward.
    fn go_back(&self);
    /// Navigates forward.
    fn go_forward(&self);
    /// Reloads the page.
    fn reload(&self);
    /// Stops loading.
    fn stop(&self);
    /// Selects the download directory.
    fn set_download_directory(&self, directory: &Path);
    /// Watches lifecycle events.
    fn watch(&self, watcher: impl Fn(ChromiumEvent) + 'static);
    /// Captures the page through Chromium.
    fn screenshot(
        &self,
        format: ScreenshotFormat,
    ) -> impl Future<Output = Result<Vec<u8>, CdpError>> + 'static;
    /// Closes the page and resolves after CEF confirms destruction.
    fn close(&self) -> impl Future<Output = Result<(), CdpError>> + 'static;
}

trait ChromiumPageHandleImpl: Any {
    fn mode(&self) -> PageMode;
    fn cdp(&self) -> CdpSession;
    fn navigate(&self, url: &Url);
    fn go_back(&self);
    fn go_forward(&self);
    fn reload(&self);
    fn stop(&self);
    fn set_download_directory(&self, directory: &Path);
    fn watch(&self, watcher: Box<dyn Fn(ChromiumEvent)>);
    fn screenshot(
        &self,
        format: ScreenshotFormat,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, CdpError>>>>;
    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), CdpError>>>>;
}

impl<T: ChromiumPageHandle> ChromiumPageHandleImpl for T {
    fn mode(&self) -> PageMode {
        ChromiumPageHandle::mode(self)
    }

    fn cdp(&self) -> CdpSession {
        CdpSession::new(ChromiumPageHandle::cdp(self))
    }

    fn navigate(&self, url: &Url) {
        ChromiumPageHandle::navigate(self, url);
    }

    fn go_back(&self) {
        ChromiumPageHandle::go_back(self);
    }

    fn go_forward(&self) {
        ChromiumPageHandle::go_forward(self);
    }

    fn reload(&self) {
        ChromiumPageHandle::reload(self);
    }

    fn stop(&self) {
        ChromiumPageHandle::stop(self);
    }

    fn set_download_directory(&self, directory: &Path) {
        ChromiumPageHandle::set_download_directory(self, directory);
    }

    fn watch(&self, watcher: Box<dyn Fn(ChromiumEvent)>) {
        ChromiumPageHandle::watch(self, watcher);
    }

    fn screenshot(
        &self,
        format: ScreenshotFormat,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, CdpError>>>> {
        Box::pin(ChromiumPageHandle::screenshot(self, format))
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), CdpError>>>> {
        Box::pin(ChromiumPageHandle::close(self))
    }
}

/// Type-erased page handle used by renderer integrations.
#[derive(Clone)]
pub struct AnyChromiumPageHandle {
    inner: Rc<dyn ChromiumPageHandleImpl>,
}

impl_debug!(AnyChromiumPageHandle);

impl AnyChromiumPageHandle {
    /// Erases a concrete page handle.
    #[must_use]
    pub fn new(handle: impl ChromiumPageHandle) -> Self {
        Self {
            inner: Rc::new(handle),
        }
    }

    /// Downcasts to a concrete engine page.
    #[must_use]
    pub fn downcast_ref<T: ChromiumPageHandle>(&self) -> Option<&T> {
        (self.inner.as_ref() as &dyn Any).downcast_ref::<T>()
    }
}

/// Visible or headless Chromium page with navigation, lifecycle, screenshot,
/// download, and CDP access.
#[derive(Clone)]
pub struct ChromiumPage {
    handle: AnyChromiumPageHandle,
    cdp: CdpSession,
}

impl_debug!(ChromiumPage);

impl ChromiumPage {
    pub(crate) fn new(handle: AnyChromiumPageHandle) -> Self {
        let cdp = handle.inner.cdp();
        Self { handle, cdp }
    }

    /// Returns the page presentation mode.
    #[must_use]
    pub fn mode(&self) -> PageMode {
        self.handle.inner.mode()
    }

    /// Returns the page CDP session.
    #[must_use]
    pub const fn cdp(&self) -> &CdpSession {
        &self.cdp
    }

    /// Navigates the main frame.
    pub fn navigate(&self, url: &Url) {
        self.handle.inner.navigate(url);
    }

    /// Navigates backward.
    pub fn go_back(&self) {
        self.handle.inner.go_back();
    }

    /// Navigates forward.
    pub fn go_forward(&self) {
        self.handle.inner.go_forward();
    }

    /// Reloads the page.
    pub fn reload(&self) {
        self.handle.inner.reload();
    }

    /// Stops the current load.
    pub fn stop(&self) {
        self.handle.inner.stop();
    }

    /// Selects the download directory.
    pub fn set_download_directory(&self, directory: &Path) {
        self.handle.inner.set_download_directory(directory);
    }

    /// Watches lifecycle events.
    pub fn watch(&self, watcher: impl Fn(ChromiumEvent) + 'static) {
        self.handle.inner.watch(Box::new(watcher));
    }

    /// Captures the page through Chromium.
    #[expect(
        clippy::future_not_send,
        reason = "CEF screenshots are browser-thread-affine"
    )]
    pub fn screenshot(
        &self,
        format: ScreenshotFormat,
    ) -> impl Future<Output = Result<Vec<u8>, CdpError>> + 'static {
        self.handle.inner.screenshot(format)
    }

    /// Closes the page.
    #[expect(
        clippy::future_not_send,
        reason = "CEF page destruction is browser-thread-affine"
    )]
    pub fn close(&self) -> impl Future<Output = Result<(), CdpError>> + 'static {
        self.handle.inner.close()
    }

    /// Returns the underlying page handle for renderer integration.
    #[must_use]
    pub const fn handle(&self) -> &AnyChromiumPageHandle {
        &self.handle
    }
}

/// Imperative page handle injectable into controls adjacent to a visible view.
#[derive(Debug, Clone)]
pub struct ChromiumProxy(ChromiumPage);

impl ChromiumProxy {
    /// Returns the page.
    #[must_use]
    pub const fn page(&self) -> &ChromiumPage {
        &self.0
    }
}

impl Extractor for ChromiumProxy {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>().cloned().ok_or_else(|| {
            Error::msg(format!(
                "{} not found in environment — wrap the visible page with \
                 `ChromiumView::with_proxy(|| ...)` to inject it",
                type_name::<Self>()
            ))
        })
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}

/// Visible raw Chromium component rendered by CEF-capable backends.
#[derive(Clone)]
pub struct ChromiumView {
    page: ChromiumPage,
}

impl_debug!(ChromiumView);

impl ChromiumView {
    pub(crate) const fn new(page: ChromiumPage) -> Self {
        Self { page }
    }

    /// Returns the visible page.
    #[must_use]
    pub const fn page(&self) -> &ChromiumPage {
        &self.page
    }

    /// Injects an imperative proxy into adjacent content.
    pub fn with_proxy<V, F>(self, content: F) -> impl View
    where
        V: View,
        F: FnOnce() -> V + 'static,
    {
        use waterui_core::env::with;
        use waterui_layout::stack::vstack;
        let proxy = ChromiumProxy(self.page.clone());
        vstack((with(content(), proxy), self))
    }
}

// A backend that hosts Chromium natively takes `ChromiumView` by type before
// `View::body` runs; the CEF runtime an application links installs a
// `Hook<ChromiumView>` that draws the page into a GPU surface. `raw_view!` would
// hard-code the second half away.
impl NativeView for ChromiumView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// What a build with neither a native host nor a linked Chromium runtime draws.
///
/// A visible Chromium page cannot be approximated, so this is the panicking
/// `Native` leaf `raw_view!` produced: asking for Chromium without a runtime is
/// a configuration error, not a missing pixel.
impl ViewConfiguration for ChromiumView {
    type View = Native<Self>;

    fn render(self) -> Self::View {
        Native::new(self)
    }
}

impl View for ChromiumView {
    fn body(self, env: &Environment) -> impl View {
        if let Some(hook) = env.get::<Hook<Self>>() {
            AnyView::new(hook.apply(env, self))
        } else {
            AnyView::new(self.render())
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Deferred visible Chromium component.
#[derive(Debug, Clone)]
pub struct Chromium {
    configuration: ChromiumConfiguration,
}

impl Chromium {
    /// Creates a visible Chromium component with general configuration.
    #[must_use]
    pub const fn new(configuration: ChromiumConfiguration) -> Self {
        Self { configuration }
    }
}

impl View for Chromium {
    fn body(self, _env: &Environment) -> impl View {
        use_env(move |controller: ChromiumController| controller.open(self.configuration))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Creates a visible Chromium component for a URL.
#[must_use]
pub fn chromium(url: impl Into<Url>) -> Chromium {
    Chromium::new(ChromiumConfiguration::new(url))
}
