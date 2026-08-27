//! Local Redis command and key completion. This deliberately only consults
//! already-rendered keyspace results, so typing never triggers network I/O.

use std::{collections::HashSet, ops::Range};

use dbx_core::{
    CellValue, QueryResult, RedisCommand as RuntimeRedisCommand, RedisCommandArgument,
    RedisCommandCatalog,
};

use super::sql_completion::{CompletionItemKind, SqlCompletionItem};

const MAX_REDIS_COMPLETIONS: usize = 14;

#[derive(Clone, Copy)]
enum KeyArguments {
    First,
    All,
    Even,
    FirstTwo,
}

#[derive(Clone, Copy)]
struct RedisCommand {
    name: &'static str,
    detail: &'static str,
    key_arguments: Option<KeyArguments>,
}

// A deliberately compact core catalog: these are common commands where the
// argument position is unambiguous. It is not exhaustive; specialized commands
// stay raw text until their key argument semantics are captured here.
const REDIS_COMMANDS: &[RedisCommand] = &[
    RedisCommand {
        name: "APPEND",
        detail: "APPEND key value",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "COPY",
        detail: "COPY source destination",
        key_arguments: Some(KeyArguments::FirstTwo),
    },
    RedisCommand {
        name: "DECR",
        detail: "DECR key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "DBSIZE",
        detail: "DBSIZE",
        key_arguments: None,
    },
    RedisCommand {
        name: "DEL",
        detail: "DEL key [key ...]",
        key_arguments: Some(KeyArguments::All),
    },
    RedisCommand {
        name: "EXISTS",
        detail: "EXISTS key [key ...]",
        key_arguments: Some(KeyArguments::All),
    },
    RedisCommand {
        name: "ECHO",
        detail: "ECHO message",
        key_arguments: None,
    },
    RedisCommand {
        name: "EXPIRE",
        detail: "EXPIRE key seconds",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "GET",
        detail: "GET key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "GETDEL",
        detail: "GETDEL key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "GETEX",
        detail: "GETEX key [options]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "GETBIT",
        detail: "GETBIT key offset",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "GETRANGE",
        detail: "GETRANGE key start end",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HDEL",
        detail: "HDEL key field [field ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HGET",
        detail: "HGET key field",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HGETALL",
        detail: "HGETALL key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HEXISTS",
        detail: "HEXISTS key field",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HKEYS",
        detail: "HKEYS key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HLEN",
        detail: "HLEN key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HMGET",
        detail: "HMGET key field [field ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HSET",
        detail: "HSET key field value [field value ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HSETNX",
        detail: "HSETNX key field value",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "HVALS",
        detail: "HVALS key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "INCR",
        detail: "INCR key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "INFO",
        detail: "INFO [section]",
        key_arguments: None,
    },
    RedisCommand {
        name: "LINDEX",
        detail: "LINDEX key index",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "LPUSH",
        detail: "LPUSH key element [element ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "LPUSHX",
        detail: "LPUSHX key element [element ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "LRANGE",
        detail: "LRANGE key start stop",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "LPOP",
        detail: "LPOP key [count]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "MGET",
        detail: "MGET key [key ...]",
        key_arguments: Some(KeyArguments::All),
    },
    RedisCommand {
        name: "MSET",
        detail: "MSET key value [key value ...]",
        key_arguments: Some(KeyArguments::Even),
    },
    RedisCommand {
        name: "PERSIST",
        detail: "PERSIST key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "PEXPIRE",
        detail: "PEXPIRE key milliseconds",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "PING",
        detail: "PING [message]",
        key_arguments: None,
    },
    RedisCommand {
        name: "PTTL",
        detail: "PTTL key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "RENAME",
        detail: "RENAME key newkey",
        key_arguments: Some(KeyArguments::FirstTwo),
    },
    RedisCommand {
        name: "RPUSH",
        detail: "RPUSH key element [element ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "RPUSHX",
        detail: "RPUSHX key element [element ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "RPOP",
        detail: "RPOP key [count]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SADD",
        detail: "SADD key member [member ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SCARD",
        detail: "SCARD key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SCAN",
        detail: "SCAN cursor [MATCH pattern] [COUNT count]",
        key_arguments: None,
    },
    RedisCommand {
        name: "SET",
        detail: "SET key value [options]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SETBIT",
        detail: "SETBIT key offset value",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SETRANGE",
        detail: "SETRANGE key offset value",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SETNX",
        detail: "SETNX key value",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SISMEMBER",
        detail: "SISMEMBER key member",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SMEMBERS",
        detail: "SMEMBERS key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "SREM",
        detail: "SREM key member [member ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "STRLEN",
        detail: "STRLEN key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "TOUCH",
        detail: "TOUCH key [key ...]",
        key_arguments: Some(KeyArguments::All),
    },
    RedisCommand {
        name: "TIME",
        detail: "TIME",
        key_arguments: None,
    },
    RedisCommand {
        name: "TTL",
        detail: "TTL key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "TYPE",
        detail: "TYPE key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "UNLINK",
        detail: "UNLINK key [key ...]",
        key_arguments: Some(KeyArguments::All),
    },
    RedisCommand {
        name: "XADD",
        detail: "XADD key id field value [field value ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "XLEN",
        detail: "XLEN key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "XRANGE",
        detail: "XRANGE key start end [COUNT count]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZADD",
        detail: "ZADD key score member [score member ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZCARD",
        detail: "ZCARD key",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZCOUNT",
        detail: "ZCOUNT key min max",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZRANGE",
        detail: "ZRANGE key start stop [options]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZRANK",
        detail: "ZRANK key member",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZREM",
        detail: "ZREM key member [member ...]",
        key_arguments: Some(KeyArguments::First),
    },
    RedisCommand {
        name: "ZSCORE",
        detail: "ZSCORE key member",
        key_arguments: Some(KeyArguments::First),
    },
];

