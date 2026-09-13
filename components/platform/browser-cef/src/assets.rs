//! CEF's side of the `waterui` asset origin: the custom scheme declaration and
//! the per-page [`SchemeHandlerFactory`] that answers it.
//!
//! The scheme is declared once for the process from
//! [`crate::app::App`]'s `on_register_custom_schemes` — CEF cannot create
//! views under an undeclared scheme. A [`crate::webview::CefWebViewController`]
//! opened with an asset server registers an [`AssetSchemeHandlerFactory`] on
//! its browser's request context — every page owns a private context, so the
//! factory answers that page alone. Asset requests therefore never touch the
//! network: CEF calls the factory on its I/O thread and the resource handler
//! streams the answer back — the same
//! [`waterui_webview::assets::dispatch`] contract every engine routes through.

use std::{
    cell::RefCell,
    io::{Cursor, Read as _, Seek as _, SeekFrom},
    os::raw::c_int,
};

use cef::{
    Browser, Callback, CefString, Frame, ImplRequest, ImplRequestContext, ImplResourceHandler,
    ImplResponse, ImplSchemeHandlerFactory, ImplSchemeRegistrar, Request, RequestContext,
    ResourceHandler, ResourceReadCallback, ResourceSkipCallback, Response, SchemeHandlerFactory,
    SchemeOptions, SchemeRegistrar, WrapResourceHandler, WrapSchemeHandlerFactory,
    rc::Rc as _,
    wrap_resource_handler, wrap_scheme_handler_factory,
};
use suiteki::Str;
use waterui_webview::assets::{self, ASSET_HOST, ASSET_ORIGIN, ASSET_SCHEME, AssetServer};

/// Declares the `waterui` asset scheme to CEF; called once per process from
/// [`crate::app::App`]'s `on_register_custom_schemes`.
pub fn register_asset_scheme(registrar: SchemeRegistrar) {
    // `secure` gives the origin `isSecureContext`; `cors_enabled` keeps
    // `fetch` honouring CORS and `fetch_enabled` lets the page issue
    // `fetch`/`XHR` at all; `standard` gives it the `scheme://host/path`
    // grammar. `LOCAL` is deliberately absent: it applies file-URL rules, under
    // which a page may only fetch the exact path that served it — the bundled
    // site could not load `app.js` next to `index.html`.
    let options = SchemeOptions::STANDARD.get_raw()
        | SchemeOptions::SECURE.get_raw()
        | SchemeOptions::CORS_ENABLED.get_raw()
        | SchemeOptions::FETCH_ENABLED.get_raw();
    if registrar
        .add_custom_scheme(
            Some(&CefString::from(ASSET_SCHEME)),
            c_int::try_from(options).expect("CEF scheme option bits fit in c_int"),
        )
        == 0
    {
        tracing::error!("CEF refused to register the `waterui` asset scheme");
    }
}

