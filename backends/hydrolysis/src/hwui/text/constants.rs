//! Constants shared by text encoding and Kotlin generation.

/// `int`s per packed style run.
pub const SPAN_WORDS: usize = 12;

codes! {
    /// Span field indices.
    span: usize => "TextWire" {
        /// The run's first UTF-16 offset.
        START = 0;
        /// The run's end UTF-16 offset.
        END = 1;
        /// The run's family list index, or [`DEFAULT_FAMILY`](super::DEFAULT_FAMILY).
        FAMILY = 2;
        /// The font size in pixels, as float bits.
        SIZE = 3;
        /// The weight, 1 to 1000.
        WEIGHT = 4;
        /// The [`run_flags`](super::run_flags).
        FLAGS = 5;
        /// The foreground `ColorLong`, low word then high word.
        FOREGROUND = 6;
        /// The background `ColorLong`, low word then high word.
        BACKGROUND = 8;
        /// The line height in pixels, as float bits.
        LINE_HEIGHT = 10;
        /// The letter spacing in ems, as float bits.
        LETTER_SPACING = 11;
    }
}

codes! {
    /// A span's flag bits.
    run_flags: i32 => "TextWire" {
        /// Italic.
        ITALIC = 1;
        /// Underlined.
        UNDERLINE = 1 << 1;
        /// Struck through.
        STRIKETHROUGH = 1 << 2;
        /// The foreground words are set.
        HAS_FOREGROUND = 1 << 3;
        /// The background words are set.
        HAS_BACKGROUND = 1 << 4;
        /// The line height word is set.
        HAS_LINE_HEIGHT = 1 << 5;
    }
}

codes! {
    /// A request's paragraph flags.
    paragraph: i32 => "TextWire" {
        /// The bits holding the alignment code.
        ALIGN_MASK = 0b11;
        /// Lines start at the paragraph's start edge.
        ALIGN_START = 0;
        /// Lines are centred.
        ALIGN_CENTER = 1;
        /// Lines end at the paragraph's end edge.
        ALIGN_END = 2;
        /// The paragraph runs right to left.
        RIGHT_TO_LEFT = 1 << 2;
        /// The last allowed line ends in an ellipsis when text remains.
        ELLIPSIS = 1 << 3;
        /// A family neither registered nor known to the system is an error,
        /// not skipped.
        STRICT_FAMILIES = 1 << 4;
    }
}

/// The family index of a run that names none: the platform default.
pub const DEFAULT_FAMILY: i32 = -1;
/// The wrap width of a request that does not wrap.
pub const UNBOUNDED: f32 = -1.0;
/// The line limit of a request without one.
pub const NO_LINE_LIMIT: i32 = -1;
/// What joins the names of one family list.
pub const FAMILY_SEPARATOR: char = '\u{1f}';

/// Floats before the first line of a reply.
pub const REPLY_HEADER: usize = 4;
/// Floats per line of a reply.
pub const LINE_WORDS: usize = 5;
