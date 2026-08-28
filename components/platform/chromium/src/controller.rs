use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use waterui_core::{impl_debug, impl_extractor};

use crate::{
    AnyChromiumPageHandle, ChromiumConfiguration, ChromiumPage, ChromiumPageHandle, ChromiumView,
};

/// Backend factory for visible and headless Chromium pages.
pub trait CustomChromiumController: 'static {
    /// Concrete page handle returned by this engine.
    type Page: ChromiumPageHandle;

    /// Creates a visible page synchronously. The handle may queue commands until
    /// CEF signals browser creation.
    fn open(&self, configuration: ChromiumConfiguration) -> Self::Page;

    /// Creates a headless page and resolves when its `DevTools` agent is attached.
    fn headless(
        &self,
        configuration: ChromiumConfiguration,
    ) -> impl Future<Output = Result<Self::Page, crate::CdpError>> + 'static;
}

trait ChromiumControllerImpl {
    fn open(&self, configuration: ChromiumConfiguration) -> AnyChromiumPageHandle;
    fn headless(
        &self,
        configuration: ChromiumConfiguration,
    ) -> Pin<Box<dyn Future<Output = Result<AnyChromiumPageHandle, crate::CdpError>>>>;
}

impl<T: CustomChromiumController> ChromiumControllerImpl for T {
    fn open(&self, configuration: ChromiumConfiguration) -> AnyChromiumPageHandle {
        AnyChromiumPageHandle::new(CustomChromiumController::open(self, configuration))
    }

    fn headless(
        &self,
        configuration: ChromiumConfiguration,
    ) -> Pin<Box<dyn Future<Output = Result<AnyChromiumPageHandle, crate::CdpError>>>> {
        let future = CustomChromiumController::headless(self, configuration);
        Box::pin(async move { future.await.map(AnyChromiumPageHandle::new) })
    }
}

/// Environment-injected Chromium runtime controller.
#[derive(Clone)]
pub struct ChromiumController {
    controller: Rc<dyn ChromiumControllerImpl>,
}

impl_debug!(ChromiumController);
impl_extractor!(ChromiumController);

impl ChromiumController {
    /// Creates a controller from a concrete CEF engine.
    #[must_use]
    pub fn new(controller: impl CustomChromiumController) -> Self {
        Self {
            controller: Rc::new(controller),
        }
    }

    /// Opens a visible Chromium page.
    #[must_use]
    pub fn open(&self, configuration: ChromiumConfiguration) -> ChromiumView {
        let page = ChromiumPage::new(self.controller.open(configuration));
        ChromiumView::new(page)
    }

    /// Creates a headless page.
    ///
    /// # Errors
    ///
    /// Returns an error when the page closes or its `DevTools` agent cannot
    /// attach.
    #[expect(
        clippy::future_not_send,
        reason = "CEF page creation and DevTools attachment are browser-thread-affine"
    )]
    pub async fn headless(
        &self,
        configuration: ChromiumConfiguration,
    ) -> Result<ChromiumPage, crate::CdpError> {
        self.controller
            .headless(configuration)
            .await
            .map(ChromiumPage::new)
    }
}