/// Registers `server` as the `waterui` handler on `context`.
///
/// The registration dies with the context — each page owns its context, so
/// dropping the page frees the server.
pub fn register_scheme_handler(context: &RequestContext, server: AssetServer) {
    let mut factory = new_scheme_handler_factory(server);
    let registered = context.register_scheme_handler_factory(
        Some(&CefString::from(ASSET_SCHEME)),
        // Restricting the factory to `ASSET_HOST` means only
        // `waterui://localhost` reaches us; CEF itself refuses any other
        // `waterui` host.
        Some(&CefString::from(ASSET_HOST)),
        Some(&mut factory),
    );
    assert_ne!(
        registered, 0,
        "CEF refused the `waterui` scheme handler factory"
    );
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_scheme_handler_factory(server: AssetServer) -> SchemeHandlerFactory {
    wrap_scheme_handler_factory! {
        struct AssetSchemeHandlerFactory {
            server: AssetServer,
        }
        impl SchemeHandlerFactory {
            fn create(
                &self,
                _browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _scheme_name: Option<&CefString>,
                request: Option<&mut Request>,
            ) -> Option<ResourceHandler> {
                let request = request?;
                let url = CefString::from(&request.url()).to_string();
                let method = CefString::from(&request.method()).to_string();
                let response = assets::asset_target(&url, ASSET_ORIGIN)
                    .map_or_else(assets::AssetResponse::not_found, |(path, query)| {
                        assets::dispatch(&self.server, &method, path, query)
                    });
                Some(new_resource_handler(response))
            }
        }
    }
    AssetSchemeHandlerFactory::new(server)
}

/// The bare MIME type and charset a `Content-Type` header carries — CEF wants
/// them on dedicated response fields rather than inside the header map.
fn mime_type(response: &assets::AssetResponse) -> Option<(String, Option<String>)> {
    let (_, value) = response
        .headers
        .iter()
        .find(|(name, _)| name.as_str().eq_ignore_ascii_case("content-type"))?;
    let mut parts = value.as_str().split(';');
    let mime = parts.next()?.trim().to_owned();
    let charset = parts
        .find_map(|part| part.trim().strip_prefix("charset="))
        .map(str::to_owned);
    Some((mime, charset))
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_resource_handler(response: assets::AssetResponse) -> ResourceHandler {
    wrap_resource_handler! {
        struct AssetResourceHandler {
            status: c_int,
            mime_type: Option<(String, Option<String>)>,
            headers: Vec<(Str, Str)>,
            body: RefCell<Cursor<Vec<u8>>>,
        }
        impl ResourceHandler {
            fn open(
                &self,
                _request: Option<&mut Request>,
                handle_request: Option<&mut c_int>,
                _callback: Option<&mut Callback>,
            ) -> c_int {
                // The response is already fully materialised: the request is
                // handled immediately rather than deferred through the
                // callback.
                if let Some(handle_request) = handle_request {
                    *handle_request = 1;
                }
                1
            }

            fn response_headers(
                &self,
                response: Option<&mut Response>,
                response_length: Option<&mut i64>,
                _redirect_url: Option<&mut CefString>,
            ) {
                let Some(response) = response else {
                    return;
                };
                response.set_status(self.status);
                if let Some((mime, charset)) = &self.mime_type {
                    response.set_mime_type(Some(&CefString::from(mime.as_str())));
                    if let Some(charset) = charset {
                        response.set_charset(Some(&CefString::from(charset.as_str())));
                    }
                }
                if let Some(response_length) = response_length {
                    *response_length =
                        i64::try_from(self.body.borrow().get_ref().len())
                            .expect("an asset body fits in i64");
                }
                for (name, value) in &self.headers {
                    response.set_header_by_name(
                        Some(&CefString::from(name.as_str())),
                        Some(&CefString::from(value.as_str())),
                        1,
                    );
                }
            }

            fn skip(
                &self,
                bytes_to_skip: i64,
                bytes_skipped: Option<&mut i64>,
                _callback: Option<&mut ResourceSkipCallback>,
            ) -> c_int {
                let mut body = self.body.borrow_mut();
                let Ok(target) = u64::try_from(bytes_to_skip) else {
                    return 0;
                };
                let remaining = body.get_ref().len() as u64 - body.position();
                let skipped = remaining.min(target);
                let skipped = i64::try_from(skipped).expect("a skip fits in i64");
                if body.seek(SeekFrom::Current(skipped)).is_err() {
                    return 0;
                }
                if let Some(bytes_skipped) = bytes_skipped {
                    *bytes_skipped = skipped;
                }
                1
            }

            fn read(
                &self,
                data_out: *mut u8,
                bytes_to_read: c_int,
                bytes_read: Option<&mut c_int>,
                _callback: Option<&mut ResourceReadCallback>,
            ) -> c_int {
                let capacity = usize::try_from(bytes_to_read).unwrap_or(0);
                // SAFETY: CEF hands `read` a buffer of at least `bytes_to_read`
                // bytes; `Read::read` writes at most `slice.len()` bytes.
                let out = unsafe { std::slice::from_raw_parts_mut(data_out, capacity) };
                let read = match self.body.borrow_mut().read(out) {
                    Ok(read) => read,
                    Err(error) => {
                        tracing::error!(%error, "an in-memory asset body refused to read");
                        return 0;
                    }
                };
                if let Some(bytes_read) = bytes_read {
                    *bytes_read =
                        c_int::try_from(read).expect("a read never exceeds the buffer CEF sized");
                }
                c_int::from(read > 0)
            }

            fn cancel(&self) {}
        }
    }
    AssetResourceHandler::new(
        c_int::from(response.status),
        mime_type(&response),
        response.headers,
        RefCell::new(Cursor::new(response.body)),
    )
}
