//! Built-in function signatures, the call around the caret, and the uses of
//! the identifier under it.
//!
//! These are lexical: they read the editor's SQL tokens and never need a
//! parse that succeeds, so they keep working while a statement is half typed.
use super::*;

/// How a built-in function is called, for signature help, hover cards and
/// completion details.
#[derive(Debug)]
pub struct FunctionSignature {
    pub name: &'static str,
    pub params: &'static [&'static str],
    /// The last parameter may repeat.
    pub variadic: bool,
    pub returns: &'static str,
    pub summary: &'static str,
}

impl FunctionSignature {
    /// The parameter list as written, e.g. `value, …`.
    pub fn parameters(&self) -> String {
        let mut list = self.params.join(", ");
        if self.variadic {
            list.push_str(", …");
        }
        list
    }

    /// `NAME(params) → returns`.
    pub fn label(&self) -> String {
        format!("{}({}) → {}", self.name, self.parameters(), self.returns)
    }

    /// The parameter the `argument`th (zero-based) argument fills.
    pub fn active_param(&self, argument: usize) -> Option<usize> {
        if self.variadic {
            Some(argument.min(self.params.len().checked_sub(1)?))
        } else {
            (argument < self.params.len()).then_some(argument)
        }
    }
}

const fn signature(
    name: &'static str,
    params: &'static [&'static str],
    variadic: bool,
    returns: &'static str,
    summary: &'static str,
) -> FunctionSignature {
    FunctionSignature {
        name,
        params,
        variadic,
        returns,
        summary,
    }
}

