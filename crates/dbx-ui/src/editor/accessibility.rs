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
