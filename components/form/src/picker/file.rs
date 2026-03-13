//! File picker component configuration.

use alloc::{string::ToString, vec::Vec};
use nami::Binding;
use waterui_controls::{Button, IntoLabel};
use waterui_core::View;
use waterui_text::Text;
use waterui_url::Url;

#[cfg(feature = "std")]
use {
    alloc::{borrow::Cow, format},
    std::{
        ffi::OsStr,
        io,
        path::{Path, PathBuf},
    },
    waterkit_dialog::FileDialog,
};

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
                    let max_num = self.num.max(1);
                    let mut urls = Vec::with_capacity(max_num);

                    for _ in 0..max_num {
                        let path = match FileDialog::new().show_open_single_file().await {
                            Ok(path) => path,
                            Err(error) => {
                                std::eprintln!("FilePicker failed to present file dialog: {error}");
                                return;
                            }
                        };

                        let Some(path) = path else {
                            break;
                        };

                        let selected_path = if self.import {
                            match import_to_sandbox(&path) {
                                Ok(imported) => imported,
                                Err(error) => {
                                    std::eprintln!(
                                        "FilePicker failed to import selected file: {error}"
                                    );
                                    return;
                                }
                            }
                        } else {
                            path
                        };

                        urls.push(Url::from_file_path_str(
                            selected_path.to_string_lossy().to_string(),
                        ));
                    }

                    // User cancel should preserve previous selection.
                    if urls.is_empty() {
                        return;
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

#[cfg(feature = "std")]
fn import_to_sandbox(path: &Path) -> io::Result<PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "selected path has no file name",
        )
    })?;

    let base_dir = std::env::temp_dir()
        .join("waterui")
        .join("file-picker-imports");
    std::fs::create_dir_all(&base_dir)?;

    let mut destination = base_dir.join(file_name);
    if destination.exists() {
        let stem = path
            .file_stem()
            .map_or_else(|| Cow::Borrowed("file"), |s: &OsStr| s.to_string_lossy());
        let extension = path
            .extension()
            .map(|e: &OsStr| e.to_string_lossy().to_string());

        let mut index = 1usize;
        loop {
            let candidate = extension.as_ref().map_or_else(
                || base_dir.join(format!("{stem}-{index}")),
                |ext| base_dir.join(format!("{stem}-{index}.{ext}")),
            );
            if !candidate.exists() {
                destination = candidate;
                break;
            }
            index += 1;
        }
    }

    std::fs::copy(path, &destination)?;
    Ok(destination)
}
