//! The text protocol shared by every DBX field and query editor.
//!
//! AccessKit positions count selectable characters within text runs, whereas
//! the editor uses UTF-8 byte offsets. Keep that conversion at this boundary.

use gpui::accesskit::{Node, NodeId, Rect, Role, TextDirection, TextPosition, TextSelection};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Default)]
pub(super) struct AccessibleText {
    pub text: String,
    pub runs: Vec<AccessibleRun>,
    word_starts: Vec<usize>,
}

#[derive(Clone)]
pub(super) struct AccessibleRun {
    pub id: NodeId,
    pub range: Range<usize>,
    /// UTF-8 offsets, including the end position, relative to the run.
    boundaries: Vec<usize>,
    geometry: Option<RunGeometry>,
}

#[derive(Clone)]
struct RunGeometry {
    bounds: Rect,
    positions: Vec<f32>,
    widths: Vec<f32>,
}

impl AccessibleText {
    pub fn new(text: String) -> Self {
        let mut offset = 0;
        let mut runs = Vec::new();
        for line in text.split_inclusive('\n') {
            let run = AccessibleRun::new(offset..offset + line.len(), line);
            // Word start indices are also encoded as u8 by AccessKit. Split
            // long lines into runs so no character or word offset truncates.
            for chunk in run.boundaries[..run.boundaries.len() - 1].chunks(200) {
                let start = chunk[0];
                let end = *run
                    .boundaries
                    .get(run.boundaries.partition_point(|byte| *byte < start) + chunk.len())
                    .unwrap();
                let mut boundaries = chunk.iter().map(|byte| byte - start).collect::<Vec<_>>();
                boundaries.push(end - start);
                runs.push(AccessibleRun {
                    id: NodeId(0),
                    range: offset + start..offset + end,
                    boundaries,
                    geometry: None,
                });
            }
            offset += line.len();
        }
        if text.is_empty() || text.ends_with('\n') {
            runs.push(AccessibleRun::new(offset..offset, ""));
        }
        let mut word_starts = vec![0];
        let mut offset = 0;
        while offset < text.len() {
            let next = super::next_word_boundary(&text, offset);
            if next <= offset {
                break;
            }
            offset = next;
            if offset < text.len() {
                word_starts.push(offset);
            }
        }
        Self {
            text,
            runs,
            word_starts,
        }
    }

    pub fn selection(&self, range: Range<usize>, reversed: bool) -> TextSelection {
        let start = self.position(range.start);
        let end = self.position(range.end);
        TextSelection {
            anchor: if reversed { end } else { start },
            focus: if reversed { start } else { end },
        }
    }

    fn position(&self, offset: usize) -> TextPosition {
        let offset = offset.min(self.text.len());
        let run = self
            .runs
            .iter()
            .rev()
            .find(|run| run.range.start <= offset)
            .expect("even an empty editor has a text run");
        let relative = offset - run.range.start;
        TextPosition {
            node: run.id,
            character_index: run.boundaries.partition_point(|byte| *byte <= relative) - 1,
        }
    }

    pub fn offset(&self, position: TextPosition) -> Option<usize> {
        let run = self.runs.iter().find(|run| run.id == position.node)?;
        Some(run.range.start + *run.boundaries.get(position.character_index)?)
    }

    pub fn node(&self, run: &AccessibleRun) -> Node {
        let value = &self.text[run.range.clone()];
        let mut node = Node::new(Role::TextRun);
        node.set_value(value);
        node.set_character_lengths(
            run.boundaries
                .windows(2)
                .map(|pair| (pair[1] - pair[0]) as u8)
                .collect::<Vec<_>>(),
        );
        // Match the editor's word navigation rather than relying on a screen
        // reader to guess SQL or Unicode word boundaries.
        let mut words = self
            .word_starts
            .iter()
            .skip_while(|byte| **byte < run.range.start)
            .take_while(|byte| **byte < run.range.end)
            .map(|byte| {
                run.boundaries
                    .partition_point(|boundary| *boundary < byte - run.range.start)
                    as u8
            })
            .collect::<Vec<_>>();
        words.sort_unstable();
        words.dedup();
        node.set_word_starts(words);
        let index = self
            .runs
            .binary_search_by_key(&run.range.start, |run| run.range.start)
            .unwrap();
        if index > 0 && !self.text[self.runs[index - 1].range.clone()].ends_with('\n') {
            node.set_previous_on_line(self.runs[index - 1].id);
        }
        if let Some(next) = self.runs.get(index + 1).filter(|_| !value.ends_with('\n')) {
            node.set_next_on_line(next.id);
        }
        if let Some(geometry) = &run.geometry {
            node.set_bounds(geometry.bounds);
            node.set_text_direction(TextDirection::LeftToRight);
            node.set_character_positions(geometry.positions.clone());
            node.set_character_widths(geometry.widths.clone());
        }
        node
    }