#[derive(Debug, Eq, PartialEq)]
struct RedisCompletionContext {
    completed: Vec<String>,
    prefix: String,
    replacement_range: Range<usize>,
    argument_index: usize,
}

pub(super) fn redis_completion_items(
    text: &str,
    cursor: usize,
    catalog: Option<&RedisCommandCatalog>,
    active_result: Option<&QueryResult>,
    browser_result: Option<&QueryResult>,
) -> Option<(Range<usize>, Vec<SqlCompletionItem>, String)> {
    let context = redis_completion_context(text, cursor)?;
    let append_space = text[context.replacement_range.end..]
        .chars()
        .next()
        .is_none_or(|character| !character.is_whitespace());
    let items = match catalog {
        Some(catalog) => redis_catalog_items(
            catalog,
            &context,
            append_space,
            [active_result, browser_result],
        ),
        None if context.completed.is_empty() => redis_command_items(&context.prefix, append_space),
        None => redis_key_items(
            context
                .completed
                .first()
                .map(String::as_str)
                .unwrap_or_default(),
            context.argument_index,
            &context.prefix,
            [active_result, browser_result],
        ),
    };
    (!items.is_empty()).then(|| {
        let signature = format!("{text}\u{0}{cursor}\u{0}{context:?}");
        (context.replacement_range, items, signature)
    })
}

fn redis_completion_context(text: &str, cursor: usize) -> Option<RedisCompletionContext> {
    let cursor = text.floor_char_boundary(cursor.min(text.len()));
    let line_start = text[..cursor].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[cursor..]
        .find('\n')
        .map_or(text.len(), |index| cursor + index);
    let line_before_cursor = &text[line_start..cursor];
    let tokens = redis_tokens(line_before_cursor);
    let (prefix, replacement_start, replacement_end, completed) = if line_before_cursor
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
    {
        (String::new(), cursor, cursor, tokens)
    } else {
        let token = tokens.last()?;
        let prefix = token.value.clone();
        let replacement_start = line_start + token.range.start;
        let replacement_end = redis_tokens(&text[line_start..line_end])
            .into_iter()
            .find(|candidate| candidate.range.start == token.range.start)
            .map_or(cursor, |candidate| line_start + candidate.range.end);
        let mut completed = tokens;
        completed.pop();
        (prefix, replacement_start, replacement_end, completed)
    };
    Some(RedisCompletionContext {
        argument_index: completed.len().saturating_sub(1),
        completed: completed.into_iter().map(|token| token.value).collect(),
        prefix,
        replacement_range: replacement_start..replacement_end,
    })
}

struct RedisToken {
    value: String,
    range: Range<usize>,
}

/// Mirrors the Redis command parser's quote/backslash rules while retaining
/// token byte ranges so replacement never consumes the next argument.
fn redis_tokens(text: &str) -> Vec<RedisToken> {
    let mut tokens = Vec::new();
    let mut value = String::new();
    let mut start = None;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if start.is_none() && !character.is_whitespace() {
            start = Some(index);
        }
        if escaped {
            value.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(expected) = quote {
            if character == expected {
                quote = None;
            } else {
                value.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            character if character.is_whitespace() => {
                if let Some(start) = start.take() {
                    tokens.push(RedisToken {
                        value: std::mem::take(&mut value),
                        range: start..index,
                    });
                }
            }
            _ => value.push(character),
        }
    }
    if escaped {
        value.push('\\');
    }
    if let Some(start) = start {
        tokens.push(RedisToken {
            value,
            range: start..text.len(),
        });
    }
    tokens
}

/// Complete against the server's `COMMAND DOCS` snapshot. The snapshot is
/// deliberately passed in rather than fetched here: this hot path is invoked
/// while rendering and must remain deterministic and network-free.
fn redis_catalog_items(
    catalog: &RedisCommandCatalog,
    context: &RedisCompletionContext,
    append_space: bool,
    results: [Option<&QueryResult>; 2],
) -> Vec<SqlCompletionItem> {
    let completed = &context.completed;
    let prefix = &context.prefix;
    let path_items = redis_catalog_path_items(catalog, completed, prefix, append_space);
    if !path_items.is_empty() {
        return path_items;
    }

    let Some((command, path_len)) = redis_catalog_command(catalog, completed) else {
        return Vec::new();
    };
    let consumed_arguments = &completed[path_len..];
    let expected = redis_expected_arguments(&command.arguments, consumed_arguments);
    let mut items = redis_argument_items(&expected.tokens, prefix, append_space);
    if expected.key {
        items.extend(redis_cached_key_items(prefix, results));
    } else if command.arguments.is_empty() && path_len == 1 {
        // Redis versions before COMMAND DOCS still advertise every command
        // name through COMMAND, but do not provide an argument grammar. Keep
        // the conservative legacy key-position knowledge for those commands
        // instead of regressing key completion on older servers.
        items.extend(redis_key_items(
            &command.name,
            consumed_arguments.len(),
            prefix,
            results,
        ));
    }
    redis_bound_items(items, prefix)
}

/// Find the next command-path segment. Redis represents subcommands with a
/// separator that differs by protocol/version (`CLIENT ID` or `client|id`), so
/// normalise both forms before matching editor tokens.
fn redis_catalog_path_items(
    catalog: &RedisCommandCatalog,
    completed: &[String],
    prefix: &str,
    append_space: bool,
) -> Vec<SqlCompletionItem> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for command in &catalog.commands {
        let path = redis_command_path(&command.name);
        let segment_index = completed.len();
        if path.len() <= segment_index
            || !redis_path_starts_with(&path, completed)
            || !redis_starts_with(&path[segment_index], prefix)
        {
            continue;
        }
        let label = path[..=segment_index].join(" ");
        if !seen.insert(label.clone()) {
            continue;
        }
        let segment = &path[segment_index];
        items.push(SqlCompletionItem::new(
            label,
            if append_space {
                format!("{segment} ")
            } else {
                segment.clone()
            },
            redis_command_detail(command),
            segment,
            CompletionItemKind::Command,
        ));
    }
    redis_bound_items(items, prefix)
}

