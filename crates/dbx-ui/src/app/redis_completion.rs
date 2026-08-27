//! Local Redis command and key completion. This deliberately only consults
//! already-rendered keyspace results, so typing never triggers network I/O.

use std::{collections::HashSet, ops::Range};

use dbx_core::{CellValue, QueryResult};

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
    command: Option<String>,
    prefix: String,
    replacement_range: Range<usize>,
    argument_index: usize,
}

pub(super) fn redis_completion_items(
    text: &str,
    cursor: usize,
    active_result: Option<&QueryResult>,
    browser_result: Option<&QueryResult>,
) -> Option<(Range<usize>, Vec<SqlCompletionItem>, String)> {
    let context = redis_completion_context(text, cursor)?;
    let items = match context.command.as_deref() {
        None => redis_command_items(
            &context.prefix,
            text[context.replacement_range.end..]
                .chars()
                .next()
                .is_none_or(|character| !character.is_whitespace()),
        ),
        Some(command) => redis_key_items(
            command,
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
    let command = completed
        .first()
        .map(|token| token.value.to_ascii_uppercase());
    Some(RedisCompletionContext {
        argument_index: completed.len().saturating_sub(1),
        command,
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

    #[test]
    fn partial_command_offers_get() {
        let (_, items, _) =
            redis_completion_items("GE", 2, None, None).expect("command completions");
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
            redis_completion_items(text, 2, None, None).expect("command completions");
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
            redis_completion_items("PI", 2, None, None).expect("server commands");
        assert!(server_items.iter().any(|item| item.label == "PING"));

        let (_, stream_items, _) =
            redis_completion_items("XR", 2, None, None).expect("stream commands");
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
            let (_, items, _) =
                redis_completion_items(prefix, prefix.len(), None, None).expect("command items");
            assert!(items.iter().any(|item| item.label == command));
        }
    }

    #[test]
    fn completion_only_uses_the_current_line() {
        let (_, items, _) = redis_completion_items("SET stale value\nGE", 18, None, None)
            .expect("current line command completions");
        assert!(items.iter().any(|item| item.label == "GET"));
        assert!(!items.iter().any(|item| item.label == "SET"));
    }

    #[test]
    fn key_completion_uses_cached_keyspace_and_preserves_the_suffix() {
        let result = keyspace_result(&[CellValue::Text("user:42".into())]);
        let text = "GET user: suffix";
        let cursor = "GET user:".len();
        let (range, items, _) =
            redis_completion_items(text, cursor, Some(&result), None).expect("key completions");
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
        let (range, items, _) =
            redis_completion_items(text, cursor, Some(&result), None).expect("key completions");
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
        let (_, items, _) = redis_completion_items("GET ", 4, Some(&active), Some(&browser))
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
        let (_, items, _) = redis_completion_items("MGET user", 9, Some(&result), None)
            .expect("variadic key completion");
        assert_eq!(items[0].insert_text, "\"user name\"");
        assert!(redis_completion_items("SET user value", 14, Some(&result), None).is_none());
        assert!(redis_completion_items("GET user ", 9, Some(&result), None).is_none());
    }

    #[test]
    fn key_completion_skips_keys_that_cannot_fit_the_line_or_decode_as_utf8() {
        let result = keyspace_result(&[
            CellValue::Text("safe:key".into()),
            CellValue::Text("line\nbreak".into()),
            CellValue::Text("vertical\u{b}tab".into()),
            CellValue::Bytes(vec![0xff]),
        ]);
        let (_, items, _) =
            redis_completion_items("GET ", 4, Some(&result), None).expect("safe key completion");

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["safe:key"]
        );
    }
}
