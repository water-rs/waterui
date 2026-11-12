//! The Android window handles wgpu creates surfaces from: one display
//! handle and one `ANativeWindow` lease, shared by the GPU painter's band
//! swapchains and the HWUI painter's hosted GPU-content surfaces.

use ndk::native_window::NativeWindow;
use raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, DisplayHandle, HasDisplayHandle, HasWindowHandle,
    RawDisplayHandle, RawWindowHandle, WindowHandle,
};

/// The Android display's raw handle: the platform has one implicit default
/// display, which `AndroidDisplayHandle` names.
#[derive(Debug)]
pub(super) struct AndroidDisplay;

impl HasDisplayHandle for AndroidDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: Android's display handle is a static system resource —
        // borrowing it raw is always sound for the process lifetime.
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Android(AndroidDisplayHandle::new()))
        })
    }
}

/// An owned `ANativeWindow` lease as the window a wgpu surface presents to.
pub(super) struct AndroidWindow(pub(super) NativeWindow);

impl HasWindowHandle for AndroidWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(self.0.ptr().cast()));
        // SAFETY: the wrapper holds its own reference to the native
        // window, so the pointer stays valid for the borrow.
        Ok(unsafe { WindowHandle::borrow_raw(handle) })
    }
}

impl HasDisplayHandle for AndroidWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, raw_window_handle::HandleError> {
        AndroidDisplay.display_handle()
    }
}