fn redis_catalog_command<'a>(
    catalog: &'a RedisCommandCatalog,
    completed: &[String],
) -> Option<(&'a RuntimeRedisCommand, usize)> {
    catalog
        .commands
        .iter()
        .filter_map(|command| {
            let path = redis_command_path(&command.name);
            (path.len() <= completed.len() && redis_path_starts_with(&path, completed))
                .then_some((command, path.len()))
        })
        // Prefer the longest command path, so `XGROUP CREATE` wins over a
        // hypothetical top-level `XGROUP` entry.
        .max_by_key(|(_, path_len)| *path_len)
}

fn redis_command_path(name: &str) -> Vec<String> {
    name.split([' ', '|'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_uppercase())
        .collect()
}

fn redis_path_starts_with(path: &[String], typed: &[String]) -> bool {
    path.iter()
        .zip(typed)
        .all(|(expected, actual)| expected.eq_ignore_ascii_case(actual))
}

fn redis_starts_with(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|_| value[..prefix.len()].eq_ignore_ascii_case(prefix))
}

fn redis_command_detail(command: &RuntimeRedisCommand) -> String {
    let mut detail = command.name.clone();
    if let Some(summary) = command
        .summary
        .as_deref()
        .filter(|summary| !summary.is_empty())
    {
        detail.push_str(" — ");
        detail.push_str(summary);
    }
    detail
}

/// Literal completion uses command metadata tokens, including tokens attached
/// to typed values and blocks. `display` is human prose and `value` may be a
/// placeholder, neither is safe to insert as an argument.
fn redis_argument_items(
    tokens: &[&str],
    prefix: &str,
    append_space: bool,
) -> Vec<SqlCompletionItem> {
    let mut seen = HashSet::new();
    tokens
        .iter()
        .copied()
        .filter(|token| redis_starts_with(token, prefix))
        .filter(|token| seen.insert(token.to_ascii_uppercase()))
        .map(|token| {
            SqlCompletionItem::new(
                token,
                if append_space {
                    format!("{token} ")
                } else {
                    token.to_owned()
                },
                "Redis option",
                token,
                CompletionItemKind::Keyword,
            )
        })
        .collect()
}

const MAX_REDIS_GRAMMAR_STATES: usize = 4_096;

#[derive(Default)]
struct RedisExpectedArguments<'a> {
    tokens: Vec<&'a str>,
    key: bool,
}

impl<'a> RedisExpectedArguments<'a> {
    fn add_token(&mut self, token: &'a str) {
        if !token.is_empty() {
            self.tokens.push(token);
        }
    }
}

/// Walk Redis's recursive argument grammar as a small bounded NFA. Optional
/// branches are explored in parallel, while `multiple` arguments can repeat.
/// This lets completion understand real shapes such as SET's one-of options,
/// SORT's `LIMIT <offset> <count>` block and `STORE <destination>` key, and
/// XADD's repeated field/value block without doing any network work.
fn redis_expected_arguments<'a>(
    arguments: &'a [RedisCommandArgument],
    consumed: &[String],
) -> RedisExpectedArguments<'a> {
    let mut expected = RedisExpectedArguments::default();
    let mut budget = MAX_REDIS_GRAMMAR_STATES;
    redis_match_argument_sequence(arguments, vec![0], consumed, &mut expected, &mut budget);
    expected
}

fn redis_match_argument_sequence<'a>(
    arguments: &'a [RedisCommandArgument],
    mut positions: Vec<usize>,
    consumed: &[String],
    expected: &mut RedisExpectedArguments<'a>,
    budget: &mut usize,
) -> Vec<usize> {
    for argument in arguments {
        let mut next = Vec::new();
        for position in positions {
            next.extend(redis_match_argument(
                argument, position, consumed, expected, budget,
            ));
        }
        positions = redis_dedup_positions(next);
        if positions.is_empty() {
            break;
        }
    }
    positions
}

