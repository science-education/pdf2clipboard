//! Unicode encoding support for PDF text.
//!
//! This module handles encoding conversions for PDF text strings,
//! particularly for CID fonts with Identity-H encoding.
//!
//! # Identity-H Encoding
//!
//! Per PDF spec Section 9.7.5.2, Identity-H is a predefined CMap that
//! maps CIDs directly to character codes (i.e., CID = GID for TrueType).
//! This allows using glyph IDs directly in content streams as:
//!
//! ```text
//! <0001002300A4> Tj
//! ```
//!
//! Where each 4-digit hex value is a glyph ID (big-endian u16).

use std::collections::HashMap;

/// Unicode encoder for PDF text strings.
///
/// Converts Unicode text to PDF string format for different encodings.
#[derive(Debug)]
pub struct UnicodeEncoder {
    /// Glyph lookup function results cache
    glyph_cache: HashMap<u32, u16>,
}

impl UnicodeEncoder {
    /// Create a new Unicode encoder.
    pub fn new() -> Self {
        Self {
            glyph_cache: HashMap::new(),
        }
    }

    /// Encode a Unicode string to Identity-H format (hex string of glyph IDs).
    ///
    /// # Arguments
    /// * `text` - Unicode text to encode
    /// * `glyph_lookup` - Function to convert Unicode codepoint to glyph ID
    ///
    /// # Returns
    /// Hex-encoded string suitable for Tj/TJ operators, e.g., "<00410042>"
    pub fn encode_identity_h(
        &mut self,
        text: &str,
        glyph_lookup: impl Fn(u32) -> Option<u16>,
    ) -> String {
        let mut hex = String::with_capacity(text.len() * 4 + 2);
        hex.push('<');

        for ch in text.chars() {
            let codepoint = ch as u32;
            let glyph_id = self
                .glyph_cache
                .get(&codepoint)
                .copied()
                .or_else(|| {
                    let gid = glyph_lookup(codepoint)?;
                    self.glyph_cache.insert(codepoint, gid);
                    Some(gid)
                })
                .unwrap_or(0); // Use .notdef for missing glyphs

            hex.push_str(&format!("{:04X}", glyph_id));
        }

        hex.push('>');
        hex
    }

    /// Encode a single character to Identity-H format.
    pub fn encode_char_identity_h(&self, glyph_id: u16) -> String {
        format!("<{:04X}>", glyph_id)
    }

    /// Encode text as PDF literal string (for WinAnsi/MacRoman encoding).
    ///
    /// Characters outside the encoding are replaced with '?'.
    pub fn encode_literal(text: &str) -> String {
        let mut result = String::with_capacity(text.len() + 2);
        result.push('(');

        for ch in text.chars() {
            match ch {
                // Escape special characters
                '(' => result.push_str("\\("),
                ')' => result.push_str("\\)"),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                // Printable ASCII
                c if c.is_ascii() && c >= ' ' => result.push(c),
                // Non-ASCII - encode as octal if in Latin-1
                c if (c as u32) < 256 => {
                    result.push_str(&format!("\\{:03o}", c as u32));
                },
                // Characters outside Latin-1 - replace with ?
                _ => result.push('?'),
            }
        }

        result.push(')');
        result
    }

    /// Encode text as PDF hex string for UTF-16BE.
    ///
    /// Used for metadata strings and bookmarks that need full Unicode.
    pub fn encode_utf16be(text: &str) -> String {
        let mut hex = String::new();
        hex.push('<');

        // BOM for UTF-16BE
        hex.push_str("FEFF");

        for ch in text.chars() {
            let codepoint = ch as u32;
            if codepoint <= 0xFFFF {
                // BMP character
                hex.push_str(&format!("{:04X}", codepoint));
            } else {
                // Supplementary plane - encode as surrogate pair
                let adjusted = codepoint - 0x10000;
                let high = ((adjusted >> 10) & 0x3FF) + 0xD800;
                let low = (adjusted & 0x3FF) + 0xDC00;
                hex.push_str(&format!("{:04X}{:04X}", high, low));
            }
        }

        hex.push('>');
        hex
    }

