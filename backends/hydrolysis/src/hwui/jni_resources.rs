//! [`ResourcePlatform`] over JNI: the Kotlin
//! `dev.waterui.hydrolysis.hwui.RenderNodeSink`'s registrations.

use std::fmt;

use jni::objects::{JByteBuffer, JObject, JValue};
use jni::signature::{Primitive, ReturnType};
use waterui_graphics::draw::ImageLimits;

use super::jni::{JniObject, Method};
use super::platform::{AgslUniform, BitmapUpload, FontReader, ResourcePlatform};
use super::{Encoder, HwuiResources};

const VOID: ReturnType = ReturnType::Primitive(Primitive::Void);
const INT: ReturnType = ReturnType::Primitive(Primitive::Int);

/// The JNI client of one `RenderNodeSink`'s registrations.
pub struct JniResources {
    sink: JniObject,
    limits: ImageLimits,
    font_bytes: Method,
    system_font: Method,
    ttc_index: Method,
    bitmap: Method,
    runtime_shader: Method,
}

impl fmt::Debug for JniResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JniResources")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

fn int(what: &str, value: impl TryInto<i32>) -> Result<i32, String> {
    value
        .try_into()
        .map_err(|_| format!("{what} is outside an int"))
}

impl HwuiResources {
    /// The resource owner of `encoder`'s frames, creating its objects
    /// through `sink`, the painter's `RenderNodeSink`, on the UI thread.
    ///
    /// # Errors
    ///
    /// A description when the sink lacks a registration method or the
    /// device's bitmap limit cannot be read.
    pub fn over_jni(
        env: &mut jni::JNIEnv<'_>,
        sink: &JObject<'_>,
        encoder: &Encoder,
    ) -> Result<Self, String> {
        let platform = JniResources::new(env, sink)?;
        Ok(Self::new(Box::new(platform), encoder.registry().clone()))
    }
}

impl JniResources {
    /// A client of `sink`, a `RenderNodeSink`, created on the UI thread.
    ///
    /// # Errors
    ///
    /// A description when a method is missing or the device's bitmap limit
    /// cannot be read.
    pub fn new(env: &mut jni::JNIEnv<'_>, sink: &JObject<'_>) -> Result<Self, String> {
        let sink = JniObject::new(env, sink, "RenderNodeSink")?;
        let limit = sink.method(env, "maxBitmapDimension", "()I")?;
        let font_bytes = sink.method(env, "registerFontBytes", "(ILjava/nio/ByteBuffer;I)V")?;
        let system_font = sink.method(
            env,
            "registerSystemFont",
            "(ILjava/lang/String;)Ljava/nio/ByteBuffer;",
        )?;
        let ttc_index = sink.method(env, "fontTtcIndex", "(I)I")?;
        let bitmap = sink.method(env, "registerBitmap", "(IIIIIZLjava/nio/ByteBuffer;)V")?;
        let runtime_shader = sink.method(
            env,
            "registerRuntimeShader",
            "(ILjava/lang/String;[Ljava/lang/String;[I)V",
        )?;
        // SAFETY: `maxBitmapDimension()I` takes nothing and returns an int.
        let dimension = sink.call(limit, 4, |env, object, method| unsafe {
            method.invoke(env, object, INT, &[])?.i()
        })?;
        let dimension = u32::try_from(dimension)
            .map_err(|_| format!("RenderNodeSink.maxBitmapDimension: {dimension}"))?;
        Ok(Self {
            sink,
            limits: ImageLimits {
                max_dimension: dimension,
                max_texels: u64::from(dimension) * u64::from(dimension),
            },
            font_bytes,
            system_font,
            ttc_index,
            bitmap,
            runtime_shader,
        })
    }
}

impl ResourcePlatform for JniResources {
    fn image_limits(&self) -> ImageLimits {
        self.limits
    }

    fn register_font(&self, id: u32, data: &[u8], index: u32) -> Result<(), String> {
        let (id, index) = (int("the font id", id)?, int("the face index", index)?);
        if data.is_empty() {
            return Err("the font data is empty".to_owned());
        }
        self.sink.call(self.font_bytes, 4, |env, object, method| {
            // SAFETY: the buffer views `data` for this call only; Kotlin
            // reads it once, copying it into Java-owned memory.
            let buffer =
                unsafe { env.new_direct_byte_buffer(data.as_ptr().cast_mut(), data.len()) }?;
            // SAFETY: `(ILjava/nio/ByteBuffer;I)V`.
            unsafe {
                method.invoke(
                    env,
                    object,
                    VOID,
                    &[
                        JValue::Int(id).as_jni(),
                        JValue::Object(&buffer).as_jni(),
                        JValue::Int(index).as_jni(),
                    ],
                )
            }?;
            Ok(())
        })
    }

