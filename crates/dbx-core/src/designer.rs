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
    AlterColumn {
        name: String,
        data_type: String,
        nullable: bool,
        default: Option<String>,
    },
    AddPrimaryKey {
        name: String,
        columns: Vec<String>,
    },
    AddForeignKey {
        name: String,
        columns: Vec<String>,
        referenced_table: TableRef,
        referenced_columns: Vec<String>,
    },
    AddCheck {
        name: String,
        expression: String,
    },
    DropConstraint {
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
            validate_type(data_type)?;
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
        TableAlteration::AlterColumn {
            name,
            data_type,
            nullable,
            default,
        } => {
            validate_type(data_type)?;
            let column = identifier(name)?;
            let default = default
                .as_deref()
                .map(crate::transfer::safe_schema_expression)
                .transpose()?;
            match kind.dialect() {
                DatabaseKind::PostgreSQL => format!(
                    "ALTER TABLE {qualified} ALTER COLUMN {column} TYPE {};\nALTER TABLE {qualified} ALTER COLUMN {column} {};\nALTER TABLE {qualified} ALTER COLUMN {column} {}",
                    data_type.trim(), if *nullable { "DROP NOT NULL" } else { "SET NOT NULL" },
                    default.map(|value| format!("SET DEFAULT {value}")).unwrap_or_else(|| "DROP DEFAULT".into())),
                DatabaseKind::MySQL => format!("ALTER TABLE {qualified} MODIFY COLUMN {column} {} {}{}", data_type.trim(),
                    if *nullable { "NULL" } else { "NOT NULL" }, default.map(|value| format!(" DEFAULT {value}")).unwrap_or_default()),
                DatabaseKind::SqlServer if default.is_none() => format!("ALTER TABLE {qualified} ALTER COLUMN {column} {} {}", data_type.trim(), if *nullable { "NULL" } else { "NOT NULL" }),
                _ => return Err(DbxError::Unsupported { operation: "alter column definition; this engine requires a reviewed table rebuild or named default-constraint SQL".into(), kind }),
            }
        }
        TableAlteration::AddPrimaryKey { name, columns } => {
            constraint_engine(kind)?;
            format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} PRIMARY KEY ({})",
                identifier(name)?,
                quoted_columns(kind, columns)?
            )
        }
        TableAlteration::AddForeignKey {
            name,
            columns,
            referenced_table,
            referenced_columns,
        } => {
            constraint_engine(kind)?;
            if columns.len() != referenced_columns.len() {
                return Err(DbxError::Parse(
                    "Foreign-key column counts must match".into(),
                ));
            }
            format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
                identifier(name)?,
                quoted_columns(kind, columns)?,
                quote_table(kind, referenced_table)?,
                quoted_columns(kind, referenced_columns)?
            )
        }
        TableAlteration::AddCheck { name, expression } => {
            constraint_engine(kind)?;
            let expression = crate::transfer::safe_schema_expression(expression)?;
            if expression.trim().is_empty() {
                return Err(DbxError::Parse("Enter a check expression".into()));
            }
            format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} CHECK ({expression})",
                identifier(name)?
            )
        }
        TableAlteration::DropConstraint { name } => {
            constraint_engine(kind)?;
            if kind == DatabaseKind::MySQL && name.eq_ignore_ascii_case("PRIMARY") {
                format!("ALTER TABLE {qualified} DROP PRIMARY KEY")
            } else {
                format!(
                    "ALTER TABLE {qualified} DROP CONSTRAINT {}",
                    identifier(name)?
                )
            }
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

/// Preserve attributes that MySQL's complete MODIFY definition would otherwise
/// erase. Generated/unknown extra attributes require explicit manual SQL.
pub async fn draft_table_alteration_for(
    engine: &crate::DatabaseEngine,
    table: &TableRef,
    change: &TableAlteration,
) -> Result<String> {
    let mut sql = draft_table_alteration(engine.kind(), table, change)?;
    let TableAlteration::AlterColumn { name, .. } = change else {
        return Ok(sql);
    };
    if engine.kind() != DatabaseKind::MySQL {
        return Ok(sql);
    }
    let metadata = engine
        .query_statement(
            &crate::SqlStatement::new(
                "SELECT COLUMN_NAME AS Field, EXTRA AS Extra, COLLATION_NAME AS Collation, CAST(COLUMN_COMMENT AS CHAR) AS Comment FROM information_schema.columns WHERE TABLE_SCHEMA = COALESCE(?, DATABASE()) AND TABLE_NAME = ?",
                vec![table.schema.clone().map(crate::CellValue::Text).unwrap_or(crate::CellValue::Null), crate::CellValue::Text(table.name.clone())],
            ),
            crate::QueryOptions::default(),
        )
        .await?;
    let position = |name: &str| {
        metadata
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                DbxError::Query(
                    "MySQL column attributes are unavailable; use reviewed manual SQL".into(),
                )
            })
    };
    let field = position("Field")?;
    let extra = position("Extra")?;
    let collation = position("Collation")?;
    let comment = position("Comment")?;
    let row = metadata
        .rows
        .iter()
        .find(|row| row.values[field].to_string() == *name)
        .ok_or_else(|| DbxError::Query("Column no longer exists; refresh Structure".into()))?;
    let extra = row.values[extra].to_string();
    let extra = extra.trim();
    let mut attributes = String::new();
    let mut remaining = extra
        .to_lowercase()
        .replace("default_generated", "")
        .trim()
        .to_owned();
    if remaining.contains("auto_increment") {
        attributes.push_str(" AUTO_INCREMENT");
        remaining = remaining.replace("auto_increment", "").trim().to_owned();
    }
    if let Some(expression) = remaining.strip_prefix("on update ") {
        attributes.push_str(&format!(
            " ON UPDATE {}",
            crate::transfer::safe_schema_expression(expression)?
        ));
        remaining.clear();
    }
    if !remaining.is_empty() {
        return Err(DbxError::Query("Generated or engine-specific column attributes require reviewed manual SQL; the designer will not drop them".into()));
    }
    if !matches!(row.values[collation], crate::CellValue::Null) {
        attributes.push_str(&format!(
            " COLLATE {}",
            quote_identifier(DatabaseKind::MySQL, &row.values[collation].to_string())?
        ));
    }
    let comment = row.values[comment].to_string();
    if !comment.is_empty() {
        let mode = engine
            .query(
                "SELECT @@sql_mode",
                crate::QueryOptions { max_rows: Some(1) },
            )
            .await?;
        let mode = mode
            .rows
            .first()
            .and_then(|row| row.values.first())
            .map(ToString::to_string)
            .unwrap_or_default();
        let comment = if mode.split(',').any(|flag| flag == "NO_BACKSLASH_ESCAPES") {
            comment
        } else {
            comment.replace('\\', "\\\\")
        };
        attributes.push_str(&format!(" COMMENT '{}'", comment.replace('\'', "''")));
    }
    let end = sql
        .rfind(';')
        .ok_or_else(|| DbxError::Parse("Missing alteration statement".into()))?;
    sql.insert_str(end, &attributes);
    Ok(sql)
}

fn validate_type(data_type: &str) -> Result<()> {
    if data_type.trim().is_empty()
        || !data_type
            .chars()
            .all(|c| c.is_alphanumeric() || "_(), []".contains(c))
    {
        return Err(DbxError::Parse(
            "Enter a column type without SQL statements or comments".into(),
        ));
    }
    Ok(())
}

fn constraint_engine(kind: DatabaseKind) -> Result<()> {
    if matches!(
        kind.dialect(),
        DatabaseKind::PostgreSQL | DatabaseKind::MySQL | DatabaseKind::SqlServer
    ) {
        Ok(())
    } else {
        Err(DbxError::Unsupported {
            operation: "ALTER TABLE constraints; use this engine’s table-rebuild or constraint SQL"
                .into(),
            kind,
        })
    }
}

fn quoted_columns(kind: DatabaseKind, columns: &[String]) -> Result<String> {
    if columns.is_empty() {
        return Err(DbxError::Parse("Choose at least one column".into()));
    }
    columns
        .iter()
        .map(|name| quote_identifier(kind, name))
        .collect::<Result<Vec<_>>>()
        .map(|items| items.join(", "))
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
