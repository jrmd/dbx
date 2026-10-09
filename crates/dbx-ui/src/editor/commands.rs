//! Text-editing commands as pure transforms of text and selection.
//!
//! Each command returns the complete next state, or `None` when it does not
//! apply, so the editor can record one undo step and tests can check the
//! exact text and caret without a window.
use super::*;

/// One indentation level, matching the SQL formatter.
pub(super) const INDENT: &str = "  ";

/// A command's resulting text and selection (UTF-8 byte offsets).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Edit {
    pub(super) text: String,
    pub(super) selection: Range<usize>,
}

impl Edit {
    fn caret(text: String, offset: usize) -> Self {
        Self {
            text,
            selection: offset..offset,
        }
    }
}

/// The byte range of the whole lines touched by `selection`. A selection
/// ending at the start of a line does not include that line.
pub(super) fn selected_lines(text: &str, selection: &Range<usize>) -> Range<usize> {
    let start = line_start(text, selection.start);
    let end_anchor = if selection.end > selection.start && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    start..line_end(text, end_anchor.max(start))
}

fn line_starts_in(text: &str, lines: &Range<usize>) -> Vec<usize> {
    std::iter::once(lines.start)
        .chain(
            text[lines.clone()]
                .match_indices('\n')
                .map(|(index, _)| lines.start + index + 1),
        )
        .collect()
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Apply per-line insertions/removals (offset, removed length, inserted
/// text), keeping each selection end on the same character.
fn edit_lines(text: &str, selection: &Range<usize>, changes: Vec<(usize, usize, &str)>) -> Edit {
    let mut output = String::with_capacity(text.len() + changes.len() * INDENT.len());
    let mut copied = 0;
    // An offset before a change is unchanged; one inside or after it moves
    // with the text that follows the change.
    let shift = |offset: usize| -> usize {
        let mut shifted = offset;
        for (at, removed, inserted) in &changes {
            if offset >= at + removed {
                shifted = shifted + inserted.len() - removed;
            } else if offset >= *at {
                shifted = shifted - (offset - at) + inserted.len();
            }
        }
        shifted
    };
    for (at, removed, inserted) in &changes {
        output.push_str(&text[copied..*at]);
        output.push_str(inserted);
        copied = at + removed;
    }
    output.push_str(&text[copied..]);
    let start = shift(selection.start).min(output.len());
    let end = shift(selection.end).min(output.len());
    Edit {
        text: output,
        selection: start..end.max(start),
    }
}

/// Tab: indent every selected line, or insert spaces to the next indent stop.
pub(super) fn indent(text: &str, selection: Range<usize>) -> Edit {
    let lines = selected_lines(text, &selection);
    if selection.is_empty()
        || !text[selection.clone()].contains('\n') && selection.start != lines.start
    {
        let column = selection.start - line_start(text, selection.start);
        let width = INDENT.len() - column % INDENT.len();
        let inserted = " ".repeat(width);
        let next = format!(
            "{}{inserted}{}",
            &text[..selection.start],
            &text[selection.end..]
        );
        return Edit::caret(next, selection.start + width);
    }
    let changes = line_starts_in(text, &lines)
        .into_iter()
        .filter(|start| line_end(text, *start) > *start)
        .map(|start| (start, 0, INDENT))
        .collect();
    let mut edit = edit_lines(text, &selection, changes);
    // Keep whole lines selected so repeated Tab continues to indent them.
    if selection.start == lines.start {
        edit.selection.start = line_start(&edit.text, edit.selection.start);
    }
    edit
}

/// Shift-Tab: remove up to one indentation level from every selected line.
pub(super) fn outdent(text: &str, selection: Range<usize>) -> Option<Edit> {
    let lines = selected_lines(text, &selection);
    let changes: Vec<_> = line_starts_in(text, &lines)
        .into_iter()
        .filter_map(|start| {
            let line = &text[start..line_end(text, start)];
            let removed = if line.starts_with('\t') {
                1
            } else {
                leading_whitespace(line)
                    .bytes()
                    .take_while(|byte| *byte == b' ')
                    .take(INDENT.len())
                    .count()
            };
            (removed > 0).then_some((start, removed, ""))
        })
        .collect();
    (!changes.is_empty()).then(|| edit_lines(text, &selection, changes))
}

/// Toggle `-- ` line comments on the selected lines, aligned to the least
/// indented line. Blank lines are left alone.
pub(super) fn toggle_line_comment(text: &str, selection: Range<usize>) -> Edit {
    let lines = selected_lines(text, &selection);
    let starts: Vec<_> = line_starts_in(text, &lines)
        .into_iter()
        .filter(|start| !text[*start..line_end(text, *start)].trim().is_empty())
        .collect();
    if starts.is_empty() {
        return Edit {
            text: text.to_owned(),
            selection,
        };
    }
    let commented = starts.iter().all(|start| {
        text[*start..line_end(text, *start)]
            .trim_start()
            .starts_with("--")
    });
    let changes = if commented {
        starts
            .iter()
            .map(|start| {
                let line = &text[*start..line_end(text, *start)];
                let indent = leading_whitespace(line).len();
                let marker = if line[indent..].starts_with("-- ") {
                    3
                } else {
                    2
                };
                (start + indent, marker, "")
            })
            .collect()
    } else {
        let column = starts
            .iter()
            .map(|start| leading_whitespace(&text[*start..line_end(text, *start)]).len())
            .min()
            .unwrap_or(0);
        starts
            .iter()
            .map(|start| (start + column, 0, "-- "))
            .collect()
    };
    edit_lines(text, &selection, changes)
}

/// Enter: keep the current line's indentation, indent after an opening
/// bracket, and put a directly following closing bracket on its own line.
pub(super) fn newline(text: &str, selection: Range<usize>) -> Edit {
    let start = line_start(text, selection.start);
    let indent = leading_whitespace(&text[start..selection.start]).to_owned();
    let before = text[start..selection.start].trim_end();
    let after = &text[selection.end..];
    let opens = before.ends_with(['(', '[', '{']);
    let closes_next = after
        .trim_start_matches([' ', '\t'])
        .starts_with([')', ']', '}']);
    let (inserted, caret) = if opens {
        let inner = format!("\n{indent}{INDENT}");
        let caret = inner.len();
        if closes_next {
            (format!("{inner}\n{indent}"), caret)
        } else {
            (inner, caret)
        }
    } else {
        let inserted = format!("\n{indent}");
        let caret = inserted.len();
        (inserted, caret)
    };
    // Trailing spaces left on the split line are dropped.
    let kept = start
        + text[start..selection.start]
            .trim_end_matches([' ', '\t'])
            .len();
    let kept = if kept < start + indent.len() {
        selection.start
    } else {
        kept
    };
    let rest = after.trim_start_matches([' ', '\t']);
    let next = format!("{}{inserted}{rest}", &text[..kept]);
    Edit::caret(next, kept + caret)
}

const PAIRS: [(char, char); 6] = [
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('\'', '\''),
    ('"', '"'),
    ('`', '`'),
];

/// Bracket and quote pairing for one typed character: wrap a selection,
/// insert a pair, or step over the closing character already present.
pub(super) fn type_character(text: &str, selection: Range<usize>, typed: &str) -> Option<Edit> {
    let mut characters = typed.chars();
    let (Some(typed), None) = (characters.next(), characters.next()) else {
        return None;
    };
    let next = text[selection.end..].chars().next();
    let previous = text[..selection.start].chars().next_back();
    let closing = PAIRS
        .iter()
        .find(|(open, _)| *open == typed)
        .map(|(_, close)| *close);
    let is_closer = PAIRS.iter().any(|(_, close)| *close == typed);
    if selection.is_empty() && is_closer && next == Some(typed) {
        let quote = PAIRS
            .iter()
            .any(|(open, close)| open == close && *open == typed);
        // Step over a closer that pairing inserted; an escaped quote is text.
        if !quote || previous != Some('\\') {
            return Some(Edit::caret(
                text.to_owned(),
                selection.end + typed.len_utf8(),
            ));
        }
    }
    let close = closing?;
    if !selection.is_empty() {
        let inner = &text[selection.clone()];
        let next = format!(
            "{}{typed}{inner}{close}{}",
            &text[..selection.start],
            &text[selection.end..]
        );
        let start = selection.start + typed.len_utf8();
        return Some(Edit {
            text: next,
            selection: start..start + inner.len(),
        });
    }
    let boundary_after =
        next.is_none_or(|next| next.is_whitespace() || matches!(next, ')' | ']' | '}' | ',' | ';'));
    let quote = typed == close;
    // `don't` or `x'` start no string; nor does a quote right after a word.
    let word_before =
        previous.is_some_and(|previous| previous.is_alphanumeric() || previous == '_');
    if !boundary_after || (quote && word_before) {
        return None;
    }
    let next = format!(
        "{}{typed}{close}{}",
        &text[..selection.start],
        &text[selection.end..]
    );
    Some(Edit::caret(next, selection.start + typed.len_utf8()))
}

/// Backspace between an empty pair removes both characters.
pub(super) fn delete_pair(text: &str, selection: &Range<usize>) -> Option<Edit> {
    if !selection.is_empty() {
        return None;
    }
    let cursor = selection.start;
    let previous = text[..cursor].chars().next_back()?;
    let next = text[cursor..].chars().next()?;
    PAIRS.contains(&(previous, next)).then(|| {
        let start = cursor - previous.len_utf8();
        let end = cursor + next.len_utf8();
        Edit::caret(format!("{}{}", &text[..start], &text[end..]), start)
    })
}

/// Duplicate the selected lines below themselves, selecting the copy's
/// equivalent range.
pub(super) fn duplicate_lines(text: &str, selection: Range<usize>) -> Edit {
    let lines = selected_lines(text, &selection);
    let block = &text[lines.clone()];
    let next = format!("{}\n{block}{}", &text[..lines.end], &text[lines.end..]);
    let shift = block.len() + 1;
    Edit {
        text: next,
        selection: selection.start + shift..selection.end + shift,
    }
}

/// Move the selected lines one line up (`-1`) or down (`1`).
pub(super) fn move_lines(text: &str, selection: Range<usize>, direction: isize) -> Option<Edit> {
    let lines = selected_lines(text, &selection);
    let block = &text[lines.clone()];
    if direction < 0 {
        let above_end = lines.start.checked_sub(1)?;
        let above_start = line_start(text, above_end);
        let above = &text[above_start..above_end];
        let next = format!(
            "{}{block}\n{above}{}",
            &text[..above_start],
            &text[lines.end..]
        );
        let shift = above.len() + 1;
        Some(Edit {
            text: next,
            selection: selection.start - shift..selection.end - shift,
        })
    } else {
        if lines.end >= text.len() {
            return None;
        }
        let below_start = lines.end + 1;
        let below_end = line_end(text, below_start);
        let below = &text[below_start..below_end];
        let next = format!(
            "{}{below}\n{block}{}",
            &text[..lines.start],
            &text[below_end..]
        );
        let shift = below.len() + 1;
        Some(Edit {
            text: next,
            selection: selection.start + shift..selection.end + shift,
        })
    }
}

/// Delete the selected lines, leaving the caret where the next line starts.
pub(super) fn delete_lines(text: &str, selection: Range<usize>) -> Edit {
    let lines = selected_lines(text, &selection);
    // Take the line break after the block, or before it on the last line.
    let removed = if lines.end < text.len() {
        lines.start..lines.end + 1
    } else {
        lines.start.saturating_sub(1)..lines.end
    };
    let next = format!("{}{}", &text[..removed.start], &text[removed.end..]);
    let caret = line_start(&next, removed.start);
    Edit::caret(next, caret)
}

/// The word (identifier characters, or a run of other non-space characters)
/// under `offset`, for double-click selection.
pub(super) fn word_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clamp_boundary(text, offset);
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let class = |character: char| {
        if is_word(character) {
            0
        } else if character.is_whitespace() {
            1
        } else {
            2
        }
    };
    let Some(target) = text[offset..]
        .chars()
        .next()
        .filter(|character| *character != '\n')
        .or_else(|| text[..offset].chars().next_back())
    else {
        return offset..offset;
    };
    let wanted = class(target);
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, character)| class(*character) == wanted && *character != '\n')
        .last()
        .map_or(offset, |(index, _)| index);
    let end = text[offset..]
        .char_indices()
        .take_while(|(_, character)| class(*character) == wanted && *character != '\n')
        .last()
        .map_or(offset, |(index, character)| {
            offset + index + character.len_utf8()
        });
    start..end
}