/// Sorted by name for binary search.
const SIGNATURES: &[FunctionSignature] = &[
    signature("ABS", &["number"], false, "number", "Absolute value."),
    signature(
        "AVG",
        &["expression"],
        false,
        "numeric",
        "Average of the non-null values in the group.",
    ),
    signature(
        "CAST",
        &["expression AS type"],
        false,
        "type",
        "Convert a value to another type.",
    ),
    signature(
        "CEIL",
        &["number"],
        false,
        "number",
        "Smallest integer not less than the argument.",
    ),
    signature(
        "CHAR_LENGTH",
        &["string"],
        false,
        "integer",
        "Number of characters in a string.",
    ),
    signature(
        "COALESCE",
        &["value"],
        true,
        "any",
        "The first argument that is not null.",
    ),
    signature(
        "CONCAT",
        &["value"],
        true,
        "text",
        "Join values as text; null arguments are ignored on most engines.",
    ),
    signature(
        "COUNT",
        &["expression"],
        false,
        "integer",
        "Rows in the group; COUNT(*) counts all, COUNT(x) counts non-null x.",
    ),
    signature(
        "CUME_DIST",
        &[],
        false,
        "double",
        "Fraction of partition rows ordered at or before the current row.",
    ),
    signature("CURRENT_DATE", &[], false, "date", "The current date."),
    signature(
        "CURRENT_TIMESTAMP",
        &[],
        false,
        "timestamp",
        "The start time of the current transaction or statement.",
    ),
    signature(
        "DATE_PART",
        &["field", "source"],
        false,
        "double",
        "A field such as 'year' or 'dow' from a date, time or interval.",
    ),
    signature(
        "DATE_TRUNC",
        &["field", "source"],
        false,
        "timestamp",
        "Truncate a timestamp to the given precision, e.g. 'month'.",
    ),
    signature(
        "DENSE_RANK",
        &[],
        false,
        "integer",
        "Rank of the current row without gaps.",
    ),
    signature(
        "EXTRACT",
        &["field FROM source"],
        false,
        "number",
        "A field such as YEAR or EPOCH from a date, time or interval.",
    ),
    signature(
        "FIRST_VALUE",
        &["value"],
        false,
        "any",
        "Value from the first row of the window frame.",
    ),
    signature(
        "FLOOR",
        &["number"],
        false,
        "number",
        "Largest integer not greater than the argument.",
    ),
    signature("GREATEST", &["value"], true, "any", "The largest argument."),
    signature(
        "GROUP_CONCAT",
        &["expression"],
        false,
        "text",
        "Join the group's values into one string (MySQL, SQLite).",
    ),
    signature(
        "IFNULL",
        &["value", "fallback"],
        false,
        "any",
        "`value` unless it is null, then `fallback`.",
    ),
    signature(
        "JSON_EXTRACT",
        &["json", "path"],
        true,
        "json",
        "The value at a JSON path (MySQL, SQLite).",
    ),
    signature(
        "LAG",
        &["value", "offset", "default"],
        false,
        "any",
        "Value from a row `offset` rows before the current row.",
    ),
    signature(
        "LAST_VALUE",
        &["value"],
        false,
        "any",
        "Value from the last row of the window frame.",
    ),
    signature(
        "LEAD",
        &["value", "offset", "default"],
        false,
        "any",
        "Value from a row `offset` rows after the current row.",
    ),
    signature("LEAST", &["value"], true, "any", "The smallest argument."),
    signature(
        "LEFT",
        &["string", "count"],
        false,
        "text",
        "The first `count` characters.",
    ),
    signature(
        "LENGTH",
        &["string"],
        false,
        "integer",
        "Length of a string (bytes on MySQL, characters elsewhere).",
    ),
    signature(
        "LOWER",
        &["string"],
        false,
        "text",
        "Convert to lower case.",
    ),
    signature(
        "LPAD",
        &["string", "length", "fill"],
        false,
        "text",
        "Pad on the left to `length` characters.",
    ),
    signature(
        "LTRIM",
        &["string", "characters"],
        false,
        "text",
        "Remove leading characters (spaces by default).",
    ),
    signature(
        "MAX",
        &["expression"],
        false,
        "any",
        "Largest non-null value in the group.",
    ),
    signature(
        "MIN",
        &["expression"],
        false,
        "any",
        "Smallest non-null value in the group.",
    ),
    signature(
        "MOD",
        &["dividend", "divisor"],
        false,
        "number",
        "Remainder of a division.",
    ),
    signature("NOW", &[], false, "timestamp", "The current date and time."),
    signature(
        "NTILE",
        &["buckets"],
        false,
        "integer",
        "Bucket number from 1 to `buckets`, dividing the partition evenly.",
    ),
    signature(
        "NULLIF",
        &["value", "compare"],
        false,
        "any",
        "Null when `value` equals `compare`, otherwise `value`.",
    ),
    signature(
        "PERCENT_RANK",
        &[],
        false,
        "double",
        "Relative rank of the current row, from 0 to 1.",
    ),
    signature(
        "POSITION",
        &["substring IN string"],
        false,
        "integer",
        "1-based position of a substring, or 0.",
    ),
    signature(
        "POWER",
        &["base", "exponent"],
        false,
        "number",
        "`base` raised to `exponent`.",
    ),
    signature("RANDOM", &[], false, "number", "A random value."),
    signature(
        "RANK",
        &[],
        false,
        "integer",
        "Rank of the current row with gaps.",
    ),
    signature(
        "REPLACE",
        &["string", "from", "to"],
        false,
        "text",
        "Replace every occurrence of `from` with `to`.",
    ),
    signature(
        "RIGHT",
        &["string", "count"],
        false,
        "text",
        "The last `count` characters.",
    ),
    signature(
        "ROUND",
        &["number", "digits"],
        false,
        "number",
        "Round to `digits` decimal places (0 by default).",
    ),
    signature(
        "ROW_NUMBER",
        &[],
        false,
        "integer",
        "Sequential number of the row within its partition.",
    ),
    signature(
        "RPAD",
        &["string", "length", "fill"],
        false,
        "text",
        "Pad on the right to `length` characters.",
    ),
    signature(
        "RTRIM",
        &["string", "characters"],
        false,
        "text",
        "Remove trailing characters (spaces by default).",
    ),
    signature(
        "SPLIT_PART",
        &["string", "delimiter", "field"],
        false,
        "text",
        "The `field`th (1-based) piece of a split string (PostgreSQL).",
    ),
    signature(
        "STRING_AGG",
        &["expression", "delimiter"],
        false,
        "text",
        "Join the group's values with a delimiter.",
    ),
    signature(
        "SUBSTR",
        &["string", "start", "length"],
        false,
        "text",
        "Part of a string from a 1-based `start`.",
    ),
    signature(
        "SUBSTRING",
        &["string", "start", "length"],
        false,
        "text",
        "Part of a string from a 1-based `start`; also `string FROM start FOR length`.",
    ),
    signature(
        "SUM",
        &["expression"],
        false,
        "number",
        "Sum of the non-null values in the group.",
    ),
    signature(
        "TO_CHAR",
        &["value", "format"],
        false,
        "text",
        "Format a date or number as text (PostgreSQL, Oracle).",
    ),
    signature(
        "TO_TIMESTAMP",
        &["text", "format"],
        false,
        "timestamp",
        "Parse text, or Unix seconds when given one number.",
    ),
    signature(
        "TRIM",
        &["[LEADING | TRAILING | BOTH] [characters FROM] string"],
        false,
        "text",
        "Remove characters (spaces by default) from the ends.",
    ),
    signature(
        "UPPER",
        &["string"],
        false,
        "text",
        "Convert to upper case.",
    ),
];

