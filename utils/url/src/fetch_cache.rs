//! The on-disk fetch cache behind [`Url::fetch`](crate::Url::fetch) on native
//! targets: a remote URL is downloaded once into the user cache directory and
//! resolved to the local copy. A browser loads and caches a remote URL itself,
//! so the web build has no such cache.

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use core::{error::Error, fmt};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{RemoteDownloadError, Url, download_remote_bytes_with_content_type};

#[derive(Debug)]
pub enum FetchError {
    CacheRootUnavailable,
    CreateCacheDir(std::io::Error),
    Download(Box<RemoteDownloadError>),
    WriteTemp(std::io::Error),
    Persist(std::io::Error),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CacheRootUnavailable => write!(f, "cache directory is unavailable"),
            Self::CreateCacheDir(error) => write!(f, "failed to create cache directory: {error}"),
            Self::Download(error) => fmt::Display::fmt(error, f),
            Self::WriteTemp(error) => write!(f, "failed to write cache temp file: {error}"),
            Self::Persist(error) => write!(f, "failed to persist cached file: {error}"),
        }
    }
}

impl Error for FetchError {}

fn fetch_cache_root() -> Option<PathBuf> {
    dirs::cache_dir()
        .map(|root| root.join("waterui").join("url-fetch"))
        .or_else(|| {
            let temp_root = std::env::temp_dir();
            (!temp_root.as_os_str().is_empty()).then(|| temp_root.join("waterui").join("url-fetch"))
        })
}

fn fetch_cache_key(url: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(url.as_bytes()))
}

fn fetch_cache_entry_dir(cache_root: &Path, key: &str) -> PathBuf {
    cache_root.join(key)
}

fn cache_payload_path(cache_entry_dir: &Path, extension: Option<&str>) -> PathBuf {
    extension.map_or_else(
        || cache_entry_dir.join("payload"),
        |extension| cache_entry_dir.join(format!("payload.{extension}")),
    )
}

fn cache_temp_path(cache_entry_dir: &Path, nonce: u128) -> PathBuf {
    cache_entry_dir.join(format!("incoming-{}-{nonce}", std::process::id()))
}

fn existing_fetch_cache_path_in(cache_root: &Path, key: &str) -> Option<PathBuf> {
    let cache_entry_dir = fetch_cache_entry_dir(cache_root, key);
    let payload = cache_entry_dir.join("payload");
    if payload.is_file() {
        return Some(payload);
    }

    let entries = std::fs::read_dir(&cache_entry_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name
            .strip_prefix("payload.")
            .is_some_and(|extension| !extension.is_empty())
        {
            return Some(path);
        }
    }

    None
}

fn existing_fetch_cache_path(url: &Url) -> Option<PathBuf> {
    let cache_root = fetch_cache_root()?;
    let key = fetch_cache_key(url.as_str());
    existing_fetch_cache_path_in(&cache_root, &key)
}

pub fn existing_fetch_cache_url(url: &Url) -> Option<Url> {
    existing_fetch_cache_path(url)
        .map(|path| Url::from_file_path_str(path.to_string_lossy().to_string()))
}

fn infer_extension(path_extension: Option<&str>, content_type: Option<&str>) -> Option<String> {
    if let Some(extension) = path_extension {
        return Some(extension.to_ascii_lowercase());
    }

    let content_type = content_type?
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    mime_guess::get_mime_extensions_str(content_type)
        .and_then(|extensions| preferred_extension(extensions))
        .map(str::to_string)
}

