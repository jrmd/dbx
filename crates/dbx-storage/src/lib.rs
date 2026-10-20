//! Local persistence that has no GPUI dependency: connection profiles, the
//! encrypted vault, query history, saved SQL files and workspace drafts.
pub mod binary_values;
pub mod config;
pub mod connection_fields;
pub mod device_unlock;
pub mod profile_transfer;
pub mod profiles;
pub mod query_history;
pub mod query_library;
pub mod vault;
pub mod workspace;
