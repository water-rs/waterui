//! A view that can be hot-reloaded at runtime.

use executor_core::{Task, spawn_local};
use libloading::Library;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    thread::spawn,
};
use thiserror::Error;
use tungstenite::connect;
use waterui_core::{AnyView, Dynamic, View, event::Associated};

/// A view that can be hot-reloaded at runtime.
#[derive(Debug)]
pub struct Hotreload<V> {
    initial: V,
}

impl<V: View> Hotreload<V> {
    /// Creates a new `Hotreload` view with the specified initial view.
    pub const fn new(initial: V) -> Self {
        Self { initial }
    }
}

impl<V: View> View for Hotreload<V> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let (sender, receiver) = async_channel::unbounded::<PathBuf>();
        let (handler, dynamic) = Dynamic::new();
        handler.set(self.initial);

        let trigger = HotReloadTrigger::new(sender);

        spawn_local(async move {
            while let Ok(path) = receiver.recv().await {
                let new_view = unsafe { reload("waterui_main", &path) };
                handler.set(new_view);
            }
        })
        .detach();

        // Start the hot-reload daemon in a separate thread
        spawn(move || hot_reload_daemon(&trigger).expect("Fail to launch hot reload daemon"));

        dynamic
    }
}

#[derive(Debug, Error)]
enum Error {
    #[error("Failed to connect to hot-reload server: {0}")]
    FailedToConnect(#[from] tungstenite::Error),
    #[error("Hot reload port not set")]
    HotReloadPortNotSet,
}

// you must call this function on main thread, otherwise it is UB
unsafe fn reload(symbol: &str, path: &Path) -> Associated<Library, AnyView> {
    let lib = unsafe { Library::new(path) }.unwrap();
    let func: libloading::Symbol<unsafe extern "C" fn() -> *mut waterui_core::AnyView> =
        unsafe { lib.get(symbol.as_bytes()) }.expect("Failed to load symbol");
    let view_ptr = unsafe { func() };
    let view = unsafe { Box::from_raw(view_ptr) };
    Associated::new(lib, *view)
}

#[derive(Debug, Clone)]
struct HotReloadTrigger {
    sender: async_channel::Sender<PathBuf>,
}

impl HotReloadTrigger {
    pub const fn new(sender: async_channel::Sender<PathBuf>) -> Self {
        Self { sender }
    }

    pub fn trigger_reload(&self, path: PathBuf) {
        let _ = self.sender.send_blocking(path);
    }
}

fn hot_reload_daemon(trigger: &HotReloadTrigger) -> Result<(), Error> {
    // The app is the client, connecting to the CLI.
    let port = std::env::var("WATERUI_HOT_RELOAD_PORT").map_err(|_| Error::HotReloadPortNotSet)?;
    let url = format!("ws://127.0.0.1:{port}/reload");

    let (mut socket, _) = connect(url)?;

    while let Ok(msg) = socket.read() {
        if msg.is_binary() {
            let data = msg.into_data();
            let lib_path = create_library(&data);
            trigger.trigger_reload(lib_path);
        }
    }

    Ok(())
}

fn library_name() -> String {
    // generate a uuid for the library name
    let uuid = uuid::Uuid::new_v4();
    format!("waterui_hot_{uuid}")
}

fn create_library(data: &[u8]) -> PathBuf {
    // all hot reload libraries are created under ./hot_reload/ directory
    let hot_reload_dir = Path::new("hot_reload");
    if !hot_reload_dir.exists() {
        std::fs::create_dir(hot_reload_dir).expect("Failed to create hot_reload directory");
    }

    let lib_name = library_name();
    let mut lib_path = hot_reload_dir.join(lib_name);
    // do not forget to add the proper extension for the library
    if cfg!(target_os = "windows") {
        lib_path.set_extension("dll");
    } else if cfg!(target_os = "macos") {
        lib_path.set_extension("dylib");
    } else {
        lib_path.set_extension("so");
    }

    let mut file = File::create(&lib_path).expect("Failed to create hot reload library");
    file.write_all(data)
        .expect("Failed to write to hot reload library");
    lib_path
}