/// The bracket matching the one at or just before `offset`, ignoring
/// brackets inside strings and comments. Returns both positions.
pub(super) fn matching_bracket(
    text: &str,
    offset: usize,
    tokens: &[SqlToken],
) -> Option<(usize, usize)> {
    // Strings, comments and quoted identifiers hide their brackets.
    let opaque = |position: usize| {
        tokens.iter().any(|token| {
            token.range.contains(&position)
                && match token.kind {
                    SqlTokenKind::String | SqlTokenKind::Comment => true,
                    SqlTokenKind::Identifier => text[token.range.clone()].starts_with(['"', '`']),
                    _ => false,
                }
        })
    };
    let bracket_at = |position: usize| -> Option<char> {
        let character = text[position..].chars().next()?;
        (matches!(character, '(' | ')' | '[' | ']' | '{' | '}') && !opaque(position))
            .then_some(character)
    };
    let position = [Some(offset), offset.checked_sub(1)]
        .into_iter()
        .flatten()
        .filter(|position| text.is_char_boundary(*position))
        .find(|position| bracket_at(*position).is_some())?;
    let bracket = bracket_at(position)?;
    let (open, close, forward) = match bracket {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        _ => ('{', '}', false),
    };
    let mut depth = 0usize;
    let candidates: Box<dyn Iterator<Item = (usize, char)>> = if forward {
        Box::new(
            text[position..]
                .char_indices()
                .map(|(index, c)| (position + index, c)),
        )
    } else {
        Box::new(text[..=position].char_indices().rev())
    };
    for (index, character) in candidates {
        if (character != open && character != close) || opaque(index) {
            continue;
        }
        if character == bracket {
            depth += 1;
        } else {
            depth -= 1;
            if depth == 0 {
                return Some((position, index));
            }
        }
    }
    None
}