    /// Match the painted viewport and caret line. Offscreen/folded runs stay
    /// readable, but have no invented glyph bounds. Selecting their text uses
    /// the editor's existing unfold/reveal path and shapes it on the next frame.
    pub fn update_geometry(
        &mut self,
        line_starts: &[usize],
        layout: &super::LineLayouts,
        bounds: gpui::Bounds<gpui::Pixels>,
        line_height: gpui::Pixels,
        scale: f32,
    ) {
        for run in &mut self.runs {
            run.geometry = None;
            let line = line_starts
                .partition_point(|start| *start <= run.range.start)
                .saturating_sub(1);
            let Some(shaped) = layout.get(line) else {
                continue;
            };
            let line_start = line_starts[line];
            let advances = run
                .boundaries
                .iter()
                .map(|byte| {
                    f32::from(shaped.x_for_index(run.range.start + byte - line_start)) * scale
                })
                .collect::<Vec<_>>();
            // GPUI's shaped-line coordinates are physical left-to-right x
            // positions even for a non-Latin string. Preserve those exact
            // advances rather than estimating each glyph from its byte count.
            let left = advances.iter().copied().fold(f32::INFINITY, f32::min);
            let right = advances.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let top = f32::from(bounds.top() + layout.top(line, line_height)) * scale;
            let origin = f32::from(bounds.left()) * scale;
            run.geometry = Some(RunGeometry {
                bounds: Rect {
                    x0: (origin + left) as f64,
                    y0: top as f64,
                    x1: (origin + right) as f64,
                    y1: (top + f32::from(line_height) * scale) as f64,
                },
                positions: advances
                    .windows(2)
                    .map(|pair| pair[0].min(pair[1]) - left)
                    .collect(),
                widths: advances
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]).abs())
                    .collect(),
            });
        }
    }
}