fn redis_match_argument<'a>(
    argument: &'a RedisCommandArgument,
    position: usize,
    consumed: &[String],
    expected: &mut RedisExpectedArguments<'a>,
    budget: &mut usize,
) -> Vec<usize> {
    if !redis_take_grammar_budget(budget) {
        return Vec::new();
    }

    let mut matches = argument
        .optional
        .then_some(position)
        .into_iter()
        .collect::<Vec<_>>();
    let first = redis_match_argument_once(argument, position, consumed, expected, budget);
    matches.extend(first.iter().copied());

    if argument.multiple {
        let mut seen = matches.iter().copied().collect::<HashSet<_>>();
        let mut frontier = first;
        while let Some(repetition_start) = frontier.pop() {
            let repeated = if argument.token.is_some()
                && !argument.multiple_token
                && !redis_is_pure_token(argument)
            {
                redis_match_argument_payload(argument, repetition_start, consumed, expected, budget)
            } else {
                redis_match_argument_once(argument, repetition_start, consumed, expected, budget)
            };
            for next in repeated {
                if next > repetition_start && seen.insert(next) {
                    matches.push(next);
                    frontier.push(next);
                }
            }
        }
    }

    redis_dedup_positions(matches)
}

fn redis_match_argument_once<'a>(
    argument: &'a RedisCommandArgument,
    mut position: usize,
    consumed: &[String],
    expected: &mut RedisExpectedArguments<'a>,
    budget: &mut usize,
) -> Vec<usize> {
    if !redis_take_grammar_budget(budget) {
        return Vec::new();
    }

    if let Some(token) = argument.token.as_deref().filter(|token| !token.is_empty()) {
        let Some(actual) = consumed.get(position) else {
            expected.add_token(token);
            return Vec::new();
        };
        if !actual.eq_ignore_ascii_case(token) {
            return Vec::new();
        }
        position += 1;
    } else if redis_is_pure_token(argument) {
        // A pure-token without a literal cannot be matched or inserted safely.
        return Vec::new();
    }

    redis_match_argument_payload(argument, position, consumed, expected, budget)
}

fn redis_match_argument_payload<'a>(
    argument: &'a RedisCommandArgument,
    position: usize,
    consumed: &[String],
    expected: &mut RedisExpectedArguments<'a>,
    budget: &mut usize,
) -> Vec<usize> {
    if !redis_take_grammar_budget(budget) {
        return Vec::new();
    }

    if redis_is_pure_token(argument) {
        return vec![position];
    }
    if redis_argument_is(argument, "oneof") {
        let mut matches = Vec::new();
        for alternative in &argument.arguments {
            matches.extend(redis_match_argument(
                alternative,
                position,
                consumed,
                expected,
                budget,
            ));
        }
        return redis_dedup_positions(matches);
    }
    if redis_argument_is(argument, "block") {
        return redis_match_argument_sequence(
            &argument.arguments,
            vec![position],
            consumed,
            expected,
            budget,
        );
    }

    let Some(_) = consumed.get(position) else {
        if redis_is_catalog_key(argument) {
            expected.key = true;
        }
        return Vec::new();
    };
    vec![position + 1]
}

fn redis_take_grammar_budget(budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    true
}

fn redis_dedup_positions(mut positions: Vec<usize>) -> Vec<usize> {
    positions.sort_unstable();
    positions.dedup();
    positions
}

fn redis_is_pure_token(argument: &RedisCommandArgument) -> bool {
    redis_argument_is(argument, "pure-token")
}

fn redis_argument_is(argument: &RedisCommandArgument, expected: &str) -> bool {
    argument
        .kind
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case(expected))
}

fn redis_is_catalog_key(argument: &RedisCommandArgument) -> bool {
    redis_argument_is(argument, "key") && argument.key_spec_index.is_some()
}

fn redis_cached_key_items(
    prefix: &str,
    results: [Option<&QueryResult>; 2],
) -> Vec<SqlCompletionItem> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for result in results.into_iter().flatten() {
        let Some(key_index) = redis_keyspace_index(result) else {
            continue;
        };
        for key in result
            .rows
            .iter()
            .filter_map(|row| row.values.get(key_index))
            .filter_map(redis_utf8_key)
            .filter(|key| redis_starts_with(key, prefix))
        {
            if seen.insert(key) {
                items.push(SqlCompletionItem::new(
                    key,
                    redis_command_argument(key),
                    "cached Redis key",
                    key,
                    CompletionItemKind::Key,
                ));
            }
        }
    }
    items
}

/// Keep the menu compact while ensuring that an exact candidate is never
/// pushed below the cutoff by lexically earlier prefix matches.
fn redis_bound_items(mut items: Vec<SqlCompletionItem>, prefix: &str) -> Vec<SqlCompletionItem> {
    items.sort_by(|left, right| {
        let left_exact = left.label.eq_ignore_ascii_case(prefix);
        let right_exact = right.label.eq_ignore_ascii_case(prefix);
        right_exact
            .cmp(&left_exact)
            .then_with(|| left.label.cmp(&right.label))
    });
    items.truncate(MAX_REDIS_COMPLETIONS);
    items
}

fn redis_command_items(prefix: &str, append_space: bool) -> Vec<SqlCompletionItem> {
    let prefix = prefix.to_ascii_lowercase();
    let mut items = REDIS_COMMANDS
        .iter()
        .filter(|command| command.name.to_ascii_lowercase().starts_with(&prefix))
        .map(|command| {
            SqlCompletionItem::new(
                command.name,
                if append_space {
                    format!("{} ", command.name)
                } else {
                    command.name.to_owned()
                },
                command.detail,
                command.name,
                CompletionItemKind::Command,
            )
        })
        .collect::<Vec<_>>();
    items.truncate(MAX_REDIS_COMPLETIONS);
    items
}