fn preferred_extension<'a>(extensions: &'a [&'a str]) -> Option<&'a str> {
    [
        "txt", "json", "html", "xml", "css", "js", "jpg", "png", "gif",
    ]
    .into_iter()
    .find(|preferred| extensions.iter().any(|candidate| candidate == preferred))
    .or_else(|| extensions.first().copied())
}

pub async fn fetch_remote_to_cache(
    url: String,
    path_extension: Option<String>,
) -> Result<Url, FetchError> {
    let cache_root = fetch_cache_root().ok_or(FetchError::CacheRootUnavailable)?;
    // Every filesystem call below goes through `blocking::unblock`: this is an
    // `async fn`, so a synchronous `create_dir_all`, `read_dir`, `write` or
    // `rename` stalls the executor thread for the duration of the syscall —
    // unbounded for the payload write. The same offload pattern as
    // `waterui_media::photo` and the preview runtime.
    let root_for_create = cache_root.clone();
    blocking::unblock(move || std::fs::create_dir_all(&root_for_create))
        .await
        .map_err(FetchError::CreateCacheDir)?;
    let key = fetch_cache_key(&url);

    let root_for_scan = cache_root.clone();
    let key_for_scan = key.clone();
    let cached =
        blocking::unblock(move || existing_fetch_cache_path_in(&root_for_scan, &key_for_scan))
            .await;
    if let Some(cached) = cached {
        return Ok(Url::from_file_path_str(
            cached.to_string_lossy().to_string(),
        ));
    }

    let cache_entry_dir = fetch_cache_entry_dir(&cache_root, &key);
    let entry_dir_for_create = cache_entry_dir.clone();
    blocking::unblock(move || std::fs::create_dir_all(&entry_dir_for_create))
        .await
        .map_err(FetchError::CreateCacheDir)?;

    let downloaded = download_remote_bytes_with_content_type(&url)
        .await
        .map_err(|error| FetchError::Download(Box::new(error)))?;
    let extension = infer_extension(
        path_extension.as_deref(),
        downloaded.content_type.as_deref(),
    );
    let cache_path = cache_payload_path(&cache_entry_dir, extension.as_deref());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temp_path = cache_temp_path(&cache_entry_dir, nonce);

    let temp_for_write = temp_path.clone();
    let bytes = downloaded.bytes;
    blocking::unblock(move || std::fs::write(&temp_for_write, &bytes))
        .await
        .map_err(FetchError::WriteTemp)?;

    let temp_for_persist = temp_path.clone();
    let cache_for_persist = cache_path.clone();
    blocking::unblock(move || {
        if let Err(error) = std::fs::rename(&temp_for_persist, &cache_for_persist) {
            // A concurrent fetch of the same URL won the rename; its payload
            // is equivalent, so drop ours rather than failing.
            if cache_for_persist.exists() {
                let _ = std::fs::remove_file(&temp_for_persist);
            } else {
                return Err(FetchError::Persist(error));
            }
        }
        Ok(())
    })
    .await?;

    Ok(Url::from_file_path_str(
        cache_path.to_string_lossy().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn infer_extension_uses_content_type_when_url_has_no_extension() {
        let url = Url::new("https://example.com/download");
        assert_eq!(
            infer_extension(url.extension(), Some("image/png; charset=utf-8")),
            Some(String::from("png"))
        );
    }

    #[test]
    fn fetch_cache_key_has_fixed_length() {
        let long_path = "a".repeat(512);
        let url = format!("https://example.com/{long_path}")
            .parse::<Url>()
            .expect("test URL must parse");
        let key = fetch_cache_key(url.as_str());
        assert_eq!(key.len(), 43);
    }

    #[test]
    fn existing_fetch_cache_path_ignores_incoming_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("waterui-url-cache-test-{unique}"));
        let url = Url::new("https://example.com/download");
        let key = fetch_cache_key(url.as_str());
        let cache_entry_dir = fetch_cache_entry_dir(&temp_dir, &key);
        std::fs::create_dir_all(&cache_entry_dir).expect("cache entry dir should be created");

        let incoming_path = cache_temp_path(&cache_entry_dir, 1);
        std::fs::write(&incoming_path, b"partial").expect("incoming file should be written");
        assert_eq!(existing_fetch_cache_path_in(&temp_dir, &key), None);

        let payload_path = cache_payload_path(&cache_entry_dir, Some("txt"));
        std::fs::write(&payload_path, b"done").expect("payload file should be written");
        assert_eq!(
            existing_fetch_cache_path_in(&temp_dir, &key),
            Some(payload_path)
        );

        std::fs::remove_dir_all(&temp_dir).expect("temp cache dir should be removed");
    }

    #[test]
    fn fetch_remote_to_cache_downloads_with_zenwave() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("server should accept one connection");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/plain\r\n",
                "Content-Length: 5\r\n",
                "Connection: close\r\n\r\n",
                "hello"
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });

        let url = format!("http://{address}/download")
            .parse::<Url>()
            .expect("test URL must parse");
        let fetched = futures_lite::future::block_on(fetch_remote_to_cache(
            url.as_str().to_owned(),
            url.extension().map(str::to_owned),
        ))
        .expect("remote fetch should succeed");
        let fetched_path = fetched
            .to_file_path()
            .expect("remote fetch should resolve to local cache path");
        let contents =
            std::fs::read_to_string(&fetched_path).expect("cached file should be readable");
        assert_eq!(contents, "hello");
        assert_eq!(fetched.extension(), Some("txt"));

        server.join().expect("server thread should finish");
    }

    #[test]
    fn fetch_remote_to_cache_handles_long_urls() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local addr");
        let long_path = "a".repeat(190);
        let expected_request_target = format!("/{long_path}");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("server should accept one connection");
            let mut request = [0u8; 4096];
            let bytes_read = stream
                .read(&mut request)
                .expect("server should read request");
            let request_text =
                core::str::from_utf8(&request[..bytes_read]).expect("request should be utf8");
            let request_line = request_text
                .lines()
                .next()
                .expect("request should contain request line");
            assert!(
                request_line.contains(&expected_request_target),
                "request line should contain long path: {request_line}"
            );
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/plain\r\n",
                "Content-Length: 5\r\n",
                "Connection: close\r\n\r\n",
                "hello"
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });

        let url = format!("http://{address}/{long_path}")
            .parse::<Url>()
            .expect("test URL must parse");
        let fetched = futures_lite::future::block_on(fetch_remote_to_cache(
            url.as_str().to_owned(),
            url.extension().map(str::to_owned),
        ))
        .expect("long-url remote fetch should succeed");
        let fetched_path = fetched
            .to_file_path()
            .expect("long-url fetch should resolve to local cache path");
        let file_name = fetched_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fetched file should have a utf8 name");
        assert_eq!(file_name, "payload.txt");
        assert_eq!(
            std::fs::read_to_string(&fetched_path).expect("cached file should be readable"),
            "hello"
        );

        server.join().expect("server thread should finish");
    }
}
