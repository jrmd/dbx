//! Reviewable schema alterations; generation never executes a statement.
use crate::sql::quote_table;
use crate::{DatabaseKind, DbxError, Result, TableRef, quote_identifier};

#[derive(Clone, Debug)]
pub enum TableAlteration {
    AddColumn {
        name: String,
        data_type: String,
        nullable: bool,
        default: Option<String>,
    },
    RenameColumn {
        name: String,
        new_name: String,
    },
    DropColumn {
        name: String,
    },
    AddIndex {
        name: String,
        columns: Vec<String>,
        unique: bool,
    },
    DropIndex {
        name: String,
    },
}

pub fn draft_table_alteration(
    kind: DatabaseKind,
    table: &TableRef,
    change: &TableAlteration,
) -> Result<String> {
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "table designer".into(),
            kind,
        });
    }
    let qualified = quote_table(kind, table)?;
    let identifier = |name: &str| quote_identifier(kind, name);
    let statement = match change {
        TableAlteration::AddColumn {
            name,
            data_type,
            nullable,
            default,
        } => {
            if data_type.trim().is_empty()
                || !data_type
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_(), []".contains(c))
            {
                return Err(DbxError::Parse(
                    "Enter a column type without SQL statements or comments".into(),
                ));
            }
            let default = default
                .as_ref()
                .map(|value| {
                    crate::transfer::safe_schema_expression(value)
                        .map(|value| format!(" DEFAULT {value}"))
                })
                .transpose()?
                .unwrap_or_default();
            format!(
                "ALTER TABLE {qualified} ADD {}{} {}{}{}",
                if kind == DatabaseKind::SqlServer {
                    ""
                } else {
                    "COLUMN "
                },
                identifier(name)?,
                data_type.trim(),
                if *nullable { "" } else { " NOT NULL" },
                default
            )
        }
        TableAlteration::RenameColumn { name, new_name } if kind == DatabaseKind::SqlServer => {
            identifier(new_name)?;
            let object = format!("{qualified}.{}", identifier(name)?);
            format!(
                "EXEC sp_rename N'{}', N'{}', 'COLUMN'",
                object.replace('\'', "''"),
                new_name.replace('\'', "''")
            )
        }
        TableAlteration::RenameColumn { name, new_name } => format!(
            "ALTER TABLE {qualified} RENAME COLUMN {} TO {}",
            identifier(name)?,
            identifier(new_name)?
        ),
        TableAlteration::DropColumn { name } => {
            format!("ALTER TABLE {qualified} DROP COLUMN {}", identifier(name)?)
        }
        TableAlteration::AddIndex {
            name,
            columns,
            unique,
        } => {
            if columns.is_empty() {
                return Err(DbxError::Parse("Choose at least one index column".into()));
            }
            if matches!(
                kind,
                DatabaseKind::BigQuery | DatabaseKind::ClickHouse | DatabaseKind::Snowflake
            ) {
                return Err(DbxError::Unsupported {
                    operation: "ordinary B-tree indexes; use engine-specific index SQL".into(),
                    kind,
                });
            }
            format!(
                "CREATE {}INDEX {} ON {qualified} ({})",
                if *unique { "UNIQUE " } else { "" },
                identifier(name)?,
                columns
                    .iter()
                    .map(|column| identifier(column))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ")
            )
        }
        TableAlteration::DropIndex { name } => {
            if matches!(kind, DatabaseKind::MySQL | DatabaseKind::SqlServer) {
                format!("DROP INDEX {} ON {qualified}", identifier(name)?)
            } else {
                format!(
                    "DROP INDEX {}",
                    quote_table(
                        kind,
                        &TableRef {
                            schema: table.schema.clone(),
                            name: name.clone()
                        }
                    )?
                )
            }
        }
    };
    Ok(format!(
        "-- Review this schema change before running. Back up affected data.\n{statement};\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alterations_quote_names_and_reject_expression_injection() {
        let table = TableRef::new("items");
        let change = TableAlteration::AddColumn {
            name: "display name".into(),
            data_type: "varchar(40)".into(),
            nullable: false,
            default: Some("'a;b'".into()),
        };
        assert!(
            draft_table_alteration(DatabaseKind::SQLite, &table, &change)
                .unwrap()
                .contains("ADD COLUMN \"display name\" varchar(40) NOT NULL DEFAULT 'a;b'")
        );
        let bad = TableAlteration::AddColumn {
            name: "x".into(),
            data_type: "text; DROP TABLE items".into(),
            nullable: true,
            default: None,
        };
        assert!(draft_table_alteration(DatabaseKind::SQLite, &table, &bad).is_err());
        let index = TableAlteration::DropIndex { name: "idx".into() };
        assert!(
            draft_table_alteration(DatabaseKind::MySQL, &table, &index)
                .unwrap()
                .contains("DROP INDEX `idx` ON `items`")
        );
    }
}
