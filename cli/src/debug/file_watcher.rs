//! File system watcher for hot reload.

use std::path::Path;
use std::sync::mpsc;
use std::time::SystemTime;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use smol::channel::{self, Receiver};

/// Watches source files for changes and emits events.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    rx: Receiver<()>,
}

impl std::fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWatcher").finish_non_exhaustive()
    }
}

impl FileWatcher {
    /// Create a new file watcher for the given project directory.
    ///
    /// Watches the `src/` directory for `.rs` file changes.
    ///
    /// # Errors
    /// Returns an error if the watcher cannot be created.
    pub fn new(project_path: &Path) -> notify::Result<Self> {
        let (tx, rx) = channel::unbounded();

        // Create a sync channel for notify (which uses std::sync::mpsc)
        let (sync_tx, sync_rx) = mpsc::channel::<notify::Result<Event>>();

        // Spawn a task to bridge sync channel to async channel
        let tx_clone = tx;
        let started_at = SystemTime::now();
        std::thread::spawn(move || {
            while let Ok(event) = sync_rx.recv() {
                if let Ok(event) = event {
                    // Only trigger on Rust file modifications
                    if is_relevant_change(&event, started_at) {
                        let _ = tx_clone.send_blocking(());
                    }
                }
            }
        });

        let watcher = notify::recommended_watcher(move |res| {
            let _ = sync_tx.send(res);
        })?;

        let mut file_watcher = Self { watcher, rx };

        // Watch src directory
        let src_path = project_path.join("src");
        if src_path.exists() {
            file_watcher
                .watcher
                .watch(&src_path, RecursiveMode::Recursive)?;
        }

        Ok(file_watcher)
    }

    /// Returns a receiver for file change events.
    ///
    /// Each receive indicates that source files have changed and a rebuild may be needed.
    #[must_use]
    pub const fn receiver(&self) -> &Receiver<()> {
        &self.rx
    }
}

/// Check if the event is a relevant change (Rust source file modification).
fn is_relevant_change(event: &Event, started_at: SystemTime) -> bool {
    use notify::{EventKind, event::ModifyKind};

    // Only care about changes that can affect a build. On macOS it's common to receive follow-up
    // metadata-only modifications for a save; ignore those to avoid redundant rebuilds.
    let kind = &event.kind;
    let is_relevant_kind = match kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(modify_kind) => !matches!(modify_kind, ModifyKind::Metadata(_)),
        _ => false,
    };

    if !is_relevant_kind {
        return false;
    }

    event
        .paths
        .iter()
        .any(|path| is_relevant_path(path, *kind, started_at))
}

fn is_relevant_path(path: &Path, kind: notify::EventKind, started_at: SystemTime) -> bool {
    use notify::{EventKind, event::ModifyKind};

    if !path
        .extension()
        .is_some_and(|ext| ext == "rs" || ext == "toml")
    {
        return false;
    }

    // Deletions are always relevant.
    if matches!(kind, EventKind::Remove(_)) {
        return true;
    }

    // Renames don't necessarily update the mtime; treat them as relevant if they touch a watched
    // file path.
    if matches!(kind, EventKind::Modify(ModifyKind::Name(_))) {
        return true;
    }

    // Some backends can emit initial "create/modify" events for pre-existing files when a watch
    // is first installed. Filter those out by requiring the file to have been modified after we
    // started watching.
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_or(true, |modified| modified > started_at)
}
