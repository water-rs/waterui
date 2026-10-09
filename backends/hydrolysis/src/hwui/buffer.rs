//! The frame's command buffer: one Rust-owned word vector, reused across
//! frames, which the Android host reads through a direct `ByteBuffer`.

#[cfg(test)]
use std::fmt::Write as _;

use super::HwuiError;
use super::protocol::{HEADER_WORDS, MAGIC, Op};

/// One payload field: its contract-log name and its value. A field is one
/// or more whole words.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the names label the contract test's event log, which only test builds keep"
    )
)]
pub enum Field<'a> {
    /// An unsigned word.
    U(&'static str, u32),
    /// A signed word.
    I(&'static str, i32),
    /// A float word.
    F(&'static str, f32),
    /// A `ColorLong`: low word, then high word.
    Color(&'static str, u64),
    /// Unsigned words.
    Us(&'static str, &'a [u32]),
    /// Float words.
    Fs(&'static str, &'a [f32]),
    /// `ColorLong`s, two words each.
    Colors(&'static str, &'a [u64]),
    /// `u16`s packed two to a word, low half first, the last word padded.
    /// Zero words that keep a fixed layout; not logged.
    Pad(usize),
}

/// The frame's command buffer.
#[derive(Debug)]
pub struct CommandBuffer {
    words: Vec<u32>,
    sequence: u32,
    recording: Option<u32>,
    #[cfg(test)]
    log: Vec<String>,
}

impl CommandBuffer {
    /// An empty buffer; [`begin_frame`](Self::begin_frame) opens the first
    /// frame.
    pub const fn new() -> Self {
        Self {
            words: Vec::new(),
            sequence: 0,
            recording: None,
            #[cfg(test)]
            log: Vec::new(),
        }
    }

    /// Clears the buffer and writes the next frame's header.
    pub fn begin_frame(&mut self) {
        self.words.clear();
        self.recording = None;
        self.sequence = self.sequence.wrapping_add(1);
        self.words.extend([MAGIC, self.sequence, 0]);
        #[cfg(test)]
        self.log.push(format!("Frame sequence={}", self.sequence));
    }

    /// Whether the open frame holds no record.
    #[cfg(test)]
    pub const fn is_empty(&self) -> bool {
        self.words.len() <= HEADER_WORDS
    }

    /// Appends one record.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] when the op is out of place — a recording op
    /// outside a recording, a node or host op inside one, nested or
    /// unbalanced recordings — when a fixed op's payload does not match its
    /// layout, or when the payload exceeds a `u16` word count. A failed op
    /// leaves the buffer as it was.
    pub fn op(&mut self, op: Op, fields: &[Field<'_>]) -> Result<(), HwuiError> {
        self.check_placement(op)?;
        let start = self.words.len();
        self.words.push(0);
        for field in fields {
            push_field(&mut self.words, *field);
        }
        let words = self.words.len() - start - 1;
        let fail = |this: &mut Self, reason: String| {
            this.words.truncate(start);
            Err(HwuiError::Encoding {
                op: op.name(),
                reason,
            })
        };
        let Ok(length) = u16::try_from(words) else {
            return fail(
                self,
                format!("its payload of {words} words exceeds the 65535-word record limit"),
            );
        };
        if let Some(fixed) = op.fixed_words()
            && fixed != length
        {
            return fail(
                self,
                format!("its payload is {length} words; the layout is {fixed}"),
            );
        }
        self.words[start] = u32::from(op.code()) | (u32::from(length) << 16);
        match op {
            Op::Record => self.recording = fields.first().map(field_word),
            Op::EndRecord => self.recording = None,
            _ => {}
        }
        #[cfg(test)]
        self.log.push(event(op, fields));
        Ok(())
    }

    fn check_placement(&self, op: Op) -> Result<(), HwuiError> {
        let group = op.code() >> 8;
        let reason = match (op, self.recording) {
            (Op::Record, Some(open)) => Some(format!("node {open}'s recording is still open")),
            (Op::EndRecord, None) => Some("no recording is open".to_owned()),
            (Op::EndRecord, Some(_)) => None,
            (_, None) if group == 0x02 => Some("it draws outside a recording".to_owned()),
            (_, Some(open)) if group == 0x01 || group == 0x04 => {
                Some(format!("node {open}'s recording is open"))
            }
            _ => None,
        };
        reason.map_or(Ok(()), |reason| {
            Err(HwuiError::Encoding {
                op: op.name(),
                reason,
            })
        })
    }

    /// Closes the frame: patches the header's byte length and returns the
    /// words.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] when a recording is still open or the frame
    /// exceeds `u32::MAX` bytes.
    pub fn finish(&mut self) -> Result<&[u32], HwuiError> {
        if let Some(open) = self.recording {
            return Err(HwuiError::Encoding {
                op: Op::EndRecord.name(),
                reason: format!("the frame ends inside node {open}'s recording"),
            });
        }
        let bytes = u32::try_from(self.words.len() * 4).map_err(|_| HwuiError::Encoding {
            op: "frame",
            reason: format!("{} words exceed the u32 byte length", self.words.len()),
        })?;
        self.words[HEADER_WORDS - 1] = bytes;
        Ok(&self.words)
    }

    /// Takes the contract-test event log accumulated since the last take.
    #[cfg(test)]
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }
}

// The host reads the words as little-endian bytes in place.
#[cfg(hydrolysis_hwui)]
const _: () = assert!(cfg!(target_endian = "little"));

const fn field_word(field: &Field<'_>) -> u32 {
    match *field {
        Field::U(_, value) => value,
        Field::I(_, value) => value.cast_unsigned(),
        Field::F(_, value) => value.to_bits(),
        _ => 0,
    }
}

fn push_field(words: &mut Vec<u32>, field: Field<'_>) {
    match field {
        Field::U(..) | Field::I(..) | Field::F(..) => words.push(field_word(&field)),
        Field::Color(_, color) => push_color(words, color),
        Field::Us(_, values) => words.extend_from_slice(values),
        Field::Fs(_, values) => words.extend(values.iter().map(|value| value.to_bits())),
        Field::Colors(_, colors) => {
            for color in colors {
                push_color(words, *color);
            }
        }
        Field::Pad(count) => words.extend(std::iter::repeat_n(0, count)),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "splits the u64 into its two words on purpose"
)]
fn push_color(words: &mut Vec<u32>, color: u64) {
    words.extend([color as u32, (color >> 32) as u32]);
}

/// One contract-log line: `Name field=value …`, unsigned and signed words
/// in decimal, floats as their bits and colours as 16 hex digits — the
/// format the JVM `LoggingSink` writes.
#[cfg(test)]
fn event(op: Op, fields: &[Field<'_>]) -> String {
    fn list<T>(out: &mut String, values: &[T], each: impl Fn(&mut String, &T)) {
        out.push('[');
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            each(out, value);
        }
        out.push(']');
    }
    let mut out = op.name().to_owned();
    for field in fields {
        let name = match *field {
            Field::U(name, _)
            | Field::I(name, _)
            | Field::F(name, _)
            | Field::Color(name, _)
            | Field::Us(name, _)
            | Field::Fs(name, _)
            | Field::Colors(name, _) => name,
            Field::Pad(_) => continue,
        };
        let _ = write!(out, " {name}=");
        match *field {
            Field::U(_, value) => {
                let _ = write!(out, "{value}");
            }
            Field::I(_, value) => {
                let _ = write!(out, "{value}");
            }
            Field::F(_, value) => {
                let _ = write!(out, "{:08x}", value.to_bits());
            }
            Field::Color(_, value) => {
                let _ = write!(out, "{value:016x}");
            }
            Field::Us(_, values) => list(&mut out, values, |out, value| {
                let _ = write!(out, "{value}");
            }),
            Field::Fs(_, values) => list(&mut out, values, |out, value| {
                let _ = write!(out, "{:08x}", value.to_bits());
            }),
            Field::Colors(_, values) => list(&mut out, values, |out, value| {
                let _ = write!(out, "{value:016x}");
            }),
            Field::Pad(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{CommandBuffer, Field};
    use crate::hwui::protocol::{MAGIC, Op};

    #[test]
    fn a_frame_is_a_header_then_aligned_length_prefixed_records() {
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        buffer.op(Op::CreateNode, &[Field::U("node", 7)]).unwrap();
        buffer
            .op(
                Op::Record,
                &[
                    Field::U("node", 7),
                    Field::I("width", 4),
                    Field::I("height", 4),
                ],
            )
            .unwrap();
        buffer
            .op(
                Op::Mesh,
                &[
                    Field::U("interpolation", 0),
                    Field::U("patches", 0),
                    Field::Fs("patch", &[]),
                ],
            )
            .unwrap();
        buffer.op(Op::EndRecord, &[]).unwrap();
        let words = buffer.finish().unwrap().to_vec();
        assert_eq!(words[0], MAGIC);
        assert_eq!(words[1], 1);
        assert_eq!(words[2] as usize, words.len() * 4);
        assert_eq!(words[3], 0x0101 | (1 << 16));
        assert_eq!(words[4], 7);
        assert_eq!(&words[5..9], &[0x0109 | (3 << 16), 7, 4, 4]);
        assert_eq!(words[9], 0x020C | (2 << 16));
        assert_eq!(&words[10..12], &[0, 0]);
        assert_eq!(&words[12..], &[0x010A]);
    }

    #[test]
    fn a_fixed_op_with_the_wrong_payload_fails_and_leaves_the_buffer() {
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        let error = buffer
            .op(Op::SetAlpha, &[Field::U("node", 1)])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("SetAlpha") && error.contains("layout is 2"),
            "{error}"
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn recording_ops_need_an_open_recording_and_node_ops_a_closed_one() {
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        assert!(buffer.op(Op::Save, &[]).is_err());
        buffer
            .op(
                Op::Record,
                &[
                    Field::U("node", 0),
                    Field::I("width", 1),
                    Field::I("height", 1),
                ],
            )
            .unwrap();
        assert!(buffer.op(Op::CreateNode, &[Field::U("node", 1)]).is_err());
        assert!(buffer.finish().is_err());
        buffer.op(Op::EndRecord, &[]).unwrap();
        assert!(buffer.finish().is_ok());
    }
}

/// A fixed-capacity field list, so building a record allocates nothing.
#[derive(Debug)]
pub struct Fields<'a> {
    items: [Field<'a>; 24],
    len: usize,
}

impl<'a> Fields<'a> {
    /// An empty list.
    pub const fn new() -> Self {
        Self {
            items: [Field::Pad(0); 24],
            len: 0,
        }
    }

    /// Appends `field`.
    ///
    /// # Panics
    ///
    /// Panics past 24 fields; the largest record layout has 20.
    pub const fn push(&mut self, field: Field<'a>) -> &mut Self {
        self.items[self.len] = field;
        self.len += 1;
        self
    }

    /// Appends every field of `fields`.
    pub fn extend(&mut self, fields: &[Field<'a>]) -> &mut Self {
        for field in fields {
            self.push(*field);
        }
        self
    }

    /// The fields pushed so far.
    pub fn as_slice(&self) -> &[Field<'a>] {
        &self.items[..self.len]
    }
}
