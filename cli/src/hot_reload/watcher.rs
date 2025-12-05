//! File system watcher for triggering hot reload rebuilds.
//!
//! This module provides an async file watching interface. The watcher uses
//! `notify` internally with a sync callback, but exposes an async receiver
//! for use with `tokio::select!`.

use std::{collections::HashSet, path::PathBuf, time::Duration};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::debug;

/// Event emitted when source files change.
#[derive(Debug, Clone)]
pub struct FileChanged;

/// Async file watcher that emits debounced change events.
///
/// # Example
///
/// ```ignore
/// let mut watcher = FileWatcher::new(vec![src_dir])?;
///
/// loop {
///     tokio::select! {
///         Some(FileChanged) = watcher.recv() => {
///             rebuild().await;
///         }
///         // ... other branches
///     }
/// }
/// ```
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<FileChanged>,
    debounce: Duration,
}

impl std::fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWatcher")
            .field("debounce", &self.debounce)
            .finish_non_exhaustive()
    }
}

impl FileWatcher {
    /// Create a new file watcher.
    ///
    /// # Arguments
    /// * `watch_paths` - Directories to watch for changes
    ///
    /// # Errors
    /// Returns an error if the file watcher cannot be initialized.
    pub fn new(watch_paths: Vec<PathBuf>) -> notify::Result<Self> {
        // Use a bounded channel to avoid unbounded memory growth
        let (tx, rx) = mpsc::channel(16);

        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                    ) {
                        // try_send avoids blocking the notify callback
                        let _ = tx.try_send(FileChanged);
                    }
                }
            })?;

        let mut seen = HashSet::new();
        for path in watch_paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            if path.exists() {
                watcher.watch(&path, RecursiveMode::Recursive)?;
            } else {
                debug!("Skipping hot reload path (not found): {}", path.display());
            }
        }

        Ok(Self {
            _watcher: watcher,
            rx,
            debounce: Duration::from_millis(250),
        })
    }

    /// Receive the next debounced file change event.
    ///
    /// This method waits for a file change event, then debounces by waiting
    /// for the debounce duration and draining any additional events that
    /// arrived during that time.
    ///
    /// Returns `None` if the watcher has been dropped.
    pub async fn recv(&mut self) -> Option<FileChanged> {
        // Wait for at least one event
        self.rx.recv().await?;

        // Debounce: wait a bit and drain any additional events
        tokio::time::sleep(self.debounce).await;

        // Drain pending events (they're part of the same "batch")
        while self.rx.try_recv().is_ok() {}

        Some(FileChanged)
    }
}
