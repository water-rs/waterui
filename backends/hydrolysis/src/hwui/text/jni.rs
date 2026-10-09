//! [`PlatformText`] over JNI: the Kotlin
//! `dev.waterui.hydrolysis.hwui.HwuiTextProvider`.

use std::fmt;

use jni::JNIEnv;
use jni::objects::{JFloatArray, JObject, JValue};
use jni::signature::{Primitive, ReturnType};

use crate::hwui::HwuiError;
use crate::hwui::jni::{JniObject, Method};

use super::provider::PlatformText;
use super::wire::{PackedRequest, unpack_position, unpack_range};

const SHAPE: &str = "(ILjava/lang/String;I[I[Ljava/lang/String;Ljava/lang/String;FII)[F";
const POINT_QUERY: &str = "(IFF)J";
const OFFSET_QUERY: &str = "(II)I";
const REGISTER_FONT: &str = "(Ljava/lang/String;Ljava/nio/ByteBuffer;IZI)V";
/// Local references one query creates at most: the reply array.
const QUERY_FRAME: i32 = 4;

/// The JNI client of one `HwuiTextProvider`. Every call runs on the UI
/// thread the host attached; a call from any other thread fails.
pub struct JniTextProvider {
    provider: JniObject,
    register_font: Method,
    shape: Method,
    caret_rect: Method,
    hit_test: Method,
    word_at: Method,
    line_at: Method,
    snap: Method,
    selection_rects: Method,
    previous_visual: Method,
    next_visual: Method,
}

impl fmt::Debug for JniTextProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JniTextProvider").finish_non_exhaustive()
    }
}

const fn text_error(reason: String) -> HwuiError {
    HwuiError::Text { reason }
}

fn jint_id(method: &str, id: u32) -> Result<i32, HwuiError> {
    i32::try_from(id).map_err(|_| {
        text_error(format!(
            "HwuiTextProvider.{method}: layout id {id} is outside an int"
        ))
    })
}

impl JniTextProvider {
    /// A client of `provider`, a `HwuiTextProvider`, created on the UI
    /// thread: its class and method ids are resolved here, once.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the JVM, a global reference or a method of
    /// the provider is unavailable.
    pub fn new(env: &mut JNIEnv<'_>, provider: &JObject<'_>) -> Result<Self, HwuiError> {
        let provider = JniObject::new(env, provider, "HwuiTextProvider").map_err(text_error)?;
        let mut method =
            |name, signature| provider.method(env, name, signature).map_err(text_error);
        Ok(Self {
            register_font: method("registerFont", REGISTER_FONT)?,
            shape: method("shape", SHAPE)?,
            caret_rect: method("caretRect", "(IIZ)[F")?,
            hit_test: method("hitTest", POINT_QUERY)?,
            word_at: method("wordAt", POINT_QUERY)?,
            line_at: method("lineAt", POINT_QUERY)?,
            snap: method("snap", OFFSET_QUERY)?,
            selection_rects: method("selectionRects", "(III)[F")?,
            previous_visual: method("previousVisual", OFFSET_QUERY)?,
            next_visual: method("nextVisual", OFFSET_QUERY)?,
            provider,
        })
    }

    /// Registers face `index` of the app font `data` under `family`, at
    /// `weight` and slant `italic`; the provider copies the bytes.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when `data` is empty, a value is outside an
    /// int, or the platform rejects the font.
    pub fn register_font(
        &self,
        family: &str,
        data: &[u8],
        weight: u16,
        italic: bool,
        index: u32,
    ) -> Result<(), HwuiError> {
        if data.is_empty() {
            return Err(text_error(format!(
                "HwuiTextProvider.registerFont: the data of `{family}` is empty"
            )));
        }
        let index = i32::try_from(index).map_err(|_| {
            text_error(format!(
                "HwuiTextProvider.registerFont: face index {index} is outside an int"
            ))
        })?;
        self.provider
            .call(self.register_font, QUERY_FRAME, |env, provider, method| {
                let family = env.new_string(family)?;
                // SAFETY: the buffer views `data` for this call only; Kotlin
                // copies it into Java-owned memory before returning.
                let buffer =
                    unsafe { env.new_direct_byte_buffer(data.as_ptr().cast_mut(), data.len()) }?;
                // SAFETY: `REGISTER_FONT`.
                unsafe {
                    method.invoke(
                        env,
                        provider,
                        ReturnType::Primitive(Primitive::Void),
                        &[
                            JValue::Object(&family).as_jni(),
                            JValue::Object(&buffer).as_jni(),
                            JValue::Int(i32::from(weight)).as_jni(),
                            JValue::Bool(u8::from(italic)).as_jni(),
                            JValue::Int(index).as_jni(),
                        ],
                    )
                }?;
                Ok(())
            })
            .map_err(text_error)
    }

    fn point(&self, method: Method, id: u32, x: f32, y: f32) -> Result<i64, HwuiError> {
        let id = jint_id("point query", id)?;
        self.provider
            .call(method, QUERY_FRAME, |env, provider, method| {
                // SAFETY: `(IFF)J`.
                unsafe {
                    method.invoke(
                        env,
                        provider,
                        ReturnType::Primitive(Primitive::Long),
                        &[
                            JValue::Int(id).as_jni(),
                            JValue::Float(x).as_jni(),
                            JValue::Float(y).as_jni(),
                        ],
                    )
                }?
                .j()
            })
            .map_err(text_error)
    }

