//! Readable previews for structured and binary cell values in the inspector.

use super::*;
use gpui_component::InteractiveElementExt as _;

/// Large values are summarized; the inspector is not a hex editor.
const HEX_PREVIEW_BYTES: usize = 512;
const IMAGE_PREVIEW_BYTES: usize = 4 * 1024 * 1024;
const TEXT_PREVIEW_CHARS: usize = 20_000;

/// Pretty JSON for JSON values and for text or UTF-8 bytes holding an object or array.
pub(super) fn json_preview(value: &CellValue) -> Option<String> {
    let document = |text: &str| {
        let trimmed = text.trim_start();
        if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            return None;
        }
        serde_json::from_str(text).ok()
    };
    let parsed = match value {
        CellValue::Json(value) => value.clone(),
        CellValue::Text(text) => document(text)?,
        CellValue::Bytes(bytes) => document(std::str::from_utf8(bytes).ok()?)?,
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

/// Reuse the editor's lexer and palette, including for truncated JSON previews.
pub(super) fn json_text(text: SharedString) -> gpui::StyledText {
    let highlights = crate::editor::lex_json(&text).into_iter().map(|token| {
        (
            token.range,
            gpui::HighlightStyle {
                color: Some(crate::editor::json_token_color(token.kind)),
                ..Default::default()
            },
        )
    });
    gpui::StyledText::new(text).with_highlights(highlights)
}

fn monospace_block(text: String, json: bool, cx: &App) -> impl IntoElement {
    use gpui_component::ActiveTheme;

    div()
        .id("cell-value-preview")
        .debug_selector(|| "cell-value-preview".into())
        .overflow_x_scroll()
        // Wide JSON/hex lines must not turn vertical wheel input into sideways
        // scrolling; let the inspector's field list handle that gesture.
        .lock_scroll_axis()
        .flex()
        .min_w_0()
        .p(crate::theme::ui_px(7.))
        .rounded(crate::theme::ui_px(5.))
        .bg(theme().panel_raised)
        .border_1()
        .border_color(theme().border)
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(crate::theme::mono_text_px(11.))
        .text_color(theme().text)
        .whitespace_nowrap()
        .child(
            div()
                .debug_selector(|| "cell-value-preview-content".into())
                .flex_none()
                .child(if json {
                    json_text(text.into()).into_any_element()
                } else {
                    text.into_any_element()
                }),
        )
}

/// The inspector body for one value: images, hex dumps, pretty JSON, or
/// the plain display text.
pub(super) fn value_view(value: Option<&CellValue>, cx: &App) -> AnyElement {
    let Some(value) = value else {
        return div()
            .text_size(crate::theme::text_px(12.))
            .text_color(theme().text_muted)
            .child("—")
            .into_any_element();
    };
    if let Some(json) = json_preview(value) {
        return monospace_block(truncate_preview(json), true, cx).into_any_element();
    }
    match value {
        CellValue::Null => div()
            .text_size(crate::theme::text_px(12.))
            .italic()
            .text_color(theme().text_muted)
            .child("NULL")
            .into_any_element(),
        CellValue::Bytes(bytes) => {
            let format = image_format(bytes).filter(|_| bytes.len() <= IMAGE_PREVIEW_BYTES);
            div()
                .flex()
                .flex_col()
                .gap(crate::theme::ui_px(5.))
                .child(
                    div()
                        .text_size(crate::theme::text_px(10.))
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
                            .max_h(crate::theme::ui_px(240.))
                            .object_fit(gpui::ObjectFit::Contain),
                    )
                })
                .when(format.is_none() && !bytes.is_empty(), |view| {
                    view.child(monospace_block(hex_dump(bytes), false, cx))
                })
                .into_any_element()
        }
        value => div()
            .text_size(crate::theme::text_px(12.))
            .text_color(theme().text)
            .whitespace_normal()
            .child(truncate_preview(value.to_string()))
            .into_any_element(),
    }
}

fn truncate_preview(text: String) -> String {
    match text.char_indices().nth(TEXT_PREVIEW_CHARS) {
        Some((end, _)) => format!("{}… (copy for the full value)", &text[..end]),
        None => text,
    }
}

