//! User-visible connector contracts, independent of marketing support counts.
use crate::DatabaseKind;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Capabilities {
    pub sql: bool,
    pub row_edits: bool,
    pub atomic_row_changes: bool,
    pub independent_query_sessions: bool,
    pub structured_filters: bool,
    pub streaming_query_export: bool,
    pub atomic_file_import: bool,
    pub native_backup: bool,
    pub ssh: bool,
    pub protected: bool,
}
impl DatabaseKind {
    pub const fn capabilities(self, protected: bool) -> Capabilities {
        let native = matches!(
            self,
            Self::PostgreSQL | Self::MySQL | Self::SQLite | Self::CockroachDB
        );
        let atomic = native || matches!(self, Self::SqlServer);
        Capabilities {
            sql: self.is_sql(),
            row_edits: self.supports_row_mutations() && !protected,
            atomic_row_changes: atomic && !protected,
            independent_query_sessions: native || matches!(self, Self::SqlServer),
            structured_filters: self.is_sql(),
            streaming_query_export: native,
            atomic_file_import: native && !protected,
            native_backup: matches!(self, Self::PostgreSQL | Self::MySQL),
            ssh: self.supports_transport(),
            protected,
        }
    }
}
impl Capabilities {
    pub fn summary(self) -> String {
        let edits = if self.protected {
            "writes protected"
        } else if self.row_edits {
            if self.atomic_row_changes {
                "atomic staged edits"
            } else {
                "staged edits with partial-success reporting"
            }
        } else {
            "grid is read-only"
        };
        let sessions = if self.independent_query_sessions {
            "independent query sessions"
        } else {
            "no tab-owned interactive transactions"
        };
        format!(
            "{edits} · {sessions} · {}",
            if self.streaming_query_export {
                "full query export"
            } else {
                "loaded-result export"
            }
        )
    }
}
