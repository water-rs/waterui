//! File picker component configuration.

use alloc::{string::ToString, vec, vec::Vec};
use nami::Binding;
use waterui_controls::{Button, IntoLabel};
use waterui_core::View;
use waterui_text::Text;
use waterui_url::Url;

#[cfg(feature = "std")]
use waterkit_dialog::FileDialog;

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

impl<Label: IntoLabel + 'static> View for FilePicker<Label> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Button::new(self.label).action_async(move || {
            let value = self.value.clone();
            async move {
                #[cfg(feature = "std")]
                {
                    let dialog = if self.import {
                        FileDialog::new().import_to_cache_subdir("waterui/file-picker-imports")
                    } else {
                        FileDialog::new()
                    };
                    let selected_paths: Option<Vec<std::path::PathBuf>> = match if self.num <= 1 {
                        dialog
                            .show_open_single_file()
                            .await
                            .map(|path| path.map(|path| vec![path]))
                    } else {
                        dialog.show_open_multiple_files().await
                    } {
                        Ok(paths) => paths,
                        Err(error) => {
                            tracing::warn!("FilePicker failed to present file dialog: {error}");
                            return;
                        }
                    };

                    let Some(selected_paths) = selected_paths else {
                        return;
                    };
                    assert!(
                        !selected_paths.is_empty(),
                        "FilePicker dialog returned an empty selection"
                    );

                    let mut urls = Vec::with_capacity(selected_paths.len());
                    for path in selected_paths.into_iter().take(self.num.max(1)) {
                        urls.push(Url::from_file_path_str(path.to_string_lossy().to_string()));
                    }
                    value.set(urls);
                }

                #[cfg(not(feature = "std"))]
                {
                    panic!("FilePicker requires the `std` feature to open native file dialogs");
                }
            }
        })
    }
}