impl AccessibleRun {
    fn new(range: Range<usize>, text: &str) -> Self {
        let mut boundaries = Vec::new();
        for (offset, grapheme) in text.grapheme_indices(true) {
            // AccessKit encodes a character length in one byte. An unusually
            // long combining sequence must be split at scalar boundaries.
            if grapheme.len() <= u8::MAX as usize {
                boundaries.push(offset);
            } else {
                boundaries.extend(grapheme.char_indices().map(|(index, _)| offset + index));
            }
        }
        boundaries.push(text.len());
        Self {
            id: NodeId(0),
            range,
            boundaries,
            geometry: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GeometryHarness;
    impl gpui::Render for GeometryHarness {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    #[gpui::test]
    fn geometry_matches_shaped_unicode_split_runs_and_physical_window_scale(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_, cx) = cx.add_window_view(|_, _| GeometryHarness);
        cx.update(|window, _| {
            let text = format!("a👩‍💻e\u{301}\n{}\noffscreen", "w".repeat(230));
            let mut snapshot = AccessibleText::new(text.clone());
            for (index, run) in snapshot.runs.iter_mut().enumerate() {
                run.id = NodeId(index as u64 + 1);
            }
            let starts = vec![0, "a👩‍💻e\u{301}\n".len(), "a👩‍💻e\u{301}\n".len() + 231];
            let shape = |line: &str| {
                window.text_system().shape_line(
                    line.to_owned().into(),
                    gpui::px(12.),
                    &[gpui::TextRun {
                        len: line.len(),
                        font: gpui::TextStyle::default().font(),
                        color: gpui::rgb(0xffffff).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
            };
            let layout = super::super::LineLayouts {
                count: 3,
                lines: vec![(0, shape("a👩‍💻e\u{301}")), (1, shape(&"w".repeat(230)))],
                rows: super::super::folding::RowMap::new(&[]),
            };
            let bounds = gpui::Bounds::new(
                gpui::point(gpui::px(50.), gpui::px(30.)),
                gpui::size(gpui::px(500.), gpui::px(54.)),
            );
            snapshot.update_geometry(&starts, &layout, bounds, gpui::px(18.), 2.);
            let first = snapshot.node(&snapshot.runs[0]);
            assert_eq!(first.bounds().unwrap().x0, 100.);
            assert_eq!(first.bounds().unwrap().y0, 60.);
            assert_eq!(first.bounds().unwrap().y1, 96.);
            assert_eq!(first.character_positions().unwrap().len(), 4);
            assert_eq!(first.character_widths().unwrap().len(), 4);
            assert_eq!(
                first.character_widths().unwrap()[3],
                0.,
                "hard break has no painted highlight width"
            );
            let chunk = snapshot.node(&snapshot.runs[1]);
            let continuation = snapshot.node(&snapshot.runs[2]);
            assert_eq!(chunk.next_on_line(), Some(snapshot.runs[2].id));
            assert_eq!(continuation.previous_on_line(), Some(snapshot.runs[1].id));
            assert_eq!(
                continuation.next_on_line(),
                None,
                "a hard break ends the line"
            );
            assert_eq!(
                chunk.bounds().unwrap().x1,
                continuation.bounds().unwrap().x0
            );
            assert_eq!(chunk.bounds().unwrap().y0, 96.);
            assert!(snapshot.node(&snapshot.runs[3]).bounds().is_none());
            // Scrolling invalidates previously painted glyph positions instead
            // of leaving stale rectangles on now-offscreen text.
            let empty = super::super::LineLayouts::default();
            snapshot.update_geometry(&starts, &empty, bounds, gpui::px(18.), 2.);
            assert!(
                snapshot
                    .runs
                    .iter()
                    .all(|run| snapshot.node(run).bounds().is_none())
            );
        });
    }

    #[test]
    fn selections_round_trip_unicode_multiline_and_reversed_ranges() {
        let text = "a👩‍💻e\u{301}\n第二行\n";
        let mut snapshot = AccessibleText::new(text.into());
        for (index, run) in snapshot.runs.iter_mut().enumerate() {
            run.id = NodeId(index as u64 + 1);
        }
        for (offset, _) in text.grapheme_indices(true).chain([(text.len(), "")]) {
            let selection = snapshot.selection(offset..offset, false);
            assert_eq!(snapshot.offset(selection.focus), Some(offset));
        }
        let selection = snapshot.selection(1..text.len(), true);
        assert_eq!(snapshot.offset(selection.anchor), Some(text.len()));
        assert_eq!(snapshot.offset(selection.focus), Some(1));
        assert_eq!(snapshot.runs.len(), 3);
        let first = snapshot.node(&snapshot.runs[0]);
        assert_eq!(first.character_lengths(), &[1, 11, 3, 1]);
        assert_eq!(first.value(), Some("a👩‍💻e\u{301}\n"));
        assert!(
            snapshot
                .offset(TextPosition {
                    node: NodeId(99),
                    character_index: 0
                })
                .is_none()
        );
        assert!(
            snapshot
                .offset(TextPosition {
                    node: NodeId(1),
                    character_index: 99
                })
                .is_none()
        );
    }

    #[test]
    fn empty_text_and_long_combining_sequences_have_valid_runs() {
        let empty = AccessibleText::new(String::new());
        assert_eq!(empty.offset(empty.selection(0..0, false).focus), Some(0));
        let long = AccessibleText::new(format!("a{}", "\u{301}".repeat(300)));
        assert_eq!(
            long.runs
                .iter()
                .map(|run| long
                    .node(run)
                    .character_lengths()
                    .iter()
                    .map(|n| *n as usize)
                    .sum::<usize>())
                .sum::<usize>(),
            long.text.len()
        );
    }
}