    /// Encode text as PDF literal string if ASCII, otherwise as UTF-16BE hex.
    ///
    /// This is the recommended approach for general PDF strings.
    pub fn encode_text(text: &str) -> String {
        if text
            .chars()
            .all(|c| c.is_ascii() && c >= ' ' && c != '(' && c != ')' && c != '\\')
        {
            // Simple ASCII - use literal string
            format!("({})", text)
        } else if text.chars().all(|c| (c as u32) < 256) {
            // Latin-1 compatible - use escaped literal
            Self::encode_literal(text)
        } else {
            // Full Unicode - use UTF-16BE
            Self::encode_utf16be(text)
        }
    }

    /// Clear the glyph cache.
    pub fn clear_cache(&mut self) {
        self.glyph_cache.clear();
    }

    /// Get cache statistics.
    pub fn cache_size(&self) -> usize {
        self.glyph_cache.len()
    }
}

impl Default for UnicodeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// WinAnsi (Windows-1252) encoding table.
///
/// Maps Unicode codepoints to WinAnsi byte values for the range 0x80-0x9F
/// which differs from Latin-1.
pub fn unicode_to_winansi(codepoint: u32) -> Option<u8> {
    // Direct mapping for 0x00-0x7F and 0xA0-0xFF
    if codepoint < 0x80 || (0xA0..=0xFF).contains(&codepoint) {
        return Some(codepoint as u8);
    }

    // Special WinAnsi characters in 0x80-0x9F range
    match codepoint {
        0x20AC => Some(0x80), // Euro sign
        0x201A => Some(0x82), // Single low-9 quotation mark
        0x0192 => Some(0x83), // Latin small letter f with hook
        0x201E => Some(0x84), // Double low-9 quotation mark
        0x2026 => Some(0x85), // Horizontal ellipsis
        0x2020 => Some(0x86), // Dagger
        0x2021 => Some(0x87), // Double dagger
        0x02C6 => Some(0x88), // Modifier letter circumflex accent
        0x2030 => Some(0x89), // Per mille sign
        0x0160 => Some(0x8A), // Latin capital letter S with caron
        0x2039 => Some(0x8B), // Single left-pointing angle quotation mark
        0x0152 => Some(0x8C), // Latin capital ligature OE
        0x017D => Some(0x8E), // Latin capital letter Z with caron
        0x2018 => Some(0x91), // Left single quotation mark
        0x2019 => Some(0x92), // Right single quotation mark
        0x201C => Some(0x93), // Left double quotation mark
        0x201D => Some(0x94), // Right double quotation mark
        0x2022 => Some(0x95), // Bullet
        0x2013 => Some(0x96), // En dash
        0x2014 => Some(0x97), // Em dash
        0x02DC => Some(0x98), // Small tilde
        0x2122 => Some(0x99), // Trade mark sign
        0x0161 => Some(0x9A), // Latin small letter s with caron
        0x203A => Some(0x9B), // Single right-pointing angle quotation mark
        0x0153 => Some(0x9C), // Latin small ligature oe
        0x017E => Some(0x9E), // Latin small letter z with caron
        0x0178 => Some(0x9F), // Latin capital letter Y with diaeresis
        _ => None,
    }
}

/// Check if a character can be encoded in WinAnsi.
pub fn is_winansi_char(ch: char) -> bool {
    unicode_to_winansi(ch as u32).is_some()
}

/// Escape a byte for PDF literal string.
fn escape_byte_for_literal(b: u8) -> String {
    match b {
        b'(' => "\\(".to_string(),
        b')' => "\\)".to_string(),
        b'\\' => "\\\\".to_string(),
        0x0A => "\\n".to_string(),
        0x0D => "\\r".to_string(),
        0x09 => "\\t".to_string(),
        0x08 => "\\b".to_string(),
        0x0C => "\\f".to_string(),
        b if (0x20..0x7F).contains(&b) => (b as char).to_string(),
        b => format!("\\{:03o}", b),
    }
}

/// Encode bytes as PDF literal string with proper escaping.
pub fn encode_bytes_as_literal(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2 + 2);
    result.push('(');
    for &b in bytes {
        result.push_str(&escape_byte_for_literal(b));
    }
    result.push(')');
    result
}

