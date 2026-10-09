//! Code folding: which blocks can fold, and how hidden lines map between
//! document lines and the rows the editor paints.
use super::*;

/// A block that can fold: lines after `start_line` up to, not including,
/// `end_line` hide, so the opening and closing lines stay visible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FoldRegion {
    /// Byte offset of the opening bracket or comment, which identifies the
    /// fold across edits.
    pub(super) open: usize,
    pub(super) start_line: usize,
    pub(super) end_line: usize,
}

impl FoldRegion {
    pub(super) fn hidden(&self) -> Range<usize> {
        self.start_line + 1..self.end_line
    }
}

fn line_of(line_starts: &[usize], offset: usize) -> usize {
    line_starts.partition_point(|start| *start <= offset) - 1
}

/// Parenthesized blocks and block comments that hide at least one line,
/// ordered by their opening offset.
pub(super) fn fold_regions(
    text: &str,
    tokens: &[SqlToken],
    line_starts: &[usize],
) -> Vec<FoldRegion> {
    let mut regions = Vec::new();
    let mut push = |open: usize, close: usize| {
        let start_line = line_of(line_starts, open);
        let end_line = line_of(line_starts, close);
        if end_line >= start_line + 2 {
            regions.push(FoldRegion {
                open,
                start_line,
                end_line,
            });
        }
    };
    let mut open = Vec::new();
    let mut position = 0;
    let scan =
        |from: usize, to: usize, open: &mut Vec<usize>, push: &mut dyn FnMut(usize, usize)| {
            for (index, byte) in text.as_bytes()[from..to].iter().enumerate() {
                match byte {
                    b'(' => open.push(from + index),
                    b')' => {
                        if let Some(start) = open.pop() {
                            push(start, from + index);
                        }
                    }
                    _ => {}
                }
            }
        };
    // Brackets count only between tokens; strings, comments and quoted
    // identifiers hide theirs.
    for token in tokens {
        scan(
            position,
            token.range.start.max(position),
            &mut open,
            &mut push,
        );
        if token.kind == SqlTokenKind::Comment && text[token.range.clone()].starts_with("/*") {
            let end = token.range.end.saturating_sub(1).max(token.range.start);
            push(token.range.start, end);
        }
        position = position.max(token.range.end);
    }
    scan(position, text.len(), &mut open, &mut push);
    regions.sort_by_key(|region| region.open);
    regions
}

/// Maps document lines to painted rows around hidden line ranges.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct RowMap {
    /// Hidden line ranges, sorted and disjoint.
    hidden: Vec<Range<usize>>,
}

impl RowMap {
    /// The rows left after folding `folded` regions; nested and overlapping
    /// folds merge.
    pub(super) fn new<'a>(folded: impl IntoIterator<Item = &'a FoldRegion>) -> Self {
        let mut ranges = folded
            .into_iter()
            .map(FoldRegion::hidden)
            .filter(|range| !range.is_empty())
            .collect::<Vec<_>>();
        ranges.sort_by_key(|range| range.start);
        let mut hidden: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            match hidden.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => hidden.push(range),
            }
        }
        Self { hidden }
    }

    /// The painted row of `line`; a hidden line maps to the row it folds into.
    pub(super) fn row(&self, line: usize) -> usize {
        let mut removed = 0;
        for range in &self.hidden {
            if line < range.start {
                break;
            }
            if range.contains(&line) {
                return range.start - 1 - removed;
            }
            removed += range.len();
        }
        line - removed
    }

    /// The document line painted at `row`.
    pub(super) fn line(&self, row: usize) -> usize {
        let mut line = row;
        for range in &self.hidden {
            if line < range.start {
                break;
            }
            line += range.len();
        }
        line
    }

    /// Painted rows for a document of `lines` lines.
    pub(super) fn rows(&self, lines: usize) -> usize {
        lines - self.hidden.iter().map(Range::len).sum::<usize>()
    }
}

/// Move byte offsets recorded in `old` to the same text in `new`, assuming
/// one contiguous change. Offsets inside the changed span are dropped.
pub(super) fn shift_offsets(old: &str, new: &str, offsets: &mut Vec<usize>) {
    let prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old[prefix..]
        .bytes()
        .rev()
        .zip(new[prefix..].bytes().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let changed_end = old.len() - suffix;
    offsets.retain_mut(|offset| {
        if *offset < prefix {
            true
        } else if *offset >= changed_end {
            *offset = *offset - changed_end + (new.len() - suffix);
            true
        } else {
            false
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(text: &str) -> Vec<(usize, usize)> {
        let tokens = lex_sql_for(text, None);
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect::<Vec<_>>();
        fold_regions(text, &tokens, &starts)
            .into_iter()
            .map(|region| (region.start_line, region.end_line))
            .collect()
    }

    #[test]
    fn blocks_that_hide_lines_fold_and_others_do_not() {
        let text = "WITH a AS (\n  SELECT 1\n), b AS (SELECT 2)\nSELECT f(\n  x,\n  '(',\n  y\n)";
        assert_eq!(regions(text), [(0, 2), (3, 7)]);
        // A two-line block hides nothing.
        assert!(regions("SELECT f(\n)").is_empty());
        assert_eq!(regions("/* one\ntwo\nthree */ SELECT 1"), [(0, 2)]);
        assert!(regions("-- (\n\n\nSELECT 1").is_empty());
    }

    #[test]
    fn rows_skip_hidden_lines() {
        let folds = [
            FoldRegion {
                open: 0,
                start_line: 1,
                end_line: 4,
            },
            FoldRegion {
                open: 1,
                start_line: 2,
                end_line: 3,
            },
            FoldRegion {
                open: 2,
                start_line: 6,
                end_line: 8,
            },
        ];
        let map = RowMap::new(&folds);
        // Lines 2..4 and 7 are hidden.
        assert_eq!(
            (0..10).map(|line| map.row(line)).collect::<Vec<_>>(),
            [0, 1, 1, 1, 2, 3, 4, 4, 5, 6]
        );
        assert_eq!(
            (0..7).map(|row| map.line(row)).collect::<Vec<_>>(),
            [0, 1, 4, 5, 6, 8, 9]
        );
        assert_eq!(map.rows(10), 7);
        assert_eq!(RowMap::default().line(3), 3);
    }

    #[test]
    fn offsets_follow_edits_before_them_and_drop_inside() {
        let mut offsets = vec![1, 6, 12];
        shift_offsets("ab (cd) (ef)", "abXX (cd) (ef)", &mut offsets);
        assert_eq!(offsets, [1, 8, 14]);
        let mut offsets = vec![3, 8];
        shift_offsets("ab (cd) (ef)", "ab (c) (ef)", &mut offsets);
        assert_eq!(offsets, [3, 7]);
        let mut offsets = vec![3];
        shift_offsets("ab (cd)", "ab cd)", &mut offsets);
        assert!(offsets.is_empty());
    }
}