/// The built-in function called `name`, case-insensitively.
pub fn function_signature(name: &str) -> Option<&'static FunctionSignature> {
    let name = name.to_ascii_uppercase();
    SIGNATURES
        .binary_search_by(|signature| signature.name.cmp(name.as_str()))
        .ok()
        .map(|index| &SIGNATURES[index])
}

/// Functions written without parentheses.
const BARE_FUNCTIONS: &[&str] = &["CURRENT_DATE", "CURRENT_TIMESTAMP"];

/// A call to a built-in function around the caret.
#[derive(Debug)]
pub(super) struct CallSite {
    /// Where the function's name is.
    pub(super) name: Range<usize>,
    pub(super) signature: &'static FunctionSignature,
    /// The zero-based argument the caret is in.
    pub(super) argument: usize,
}

/// How far back the caret's enclosing call is searched for.
pub(super) const CALL_SCAN_LIMIT: usize = 4096;

fn is_name(text: &str, token: &SqlToken) -> bool {
    matches!(
        token.kind,
        SqlTokenKind::Keyword | SqlTokenKind::Identifier | SqlTokenKind::Type
    ) && !text[token.range.clone()].starts_with(['"', '`', '['])
}

/// The innermost built-in function call whose argument list holds `cursor`.
///
/// Parentheses and commas are read from the text between tokens, so strings,
/// comments and quoted identifiers never count. Parentheses that are not a
/// known call (`IN (…)`, grouping) are stepped out of; a subquery ends the
/// search, since its columns are not the outer call's arguments.
pub(super) fn call_at(text: &str, cursor: usize, tokens: &[SqlToken]) -> Option<CallSite> {
    let cursor = clamp_boundary(text, cursor);
    // Invariant: `tokens[..next]` are exactly the tokens starting before
    // `position`, so `tokens[next - 1]` is the only one that can cover the
    // byte just before it.
    let mut next = tokens.partition_point(|token| token.range.start < cursor);
    // A line comment also holds the caret at its end.
    if let Some(comment) = next.checked_sub(1).map(|index| &tokens[index])
        && comment.kind == SqlTokenKind::Comment
        && (comment.range.end > cursor
            || comment.range.end == cursor && !text[comment.range.clone()].ends_with("*/"))
    {
        return None;
    }
    let floor = cursor.saturating_sub(CALL_SCAN_LIMIT);
    let bytes = text.as_bytes();
    let mut position = cursor;
    let mut depth = 0usize;
    let mut argument = 0usize;
    while position > floor {
        if next > 0 && tokens[next - 1].range.end >= position {
            next -= 1;
            position = tokens[next].range.start;
            continue;
        }
        position -= 1;
        match bytes[position] {
            b';' => return None,
            b')' => depth += 1,
            b',' if depth == 0 => argument += 1,
            b'(' if depth > 0 => depth -= 1,
            b'(' => {
                if let Some((name, signature)) = called_name(text, &tokens[..next], position) {
                    return Some(CallSite {
                        name,
                        signature,
                        argument,
                    });
                }
                let opens_query = tokens.get(next).is_some_and(|token| {
                    token.kind == SqlTokenKind::Keyword
                        && text[position + 1..token.range.start].trim().is_empty()
                        && ["select", "with", "values"]
                            .iter()
                            .any(|word| text[token.range.clone()].eq_ignore_ascii_case(word))
                });
                if opens_query {
                    return None;
                }
                argument = 0;
            }
            _ => {}
        }
    }
    None
}

