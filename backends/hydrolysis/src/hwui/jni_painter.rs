//! The per-frame transport into the JVM `HwuiPainter`: one direct buffer
//! the JVM owns, refilled and replayed once per frame.
//!
//! The decoder caches the source `ByteBuffer`'s views, so the bytes must
//! live on the Java side: a borrowed `Vec`'s pointer would dangle after the
//! vec reallocates or drops. This painter allocates the buffer with
//! `ByteBuffer.allocateDirect` on the JVM, grows it only when a frame
//! exceeds its capacity, and keeps it as a global reference Java owns.
//!
//! # Wire endianness
//!
//! The encoded words are copied verbatim; Android targets are little-endian.
#[cfg(target_endian = "big")]
compile_error!("the HWUI wire encodes little-endian words; a big-endian host would need swizzling");

use std::{fmt, marker::PhantomData, rc::Rc};

use jni::objects::{Global, JByteBuffer, JObject, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::{Env, jni_sig, jni_str};

use super::jni::{JniObject, Method};

/// The session's painter on the UI thread: replays each frame's words into
/// the `HwuiPainter` through one JNI call.
///
/// `replay` runs on the calling thread; the session only calls it on the UI
/// thread, and the `Rc` marker keeps the type `!Send` so that cannot
/// change silently.
pub struct JniPainter {
    painter: JniObject,
    replay: Method,
    /// The JVM-owned frame buffer the decoder caches views of; `None` until
    /// the first replay allocates it.
    buffer: Option<Global<JByteBuffer<'static>>>,
    /// `buffer`'s capacity in bytes.
    capacity: usize,
    _ui_thread: PhantomData<Rc<()>>,
}

impl fmt::Debug for JniPainter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JniPainter")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl JniPainter {
    /// A painter calling `painter`'s `replay(ByteBuffer, int, long, boolean)`.
    ///
    /// # Errors
    ///
    /// A description when the JVM, a global reference or the method is
    /// unavailable.
    pub fn new(env: &mut Env<'_>, painter: &JObject<'_>) -> Result<Self, String> {
        let painter = JniObject::new(env, painter, "dev.waterui.hydrolysis.hwui.HwuiPainter")?;
        let replay = painter.method(env, "replay", "(Ljava/nio/ByteBuffer;IJZ)V")?;
        Ok(Self {
            painter,
            replay,
            buffer: None,
            capacity: 0,
            _ui_thread: PhantomData,
        })
    }

    /// Replays `words` — one encoded frame — with the window's `clear`.
    ///
    /// The JVM buffer grows only when `words` exceeds its capacity; the
    /// decoder reads it synchronously during the call and keeps the
    /// reference, so the next refill reuses the same object.
    ///
    /// # Errors
    ///
    /// A description when the frame exceeds the `int` length range, the
    /// buffer cannot be allocated, or the Java call throws.
    ///
    /// # Panics
    ///
    /// When the internal frame-buffer capacity invariant is violated.
    pub fn replay(&mut self, words: &[u32], clear: Option<u64>) -> Result<(), String> {
        let length = std::mem::size_of_val(words);
        let length_i32 = i32::try_from(length)
            .map_err(|_| format!("dev.waterui.hydrolysis.hwui.HwuiPainter.replay: a {length}-byte frame exceeds the int length range"))?;
        let buffer = &mut self.buffer;
        let capacity = &mut self.capacity;
        self.painter.call(self.replay, 8, |env, object, method| {
            if length > *capacity || buffer.is_none() {
                let requested = length
                    .max(1)
                    .checked_next_power_of_two()
                    .unwrap_or(length)
                    .min(i32::MAX as usize);
                let requested_i32 =
                    i32::try_from(requested).expect("the power-of-two bound is within i32");
                let allocated = env
                    .call_static_method(
                        jni_str!("java/nio/ByteBuffer"),
                        jni_str!("allocateDirect"),
                        jni_sig!("(I)Ljava/nio/ByteBuffer;"),
                        &[JValue::Int(requested_i32)],
                    )?
                    .l()?;
                let allocated = JByteBuffer::cast_local(env, allocated)?;
                *buffer = Some(env.new_global_ref(allocated)?);
                *capacity = requested;
            }
            let buffer = buffer
                .as_ref()
                .expect("the frame buffer exists after the grow branch");

            let address = env.get_direct_buffer_address(buffer)?;
            // SAFETY: `address` is a direct buffer of at least `length`
            // bytes the JVM uniquely owns on this thread; `words` lives
            // until the call returns and the decoder reads synchronously.
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr().cast::<u8>(), address, length) };

            // SAFETY: `method` was resolved with `(ByteBuffer, I, J, Z)V`
            // on `object`'s class and `args`/`ret` match it exactly.
            let value = unsafe {
                method.invoke(
                    env,
                    object,
                    ReturnType::Primitive(Primitive::Void),
                    &[
                        jni::objects::JValue::from(buffer.as_obj()).as_jni(),
                        JValue::Int(length_i32).as_jni(),
                        JValue::Long(i64::from_ne_bytes(clear.unwrap_or(0).to_ne_bytes())).as_jni(),
                        JValue::Bool(clear.is_some()).as_jni(),
                    ],
                )
            }?;
            value.v()?;
            Ok(())
        })
    }
}
