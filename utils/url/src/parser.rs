//! Const-compatible URL parsing functions.
//!
//! This module provides complete URL parsing logic that can be evaluated at compile time.
//! All functions use byte-level operations to work in const contexts.

use crate::{BlobComponents, DataComponents, LocalComponents, ParsedComponents, Span, WebComponents};

// ============================================================================
// Public API
// ============================================================================

/// Main entry point for URL parsing with validation.
///
/// Detects the URL type and parses all components into a `ParsedComponents` struct.
/// This function can be evaluated at compile time.
///
/// # Panics
///
/// Panics if the URL is malformed. This enables compile-time syntax checking
/// when used in const contexts.
pub const fn parse_url(bytes: &[u8]) -> ParsedComponents {
    let len = bytes.len();

    // Check for empty URL
    assert!(len != 0, "URL string is empty");

    // Check for data: URLs
    if len >= 5 && starts_with(bytes, b"data:") {
        return ParsedComponents::Data(parse_data_url(bytes));
    }

    // Check for blob: URLs
    if len >= 5 && starts_with(bytes, b"blob:") {
        return ParsedComponents::Blob(parse_blob_url(bytes));
    }

    // Check for web URLs (http://, https://, etc.)
    if let Some(scheme_end) = find_scheme_end(bytes) {
        if is_web_scheme(bytes, scheme_end) {
            let web = parse_web_url(bytes, scheme_end);
            validate_web_url(&web, bytes);
            return ParsedComponents::Web(web);
        }
    }

    // Default to local file path
    ParsedComponents::Local(parse_local_path(bytes))
}

/// Validates a parsed web URL and panics if malformed.
const fn validate_web_url(web: &WebComponents, bytes: &[u8]) {
    // Scheme must be present
    assert!(web.scheme.is_present(), "Web URL must have a scheme");

    // Host must be present for web URLs
    assert!(web.host.is_present(), "Web URL must have a host");

    // Validate port if present (must be valid digits)
    if web.port.is_present() {
        let port_start = web.port.start as usize;
        let port_end = web.port.end as usize;

        assert!(port_start < port_end, "Invalid port: empty");

        // Check all characters are digits
        let mut i = port_start;
        while i < port_end {
            assert!(is_digit(bytes[i]), "Invalid port: contains non-digit characters");
            i += 1;
        }

        // Port number should be reasonable (1-65535)
        assert!(port_end - port_start <= 5, "Invalid port: too many digits")
    }
}

/// Check if a byte is an ASCII digit
const fn is_digit(b: u8) -> bool {
    b >= b'0' && b <= b'9'
}

// ============================================================================
// Web URL Parser
// ============================================================================

const fn parse_web_url(bytes: &[u8], scheme_end: usize) -> WebComponents {
    let len = bytes.len();

    // Extract scheme: [0..scheme_end]
    let scheme = Span {
        start: 0,
        end: scheme_end as u16,
    };

    // Skip "://"
    let mut pos = scheme_end + 3;
    if pos > len {
        // Malformed URL, return minimal components
        return WebComponents {
            scheme,
            authority: Span::NONE,
            host: Span::NONE,
            port: Span::NONE,
            path: Span::NONE,
            query: Span::NONE,
            fragment: Span::NONE,
        };
    }

    // Find authority end (first '/', '?', or '#')
    let authority_start = pos;
    let authority_end = find_char_or_end(bytes, pos, b"/?#");

    let authority = if authority_end > authority_start {
        Span {
            start: authority_start as u16,
            end: authority_end as u16,
        }
    } else {
        Span::NONE
    };

    // Parse host and port from authority
    let (host, port) = parse_host_port(bytes, authority_start, authority_end);

    pos = authority_end;

    // Parse path
    let path = if pos < len && bytes[pos] == b'/' {
        let path_start = pos;
        let path_end = find_char_or_end(bytes, pos, b"?#");
        pos = path_end;

        Span {
            start: path_start as u16,
            end: path_end as u16,
        }
    } else {
        Span::NONE
    };

    // Parse query
    let query = if pos < len && bytes[pos] == b'?' {
        pos += 1; // Skip '?'
        let query_start = pos;
        let query_end = find_char_or_end(bytes, pos, b"#");
        pos = query_end;

        Span {
            start: query_start as u16,
            end: query_end as u16,
        }
    } else {
        Span::NONE
    };

    // Parse fragment
    let fragment = if pos < len && bytes[pos] == b'#' {
        pos += 1; // Skip '#'
        Span {
            start: pos as u16,
            end: len as u16,
        }
    } else {
        Span::NONE
    };

    WebComponents {
        scheme,
        authority,
        host,
        port,
        path,
        query,
        fragment,
    }
}

