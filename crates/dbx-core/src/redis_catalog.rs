//! Runtime Redis command metadata for command editors and other clients.
//!
//! Redis owns this schema: DBX asks the connected server for `COMMAND DOCS`
//! rather than carrying a potentially stale copy of Redis' command table.

use std::collections::HashSet;

use redis::Value;

use crate::{DbxError, Result};

const MAX_COMMANDS: usize = 10_000;
const MAX_ARGUMENTS_PER_LEVEL: usize = 256;
const MAX_METADATA_DEPTH: usize = 16;

/// The complete command catalogue advertised by one Redis server.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RedisCommandCatalog {
    pub commands: Vec<RedisCommand>,
}

/// A Redis command or subcommand, such as `GET` or `ACL CAT`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisCommand {
    pub name: String,
    pub summary: Option<String>,
    pub group: Option<String>,
    pub since: Option<String>,
    pub arguments: Vec<RedisCommandArgument>,
}

/// Recursive argument metadata supplied by `COMMAND DOCS`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RedisCommandArgument {
    pub kind: Option<String>,
    pub token: Option<String>,
    pub display: Option<String>,
    pub value: Option<String>,
    pub optional: bool,
    pub multiple: bool,
    pub multiple_token: bool,
    pub key_spec_index: Option<usize>,
    pub arguments: Vec<RedisCommandArgument>,
}

impl RedisCommandCatalog {
    /// Decode a `COMMAND DOCS` reply from either RESP2's flattened maps or
    /// RESP3 maps. Unknown fields are intentionally ignored.
    pub fn from_command_docs(value: &Value) -> Result<Self> {
        let entries = value_map(value).ok_or_else(|| {
            DbxError::Decode("Redis COMMAND DOCS reply was not a command map".into())
        })?;
        let mut commands = Vec::new();
        let mut names = HashSet::new();
        for (name, document) in entries.into_iter().take(MAX_COMMANDS) {
            collect_document(&name, document, 0, &mut names, &mut commands);
        }
        if commands.is_empty() {
            return Err(DbxError::Decode(
                "Redis COMMAND DOCS reply contained no command documents".into(),
            ));
        }
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { commands })
    }

    /// Decode the older `COMMAND` metadata reply. It contains less detail,
    /// but still gives DBX every command and subcommand advertised in it.
    pub fn from_command_metadata(value: &Value) -> Result<Self> {
        let mut commands = Vec::new();
        let mut names = HashSet::new();
        collect_command_metadata(value, 0, &mut names, &mut commands);
        if commands.is_empty() {
            return Err(DbxError::Decode(
                "Redis COMMAND reply contained no command metadata".into(),
            ));
        }
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { commands })
    }
}

fn collect_document(
    name: &str,
    document: &Value,
    depth: usize,
    names: &mut HashSet<String>,
    commands: &mut Vec<RedisCommand>,
) {
    if depth > MAX_METADATA_DEPTH || commands.len() >= MAX_COMMANDS {
        return;
    }
    let normalized = normalize_name(name);
    let Some(fields) = value_map(document) else {
        return;
    };
    if commands.len() < MAX_COMMANDS && names.insert(normalized.clone()) {
        commands.push(RedisCommand {
            name: normalized,
            summary: field_text(&fields, "summary"),
            group: field_text(&fields, "group"),
            since: field_text(&fields, "since"),
            arguments: field_value(&fields, "arguments")
                .map(|arguments| parse_arguments(arguments, depth + 1))
                .unwrap_or_default(),
        });
    }
    if let Some(subcommands) = field_value(&fields, "subcommands").and_then(value_map) {
        for (subcommand_name, subcommand) in subcommands.into_iter().take(MAX_COMMANDS) {
            collect_document(&subcommand_name, subcommand, depth + 1, names, commands);
        }
    }
}

fn collect_command_metadata(
    value: &Value,
    depth: usize,
    names: &mut HashSet<String>,
    commands: &mut Vec<RedisCommand>,
) {
    if depth > MAX_METADATA_DEPTH || commands.len() >= MAX_COMMANDS {
        return;
    }
    match value {
        Value::Map(entries) => {
            for (key, metadata) in entries {
                if let Some(name) = value_text(key) {
                    if metadata_record(metadata) {
                        push_command(name, names, commands);
                        collect_metadata_subcommands(metadata, depth + 1, names, commands);
                    } else {
                        collect_command_metadata(metadata, depth + 1, names, commands);
                    }
                } else {
                    collect_command_metadata(metadata, depth + 1, names, commands);
                }
            }
        }
        Value::Array(entries) | Value::Set(entries) => {
            if metadata_record(value) {
                if let Some(name) = entries.first().and_then(value_text) {
                    push_command(name, names, commands);
                    collect_metadata_subcommands(value, depth + 1, names, commands);
                }
            } else {
                for entry in entries.iter().take(MAX_COMMANDS) {
                    collect_command_metadata(entry, depth + 1, names, commands);
                }
            }
        }
        Value::Attribute { data, .. } => collect_command_metadata(data, depth + 1, names, commands),
        _ => {}
    }
}

