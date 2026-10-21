use dbx_core::CellValue;
use uuid::Uuid;

/// Binary representations use explicit byte order; MySQL's swapped form matches UUID_TO_BIN(uuid, 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BinaryFormat {
    Text,
    Binary,
    #[default]
    Hex,
    Uuid,
    UuidSwapped,
}

impl BinaryFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "UTF-8 text",
            Self::Binary => "binary digits",
            Self::Hex => "hex bytes",
            Self::Uuid => "UUID",
            Self::UuidSwapped => "UUID (MySQL swapped)",
        }
    }

    pub fn limited_text(self, value: &CellValue, max_bytes: usize) -> String {
        if self == Self::Text {
            if let Some(text) = Self::utf8_text(value) {
                if text.len() > max_bytes {
                    let end = text.floor_char_boundary(max_bytes);
                    return format!("{}… ({} bytes total)", &text[..end], text.len());
                }
                return text.to_owned();
            }
            return Self::Hex.limited_text(value, max_bytes);
        }
        if matches!(self, Self::Binary | Self::Hex)
            && let CellValue::Bytes(bytes) = value
            && bytes.len() > max_bytes
        {
            return format!(
                "{}… ({} bytes total)",
                self.text(&CellValue::Bytes(bytes[..max_bytes].to_vec())),
                bytes.len()
            );
        }
        self.text(value)
    }

    pub fn text(self, value: &CellValue) -> String {
        let CellValue::Bytes(bytes) = value else {
            return value.to_string();
        };
        match self {
            Self::Text => Self::utf8_text(value)
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string()),
            Self::Binary => bytes
                .iter()
                .map(|byte| format!("{byte:08b}"))
                .collect::<Vec<_>>()
                .join(" "),
            Self::Hex => value.to_string(),
            Self::Uuid | Self::UuidSwapped => {
                let Ok(mut bytes) = <[u8; 16]>::try_from(bytes.as_slice()) else {
                    return format!("Invalid UUID · {} bytes (requires 16 bytes)", bytes.len());
                };
                if self == Self::UuidSwapped {
                    let original = bytes;
                    bytes[..4].copy_from_slice(&original[4..8]);
                    bytes[4..6].copy_from_slice(&original[2..4]);
                    bytes[6..8].copy_from_slice(&original[..2]);
                }
                Uuid::from_bytes(bytes).hyphenated().to_string()
            }
        }
    }

    pub fn editor_text(self, value: &CellValue) -> String {
        self.text(value)
    }

    /// Only lossless, readable UTF-8 is shown as text. Control bytes and invalid
    /// encodings stay hex so opening an editor cannot replace bytes with U+FFFD.
    pub fn utf8_text(value: &CellValue) -> Option<&str> {
        let CellValue::Bytes(bytes) = value else {
            return None;
        };
        let text = std::str::from_utf8(bytes).ok()?;
        text.chars()
            .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
            .then_some(text)
    }

    pub fn for_edit(self, value: &CellValue) -> Self {
        match value {
            CellValue::Bytes(bytes)
                if (self == Self::Text && Self::utf8_text(value).is_none())
                    || (matches!(self, Self::Uuid | Self::UuidSwapped) && bytes.len() != 16) =>
            {
                Self::Hex
            }
            _ => self,
        }
    }

    pub fn parse(self, text: &str) -> Result<CellValue, String> {
        if self == Self::Text {
            Ok(CellValue::Bytes(text.as_bytes().to_vec()))
        } else if matches!(self, Self::Uuid | Self::UuidSwapped) {
            let uuid = Uuid::parse_str(text.trim()).map_err(|_| {
                "Enter a valid UUID (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)".to_owned()
            })?;
            let mut bytes = *uuid.as_bytes();
            if self == Self::UuidSwapped {
                let original = bytes;
                bytes[..2].copy_from_slice(&original[6..8]);
                bytes[2..4].copy_from_slice(&original[4..6]);
                bytes[4..8].copy_from_slice(&original[..4]);
            }
            Ok(CellValue::Bytes(bytes.to_vec()))
        } else if self == Self::Binary {
            let digits: String = text
                .chars()
                .filter(|ch| !ch.is_ascii_whitespace())
                .collect();
            if !digits.len().is_multiple_of(8)
                || !digits.bytes().all(|byte| matches!(byte, b'0' | b'1'))
            {
                return Err("Enter groups of eight binary digits (0 or 1)".into());
            }
            Ok(CellValue::Bytes(
                digits
                    .as_bytes()
                    .chunks(8)
                    .map(|chunk| {
                        u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 2).unwrap()
                    })
                    .collect(),
            ))
        } else {
            parse_hex_bytes(text)
                .map(CellValue::Bytes)
                .map_err(str::to_owned)
        }
    }
}

pub fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, &'static str> {
    let compact: String = text
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    let digits = compact
        .strip_prefix("0x")
        .or_else(|| compact.strip_prefix("0X"))
        .unwrap_or(&compact);
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("use hexadecimal digits only");
    }
    if !digits.len().is_multiple_of(2) {
        return Err("use an even number of hexadecimal digits, optionally prefixed with 0x");
    }
    digits
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_| "use hexadecimal digits only")?;
            u8::from_str_radix(pair, 16).map_err(|_| "use hexadecimal digits only")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_binary_editing_preserves_exact_bytes_and_hex_fallback() {
        for text in [
            "",
            "null",
            "0x1234",
            " \t\r\n",
            " {\"label\":\"é🙂\", \"id\":900719925474099312345}\n",
        ] {
            let value = CellValue::Bytes(text.as_bytes().to_vec());
            let format = BinaryFormat::Text.for_edit(&value);
            assert_eq!(format, BinaryFormat::Text);
            assert_eq!(format.editor_text(&value), text);
            assert_eq!(format.parse(&format.editor_text(&value)).unwrap(), value);
        }
        for bytes in [vec![0xff, 0xc3], vec![b'a', 0, b'b'], vec![0; 16]] {
            let value = CellValue::Bytes(bytes);
            let format = BinaryFormat::Text.for_edit(&value);
            assert_eq!(format, BinaryFormat::Hex);
            assert_eq!(format.parse(&format.editor_text(&value)).unwrap(), value);
            assert_eq!(BinaryFormat::Text.text(&value), format.text(&value));
        }
        let value = CellValue::Bytes("é🙂tail".as_bytes().to_vec());
        assert_eq!(
            BinaryFormat::Text.limited_text(&value, 4),
            "é… (10 bytes total)"
        );
        assert_eq!(BinaryFormat::Text.editor_text(&value), "é🙂tail");
    }
}