/// Parse host and port from authority section
const fn parse_host_port(bytes: &[u8], start: usize, end: usize) -> (Span, Span) {
    if start >= end {
        return (Span::NONE, Span::NONE);
    }

    // Look for '@' to skip userinfo
    let host_start = if let Some(at_pos) = find_byte(bytes, start, end, b'@') {
        at_pos + 1
    } else {
        start
    };

    if host_start >= end {
        return (Span::NONE, Span::NONE);
    }

    // Look for ':' for port (but watch out for IPv6 '[...]')
    let mut in_ipv6 = false;
    let mut i = host_start;
    let mut colon_pos = None;

    while i < end {
        match bytes[i] {
            b'[' => in_ipv6 = true,
            b']' => in_ipv6 = false,
            b':' if !in_ipv6 => colon_pos = Some(i),
            _ => {}
        }
        i += 1;
    }

    if let Some(colon) = colon_pos {
        (
            Span {
                start: host_start as u16,
                end: colon as u16,
            },
            Span {
                start: (colon + 1) as u16,
                end: end as u16,
            },
        )
    } else {
        (
            Span {
                start: host_start as u16,
                end: end as u16,
            },
            Span::NONE,
        )
    }
}

// ============================================================================
// Data URL Parser
// ============================================================================

const fn parse_data_url(bytes: &[u8]) -> DataComponents {
    // Format: data:[<mediatype>][;base64],<data>
    let len = bytes.len();
    let pos = 5; // Skip "data:"

    // Find the comma that separates metadata from data
    let comma_pos = find_byte(bytes, pos, len, b',');

    let (mime_type, encoding, data_start) = if let Some(comma) = comma_pos {
        // Parse metadata part
        let metadata_end = comma;

        // Look for ";" encoding separator
        let semicolon_pos = find_byte(bytes, pos, metadata_end, b';');

        if let Some(semi) = semicolon_pos {
            // Has encoding
            let mime = if semi > pos {
                Span {
                    start: pos as u16,
                    end: semi as u16,
                }
            } else {
                Span::NONE
            };
            let enc = if metadata_end > semi + 1 {
                Span {
                    start: (semi + 1) as u16,
                    end: metadata_end as u16,
                }
            } else {
                Span::NONE
            };
            (mime, enc, comma + 1)
        } else {
            // No encoding, just mime type
            let mime = if metadata_end > pos {
                Span {
                    start: pos as u16,
                    end: metadata_end as u16,
                }
            } else {
                Span::NONE
            };
            (mime, Span::NONE, comma + 1)
        }
    } else {
        // Malformed, but handle gracefully - treat entire rest as data
        (Span::NONE, Span::NONE, pos)
    };

    let data = if data_start < len {
        Span {
            start: data_start as u16,
            end: len as u16,
        }
    } else {
        Span::NONE
    };

    DataComponents {
        mime_type,
        encoding,
        data,
    }
}

// ============================================================================
// Blob URL Parser
// ============================================================================

const fn parse_blob_url(bytes: &[u8]) -> BlobComponents {
    let len = bytes.len();
    let start = 5; // Skip "blob:"

    BlobComponents {
        identifier: if start < len {
            Span {
                start: start as u16,
                end: len as u16,
            }
        } else {
            Span::NONE
        },
    }
}

// ============================================================================
// Local Path Parser
// ============================================================================

const fn parse_local_path(bytes: &[u8]) -> LocalComponents {
    let len = bytes.len();

    // Determine if absolute
    let is_absolute = if len > 0 {
        // Unix absolute: starts with '/'
        bytes[0] == b'/'
            // Windows absolute: "C:\..." or "C:/"
            || (len >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
    } else {
        false
    };

    // Determine if Windows path
    let is_windows = if len >= 3 {
        // Drive letter pattern
        bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
    } else {
        // Check for backslashes anywhere in path
        contains_byte(bytes, b'\\')
    };

    LocalComponents {
        path: Span {
            start: 0,
            end: len as u16,
        },
        is_absolute,
        is_windows,
    }
}

// ============================================================================
// Const Helper Functions
// ============================================================================

/// Find the end of the scheme (position of ':')
const fn find_scheme_end(bytes: &[u8]) -> Option<usize> {
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b':' => return Some(i),
            b'/' | b'?' | b'#' => return None, // No scheme
            _ => i += 1,
        }
    }

    None
}