fn collect_metadata_subcommands(
    metadata: &Value,
    depth: usize,
    names: &mut HashSet<String>,
    commands: &mut Vec<RedisCommand>,
) {
    let Some(entries) = value_array(metadata) else {
        return;
    };
    // Redis 7's documented COMMAND record puts subcommands at index 9, after
    // ACL categories, tips, and key specifications. Do not recursively scan
    // those metadata fields: RESP2 key-spec maps contain pairs such as
    // `index, 1`, which otherwise resemble a tiny command record.
    if let Some(subcommands) = entries.get(9) {
        collect_command_metadata(subcommands, depth, names, commands);
    }
}

fn metadata_record(value: &Value) -> bool {
    let Some(entries) = value_array(value) else {
        return false;
    };
    matches!(entries.first().and_then(value_text), Some(name) if !name.is_empty())
        && matches!(entries.get(1), Some(Value::Int(_)))
}

fn push_command(name: &str, names: &mut HashSet<String>, commands: &mut Vec<RedisCommand>) {
    let name = normalize_name(name);
    if commands.len() < MAX_COMMANDS && names.insert(name.clone()) {
        commands.push(RedisCommand {
            name,
            summary: None,
            group: None,
            since: None,
            arguments: Vec::new(),
        });
    }
}

fn parse_arguments(value: &Value, depth: usize) -> Vec<RedisCommandArgument> {
    if depth > MAX_METADATA_DEPTH {
        return Vec::new();
    }
    value_array(value)
        .into_iter()
        .flatten()
        .take(MAX_ARGUMENTS_PER_LEVEL)
        .filter_map(|argument| parse_argument(argument, depth))
        .collect()
}

fn parse_argument(value: &Value, depth: usize) -> Option<RedisCommandArgument> {
    let fields = value_map(value)?;
    let optional = field_flag(&fields, "optional") || field_bool(&fields, "optional");
    let multiple = field_flag(&fields, "multiple") || field_bool(&fields, "multiple");
    let multiple_token = field_flag(&fields, "multiple_token")
        || field_flag(&fields, "multiple-token")
        || field_bool(&fields, "multiple_token");
    let key_spec_index = field_value(&fields, "key_spec_index").and_then(value_usize);
    Some(RedisCommandArgument {
        kind: field_text(&fields, "type"),
        token: field_text(&fields, "token"),
        display: field_text(&fields, "display_text").or_else(|| field_text(&fields, "display")),
        value: field_text(&fields, "name").or_else(|| field_text(&fields, "value")),
        optional,
        multiple,
        multiple_token,
        key_spec_index,
        arguments: field_value(&fields, "arguments")
            .or_else(|| field_value(&fields, "value"))
            .map(|arguments| parse_arguments(arguments, depth + 1))
            .unwrap_or_default(),
    })
}

fn value_map(value: &Value) -> Option<Vec<(String, &Value)>> {
    match value {
        Value::Map(entries) => Some(
            entries
                .iter()
                .filter_map(|(key, value)| value_text(key).map(|key| (key.to_owned(), value)))
                .collect(),
        ),
        Value::Array(entries) if entries.len() % 2 == 0 => Some(
            entries
                .as_chunks::<2>()
                .0
                .iter()
                .filter_map(|pair| value_text(&pair[0]).map(|key| (key.to_owned(), &pair[1])))
                .collect(),
        ),
        Value::Attribute { data, .. } => value_map(data),
        _ => None,
    }
}

fn value_array(value: &Value) -> Option<&[Value]> {
    match value {
        Value::Array(values) | Value::Set(values) => Some(values),
        Value::Attribute { data, .. } => value_array(data),
        _ => None,
    }
}

fn field_value<'a>(fields: &'a [(String, &'a Value)], name: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
}

fn field_text(fields: &[(String, &Value)], name: &str) -> Option<String> {
    field_value(fields, name)
        .and_then(value_text)
        .map(str::to_owned)
}

fn field_bool(fields: &[(String, &Value)], name: &str) -> bool {
    field_value(fields, name)
        .map(|value| match value {
            Value::Boolean(value) => *value,
            Value::Int(value) => *value != 0,
            _ => value_text(value)
                .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1"),
        })
        .unwrap_or(false)
}

fn field_flag(fields: &[(String, &Value)], name: &str) -> bool {
    field_value(fields, "flags")
        .and_then(value_array)
        .is_some_and(|flags| {
            flags
                .iter()
                .filter_map(value_text)
                .any(|flag| flag.eq_ignore_ascii_case(name))
        })
}

fn value_text(value: &Value) -> Option<&str> {
    match value {
        Value::SimpleString(value) | Value::VerbatimString { text: value, .. } => Some(value),
        Value::BulkString(value) => std::str::from_utf8(value).ok(),
        _ => None,
    }
}

fn value_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Int(value) => (*value).try_into().ok(),
        _ => value_text(value)?.parse().ok(),
    }
}

