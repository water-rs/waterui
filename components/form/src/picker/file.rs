//! File picker component configuration.

use core::pin::Pin;

use alloc::{boxed::Box, rc::Rc, vec::Vec};
use nami::Binding;
use waterui_controls::Button;
use waterui_core::{View, impl_debug, impl_extractor};
use waterui_text::Text;
use waterui_url::Url;

/// Configuration for a file picker component.
#[derive(Debug, Clone)]
pub struct FilePicker<Label> {
    label: Label,
    /// The selected file URLs.
    value: Binding<Vec<Url>>,
    /// The maximum number of files that can be selected.
    num: usize,
    /// Copy these files to the app's sandboxed storage.
    import: bool,
}

impl FilePicker<Text> {
    /// Select files without importing them.
    ///
    /// You will get URLs that point to the original file locations.
    #[must_use]
    pub fn open(value: &Binding<Vec<Url>>) -> Self {
        Self {
            label: Text::new("Select Files"),
            value: value.clone(),
            num: 1,
            import: false,
        }
    }

    /// Select files and import them into the app's sandboxed storage.
    pub fn import(value: &Binding<Vec<Url>>) -> Self {
        Self {
            label: Text::new("Import Files"),
            value: value.clone(),
            num: 1,
            import: true,
        }
    }

    /// Set the maximum number of files that can be selected.
    ///
    /// One file can be selected by default.
    #[must_use]
    pub fn num(mut self, value: usize) -> Self {
        debug_assert!(value >= 1, "num must be at least 1");
        self.num = value;
        self
    }
}

/// A custom file picker controller.
pub trait CustomFilePickerController {
    /// Present the file picker UI, allowing the user to select up to `max_num` files.
    fn present(&self, import: bool, max_num: usize) -> impl Future<Output = Vec<Url>>;
}

trait CustomFilePickerControllerImpl {
    fn present<'a>(
        &'a self,
        import: bool,
        max_num: usize,
    ) -> Pin<Box<dyn 'a + Future<Output = Vec<Url>>>>;
}

impl<T: CustomFilePickerController> CustomFilePickerControllerImpl for T {
    fn present<'a>(
        &'a self,
        import: bool,
        max_num: usize,
    ) -> Pin<Box<dyn 'a + Future<Output = Vec<Url>>>> {
        Box::pin(self.present(import, max_num))
    }
}

/// A file picker controller that uses a custom implementation.
#[derive(Clone)]
pub struct FilePickerController {
    inner: Rc<dyn CustomFilePickerControllerImpl>,
}

impl_debug!(FilePickerController);

impl<Label: View> View for FilePicker<Label> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Button::new(self.label)
            .extract::<Option<FilePickerController>>()
            .action_async(move |controller| {
                let value = self.value.clone();
                async move {
                    let Some(controller) = controller else {
                        return;
                    };
                    let urls = controller.present(self.import, self.num).await;
                    value.set(urls);
                }
            })
    }
}

impl_extractor!(FilePickerController);

impl FilePickerController {
    /// Create a new file picker controller from a custom implementation.
    pub fn new<T: CustomFilePickerController + 'static>(controller: T) -> Self {
        Self {
            inner: Rc::new(controller),
        }
    }

    /// Present the file picker UI.
    pub async fn present(&self, import: bool, max_num: usize) -> Vec<Url> {
        self.inner.present(import, max_num).await
    }
}
