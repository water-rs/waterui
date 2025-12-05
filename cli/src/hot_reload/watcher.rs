//! File system watcher for triggering hot reload rebuilds.
//!
//! This module provides event-based file watching instead of callback-based.
//! The watcher emits events through a channel, and the caller is responsible
//! for handling them in their event loop.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::debug;

/// Event emitted when source files change.
#[derive(Debug, Clone)]
pub struct FileChanged;

/// Watches source files and emits events when changes are detected.
///
/// The watcher debounces rapid changes internally to avoid emitting
/// multiple events for a single save operation.
///
/// # Example
///
/// ```ignore
/// let (watcher, rx) = FileWatcher::new(vec![src_dir])?;
///
/// loop {
///     match rx.try_recv() {
///         Ok(FileChanged) => rebuild(),
///         Err(TryRecvError::Empty) => {},
///         Err(TryRecvError::Disconnected) => break,
///     }
/// }
/// ```
pub struct FileWatcher {
    // Note: RecommendedWatcher doesn't implement Debug, so manual impl is not practical
    _watcher: RecommendedWatcher,
    last_event: Instant,
    debounce: Duration,
}

impl std::fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWatcher")
            .field("last_event", &self.last_event)
            .field("debounce", &self.debounce)
            .finish_non_exhaustive()
    }
}

impl FileWatcher {
    /// Create a new file watcher.
    ///
    /// Returns the watcher and a receiver for change events. The watcher
    /// must be kept alive for events to be emitted.
    ///
    /// # Arguments
    /// * `watch_paths` - Directories to watch for changes
    ///
    /// # Errors
    /// Returns an error if the file watcher cannot be initialized.
    pub fn new(watch_paths: Vec<PathBuf>) -> notify::Result<(Self, Receiver<FileChanged>)> {
        let (tx, rx) = mpsc::channel();

        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                    ) {
                        let _ = tx.send(FileChanged);
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

        Ok((
            Self {
                _watcher: watcher,
                last_event: Instant::now() - Duration::from_secs(1), // Allow immediate first event
                debounce: Duration::from_millis(250),
            },
            rx,
        ))
    }

    /// Check if enough time has passed since the last event to trigger a rebuild.
    ///
    /// Call this when you receive a `FileChanged` event to determine if it
    /// should be acted upon or debounced.
    pub fn should_rebuild(&mut self) -> bool {
        if self.last_event.elapsed() < self.debounce {
            return false;
        }
        self.last_event = Instant::now();
        true
    }
}

/// Convenience wrapper that drains and debounces a file change receiver.
///
/// Returns `true` if there are pending changes that should trigger a rebuild.
pub fn poll_file_changes(rx: &Receiver<FileChanged>, watcher: &mut FileWatcher) -> bool {
    let mut has_changes = false;

    // Drain all pending events
    loop {
        match rx.try_recv() {
            Ok(FileChanged) => has_changes = true,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }

    has_changes && watcher.should_rebuild()
}