/// The built-in function named just before the `(` at `paren`, looking past
/// whitespace and comments. `before` holds the tokens preceding it.
fn called_name(
    text: &str,
    before: &[SqlToken],
    paren: usize,
) -> Option<(Range<usize>, &'static FunctionSignature)> {
    let mut end = paren;
    for token in before.iter().rev() {
        if !text[token.range.end..end].trim().is_empty() {
            return None;
        }
        if token.kind == SqlTokenKind::Comment {
            end = token.range.start;
            continue;
        }
        if !is_name(text, token) {
            return None;
        }
        let signature = function_signature(&text[token.range.clone()])?;
        return Some((token.range.clone(), signature));
    }
    None
}

/// The built-in function whose name is at `offset`.
pub(super) fn function_at(
    text: &str,
    offset: usize,
    tokens: &[SqlToken],
) -> Option<(Range<usize>, &'static FunctionSignature)> {
    let token = tokens
        .iter()
        .find(|token| token.range.contains(&offset) && is_name(text, token))?;
    let signature = function_signature(&text[token.range.clone()])?;
    let called = text[token.range.end..].trim_start().starts_with('(');
    (called || BARE_FUNCTIONS.contains(&signature.name)).then(|| (token.range.clone(), signature))
}

/// Compare identifiers the way most engines resolve unquoted names.
fn identifier_key(raw: &str) -> String {
    raw.trim_matches(['"', '`', '[', ']']).to_lowercase()
}

/// Every use of the identifier touching `cursor` within `scope`, or nothing
/// when it is used only once.
pub(super) fn occurrences(
    text: &str,
    cursor: usize,
    tokens: &[SqlToken],
    scope: Range<usize>,
) -> Vec<Range<usize>> {
    let identifiers = || {
        tokens.iter().filter(|token| {
            token.kind == SqlTokenKind::Identifier
                && token.range.start >= scope.start
                && token.range.end <= scope.end
        })
    };
    let Some(current) =
        identifiers().find(|token| token.range.start <= cursor && cursor <= token.range.end)
    else {
        return Vec::new();
    };
    let key = identifier_key(&text[current.range.clone()]);
    let found = identifiers()
        .filter(|token| identifier_key(&text[token.range.clone()]) == key)
        .map(|token| token.range.clone())
        .collect::<Vec<_>>();
    if found.len() < 2 { Vec::new() } else { found }
}