fn redis_key_items(
    command: &str,
    argument_index: usize,
    prefix: &str,
    results: [Option<&QueryResult>; 2],
) -> Vec<SqlCompletionItem> {
    let Some(spec) = REDIS_COMMANDS
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(command))
    else {
        return Vec::new();
    };
    let Some(key_arguments) = spec.key_arguments else {
        return Vec::new();
    };
    if !is_key_argument(key_arguments, argument_index) {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for result in results.into_iter().flatten() {
        // Redis SCAN (and the keyspace browser built from it) has this stable
        // shape. Do not mistake an HGETALL-style `key` result for a key cache.
        let Some(key_index) = redis_keyspace_index(result) else {
            continue;
        };
        for key in result
            .rows
            .iter()
            .filter_map(|row| row.values.get(key_index))
            .filter_map(redis_utf8_key)
            .filter(|key| key.starts_with(prefix))
        {
            if !seen.insert(key) {
                continue;
            }
            items.push(SqlCompletionItem::new(
                key,
                redis_command_argument(key),
                "cached Redis key",
                key,
                CompletionItemKind::Key,
            ));
            if items.len() == MAX_REDIS_COMPLETIONS {
                return items;
            }
        }
    }
    items
}

fn redis_keyspace_index(result: &QueryResult) -> Option<usize> {
    matches!(
        result.columns.as_slice(),
        [key, kind, ttl] if key.name == "key" && kind.name == "type" && ttl.name == "ttl"
    )
    .then(|| {
        result
            .columns
            .iter()
            .position(|column| column.name == "key")
    })
    .flatten()
}

fn is_key_argument(arguments: KeyArguments, index: usize) -> bool {
    match arguments {
        KeyArguments::First => index == 0,
        KeyArguments::All => true,
        KeyArguments::Even => index.is_multiple_of(2),
        KeyArguments::FirstTwo => index < 2,
    }
}

fn redis_utf8_key(value: &CellValue) -> Option<&str> {
    let key = match value {
        CellValue::Text(value) => Some(value.as_str()),
        CellValue::Bytes(value) => std::str::from_utf8(value).ok(),
        _ => None,
    }?;
    (!key.chars().any(char::is_control)).then_some(key)
}