/// Encode bytes as PDF hex string.
pub fn encode_bytes_as_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2 + 2);
    result.push('<');
    for b in bytes {
        result.push_str(&format!("{:02X}", b));
    }
    result.push('>');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_identity_h() {
        let mut encoder = UnicodeEncoder::new();
        // Simple lookup: A=1, B=2
        let lookup = |cp: u32| match cp {
            0x41 => Some(1_u16),
            0x42 => Some(2_u16),
            _ => None,
        };

        let result = encoder.encode_identity_h("AB", lookup);
        assert_eq!(result, "<00010002>");
    }

    #[test]
    fn test_encode_identity_h_missing_glyph() {
        let mut encoder = UnicodeEncoder::new();
        let lookup = |_: u32| None;

        let result = encoder.encode_identity_h("A", lookup);
        assert_eq!(result, "<0000>"); // .notdef
    }

    #[test]
    fn test_encode_literal_simple() {
        let result = UnicodeEncoder::encode_literal("Hello");
        assert_eq!(result, "(Hello)");
    }

    #[test]
    fn test_encode_literal_escapes() {
        let result = UnicodeEncoder::encode_literal("(test)");
        assert_eq!(result, "(\\(test\\))");

        let result = UnicodeEncoder::encode_literal("back\\slash");
        assert_eq!(result, "(back\\\\slash)");
    }

    #[test]
    fn test_encode_utf16be() {
        // Simple ASCII
        let result = UnicodeEncoder::encode_utf16be("A");
        assert_eq!(result, "<FEFF0041>");

        // BMP character (Euro sign)
        let result = UnicodeEncoder::encode_utf16be("\u{20AC}");
        assert_eq!(result, "<FEFF20AC>");
    }

    #[test]
    fn test_encode_utf16be_supplementary() {
        // Emoji (U+1F600 = GRINNING FACE)
        let result = UnicodeEncoder::encode_utf16be("\u{1F600}");
        // U+1F600 = surrogate pair D83D DE00
        assert_eq!(result, "<FEFFD83DDE00>");
    }

    #[test]
    fn test_encode_text_auto() {
        // Pure ASCII -> literal
        let result = UnicodeEncoder::encode_text("Hello");
        assert_eq!(result, "(Hello)");

        // Unicode -> UTF-16BE
        let result = UnicodeEncoder::encode_text("Hello\u{20AC}World");
        assert!(result.starts_with("<FEFF"));
    }

    #[test]
    fn test_winansi_mapping() {
        assert_eq!(unicode_to_winansi(0x41), Some(0x41)); // 'A'
        assert_eq!(unicode_to_winansi(0x20AC), Some(0x80)); // Euro
        assert_eq!(unicode_to_winansi(0x2019), Some(0x92)); // Right single quote
        assert_eq!(unicode_to_winansi(0x10000), None); // Outside range
    }

    #[test]
    fn test_is_winansi_char() {
        assert!(is_winansi_char('A'));
        assert!(is_winansi_char('\u{20AC}')); // Euro
        assert!(!is_winansi_char('\u{4E2D}')); // Chinese character
    }

    #[test]
    fn test_encode_bytes_as_hex() {
        let result = encode_bytes_as_hex(&[0x41, 0x42, 0x43]);
        assert_eq!(result, "<414243>");
    }

    #[test]
    fn test_encode_bytes_as_literal() {
        let result = encode_bytes_as_literal(b"ABC");
        assert_eq!(result, "(ABC)");

        let result = encode_bytes_as_literal(&[0x28, 0x29]); // '(' ')'
        assert_eq!(result, "(\\(\\))");
    }

    #[test]
    fn test_encoder_caching() {
        let mut encoder = UnicodeEncoder::new();
        let lookup = |cp: u32| Some(cp as u16);

        encoder.encode_identity_h("AAA", lookup);
        assert_eq!(encoder.cache_size(), 1); // Only 'A' cached

        encoder.encode_identity_h("ABC", lookup);
        assert_eq!(encoder.cache_size(), 3); // A, B, C cached

        encoder.clear_cache();
        assert_eq!(encoder.cache_size(), 0);
    }
}