/// The result of editing several selections at once: the next text and
/// every selection, in document order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MultiEdit {
    pub(super) text: String,
    pub(super) selections: Vec<Range<usize>>,
}

/// Replace every selection with `inserted`, leaving a caret after each.
/// `selections` must be sorted and must not overlap.
pub(super) fn insert_at_all(text: &str, selections: &[Range<usize>], inserted: &str) -> MultiEdit {
    replace_all(
        text,
        selections.iter().map(|range| (range.clone(), inserted)),
    )
}

/// Delete every selection, or for an empty one the grapheme before it
/// (`backward`) or after it.
pub(super) fn delete_at_all(text: &str, selections: &[Range<usize>], backward: bool) -> MultiEdit {
    let mut ranges = selections
        .iter()
        .map(|range| {
            if !range.is_empty() {
                range.clone()
            } else if backward {
                previous_boundary(text, range.start)..range.start
            } else {
                range.start..next_boundary(text, range.start)
            }
        })
        .collect::<Vec<_>>();
    // Deleting at neighbouring carets can reach the same character twice.
    for index in 1..ranges.len() {
        ranges[index].start = ranges[index].start.max(ranges[index - 1].end);
        ranges[index].end = ranges[index].end.max(ranges[index].start);
    }
    replace_all(text, ranges.into_iter().map(|range| (range, "")))
}

