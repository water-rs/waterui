//! A view that can be hot-reloaded at runtime.

use std::{
    io::{BufRead, BufReader},
    net::TcpStream,
    path::Path,
    thread,
    time::Duration,
};

use crate::component::{Dynamic, dynamic::DynamicHandler};
use crate::prelude::loading;
use async_channel::Sender;
use executor_core::spawn_local;
use libloading::Library;
use waterui_core::{AnyView, View, event::Associated};
use waterui_layout::stack::vstack;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 51230;
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// A view that can be hot-reloaded at runtime.
#[derive(Debug)]
pub struct Hotreload {
    dynamic: Dynamic,
}

fn preparing_view() -> impl View {
    vstack((loading(), "Preparing for hot reload..."))
}

impl Hotreload {
    /// Creates a new `Hotreload` view that listens for updates from the `WaterUI` CLI.
    ///
    /// # Safety
    ///
    /// The `function_name` must be the name of an exported `extern "C"` function with the
    /// signature `fn() -> *mut ()` that returns a heap-allocated `AnyView` (boxed and leaked
    /// with `Box::into_raw`). The dynamic library must remain compatible with the running
    /// process.
    pub fn new(function_name: &'static str) -> Self {
        let (handler, dynamic) = Dynamic::new();
        handler.set(preparing_view);
        start_listener(handler, function_name);
        Self { dynamic }
    }
}

impl View for Hotreload {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.dynamic
    }
}

fn start_listener(handler: DynamicHandler, function_name: &'static str) {
    let (tx, rx) = async_channel::unbounded::<String>();

    thread::spawn(move || connection_loop(&tx));

    spawn_local(async move {
        while let Ok(path) = rx.recv().await {
            match load_view_from_library(Path::new(&path), function_name) {
                Ok(view) => handler.set(view),
                Err(err) => log::error!("Failed to load hot reload view: {err}"),
            }
        }
    });
}

fn connection_loop(tx: &Sender<String>) {
    loop {
        match TcpStream::connect((HOST, PORT)) {
            Ok(stream) => {
                log::info!("Connected to WaterUI CLI hot reload server on {HOST}:{PORT}");
                if let Err(err) = read_updates(stream, tx) {
                    log::warn!("Hot reload connection lost: {err}");
                }
            }
            Err(err) => {
                log::warn!(
                    "Failed to connect to WaterUI CLI hot reload server on {HOST}:{PORT}: {err}"
                );
            }
        }
        thread::sleep(RETRY_DELAY);
    }
}

fn read_updates(stream: TcpStream, tx: &Sender<String>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let path = line.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if tx.send_blocking(path).is_err() {
            break;
        }
    }
    Ok(())
}

fn load_view_from_library(path: &Path, function_name: &'static str) -> Result<AnyView, String> {
    unsafe {
        let library = Library::new(path)
            .map_err(|err| format!("unable to load dynamic library {}: {err}", path.display()))?;

        let symbol: libloading::Symbol<unsafe extern "C" fn() -> *mut ()> =
            library.get(function_name.as_bytes()).map_err(|err| {
                format!(
                    "symbol {function_name} missing from {}: {err}",
                    path.display()
                )
            })?;

        let raw = symbol();
        if raw.is_null() {
            return Err(format!(
                "symbol {function_name} returned null pointer from {}",
                path.display()
            ));
        }

        let boxed: Box<AnyView> = Box::from_raw(raw.cast());
        let view = *boxed;
        let associated = Associated::new(library, view);
        Ok(AnyView::new(associated))
    }
}

/// A macro to create a hot-reloadable view.
#[macro_export]
macro_rules! hot_reload {
    ($function_name:expr) => {{ $crate::hot_reload::Hotreload::new($function_name) }};
}
