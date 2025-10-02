use anyhow::Result;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogLevel {
    Info = 1,
    Debug = 2,
}

impl LogLevel {
    pub fn from_count(count: u8) -> Self {
        match count {
            0 => LogLevel::Info,
            _ => LogLevel::Debug,
        }
    }
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

pub fn init_logging(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

fn enabled(level: LogLevel) -> bool {
    LOG_LEVEL.load(Ordering::Relaxed) >= level as u8
}

pub fn info(message: impl fmt::Display) {
    if enabled(LogLevel::Info) {
        println!("{message}");
    }
}

pub fn debug(message: impl fmt::Display) {
    if enabled(LogLevel::Debug) {
        eprintln!("[debug] {message}");
    }
}

pub fn warn(message: impl fmt::Display) {
    eprintln!("[warn] {message}");
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI manifest to have parent")
        .to_path_buf()
}

pub fn kebab_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() {
                if !result.is_empty() && !prev_dash {
                    result.push('-');
                }
                result.push(ch.to_ascii_lowercase());
            } else {
                result.push(ch);
            }
            prev_dash = false;
        } else {
            if !result.ends_with('-') {
                result.push('-');
            }
            prev_dash = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() {
        "waterui-app".to_string()
    } else {
        result
    }
}

pub fn pascal_case(name: &str) -> String {
    let mut result = String::new();
    let mut capitalize = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize {
                result.push(ch.to_ascii_uppercase());
                capitalize = false;
            } else {
                result.push(ch.to_ascii_lowercase());
            }
        } else {
            capitalize = true;
        }
    }
    if result.is_empty() {
        "WaterUIApp".to_string()
    } else {
        result
    }
}

pub fn ensure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}