    fn offset(&self, method: Method, id: u32, offset: i32) -> Result<i32, HwuiError> {
        let id = jint_id("offset query", id)?;
        self.provider
            .call(method, QUERY_FRAME, |env, provider, method| {
                // SAFETY: `(II)I`.
                unsafe {
                    method.invoke(
                        env,
                        provider,
                        ReturnType::Primitive(Primitive::Int),
                        &[JValue::Int(id).as_jni(), JValue::Int(offset).as_jni()],
                    )
                }?
                .i()
            })
            .map_err(text_error)
    }

    fn floats_reply(
        &self,
        method: Method,
        args: &[jni::sys::jvalue],
    ) -> Result<Vec<f32>, HwuiError> {
        self.provider
            .call(method, QUERY_FRAME, |env, provider, method| {
                // SAFETY: every caller passes the arguments of its method's
                // signature, which returns `[F`.
                let reply =
                    unsafe { method.invoke(env, provider, ReturnType::Array, args) }?.l()?;
                floats(env, &JFloatArray::from(reply))
            })
            .map_err(text_error)
    }
}

fn floats(env: &JNIEnv<'_>, array: &JFloatArray<'_>) -> jni::errors::Result<Vec<f32>> {
    let length = env.get_array_length(array)?;
    let mut out = vec![0.0; usize::try_from(length).unwrap_or(0)];
    env.get_float_array_region(array, 0, &mut out)?;
    Ok(out)
}

impl PlatformText for JniTextProvider {
    fn shape(&self, id: u32, text: &str, request: &PackedRequest) -> Result<Vec<f32>, HwuiError> {
        let id = jint_id("shape", id)?;
        let too_many =
            |what: &str| text_error(format!("HwuiTextProvider.shape: {what} exceed an array"));
        let span_words =
            i32::try_from(request.spans.len()).map_err(|_| too_many("the packed runs"))?;
        let family_count =
            i32::try_from(request.families.len()).map_err(|_| too_many("the families"))?;
        let frame = family_count
            .checked_add(8)
            .ok_or_else(|| too_many("the families"))?;
        self.provider
            .call(self.shape, frame, |env, provider, method| {
                let text = env.new_string(text)?;
                let spans = env.new_int_array(span_words)?;
                env.set_int_array_region(&spans, 0, &request.spans)?;
                let families =
                    env.new_object_array(family_count, "java/lang/String", JObject::null())?;
                for (index, family) in (0..).zip(&request.families) {
                    let name = env.new_string(family)?;
                    env.set_object_array_element(&families, index, &name)?;
                }
                let locale = env.new_string(&request.locale)?;
                // SAFETY: `SHAPE`.
                let reply = unsafe {
                    method.invoke(
                        env,
                        provider,
                        ReturnType::Array,
                        &[
                            JValue::Int(id).as_jni(),
                            JValue::Object(&text).as_jni(),
                            JValue::Int(request.span_count).as_jni(),
                            JValue::Object(&spans).as_jni(),
                            JValue::Object(&families).as_jni(),
                            JValue::Object(&locale).as_jni(),
                            JValue::Float(request.max_width).as_jni(),
                            JValue::Int(request.max_lines).as_jni(),
                            JValue::Int(request.paragraph).as_jni(),
                        ],
                    )
                }?
                .l()?;
                floats(env, &JFloatArray::from(reply))
            })
            .map_err(text_error)
    }

    fn caret_rect(&self, id: u32, offset: i32, upstream: bool) -> Result<[f32; 4], HwuiError> {
        let id = jint_id("caretRect", id)?;
        let rect = self.floats_reply(
            self.caret_rect,
            &[
                JValue::Int(id).as_jni(),
                JValue::Int(offset).as_jni(),
                JValue::Bool(u8::from(upstream)).as_jni(),
            ],
        )?;
        <[f32; 4]>::try_from(rect.as_slice()).map_err(|_| {
            text_error(format!(
                "HwuiTextProvider.caretRect: {} floats, not 4",
                rect.len()
            ))
        })
    }

    fn hit_test(&self, id: u32, x: f32, y: f32) -> Result<(i32, bool), HwuiError> {
        unpack_position(self.point(self.hit_test, id, x, y)?)
    }

    fn word_at(&self, id: u32, x: f32, y: f32) -> Result<(i32, i32), HwuiError> {
        Ok(unpack_range(self.point(self.word_at, id, x, y)?))
    }

    fn line_at(&self, id: u32, x: f32, y: f32) -> Result<(i32, i32), HwuiError> {
        Ok(unpack_range(self.point(self.line_at, id, x, y)?))
    }

    fn snap(&self, id: u32, offset: i32) -> Result<i32, HwuiError> {
        self.offset(self.snap, id, offset)
    }

    fn selection_rects(&self, id: u32, start: i32, end: i32) -> Result<Vec<[f32; 4]>, HwuiError> {
        let id = jint_id("selectionRects", id)?;
        let packed = self.floats_reply(
            self.selection_rects,
            &[
                JValue::Int(id).as_jni(),
                JValue::Int(start).as_jni(),
                JValue::Int(end).as_jni(),
            ],
        )?;
        let (rects, partial) = packed.as_chunks::<4>();
        if !partial.is_empty() {
            return Err(text_error(format!(
                "HwuiTextProvider.selectionRects: {} floats, not whole rectangles",
                packed.len()
            )));
        }
        Ok(rects.to_vec())
    }

    fn previous_visual(&self, id: u32, offset: i32) -> Result<i32, HwuiError> {
        self.offset(self.previous_visual, id, offset)
    }

    fn next_visual(&self, id: u32, offset: i32) -> Result<i32, HwuiError> {
        self.offset(self.next_visual, id, offset)
    }
}
