//! Handlers the page can call, written the way `Button::action` is written.
//!
//! A handler used to be `Fn(&[u8]) -> Vec<u8>`: the payload arrived as bytes,
//! the answer left as bytes, nothing could be awaited, and a failure had to be
//! smuggled through the success channel. Handlers now take extractors and may be
//! `async`, using the same [`Handler`](waterui_core::handler::Handler) machinery
//! that powers `Button::action`, so the two read alike.
//!
//! ```ignore
//! WebView::open(url)
//!     .handler("greet", |Json(req): Json<Greet>| async move {
//!         Json(Greeting { text: format!("Hi {}", req.name) })
//!     })
//!     .handler("save", |Json(doc): Json<Doc>, State(db): State<Db>| async move {
//!         db.save(doc).await?;
//!         Ok::<_, SaveError>(())
//!     })
//! ```

use std::rc::Rc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use waterui_core::extract::{ExtractionState, Extractor};
use waterui_core::{Environment, Error};
use waterui_str::Str;

/// The request being handled, placed in the environment so extractors can read
/// it the way `State<T>` reads its value.
#[derive(Debug, Clone)]
pub struct JsRequest {
    pub name: Str,
    pub payload: Rc<[u8]>,
}

/// Extracts the payload as JSON, deserialized into `T`.
#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

/// Extracts the payload as raw bytes.
#[derive(Debug, Clone)]
pub struct Bytes(pub Vec<u8>);

/// Extracts the handler name that was called.
///
/// Useful when one closure serves several names.
#[derive(Debug, Clone)]
pub struct HandlerName(pub Str);

fn request(env: &Environment) -> Result<JsRequest, Error> {
    env.get::<JsRequest>().cloned().ok_or_else(|| {
        Error::msg("a WaterUI web view handler argument was extracted outside a handler call")
    })
}

impl<T: DeserializeOwned + 'static> Extractor for Json<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        let request = request(env)?;
        serde_json::from_slice(&request.payload)
            .map(Self)
            .map_err(|error| {
                Error::msg(format!(
                    "handler `{}` received a payload that is not {}: {error}",
                    request.name,
                    core::any::type_name::<T>()
                ))
            })
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}

impl Extractor for Bytes {
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(Self(request(env)?.payload.to_vec()))
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}

impl Extractor for HandlerName {
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(Self(request(env)?.name))
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}

/// What a handler may return.
///
/// Users depend on this but never implement it, the same way they depend on
/// `IntoView`. A `Result` rejects the page's promise on `Err`, so a handler can
/// use `?` and the failure reaches JavaScript as a rejection instead of being
/// encoded into a successful reply.
pub trait IntoJsReply {
    /// Converts into the bytes to send, or the message to reject with.
    ///
    /// # Errors
    ///
    /// Returns the message the page's promise rejects with.
    fn into_js_reply(self) -> Result<Vec<u8>, String>;
}

impl IntoJsReply for () {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

impl IntoJsReply for Vec<u8> {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        Ok(self)
    }
}

impl IntoJsReply for Bytes {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        Ok(self.0)
    }
}

impl IntoJsReply for String {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        Ok(self.into_bytes())
    }
}

impl IntoJsReply for Str {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        Ok(self.as_str().as_bytes().to_vec())
    }
}

impl<T: Serialize> IntoJsReply for Json<T> {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&self.0).map_err(|error| error.to_string())
    }
}

impl<T: IntoJsReply, E: core::fmt::Display> IntoJsReply for Result<T, E> {
    fn into_js_reply(self) -> Result<Vec<u8>, String> {
        match self {
            Ok(value) => value.into_js_reply(),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Builds the boxed handler a backend installs, from a closure written the way
/// `Button::action` is written.
///
/// The request is placed in a scoped environment, so `Json<T>`, `Bytes`,
/// `HandlerName` and every other `Extractor` — `State<T>`, `WebViewProxy` —
/// resolve exactly as they do in an action.
pub fn boxed_handler<F, Args, Fut, Reply>(
    environment: Environment,
    name: Str,
    handler: F,
) -> Box<crate::ScriptMessageHandler>
where
    F: waterui_core::handler::Handler<Args, Fut> + 'static,
    Fut: core::future::Future<Output = Reply> + 'static,
    Reply: IntoJsReply + 'static,
{
    // `Handler::call` needs `&mut self`, but the page may call a handler again
    // before an earlier call has finished. The borrow is released as soon as the
    // future is built, so overlapping calls are fine.
    let handler = core::cell::RefCell::new(handler);
    Box::new(move |payload: &[u8]| {
        let scoped = environment.clone().extending(JsRequest {
            name: name.clone(),
            payload: Rc::from(payload),
        });
        let future = handler.borrow_mut().call(&scoped);
        Box::pin(async move { future.await.into_js_reply() })
    })
}

#[cfg(test)]
mod tests {
    use super::{Bytes, HandlerName, IntoJsReply, JsRequest, Json};
    use std::rc::Rc;
    use waterui_core::Environment;
    use waterui_core::extract::Extractor;
    use waterui_str::Str;

    fn env_with(name: &str, payload: &[u8]) -> Environment {
        let mut env = Environment::new();
        env.insert(JsRequest {
            name: Str::from(name.to_owned()),
            payload: Rc::from(payload),
        });
        env
    }

    #[derive(Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    struct Greet {
        name: String,
    }

    #[test]
    fn json_bytes_and_name_all_come_from_the_request() {
        let env = env_with("greet", br#"{"name":"Lexo"}"#);

        let Json(greet): Json<Greet> = Extractor::extract(&env).expect("extracts");
        assert_eq!(greet.name, "Lexo");

        let Bytes(raw) = Extractor::extract(&env).expect("extracts");
        assert_eq!(raw, br#"{"name":"Lexo"}"#);

        let HandlerName(name) = Extractor::extract(&env).expect("extracts");
        assert_eq!(name.as_str(), "greet");
    }

    #[test]
    fn a_payload_of_the_wrong_shape_names_the_handler_and_the_type() {
        let env = env_with("greet", b"[]");
        let error = Json::<Greet>::extract(&env).expect_err("rejects");
        let message = error.to_string();
        assert!(message.contains("greet"), "{message}");
    }

    #[test]
    fn extracting_outside_a_handler_call_is_an_error_not_a_panic() {
        let env = Environment::new();
        assert!(Bytes::extract(&env).is_err());
    }

    #[test]
    fn a_result_error_becomes_a_rejection() {
        let ok: Result<Json<Greet>, std::io::Error> = Ok(Json(Greet {
            name: String::from("Lexo"),
        }));
        assert!(ok.into_js_reply().is_ok());

        let failed: Result<(), std::io::Error> = Err(std::io::Error::other("disk full"));
        let message = failed.into_js_reply().expect_err("rejects");
        assert!(message.contains("disk full"));
    }

    #[test]
    fn unit_replies_with_nothing() {
        assert_eq!(().into_js_reply().expect("ok"), Vec::<u8>::new());
    }
}