/// Check if scheme is a web scheme (requires "://")
const fn is_web_scheme(bytes: &[u8], scheme_end: usize) -> bool {
    let len = bytes.len();

    // Must have "://" after scheme
    if len < scheme_end + 3 {
        return false;
    }
    if bytes[scheme_end] != b':' || bytes[scheme_end + 1] != b'/' || bytes[scheme_end + 2] != b'/' {
        return false;
    }

    // Check for known web schemes
    starts_with(bytes, b"http://")
        || starts_with(bytes, b"https://")
        || starts_with(bytes, b"ftp://")
        || starts_with(bytes, b"ftps://")
        || starts_with(bytes, b"ws://")
        || starts_with(bytes, b"wss://")
        || starts_with(bytes, b"rtsp://")
        || starts_with(bytes, b"rtmp://")
}

/// Find first occurrence of any character in set, or end of string
const fn find_char_or_end(bytes: &[u8], start: usize, chars: &[u8]) -> usize {
    let len = bytes.len();
    let mut i = start;

    while i < len {
        let mut j = 0;
        while j < chars.len() {
            if bytes[i] == chars[j] {
                return i;
            }
            j += 1;
        }
        i += 1;
    }

    len
}

/// Find first occurrence of a byte in range
const fn find_byte(bytes: &[u8], start: usize, end: usize, byte: u8) -> Option<usize> {
    let mut i = start;
    while i < end {
        if bytes[i] == byte {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Check if bytes contains a specific byte
const fn contains_byte(bytes: &[u8], byte: u8) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == byte {
            return true;
        }
        i += 1;
    }
    false
}

/// Check if bytes starts with prefix
const fn starts_with(bytes: &[u8], prefix: &[u8]) -> bool {
    if bytes.len() < prefix.len() {
        return false;
    }

    let mut i = 0;
    while i < prefix.len() {
        if bytes[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_web_url_complete() {
        let components = parse_url(b"https://user:pass@example.com:8080/path?query=1#frag");
        if let ParsedComponents::Web(web) = components {
            assert!(web.scheme.is_present());
            assert!(web.authority.is_present());
            assert!(web.host.is_present());
            assert!(web.port.is_present());
            assert!(web.path.is_present());
            assert!(web.query.is_present());
            assert!(web.fragment.is_present());
        } else {
            panic!("Expected Web components");
        }
    }

    #[test]
    fn test_parse_data_url() {
        let components = parse_url(b"data:text/plain;base64,SGVsbG8=");
        if let ParsedComponents::Data(data) = components {
            assert!(data.mime_type.is_present());
            assert!(data.encoding.is_present());
            assert!(data.data.is_present());
        } else {
            panic!("Expected Data components");
        }
    }

    #[test]
    fn test_parse_local_path() {
        let components = parse_url(b"/absolute/path");
        if let ParsedComponents::Local(local) = components {
            assert!(local.is_absolute);
            assert!(!local.is_windows);
        } else {
            panic!("Expected Local components");
        }
    }

    #[test]
    fn test_parse_blob_url() {
        let components = parse_url(b"blob:https://example.com/uuid");
        if let ParsedComponents::Blob(blob) = components {
            assert!(blob.identifier.is_present());
        } else {
            panic!("Expected Blob components");
        }
    }

    #[test]
    fn test_const_evaluation() {
        // Verify const compatibility
        const PARSED: ParsedComponents = parse_url(b"https://example.com/test");
        assert!(matches!(PARSED, ParsedComponents::Web(_)));
    }

    // ========================================================================
    // Error Validation Tests
    // ========================================================================

    #[test]
    #[should_panic(expected = "URL string is empty")]
    fn test_empty_url_panics() {
        parse_url(b"");
    }

    #[test]
    #[should_panic(expected = "Web URL must have a host")]
    fn test_missing_host_panics() {
        parse_url(b"https://");
    }

    #[test]
    #[should_panic(expected = "Web URL must have a host")]
    fn test_empty_host_panics() {
        // URLs like "https:///path" have no authority section, so no host
        parse_url(b"https:///path");
    }

    #[test]
    #[should_panic(expected = "Invalid port: contains non-digit characters")]
    fn test_invalid_port_characters_panics() {
        parse_url(b"https://example.com:abc/path");
    }

    #[test]
    #[should_panic(expected = "Invalid port: too many digits")]
    fn test_port_too_long_panics() {
        parse_url(b"https://example.com:123456/path");
    }

    #[test]
    fn test_valid_port_accepted() {
        // Should not panic
        let components = parse_url(b"https://example.com:8080/path");
        if let ParsedComponents::Web(web) = components {
            assert!(web.port.is_present());
        } else {
            panic!("Expected Web components");
        }
    }

    #[test]
    fn test_max_valid_port() {
        // Port 65535 is the max valid port number (5 digits)
        let components = parse_url(b"https://example.com:65535/path");
        if let ParsedComponents::Web(web) = components {
            assert!(web.port.is_present());
        } else {
            panic!("Expected Web components");
        }
    }
}