fn redis_command_argument(key: &str) -> String {
    if key.is_empty()
        || key
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, '\\' | '\'' | '"'))
    {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        key.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use dbx_core::{ColumnInfo, RowData};

    use super::*;

    fn keyspace_result(keys: &[CellValue]) -> QueryResult {
        QueryResult {
            columns: vec![
                ColumnInfo::result("key", 0, "string"),
                ColumnInfo::result("type", 1, "string"),
                ColumnInfo::result("ttl", 2, "integer"),
            ],
            rows: keys
                .iter()
                .cloned()
                .map(|key| {
                    RowData::new(vec![
                        key,
                        CellValue::Text("string".into()),
                        CellValue::Integer(-1),
                    ])
                })
                .collect(),
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        }
    }

    fn catalog(commands: Vec<RuntimeRedisCommand>) -> RedisCommandCatalog {
        RedisCommandCatalog { commands }
    }

    fn command(name: &str, arguments: Vec<RedisCommandArgument>) -> RuntimeRedisCommand {
        RuntimeRedisCommand {
            name: name.into(),
            summary: Some(format!("{name} summary")),
            group: None,
            since: None,
            arguments,
        }
    }

    fn argument(
        kind: &str,
        token: Option<&str>,
        key_spec_index: Option<usize>,
        arguments: Vec<RedisCommandArgument>,
    ) -> RedisCommandArgument {
        RedisCommandArgument {
            kind: Some(kind.into()),
            token: token.map(str::to_owned),
            display: None,
            value: None,
            optional: false,
            multiple: false,
            multiple_token: false,
            key_spec_index,
            arguments,
        }
    }

    fn fixture_catalog() -> RedisCommandCatalog {
        catalog(vec![
            command("GET", vec![argument("key", None, Some(0), Vec::new())]),
            command(
                "SET",
                vec![
                    argument("key", None, Some(0), Vec::new()),
                    argument("string", None, None, Vec::new()),
                    argument(
                        "oneof",
                        None,
                        None,
                        vec![
                            argument("pure-token", Some("NX"), None, Vec::new()),
                            argument("pure-token", Some("XX"), None, Vec::new()),
                        ],
                    ),
                ],
            ),
            command("CLIENT ID", Vec::new()),
            command("CONFIG GET", Vec::new()),
            command("XGROUP CREATE", Vec::new()),
            command("TOPK.RESERVE", Vec::new()),
            command("SUBSTR", Vec::new()),
            command("SLAVEOF", Vec::new()),
        ])
    }

    fn catalog_completion(
        text: &str,
        catalog: &RedisCommandCatalog,
    ) -> (Range<usize>, Vec<SqlCompletionItem>, String) {
        redis_completion_items(text, text.len(), Some(catalog), None, None)
            .unwrap_or_else(|| panic!("expected completions for {text:?}"))
    }

    #[test]
    fn runtime_catalog_covers_modules_compound_commands_and_aliases() {
        let catalog = fixture_catalog();
        for (text, expected) in [
            ("TOPK.", "TOPK.RESERVE"),
            ("CLIENT I", "CLIENT ID"),
            ("CONFIG G", "CONFIG GET"),
            ("XGROUP C", "XGROUP CREATE"),
            ("SUBS", "SUBSTR"),
            ("SLAVE", "SLAVEOF"),
        ] {
            let (_, items, _) = catalog_completion(text, &catalog);
            assert!(
                items.iter().any(|item| item.label == expected),
                "{expected} was not discoverable from {text:?}"
            );
        }
    }

    #[test]
    fn runtime_catalog_is_case_insensitive_and_preserves_the_untyped_suffix() {
        let catalog = fixture_catalog();
        let text = "client i trailing";
        let (range, items, _) =
            redis_completion_items(text, "client i".len(), Some(&catalog), None, None)
                .expect("CLIENT ID completion");
        let item = items
            .iter()
            .find(|item| item.label == "CLIENT ID")
            .expect("CLIENT ID item");

        assert_eq!(range, 7..8);
        assert_eq!(item.insert_text, "ID");
        assert_eq!(
            format!(
                "{}{}{}",
                &text[..range.start],
                item.insert_text,
                &text[range.end..]
            ),
            "client ID trailing"
        );
    }

    #[test]
    fn runtime_catalog_offers_recursive_pure_token_options_but_not_values_as_keys() {
        let catalog = fixture_catalog();
        let key_cache = keyspace_result(&[CellValue::Text("user:42".into())]);

        let (_, key_items, _) =
            redis_completion_items("SET us", 6, Some(&catalog), Some(&key_cache), None)
                .expect("key completion");
        assert!(key_items.iter().any(|item| item.label == "user:42"));

        let (_, option_items, _) = redis_completion_items(
            "SET user:42 value N",
            "SET user:42 value N".len(),
            Some(&catalog),
            Some(&key_cache),
            None,
        )
        .expect("option completion");
        assert!(option_items.iter().any(|item| item.label == "NX"));
        assert!(
            !option_items
                .iter()
                .any(|item| item.kind == CompletionItemKind::Key)
        );

        assert!(
            redis_completion_items(
                "SET user:42 va",
                "SET user:42 va".len(),
                Some(&catalog),
                Some(&key_cache),
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn runtime_catalog_offers_typed_argument_tokens_without_reoffering_them_as_values() {
        let mut count = argument("integer", Some("COUNT"), None, Vec::new());
        count.optional = true;
        let mut r#match = argument("pattern", Some("MATCH"), None, Vec::new());
        r#match.optional = true;
        let catalog = catalog(vec![command(
            "SCAN",
            vec![argument("integer", None, None, Vec::new()), count, r#match],
        )]);

        let (_, initial_items, _) = catalog_completion("SCAN 0 ", &catalog);
        assert!(initial_items.iter().any(|item| item.label == "COUNT"));
        assert!(initial_items.iter().any(|item| item.label == "MATCH"));

        let (_, after_value_items, _) = catalog_completion("SCAN 0 COUNT 10 ", &catalog);
        assert!(after_value_items.iter().any(|item| item.label == "MATCH"));
        assert!(!after_value_items.iter().any(|item| item.label == "COUNT"));

        assert!(
            redis_completion_items(
                "SCAN 0 COUNT 10 C",
                "SCAN 0 COUNT 10 C".len(),
                Some(&catalog),
                None,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn runtime_catalog_walks_nested_oneofs_and_skipped_optional_siblings() {
        let mut condition = argument(
            "oneof",
            None,
            None,
            vec![
                argument("pure-token", Some("NX"), None, Vec::new()),
                argument("integer", Some("IFEQ"), None, Vec::new()),
            ],
        );
        condition.optional = true;
        let mut get = argument("pure-token", Some("GET"), None, Vec::new());
        get.optional = true;
        let mut expiration = argument(
            "oneof",
            None,
            None,
            vec![
                argument("integer", Some("EX"), None, Vec::new()),
                argument("integer", Some("PX"), None, Vec::new()),
                argument("pure-token", Some("KEEPTTL"), None, Vec::new()),
            ],
        );
        expiration.optional = true;
        let catalog = catalog(vec![command(
            "SET",
            vec![
                argument("key", None, Some(0), Vec::new()),
                argument("string", None, None, Vec::new()),
                condition,
                get,
                expiration,
            ],
        )]);

        let (_, initial, _) = catalog_completion("SET user value ", &catalog);
        for expected in ["NX", "IFEQ", "GET", "EX", "PX", "KEEPTTL"] {
            assert!(
                initial.iter().any(|item| item.label == expected),
                "missing nested SET option {expected}"
            );
        }

        let (_, after_get, _) = catalog_completion("SET user value GET P", &catalog);
        assert!(after_get.iter().any(|item| item.label == "PX"));
    }

    #[test]
    fn runtime_catalog_consumes_blocks_and_repeats_tokenized_arguments() {
        let mut limit = argument(
            "block",
            Some("LIMIT"),
            None,
            vec![
                argument("integer", None, None, Vec::new()),
                argument("integer", None, None, Vec::new()),
            ],
        );
        limit.optional = true;
        let mut get = argument("pattern", Some("GET"), Some(1), Vec::new());
        get.optional = true;
        get.multiple = true;
        get.multiple_token = true;
        let mut order = argument(
            "oneof",
            None,
            None,
            vec![
                argument("pure-token", Some("ASC"), None, Vec::new()),
                argument("pure-token", Some("DESC"), None, Vec::new()),
            ],
        );
        order.optional = true;
        let mut store = argument("key", Some("STORE"), Some(2), Vec::new());
        store.optional = true;
        let catalog = catalog(vec![command(
            "SORT",
            vec![
                argument("key", None, Some(0), Vec::new()),
                limit,
                get,
                order,
                store,
            ],
        )]);

        assert!(
            redis_completion_items(
                "SORT source LIMIT 0 ",
                "SORT source LIMIT 0 ".len(),
                Some(&catalog),
                None,
                None,
            )
            .is_none(),
            "later options must wait for LIMIT's required count"
        );

        let (_, after_limit, _) = catalog_completion("SORT source LIMIT 0 10 S", &catalog);
        assert!(after_limit.iter().any(|item| item.label == "STORE"));

        let (_, repeated_get, _) = catalog_completion("SORT source GET pattern G", &catalog);
        assert!(repeated_get.iter().any(|item| item.label == "GET"));
    }

    #[test]
    fn runtime_catalog_finds_keys_nested_behind_tokens_and_blocks() {
        let mut store = argument("key", Some("STORE"), Some(2), Vec::new());
        store.optional = true;
        let sort_catalog = catalog(vec![command(
            "SORT",
            vec![argument("key", None, Some(0), Vec::new()), store],
        )]);
        let mut stream_keys = argument("key", None, Some(0), Vec::new());
        stream_keys.multiple = true;
        let mut stream_ids = argument("string", None, None, Vec::new());
        stream_ids.multiple = true;
        let xread_catalog = catalog(vec![command(
            "XREAD",
            vec![argument(
                "block",
                Some("STREAMS"),
                None,
                vec![stream_keys, stream_ids],
            )],
        )]);
        let key_cache = keyspace_result(&[
            CellValue::Text("destination:list".into()),
            CellValue::Text("user:stream".into()),
        ]);

        let (_, store_items, _) = redis_completion_items(
            "SORT source STORE dest",
            "SORT source STORE dest".len(),
            Some(&sort_catalog),
            Some(&key_cache),
            None,
        )
        .expect("STORE destination key completion");
        assert!(
            store_items
                .iter()
                .any(|item| item.label == "destination:list")
        );

        let (_, stream_items, _) = redis_completion_items(
            "XREAD STREAMS user:",
            "XREAD STREAMS user:".len(),
            Some(&xread_catalog),
            Some(&key_cache),
            None,
        )
        .expect("nested STREAMS key completion");
        assert!(stream_items.iter().any(|item| item.label == "user:stream"));
    }

    #[test]
    fn runtime_catalog_walks_untokenized_nested_blocks() {
        let strategy = argument(
            "oneof",
            None,
            None,
            vec![
                argument("pure-token", Some("MAXLEN"), None, Vec::new()),
                argument("pure-token", Some("MINID"), None, Vec::new()),
            ],
        );
        let mut operator = argument(
            "oneof",
            None,
            None,
            vec![
                argument("pure-token", Some("="), None, Vec::new()),
                argument("pure-token", Some("~"), None, Vec::new()),
            ],
        );
        operator.optional = true;
        let mut count = argument("integer", Some("LIMIT"), None, Vec::new());
        count.optional = true;
        let mut trim = argument(
            "block",
            None,
            None,
            vec![
                strategy,
                operator,
                argument("string", None, None, Vec::new()),
                count,
            ],
        );
        trim.optional = true;
        let catalog = catalog(vec![command(
            "XADD",
            vec![argument("key", None, Some(0), Vec::new()), trim],
        )]);

        let (_, strategy_items, _) = catalog_completion("XADD stream M", &catalog);
        assert!(strategy_items.iter().any(|item| item.label == "MAXLEN"));

        let (_, limit_items, _) = catalog_completion("XADD stream MAXLEN ~ 100 L", &catalog);
        assert!(limit_items.iter().any(|item| item.label == "LIMIT"));
    }

    #[test]
    fn runtime_catalog_offers_cached_keys_after_repeated_direct_key_arguments() {
        let mut key = argument("key", None, Some(0), Vec::new());
        key.multiple = true;
        let catalog = catalog(vec![command("DEL", vec![key])]);
        let key_cache = keyspace_result(&[
            CellValue::Text("one".into()),
            CellValue::Text("user:42".into()),
        ]);

        let (_, items, _) = redis_completion_items(
            "DEL one us",
            "DEL one us".len(),
            Some(&catalog),
            Some(&key_cache),
            None,
        )
        .expect("repeated key completion");

        assert!(
            items
                .iter()
                .any(|item| { item.kind == CompletionItemKind::Key && item.label == "user:42" })
        );
    }

    #[test]
    fn command_metadata_fallback_keeps_safe_legacy_key_completion() {
        let catalog = catalog(vec![command("GET", Vec::new())]);
        let key_cache = keyspace_result(&[CellValue::Text("user:42".into())]);

        let (_, items, _) = redis_completion_items(
            "GET user:",
            "GET user:".len(),
            Some(&catalog),
            Some(&key_cache),
            None,
        )
        .expect("legacy key completion with COMMAND-only metadata");

        assert!(items.iter().any(|item| item.label == "user:42"));
    }

    #[test]
    fn exact_runtime_candidate_survives_the_menu_bound() {
        let mut commands = (0..MAX_REDIS_COMPLETIONS + 4)
            .map(|index| command(&format!("COMMAND{index}"), Vec::new()))
            .collect::<Vec<_>>();
        commands.push(command("COMMAND", Vec::new()));
        let catalog = catalog(commands);

        let (_, items, _) = catalog_completion("COMMAND", &catalog);
        assert!(items.len() <= MAX_REDIS_COMPLETIONS);
        assert_eq!(
            items.first().map(|item| item.label.as_str()),
            Some("COMMAND")
        );
    }

    #[test]
    fn partial_command_offers_get() {
        let (_, items, _) =
            redis_completion_items("GE", 2, None, None, None).expect("command completions");
        let get = items
            .iter()
            .find(|item| item.kind == CompletionItemKind::Command && item.label == "GET")
            .expect("GET command");
        assert_eq!(get.insert_text, "GET ");
    }

    #[test]
    fn command_completion_preserves_following_arguments_without_duplicate_spacing() {
        let text = "GE existing:key";
        let (range, items, _) =
            redis_completion_items(text, 2, None, None, None).expect("command completions");
        let get = items
            .iter()
            .find(|item| item.kind == CompletionItemKind::Command && item.label == "GET")
            .expect("GET command");

        assert_eq!(range, 0..2);
        assert_eq!(
            format!(
                "{}{}{}",
                &text[..range.start],
                get.insert_text,
                &text[range.end..]
            ),
            "GET existing:key"
        );
    }

    #[test]
    fn common_server_and_stream_commands_are_discoverable() {
        let (_, server_items, _) =
            redis_completion_items("PI", 2, None, None, None).expect("server commands");
        assert!(server_items.iter().any(|item| item.label == "PING"));

        let (_, stream_items, _) =
            redis_completion_items("XR", 2, None, None, None).expect("stream commands");
        assert!(stream_items.iter().any(|item| item.label == "XRANGE"));
    }

    #[test]
    fn common_string_hash_list_set_and_sorted_set_commands_are_discoverable() {
        for (prefix, command) in [
            ("SETN", "SETNX"),
            ("GETB", "GETBIT"),
            ("HSETN", "HSETNX"),
            ("LPUSHX", "LPUSHX"),
            ("SIS", "SISMEMBER"),
            ("ZRAN", "ZRANK"),
        ] {
            let (_, items, _) = redis_completion_items(prefix, prefix.len(), None, None, None)
                .expect("command items");
            assert!(items.iter().any(|item| item.label == command));
        }
    }

    #[test]
    fn completion_only_uses_the_current_line() {
        let (_, items, _) = redis_completion_items("SET stale value\nGE", 18, None, None, None)
            .expect("current line command completions");
        assert!(items.iter().any(|item| item.label == "GET"));
        assert!(!items.iter().any(|item| item.label == "SET"));
    }

    #[test]
    fn key_completion_uses_cached_keyspace_and_preserves_the_suffix() {
        let result = keyspace_result(&[CellValue::Text("user:42".into())]);
        let text = "GET user: suffix";
        let cursor = "GET user:".len();
        let (range, items, _) = redis_completion_items(text, cursor, None, Some(&result), None)
            .expect("key completions");
        let item = items
            .iter()
            .find(|item| item.label == "user:42")
            .expect("cached key");
        assert_eq!(range, 4..9);
        assert_eq!(item.insert_text, "user:42");
        assert_eq!(
            format!(
                "{}{}{}",
                &text[..range.start],
                item.insert_text,
                &text[range.end..]
            ),
            "GET user:42 suffix"
        );
    }

    #[test]
    fn key_completion_replaces_the_whole_argument_when_the_caret_is_inside_it() {
        let result = keyspace_result(&[CellValue::Text("user:7".into())]);
        let text = "GET user:42";
        let cursor = "GET user:".len();
        let (range, items, _) = redis_completion_items(text, cursor, None, Some(&result), None)
            .expect("key completions");
        let item = items
            .iter()
            .find(|item| item.label == "user:7")
            .expect("cached key");

        assert_eq!(range, 4..text.len());
        assert_eq!(
            format!(
                "{}{}{}",
                &text[..range.start],
                item.insert_text,
                &text[range.end..]
            ),
            "GET user:7"
        );
    }

    #[test]
    fn key_completion_prefers_active_query_results_then_deduplicates_browser_results() {
        let active = keyspace_result(&[
            CellValue::Text("active:key".into()),
            CellValue::Text("shared:key".into()),
        ]);
        let browser = keyspace_result(&[
            CellValue::Text("shared:key".into()),
            CellValue::Text("browser:key".into()),
        ]);
        let (_, items, _) = redis_completion_items("GET ", 4, None, Some(&active), Some(&browser))
            .expect("cached key completions");

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["active:key", "shared:key", "browser:key"]
        );
    }

    #[test]
    fn keys_only_appear_in_key_positions_and_are_escaped_for_redis() {
        let result = keyspace_result(&[CellValue::Text("user name".into())]);
        let (_, items, _) = redis_completion_items("MGET user", 9, None, Some(&result), None)
            .expect("variadic key completion");
        assert_eq!(items[0].insert_text, "\"user name\"");
        assert!(redis_completion_items("SET user value", 14, None, Some(&result), None).is_none());
        assert!(redis_completion_items("GET user ", 9, None, Some(&result), None).is_none());
    }

    #[test]
    fn key_completion_skips_keys_that_cannot_fit_the_line_or_decode_as_utf8() {
        let result = keyspace_result(&[
            CellValue::Text("safe:key".into()),
            CellValue::Text("line\nbreak".into()),
            CellValue::Text("vertical\u{b}tab".into()),
            CellValue::Bytes(vec![0xff]),
        ]);
        let (_, items, _) = redis_completion_items("GET ", 4, None, Some(&result), None)
            .expect("safe key completion");

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["safe:key"]
        );
    }
}