fn normalize_name(name: &str) -> String {
    name.replace('|', " ").to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Value {
        Value::BulkString(value.as_bytes().to_vec())
    }
    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Array(
            entries
                .into_iter()
                .flat_map(|(key, value)| [text(key), value])
                .collect(),
        )
    }

    #[test]
    fn parses_resp2_docs_with_nested_options() {
        let reply = map(vec![(
            "set",
            map(vec![
                ("summary", text("Set a value")),
                ("group", text("string")),
                ("since", text("1.0.0")),
                (
                    "arguments",
                    Value::Array(vec![
                        map(vec![
                            ("name", text("key")),
                            ("type", text("key")),
                            ("display", text("key-name")),
                            ("key_spec_index", Value::Int(0)),
                        ]),
                        map(vec![
                            ("name", text("condition")),
                            ("type", text("oneof")),
                            (
                                "flags",
                                Value::Array(vec![
                                    text("optional"),
                                    text("multiple"),
                                    text("multiple_token"),
                                ]),
                            ),
                            (
                                "arguments",
                                Value::Array(vec![map(vec![
                                    ("token", text("NX")),
                                    ("type", text("pure-token")),
                                ])]),
                            ),
                        ]),
                    ]),
                ),
            ]),
        )]);
        let catalog = RedisCommandCatalog::from_command_docs(&reply).unwrap();
        let command = &catalog.commands[0];
        assert_eq!(command.name, "SET");
        assert_eq!(command.arguments[0].key_spec_index, Some(0));
        assert_eq!(command.arguments[0].display.as_deref(), Some("key-name"));
        assert!(command.arguments[1].optional);
        assert!(command.arguments[1].multiple);
        assert!(command.arguments[1].multiple_token);
        assert_eq!(
            command.arguments[1].arguments[0].token.as_deref(),
            Some("NX")
        );
    }

    #[test]
    fn parses_resp3_docs_and_compound_subcommands() {
        let reply = Value::Map(vec![(
            text("acl"),
            Value::Map(vec![
                (text("summary"), text("ACL command")),
                (
                    text("subcommands"),
                    Value::Map(vec![(
                        text("acl|cat"),
                        Value::Map(vec![
                            (text("summary"), text("List ACL categories")),
                            (text("group"), text("server")),
                        ]),
                    )]),
                ),
            ]),
        )]);
        let catalog = RedisCommandCatalog::from_command_docs(&reply).unwrap();
        assert_eq!(
            catalog
                .commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["ACL", "ACL CAT"]
        );
    }

    #[test]
    fn command_metadata_fallback_keeps_commands_and_subcommands() {
        let reply = Value::Array(vec![
            Value::Array(vec![
                text("get"),
                Value::Int(2),
                Value::Array(vec![]),
                Value::Int(1),
                Value::Int(1),
                Value::Int(1),
            ]),
            Value::Array(vec![
                text("acl"),
                Value::Int(-2),
                Value::Array(vec![]),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Array(vec![text("@slow")]),
                Value::Array(vec![]),
                Value::Array(vec![Value::Array(vec![
                    text("begin_search"),
                    Value::Array(vec![
                        text("type"),
                        text("index"),
                        text("spec"),
                        Value::Array(vec![text("index"), Value::Int(1)]),
                    ]),
                ])]),
                Value::Array(vec![Value::Array(vec![
                    text("acl|cat"),
                    Value::Int(-2),
                    Value::Array(vec![]),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                ])]),
            ]),
        ]);
        let catalog = RedisCommandCatalog::from_command_metadata(&reply).unwrap();
        assert_eq!(
            catalog
                .commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["ACL", "ACL CAT", "GET"]
        );
    }

    #[test]
    fn malformed_deep_argument_metadata_is_bounded() {
        let mut nested = map(vec![("name", text("leaf"))]);
        for _ in 0..(MAX_METADATA_DEPTH + 2) {
            nested = map(vec![
                ("name", text("option")),
                ("arguments", Value::Array(vec![nested])),
            ]);
        }
        let reply = map(vec![(
            "module.command",
            map(vec![("arguments", Value::Array(vec![nested]))]),
        )]);
        let catalog = RedisCommandCatalog::from_command_docs(&reply).unwrap();
        assert!(catalog.commands[0].arguments.len() <= MAX_ARGUMENTS_PER_LEVEL);
    }
}