/// The clipboard form of a value: full JSON or text, and hex for binary bytes.
pub(super) fn value_clipboard_text(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".into(),
        CellValue::Text(text) => text.clone(),
        CellValue::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        CellValue::Bytes(_) => json_preview(value).unwrap_or_else(|| value.to_string()),
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
    fn json_bytes_preview_and_copy_preserve_nested_values_and_numeric_precision() {
        let raw = br#" {"actor":{"kind":"user","id":900719925474099312345},"roles":["admin"],"amount":0.123456789012345678901,"missing":null,"label":"\u00e9"} "#;
        let value = CellValue::Bytes(raw.to_vec());
        let preview = json_preview(&value).unwrap();
        assert!(preview.contains("\n  \"actor\": {\n"));
        assert!(preview.contains("900719925474099312345"));
        assert!(preview.contains("0.123456789012345678901"));
        assert!(preview.contains('é'));
        assert_eq!(value_clipboard_text(&value), preview);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&preview).unwrap(),
            serde_json::from_slice::<serde_json::Value>(raw).unwrap()
        );
        assert!(json_preview(&CellValue::Bytes(b"[1,true,null]".to_vec())).is_some());
    }

    #[test]
    fn binary_and_invalid_json_bytes_keep_their_hex_clipboard_form() {
        for bytes in [
            b"{not json".as_slice(),
            b"{\"a\":1} trailing",
            b"[\xff]",
            b"DBX\x00\x01",
            b"42",
            b"",
        ] {
            let value = CellValue::Bytes(bytes.to_vec());
            assert_eq!(json_preview(&value), None);
            assert_eq!(value_clipboard_text(&value), value.to_string());
        }
    }

    #[test]
    fn large_json_preview_is_bounded_but_copy_contains_the_complete_value() {
        let value = CellValue::Bytes(
            serde_json::json!({"text": "é".repeat(TEXT_PREVIEW_CHARS + 1)})
                .to_string()
                .into_bytes(),
        );
        let preview = truncate_preview(json_preview(&value).unwrap());
        assert!(preview.ends_with("… (copy for the full value)"));
        let copied: serde_json::Value =
            serde_json::from_str(&value_clipboard_text(&value)).unwrap();
        assert_eq!(
            copied["text"].as_str().unwrap().chars().count(),
            TEXT_PREVIEW_CHARS + 1
        );
    }

    struct PreviewHarness(CellValue);

    impl Render for PreviewHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(200.)).child(value_view(Some(&self.0), cx))
        }
    }

    #[gpui::test]
    fn structured_previews_scroll_long_lines_without_wrapping_or_widening_the_inspector(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (harness, cx) = cx.add_window_view(|_, _| {
            PreviewHarness(CellValue::Bytes(
                serde_json::json!({"long_key": "x".repeat(100)})
                    .to_string()
                    .into_bytes(),
            ))
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let json_bounds = cx.debug_bounds("cell-value-preview").unwrap();
        let content = cx.debug_bounds("cell-value-preview-content").unwrap();
        assert!(json_bounds.size.width <= px(200.));
        assert!(content.size.width > json_bounds.size.width);
        harness.update(cx, |harness, cx| {
            harness.0 = CellValue::Bytes(vec![0; 32]);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let hex_bounds = cx.debug_bounds("cell-value-preview").unwrap();
        let content = cx.debug_bounds("cell-value-preview-content").unwrap();
        assert!(hex_bounds.size.width <= px(200.));
        assert!(content.size.width > hex_bounds.size.width);
        // Both previews contain three lines (the dump has a trailing newline).
        assert_eq!(hex_bounds.size.height, json_bounds.size.height);
    }

    struct ScrollingPreviewHarness {
        value: CellValue,
        scroll: gpui::ScrollHandle,
    }

    impl Render for ScrollingPreviewHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("preview-fields-scroll")
                .w(px(200.))
                .h(px(240.))
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .lock_scroll_axis()
                .track_scroll(&self.scroll)
                .child(div().flex_none().child(value_view(Some(&self.value), cx)))
                .child(div().h(px(600.)).flex_none())
        }
    }

    #[gpui::test]
    fn wide_previews_leave_vertical_wheel_input_to_the_field_list(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (harness, cx) = cx.add_window_view(|_, _| ScrollingPreviewHarness {
            value: CellValue::Json(serde_json::json!({"long_key": "x".repeat(100)})),
            scroll: gpui::ScrollHandle::default(),
        });
        for value in [
            CellValue::Json(serde_json::json!({"long_key": "x".repeat(100)})),
            CellValue::Bytes(vec![0; 32]),
        ] {
            harness.update(cx, |harness, cx| {
                harness.value = value;
                harness.scroll.set_offset(gpui::point(px(0.), px(0.)));
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let position = cx.debug_bounds("cell-value-preview").unwrap().center();
            let before = cx.debug_bounds("cell-value-preview-content").unwrap();

            // Sideways scrolling stays within the preview.
            cx.simulate_event(gpui::ScrollWheelEvent {
                position,
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(-40.), px(0.))),
                touch_phase: gpui::TouchPhase::Started,
                ..Default::default()
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let after = cx.debug_bounds("cell-value-preview-content").unwrap();
            assert!(after.left() < before.left());
            harness.read_with(cx, |harness, _| {
                assert_eq!(harness.scroll.offset().y, px(0.))
            });

            // Both a trackpad and a mouse wheel move the parent field list,
            // even when the pointer is over a line wider than the viewport.
            for delta in [
                gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-20.))),
                gpui::ScrollDelta::Lines(gpui::point(0., -1.)),
            ] {
                let position = cx.debug_bounds("cell-value-preview").unwrap().center();
                let before = cx.debug_bounds("cell-value-preview-content").unwrap();
                let offset = harness.read_with(cx, |harness, _| harness.scroll.offset().y);
                cx.simulate_event(gpui::ScrollWheelEvent {
                    position,
                    delta,
                    touch_phase: gpui::TouchPhase::Started,
                    ..Default::default()
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let after = cx.debug_bounds("cell-value-preview-content").unwrap();
                assert_eq!(after.left(), before.left());
                harness.read_with(cx, |harness, _| assert!(harness.scroll.offset().y < offset));
            }
        }
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
