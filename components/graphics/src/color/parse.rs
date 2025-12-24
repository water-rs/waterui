use super::HexColorError;

const fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => panic!("invalid hex digit"),
    }
}

const fn from_hex_byte(s: &[u8], i: usize) -> u8 {
    (hex_digit(s[i]) << 4) | hex_digit(s[i + 1])
}

pub const fn parse_hex_color(s: &str) -> (u8, u8, u8) {
    let bytes = s.as_bytes();
    let mut i = 0;

    if !bytes.is_empty() && bytes[0] == b'#' {
        i = 1;
    } else if bytes.len() >= 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
        i = 2;
    }

    if bytes.len() - i == 6 {
        (
            from_hex_byte(bytes, i),
            from_hex_byte(bytes, i + 2),
            from_hex_byte(bytes, i + 4),
        )
    } else {
        panic!("expected 6 hex digits");
    }
}

fn parse_runtime_prefix(bytes: &[u8]) -> usize {
    if !bytes.is_empty() && bytes[0] == b'#' {
        1
    } else if bytes.len() >= 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
        2
    } else {
        0
    }
}

const fn parse_runtime_hex_digit(b: u8, index: usize) -> Result<u8, HexColorError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(HexColorError::InvalidDigit(index)),
    }
}

fn parse_runtime_hex_byte(bytes: &[u8], index: usize) -> Result<u8, HexColorError> {
    let hi = parse_runtime_hex_digit(bytes[index], index)?;
    let lo = parse_runtime_hex_digit(bytes[index + 1], index + 1)?;
    Ok((hi << 4) | lo)
}

pub fn parse_hex_color_runtime(s: &str) -> Result<(u8, u8, u8), HexColorError> {
    let bytes = s.as_bytes();
    let offset = parse_runtime_prefix(bytes);
    if bytes.len().saturating_sub(offset) != 6 {
        return Err(HexColorError::InvalidLength);
    }

    Ok((
        parse_runtime_hex_byte(bytes, offset)?,
        parse_runtime_hex_byte(bytes, offset + 2)?,
        parse_runtime_hex_byte(bytes, offset + 4)?,
    ))
}