fn replace_all<'a>(
    text: &str,
    replacements: impl Iterator<Item = (Range<usize>, &'a str)>,
) -> MultiEdit {
    let mut next = String::with_capacity(text.len());
    let mut selections = Vec::new();
    let mut copied = 0;
    for (range, inserted) in replacements {
        next.push_str(&text[copied..range.start]);
        next.push_str(inserted);
        selections.push(next.len()..next.len());
        copied = range.end;
    }
    next.push_str(&text[copied..]);
    MultiEdit {
        text: next,
        selections,
    }
}

/// The next match of `needle` after `after`, wrapping around, that is not
/// already one of `taken`.
pub(super) fn next_occurrence(
    text: &str,
    needle: &str,
    after: usize,
    taken: &[Range<usize>],
) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let free = |start: usize| {
        let range = start..start + needle.len();
        (!taken.contains(&range)).then_some(range)
    };
    text[after..]
        .match_indices(needle)
        .find_map(|(index, _)| free(after + index))
        .or_else(|| {
            text[..after.min(text.len())]
                .match_indices(needle)
                .find_map(|(index, _)| free(index))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render `text` with `|` at the caret, or `[`/`]` around a selection.
    fn show(edit: &Edit) -> String {
        let Edit { text, selection } = edit;
        if selection.is_empty() {
            format!("{}|{}", &text[..selection.start], &text[selection.start..])
        } else {
            format!(
                "{}[{}]{}",
                &text[..selection.start],
                &text[selection.clone()],
                &text[selection.end..]
            )
        }
    }

    /// Parse the `show` notation back into text and selection.
    fn parse(marked: &str) -> (String, Range<usize>) {
        if let Some(caret) = marked.find('|') {
            return (marked.replacen('|', "", 1), caret..caret);
        }
        let start = marked.find('[').unwrap();
        let end = marked.find(']').unwrap() - 1;
        (marked.replacen('[', "", 1).replacen(']', "", 1), start..end)
    }

    fn run(marked: &str, command: impl Fn(&str, Range<usize>) -> Edit) -> String {
        let (text, selection) = parse(marked);
        show(&command(&text, selection))
    }

    #[test]
    fn tab_indents_to_the_next_stop_or_every_selected_line() {
        assert_eq!(run("SELECT|", indent), "SELECT  |");
        assert_eq!(run("SELECT |", indent), "SELECT  |");
        assert_eq!(run("[a\nb]\n\nc", indent), "[  a\n  b]\n\nc");
        assert_eq!(run("a\n[b\n]c", indent), "a\n[  b\n]c");
        assert_eq!(
            run("x [ab]", indent),
            "x   |",
            "a selection within one line is replaced like typing"
        );
    }

    #[test]
    fn shift_tab_removes_one_level() {
        let outdent = |text: &str, selection| outdent(text, selection).unwrap();
        assert_eq!(run("    a|", outdent), "  a|");
        assert_eq!(run("[  a\n b\n\tc]", outdent), "[a\nb\nc]");
        assert!(outdent_none("a|"));
    }

    fn outdent_none(marked: &str) -> bool {
        let (text, selection) = parse(marked);
        outdent(&text, selection).is_none()
    }

    #[test]
    fn line_comments_toggle_at_the_shared_indent() {
        assert_eq!(
            run("[  SELECT 1\n\n    FROM t]", toggle_line_comment),
            "[  -- SELECT 1\n\n  --   FROM t]"
        );
        assert_eq!(
            run("  -- SELECT 1|\n", toggle_line_comment),
            "  SELECT 1|\n"
        );
        assert_eq!(run("--x|", toggle_line_comment), "x|");
    }

    #[test]
    fn enter_keeps_indentation_and_opens_bracket_blocks() {
        assert_eq!(run("  SELECT a,|", newline), "  SELECT a,\n  |");
        assert_eq!(run("WHERE id IN (|)", newline), "WHERE id IN (\n  |\n)");
        assert_eq!(run("  f(|x", newline), "  f(\n    |x");
        assert_eq!(run("a   |b", newline), "a\n|b");
    }

    #[test]
    fn typing_pairs_wraps_and_steps_over() {
        let typed = |marked: &str, character: &str| {
            let (text, selection) = parse(marked);
            type_character(&text, selection, character).map(|edit| show(&edit))
        };
        assert_eq!(typed("count|", "(").as_deref(), Some("count(|)"));
        assert_eq!(typed("count(|)", ")").as_deref(), Some("count()|"));
        assert_eq!(typed("= |", "'").as_deref(), Some("= '|'"));
        assert_eq!(typed("'abc|'", "'").as_deref(), Some("'abc'|"));
        assert_eq!(typed("[name]", "\"").as_deref(), Some("\"[name]\""));
        assert_eq!(typed("don|", "'"), None);
        assert_eq!(typed("|abc", "("), None);
        let (text, selection) = parse("(|)");
        assert_eq!(show(&delete_pair(&text, &selection).unwrap()), "|");
    }

    #[test]
    fn line_operations_duplicate_move_and_delete() {
        assert_eq!(run("a\nb|\nc", duplicate_lines), "a\nb\nb|\nc");
        let up = |text: &str, selection| move_lines(text, selection, -1).unwrap();
        let down = |text: &str, selection| move_lines(text, selection, 1).unwrap();
        assert_eq!(run("a\nb|\nc", up), "b|\na\nc");
        assert_eq!(run("a\nb|\nc", down), "a\nc\nb|");
        assert_eq!(run("a\nb|\nc", delete_lines), "a\n|c");
        assert_eq!(run("a\nb|", delete_lines), "|a");
        assert_eq!(run("only|", delete_lines), "|");
    }

    #[test]
    fn double_click_selects_words_and_symbol_runs() {
        let text = "SELECT user_id, 'é' >= 3";
        assert_eq!(&text[word_at(text, 9)], "user_id");
        assert_eq!(&text[word_at(text, text.find(">=").unwrap())], ">=");
        assert_eq!(&text[word_at(text, text.len())], "3");
    }

    #[test]
    fn brackets_match_outside_strings_and_comments() {
        let text = "SELECT f(a, ')', (b)) -- (";
        let tokens = lex_sql(text);
        let open = text.find('(').unwrap();
        let close = text.rfind(')').unwrap();
        assert_eq!(matching_bracket(text, open, &tokens), Some((open, close)));
        assert_eq!(
            matching_bracket(text, close + 1, &tokens),
            Some((close, open))
        );
        assert_eq!(matching_bracket(text, text.len(), &tokens), None);
    }

    #[test]
    fn multiple_selections_insert_and_delete_together() {
        let text = "a b a";
        let edit = insert_at_all(text, &[0..1, 4..5], "xy");
        assert_eq!(edit.text, "xy b xy");
        assert_eq!(edit.selections, [2..2, 7..7]);
        let edit = delete_at_all(&edit.text, &edit.selections, true);
        assert_eq!(edit.text, "x b x");
        assert_eq!(edit.selections, [1..1, 5..5]);
        let edit = delete_at_all("ab🦀cd", &[0..0, 2..2], false);
        assert_eq!(edit.text, "bcd");
        assert_eq!(edit.selections, [0..0, 1..1]);
        // Adjacent carets never delete a character twice.
        let edit = delete_at_all("abc", &[1..1, 2..2], true);
        assert_eq!(edit.text, "c");
    }

    #[test]
    fn next_occurrence_wraps_and_skips_selected_matches() {
        let text = "id, name, id, id";
        assert_eq!(next_occurrence(text, "id", 2, &[0..2]), Some(10..12));
        assert_eq!(
            next_occurrence(text, "id", 16, &[10..12, 14..16]),
            Some(0..2)
        );
        assert_eq!(
            next_occurrence(text, "id", 0, &[0..2, 10..12, 14..16]),
            None
        );
        assert_eq!(next_occurrence(text, "", 0, &[]), None);
    }
}