    fn register_system_font(
        &self,
        id: u32,
        family: &str,
        read: &mut FontReader<'_>,
    ) -> Result<(), String> {
        let id = int("the font id", id)?;
        let mut outcome = Ok(());
        let ttc_index = self.ttc_index;
        self.sink.call(self.system_font, 8, |env, object, method| {
            let name = env.new_string(family)?;
            // SAFETY: `(ILjava/lang/String;)Ljava/nio/ByteBuffer;`.
            let buffer = unsafe {
                method.invoke(
                    env,
                    object,
                    ReturnType::Object,
                    &[JValue::Int(id).as_jni(), JValue::Object(&name).as_jni()],
                )
            }?
            .l()?;
            let buffer = JByteBuffer::from(buffer);
            let address = env.get_direct_buffer_address(&buffer)?;
            let length = env.get_direct_buffer_capacity(&buffer)?;
            // SAFETY: `fontTtcIndex(I)I`.
            let index =
                unsafe { ttc_index.invoke(env, object, INT, &[JValue::Int(id).as_jni()]) }?.i()?;
            // SAFETY: the address and capacity are the live direct buffer's,
            // which the registered `Font` keeps alive while this reads it.
            let data = unsafe { std::slice::from_raw_parts(address.cast_const(), length) };
            outcome = u32::try_from(index)
                .map_err(|_| format!("system font `{family}` has face index {index}"))
                .and_then(|index| read(data, index));
            Ok(())
        })?;
        outcome
    }

    fn register_bitmap(&self, id: u32, bitmap: &BitmapUpload<'_>) -> Result<(), String> {
        let args = [
            int("the bitmap id", id)?,
            int("the bitmap width", bitmap.width)?,
            int("the bitmap height", bitmap.height)?,
            int("the bitmap format", bitmap.format)?,
            int("the bitmap colour space", bitmap.color_space)?,
        ];
        if bitmap.pixels.is_empty() {
            return Err("the bitmap has no pixels".to_owned());
        }
        self.sink.call(self.bitmap, 4, |env, object, method| {
            // SAFETY: the buffer views the texels for this call only; Kotlin
            // copies them into a bitmap before it returns.
            let pixels = unsafe {
                env.new_direct_byte_buffer(bitmap.pixels.as_ptr().cast_mut(), bitmap.pixels.len())
            }?;
            // SAFETY: `(IIIIIZLjava/nio/ByteBuffer;)V`.
            unsafe {
                method.invoke(
                    env,
                    object,
                    VOID,
                    &[
                        JValue::Int(args[0]).as_jni(),
                        JValue::Int(args[1]).as_jni(),
                        JValue::Int(args[2]).as_jni(),
                        JValue::Int(args[3]).as_jni(),
                        JValue::Int(args[4]).as_jni(),
                        JValue::Bool(u8::from(bitmap.premultiplied)).as_jni(),
                        JValue::Object(&pixels).as_jni(),
                    ],
                )
            }?;
            Ok(())
        })
    }

    fn register_runtime_shader(
        &self,
        id: u32,
        source: &str,
        uniforms: &[AgslUniform],
    ) -> Result<(), String> {
        let id = int("the runtime shader id", id)?;
        let count = int("the uniform count", uniforms.len())?;
        let sizes = uniforms
            .iter()
            .map(|uniform| int("a uniform size", uniform.floats))
            .collect::<Result<Vec<i32>, String>>()?;
        self.sink.call(
            self.runtime_shader,
            count.saturating_add(8),
            |env, object, method| {
                let source = env.new_string(source)?;
                let names = env.new_object_array(count, "java/lang/String", JObject::null())?;
                for (index, uniform) in (0..).zip(uniforms) {
                    let name = env.new_string(&uniform.name)?;
                    env.set_object_array_element(&names, index, &name)?;
                }
                let sizes_array = env.new_int_array(count)?;
                env.set_int_array_region(&sizes_array, 0, &sizes)?;
                // SAFETY: `(ILjava/lang/String;[Ljava/lang/String;[I)V`.
                unsafe {
                    method.invoke(
                        env,
                        object,
                        VOID,
                        &[
                            JValue::Int(id).as_jni(),
                            JValue::Object(&source).as_jni(),
                            JValue::Object(&names).as_jni(),
                            JValue::Object(&sizes_array).as_jni(),
                        ],
                    )
                }?;
                Ok(())
            },
        )
    }
}
