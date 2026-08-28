//! File picker component configuration.

use alloc::vec::Vec;
// The native dialog hands back `std::path::PathBuf`, so the picker needs `std`
// linked as well as a target that has a dialog to present.
#[cfg(all(
    feature = "std",
    not(any(target_os = "espidf", target_arch = "wasm32"))
))]
use alloc::{string::ToString, vec};
use nami::Binding;
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::{Button, IntoLabel};
use waterui_core::View;
use waterui_url::Url;

#[cfg(all(
    feature = "std",
    not(any(target_os = "espidf", target_arch = "wasm32"))
))]
use waterkit_dialog::FileDialog;

/// Configuration for a file picker component.
#[derive(Debug, Clone)]
pub struct FilePicker {
    label: Label,
    /// The selected file URLs.
    value: Binding<Vec<Url>>,
    /// The maximum number of files that can be selected.
    num: usize,
    /// Copy these files to the app's sandboxed storage.
    import: bool,
}

impl FilePicker {
    /// Select files without importing them, with the given semantic label
    /// driving the picker button.
    ///
    /// You will get URLs that point to the original file locations.
    #[must_use]
    pub fn open(label: impl IntoLabel, value: &Binding<Vec<Url>>) -> Self {
        Self {
            label: label.into_label(),
            value: value.clone(),
            num: 1,
            import: false,
        }
    }

    /// Select files and import them into the app's sandboxed storage, with
    /// the given semantic label driving the picker button.
    #[must_use]
    pub fn import(label: impl IntoLabel, value: &Binding<Vec<Url>>) -> Self {
        Self {
            label: label.into_label(),
            value: value.clone(),
            num: 1,
            import: true,
        }
    }

    /// Set the maximum number of files that can be selected.
    ///
    /// One file can be selected by default.
    #[must_use]
    pub fn max_count(mut self, value: usize) -> Self {
        debug_assert!(value >= 1, "max_count must be at least 1");
        self.num = value;
        self
    }

    /// Sets the visual presentation mode of the picker label. The semantic
    /// identity is always retained for assistive technology.
    #[must_use]
    pub const fn label_style(mut self, mode: LabelDisplayMode) -> Self {
        self.label.set_display_mode(mode);
        self
    }

    /// Visually hides the picker label while preserving its semantic text for
    /// assistive technology.
    #[must_use]
    pub const fn hide_label(self) -> Self {
        self.label_style(LabelDisplayMode::Hidden)
    }
}

impl View for FilePicker {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Button::new(self.label).action_async(move || {
            let value = self.value.clone();
            async move {
                #[cfg(all(
                    feature = "std",
                    not(any(target_os = "espidf", target_arch = "wasm32"))
                ))]
                {
                    let dialog = if self.import {
                        FileDialog::new().with_import_to_cache("waterui/file-picker-imports")
                    } else {
                        FileDialog::new()
                    };
                    let selected_paths: Option<Vec<std::path::PathBuf>> = match if self.num <= 1 {
                        dialog
                            .pick_single()
                            .await
                            .map(|path| path.map(|path| vec![path]))
                    } else {
                        dialog.pick_multiple().await
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

                #[cfg(not(all(
                    feature = "std",
                    not(any(target_os = "espidf", target_arch = "wasm32"))
                )))]
                {
                    let _ = value;
                    panic!(
                        "FilePicker native file dialog is unavailable on this target (import={}, max_count={})",
                        self.import, self.num
                    );
                }
            }
        })
    }
}
