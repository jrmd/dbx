//! Readable previews for structured and binary cell values in the inspector.

use super::*;

/// Large values are summarized; the inspector is not a hex editor.
const HEX_PREVIEW_BYTES: usize = 512;
const IMAGE_PREVIEW_BYTES: usize = 4 * 1024 * 1024;
const TEXT_PREVIEW_CHARS: usize = 20_000;

/// Pretty JSON for JSON values and for text holding a JSON object or array.
pub(super) fn json_preview(value: &CellValue) -> Option<String> {
    let parsed = match value {
        CellValue::Json(value) => value.clone(),
        CellValue::Text(text) => {
            let trimmed = text.trim_start();
            if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
                return None;
            }
            serde_json::from_str(text).ok()?
        }
        _ => return None,
    };
    serde_json::to_string_pretty(&parsed).ok()
}

/// A classic offset/hex/ASCII dump of the first bytes.
pub(super) fn hex_dump(bytes: &[u8]) -> String {
    let mut output = String::new();
    for (line, chunk) in bytes.chunks(16).take(HEX_PREVIEW_BYTES / 16).enumerate() {
        let hex = chunk
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let ascii = chunk
            .iter()
            .map(|byte| {
                if byte.is_ascii_graphic() || *byte == b' ' {
                    *byte as char
                } else {
                    '.'
                }
            })
            .collect::<String>();
        output.push_str(&format!("{:08x}  {hex:<47}  {ascii}\n", line * 16));
    }
    if bytes.len() > HEX_PREVIEW_BYTES {
        output.push_str(&format!("… {} more bytes", bytes.len() - HEX_PREVIEW_BYTES));
    }
    output
}

fn image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

fn byte_size(length: usize) -> String {
    match length {
        0..1024 => counted(length, "byte", "bytes"),
        1024..1_048_576 => format!("{:.1} KiB", length as f64 / 1024.0),
        _ => format!("{:.1} MiB", length as f64 / 1_048_576.0),
    }
}

fn monospace_block(text: String) -> Div {
    div()
        .p(px(7.))
        .rounded(px(5.))
        .bg(theme().panel_raised)
        .border_1()
        .border_color(theme().border)
        .font_family("monospace")
        .text_size(px(11.))
        .text_color(theme().text)
        .whitespace_normal()
        .child(text)
}

/// The inspector body for one value: images, hex dumps, pretty JSON, or
/// the plain display text.
pub(super) fn value_view(value: Option<&CellValue>) -> AnyElement {
    let Some(value) = value else {
        return div()
            .text_size(px(12.))
            .text_color(theme().text_muted)
            .child("—")
            .into_any_element();
    };
    match value {
        CellValue::Null => div()
            .text_size(px(12.))
            .italic()
            .text_color(theme().text_muted)
            .child("NULL")
            .into_any_element(),
        CellValue::Bytes(bytes) => {
            let format = image_format(bytes).filter(|_| bytes.len() <= IMAGE_PREVIEW_BYTES);
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme().text_muted)
                        .child(match format {
                            Some(format) => {
                                format!("{format:?} image · {}", byte_size(bytes.len()))
                            }
                            None => format!("Binary · {}", byte_size(bytes.len())),
                        }),
                )
                .when_some(format, |view, format| {
                    view.child(
                        img(Arc::new(Image::from_bytes(format, bytes.clone())))
                            .max_w_full()
                            .max_h(px(240.))
                            .object_fit(gpui::ObjectFit::Contain),
                    )
                })
                .when(format.is_none() && !bytes.is_empty(), |view| {
                    view.child(monospace_block(hex_dump(bytes)))
                })
                .into_any_element()
        }
        value => match json_preview(value) {
            Some(json) => monospace_block(truncate_preview(json)).into_any_element(),
            None => div()
                .text_size(px(12.))
                .text_color(theme().text)
                .whitespace_normal()
                .child(truncate_preview(value.to_string()))
                .into_any_element(),
        },
    }
}

fn truncate_preview(text: String) -> String {
    match text.char_indices().nth(TEXT_PREVIEW_CHARS) {
        Some((end, _)) => format!("{}… (copy for the full value)", &text[..end]),
        None => text,
    }
}

/// The clipboard form of a value: full JSON or text, and hex for bytes.
pub(super) fn value_clipboard_text(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".into(),
        CellValue::Text(text) => text.clone(),
        CellValue::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        value => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_text_is_pretty_printed_but_plain_text_is_not() {
        assert_eq!(
            json_preview(&CellValue::Text("{\"a\":[1,2]}".into())).unwrap(),
            "{\n  \"a\": [\n    1,\n    2\n  ]\n}"
        );
        assert_eq!(json_preview(&CellValue::Text("42".into())), None);
        assert_eq!(json_preview(&CellValue::Text("{not json".into())), None);
    }

    #[test]
    fn hex_dump_shows_offsets_hex_and_printable_ascii() {
        let dump = hex_dump(b"DBX\x00\x01 ok");
        assert_eq!(
            dump,
            "00000000  44 42 58 00 01 20 6f 6b                          DBX.. ok\n"
        );
        assert!(hex_dump(&[0; 600]).ends_with("… 88 more bytes"));
        assert_eq!(
            image_format(b"\x89PNG\r\n\x1a\nrest"),
            Some(ImageFormat::Png)
        );
        assert_eq!(image_format(b"plain"), None);
    }
}
