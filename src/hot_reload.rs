use std::path::{Path, PathBuf};

use async_channel::Receiver;
use executor_core::spawn_local;
use waterui_core::{
    AnyView, View,
    components::{Dynamic, dynamic::DynamicHandler},
};
use waterui_layout::stack::vstack;

use crate::prelude::loading;

/// A view that can be hot-reloaded at runtime.
#[derive(Debug)]
pub struct Hotreload {
    dynamic: Dynamic,
}

fn preparing_view() -> impl View {
    vstack((loading(), "Preparing for hot reload..."))
}

type ReloadableView = extern "C" fn() -> *mut ();

unsafe fn load_new_version(
    function_name: &'static str,
    crate_dir: &Path,
    target_dir: &Path,
    crate_name: &str,
) -> AnyView {
    unsafe {
        // Load the dynamic library
        log::debug!(
            "Loading dynamic library from {}",
            current_dylib_path(target_dir, crate_name).display()
        );
        let lib = libloading::Library::new(current_dylib_path(target_dir, crate_name)).unwrap();

        // Get a symbol from the library
        let func: libloading::Symbol<ReloadableView> = lib.get(function_name.as_bytes()).unwrap();
        let boxed: Box<AnyView> = Box::from_raw(func().cast::<AnyView>());
        *boxed
    }
}

// target_dir:
fn current_dylib_path(target_dir: &Path, crate_name: &str) -> PathBuf {
    let mut dir = target_dir.to_path_buf();
    if cfg!(debug_assertions) {
        dir.push("debug");
    } else {
        dir.push("release");
    }

    let libname = crate_name;

    let filename = if cfg!(target_os = "windows") {
        format!("{libname}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{libname}.dylib")
    } else {
        format!("lib{libname}.so")
    };

    dir.push(filename);
    dir
}

fn start_hot_reload(
    function_name: &'static str,
    dir: PathBuf,
    crate_name: &'static str,
    target_dir: PathBuf,
    handler: DynamicHandler,
) {
    log::debug!("Watching directory: {dir:?}");
    let receiver = observe_directory_changes(&dir);

    spawn_local(async move {
        while receiver.recv().await == Ok(()) {
            log::debug!("Change detected, recompiling...");
            recompile(&dir);
            log::debug!("Recompilation finished, reloading...");

            let new_view =
                unsafe { load_new_version(function_name, &dir, &target_dir, crate_name) };
            handler.set(new_view);
            log::debug!("Reloaded new version of the view.");

            // Update the dynamic view
            // Note: This requires that you have access to the Dynamic instance
            // Here we assume you have a way to get it, e.g., through a global state or passed reference
            // For demonstration, we will just print a message
            // In practice, you would call something like `dynamic.set(new_view);`
            log::debug!("Dynamic view should be updated here.");
        }
    });
}

fn observe_directory_changes(dir: &Path) -> Receiver<()> {
    let (sender, receiver) = async_channel::unbounded();

    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        use notify::{RecommendedWatcher, RecursiveMode, Watcher};
        use std::sync::mpsc::channel;

        let (tx, rx) = channel();

        let mut watcher: RecommendedWatcher = Watcher::new(tx, notify::Config::default()).unwrap();

        watcher.watch(&dir, RecursiveMode::Recursive).unwrap();

        for res in rx {
            match res {
                Ok(event) => {
                    log::debug!("Change detected: {event:?}");
                    // Notify the async channel about the change
                    let _ = sender.try_send(());
                }
                Err(e) => log::error!("watch error: {e:?}"),
            }
        }
    });

    receiver
}

fn recompile(dir: &Path) {
    let cargo = env!("CARGO");

    // Trigger recompilation using cargo
    let status = std::process::Command::new(cargo)
        .current_dir(dir)
        .args(["build"])
        .env(
            "PATH",
            format!(
                "{}/.cargo/bin:{}",
                std::env::var("HOME").unwrap(),
                std::env::var("PATH").unwrap()
            ),
        )
        .status()
        .expect("Failed to execute cargo build");

    if !status.success() {
        log::debug!("Recompilation failed");
    }
}

impl Hotreload {
    /// Creates a new `Hotreload` view that can be updated at runtime.
    ///
    /// # Safety
    ///
    /// The `function_name` must be the name of an `extern "C"` function
    /// with the signature `fn() -> *mut ()` which returns a raw pointer of `Box<AnyView>`
    /// that has been converted to a raw pointer using `Box::into_raw`.
    pub unsafe fn new(
        function_name: &'static str,
        dir: PathBuf,
        target_dir: PathBuf,
        crate_name: &'static str,
    ) -> Self {
        let (handler, dynamic) = Dynamic::new();
        handler.set(preparing_view);
        start_hot_reload(function_name, dir, crate_name, target_dir, handler);
        Self { dynamic }
    }
}

#[macro_export]
macro_rules! hot_reload {
    ($function_name:expr) => {{
        let dir = std::path::PathBuf::from(
            option_env!("CARGO_WORKSPACE_DIR").unwrap_or(env!("CARGO_MANIFEST_DIR")),
        );
        let crate_name = env!("CARGO_PKG_NAME");
        let target_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target");
        $crate::hot_reload::Hotreload::new($function_name, dir, target_dir, crate_name)
    }};
}

impl View for Hotreload {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.dynamic
    }
}