/// Where the alias or CTE named at `offset` is defined in its statement,
/// when `offset` is a use of it rather than the definition itself.
///
/// A definition is a name followed by `AS (` (a CTE, optionally with a
/// column list), or a name directly after a relation or `AS` (an alias).
pub(super) fn local_definition(
    text: &str,
    offset: usize,
    tokens: &[SqlToken],
) -> Option<Range<usize>> {
    let words = tokens
        .iter()
        .filter(|token| token.kind != SqlTokenKind::Comment)
        .collect::<Vec<_>>();
    let used = words.iter().position(|token| {
        token.kind == SqlTokenKind::Identifier
            && token.range.start <= offset
            && offset < token.range.end
    })?;
    let key = identifier_key(&text[words[used].range.clone()]);
    let keyword = |index: usize, word: &str| {
        words.get(index).is_some_and(|token| {
            token.kind == SqlTokenKind::Keyword
                && text[token.range.clone()].eq_ignore_ascii_case(word)
        })
    };
    let defines = |index: usize| {
        let token = words[index];
        let after = text[token.range.end..].trim_start();
        let as_paren = |next: usize| {
            keyword(next, "as") && text[words[next].range.end..].trim_start().starts_with('(')
        };
        // `name AS (` or `name (columns) AS (`.
        let cte = as_paren(index + 1)
            || after.starts_with('(')
                && words[index + 1..]
                    .iter()
                    .position(|next| {
                        next.kind == SqlTokenKind::Keyword
                            && text[next.range.clone()].eq_ignore_ascii_case("as")
                    })
                    .is_some_and(|offset| {
                        let next = index + 1 + offset;
                        text[token.range.end..words[next].range.start]
                            .trim_end()
                            .ends_with(')')
                            && as_paren(next)
                    });
        // `relation alias`, `… AS alias` or `(subquery) alias`.
        let alias = index > 0 && !after.starts_with('.') && {
            let previous = words[index - 1];
            let gap = text[previous.range.end..token.range.start].trim();
            gap.is_empty()
                && (keyword(index - 1, "as") || previous.kind == SqlTokenKind::Identifier)
                || gap.ends_with(')') && gap.trim_end_matches(')').trim().is_empty()
        };
        cte || alias
    };
    let definition = (0..words.len()).find(|&index| {
        words[index].kind == SqlTokenKind::Identifier
            && identifier_key(&text[words[index].range.clone()]) == key
            && defines(index)
    })?;
    (definition != used).then(|| words[definition].range.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(marked: &str) -> Option<(String, usize)> {
        let cursor = marked.find('|').unwrap();
        let text = marked.replace('|', "");
        let tokens = lex_sql_for(&text, None);
        call_at(&text, cursor, &tokens).map(|call| {
            assert_eq!(
                text[call.name.clone()].to_ascii_uppercase(),
                call.signature.name
            );
            (call.signature.name.to_owned(), call.argument)
        })
    }

    fn named(name: &str, argument: usize) -> Option<(String, usize)> {
        Some((name.to_owned(), argument))
    }

    #[test]
    fn signatures_are_sorted_and_cover_the_completion_vocabulary() {
        assert!(
            SIGNATURES
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
        );
        for function in sql_completion_functions() {
            assert!(function_signature(function).is_some(), "{function}");
        }
        let coalesce = function_signature("coalesce").unwrap();
        assert_eq!(coalesce.label(), "COALESCE(value, …) → any");
        assert_eq!(coalesce.active_param(3), Some(0));
        let replace = function_signature("REPLACE").unwrap();
        assert_eq!(replace.active_param(2), Some(2));
        assert_eq!(replace.active_param(3), None);
        assert_eq!(function_signature("NOW").unwrap().active_param(0), None);
    }

    #[test]
    fn call_at_counts_top_level_arguments() {
        assert_eq!(call("SELECT coalesce(|"), named("COALESCE", 0));
        assert_eq!(call("SELECT coalesce(a, b|"), named("COALESCE", 1));
        assert_eq!(call("SELECT replace(name, ',', |'x')"), named("REPLACE", 2));
        assert_eq!(call("SELECT round(sum(a, b), |"), named("ROUND", 1));
        assert_eq!(call("SELECT round(lower(x|), 2)"), named("LOWER", 0));
        assert_eq!(call("SELECT round /* ( */ (a, |"), named("ROUND", 1));
        assert_eq!(call("SELECT concat('a(', 'b,', |"), named("CONCAT", 2));
        assert_eq!(call("SELECT concat('a, b|')"), named("CONCAT", 0));
    }

    #[test]
    fn call_at_steps_out_of_groups_but_not_queries_or_statements() {
        assert_eq!(call("SELECT coalesce(a IN (1, 2|"), named("COALESCE", 0));
        assert_eq!(call("SELECT coalesce((a + b) * 2, |"), named("COALESCE", 1));
        assert_eq!(call("SELECT coalesce((SELECT max(id), |"), None);
        assert_eq!(call("SELECT lower(a); SELECT |"), None);
        assert_eq!(call("SELECT lower(a)|"), None);
        assert_eq!(call("SELECT my_function(a, |"), None);
        assert_eq!(call("SELECT \"lower\"(a|"), None);
        assert_eq!(call("SELECT lower(a -- note |"), None);
    }

    #[test]
    fn function_at_requires_a_call_except_for_bare_functions() {
        let text = "SELECT lower (name), lower, current_date FROM t";
        let tokens = lex_sql_for(text, None);
        let lower = text.find("lower").unwrap();
        assert_eq!(
            function_at(text, lower + 1, &tokens).map(|(range, signature)| (range, signature.name)),
            Some((lower..lower + 5, "LOWER"))
        );
        assert!(function_at(text, text.find("lower,").unwrap(), &tokens).is_none());
        assert!(function_at(text, text.find("current_date").unwrap(), &tokens).is_some());
        assert!(function_at(text, text.find("FROM").unwrap(), &tokens).is_none());
    }

    #[test]
    fn occurrences_match_identifiers_in_scope_ignoring_case_and_quotes() {
        let text = "SELECT u.id, \"U\".name FROM users u WHERE u.id > 1; SELECT u FROM x";
        let tokens = lex_sql_for(text, None);
        let first = text.find(';').unwrap();
        let found = occurrences(text, 8, &tokens, 0..first);
        assert_eq!(
            found
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            ["u", "\"U\"", "u", "u"]
        );
        // Touching the end of the word still counts.
        assert_eq!(occurrences(text, 8, &tokens, 0..first), found);
        // A name used once, keywords and other statements are not highlighted.
        assert!(occurrences(text, text.find("name").unwrap(), &tokens, 0..first).is_empty());
        assert!(occurrences(text, 2, &tokens, 0..first).is_empty());
    }

    #[test]
    fn local_definitions_find_aliases_and_ctes_from_their_uses() {
        let definition = |text: &str, used: &str| {
            let tokens = lex_sql_for(text, None);
            let offset = text.rfind(used).unwrap();
            local_definition(text, offset, &tokens).map(|range| range.start)
        };
        let text = "SELECT u.id FROM users u JOIN teams AS t ON t.id = u.team_id";
        assert_eq!(definition(text, "u.team_id"), text.find("u JOIN"));
        assert_eq!(definition(text, "t.id"), text.find("t ON"));
        // The definition itself, and names that are not defined here.
        assert_eq!(definition(text, "u JOIN"), None);
        assert_eq!(definition(text, "team_id"), None);
        let text =
            "WITH recent (id) AS (SELECT 1), other AS (SELECT 2) SELECT * FROM recent, other";
        assert_eq!(definition(text, "recent,"), text.find("recent"));
        assert_eq!(definition(text, "other"), text.find("other AS"));
        let text = "SELECT d.n FROM (SELECT 1 AS n) d";
        assert_eq!(
            definition(text, "d.n"),
            text.rfind(" d").map(|index| index + 1)
        );
        assert_eq!(definition(text, "n FROM"), text.find("n)"));
    }
}
