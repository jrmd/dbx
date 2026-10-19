use dbx_core::CellValue;
use uuid::Uuid;

/// Binary representations use explicit byte order; MySQL's swapped form matches UUID_TO_BIN(uuid, 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum BinaryFormat {
    Binary,
    #[default]
    Hex,
    Uuid,
    UuidSwapped,
}

impl BinaryFormat {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Binary => "binary digits",
            Self::Hex => "hex bytes",
            Self::Uuid => "UUID",
            Self::UuidSwapped => "UUID (MySQL swapped)",
        }
    }

    pub(crate) fn limited_text(self, value: &CellValue, max_bytes: usize) -> String {
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

    pub(crate) fn text(self, value: &CellValue) -> String {
        let CellValue::Bytes(bytes) = value else {
            return value.to_string();
        };
        match self {
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

    pub(crate) fn editor_text(self, value: &CellValue) -> String {
        self.text(value)
    }

    pub(crate) fn parse(self, text: &str) -> Result<CellValue, String> {
        if matches!(self, Self::Uuid | Self::UuidSwapped) {
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
            crate::row_drafts::parse_hex_bytes(text)
                .map(CellValue::Bytes)
                .map_err(str::to_owned)
        }
    }
}
