use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub const ICON_DATABASE: &str = "icons/database.svg";
pub const ICON_TABLE: &str = "icons/table.svg";
pub const ICON_QUERY: &str = "icons/query.svg";
pub const ICON_STRUCTURE: &str = "icons/structure.svg";
pub const ICON_DIAGRAM: &str = "icons/diagram.svg";
pub const ICON_SEARCH: &str = "icons/search.svg";
pub const ICON_REFRESH: &str = "icons/refresh.svg";
pub const ICON_SETTINGS: &str = "icons/settings.svg";
pub const ICON_SUN: &str = "icons/sun.svg";
pub const ICON_MOON: &str = "icons/moon.svg";
pub const ICON_ADD: &str = "icons/add.svg";
pub const ICON_CLOSE: &str = "icons/close.svg";
pub const ICON_MORE: &str = "icons/more.svg";
pub const ICON_ARROW_RIGHT: &str = "icons/arrow-right.svg";
pub const ICON_MINIMIZE: &str = "icons/minimize.svg";
pub const ICON_MAXIMIZE: &str = "icons/maximize.svg";
pub const ICON_RESTORE: &str = "icons/restore.svg";
pub const ICON_SIDEBAR: &str = "icons/sidebar.svg";
pub const ICON_APPEARANCE: &str = "icons/appearance.svg";
pub const ICON_LOCK: &str = "icons/lock.svg";
pub const ICON_SPARKLES: &str = "icons/sparkles.svg";
pub const ICON_TRASH: &str = "icons/trash.svg";
pub const ICON_PENCIL: &str = "icons/pencil.svg";
pub const ICON_TAG: &str = "icons/tag.svg";
pub const ICON_DOWNLOAD: &str = "icons/download.svg";
pub const ICON_CHEVRON_UP: &str = "icons/chevron-up.svg";
pub const ICON_CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub const LOGO_POSTGRESQL: &str = "icons/postgresql.svg";
pub const LOGO_MYSQL: &str = "icons/mysql.svg";
pub const LOGO_SQLITE: &str = "icons/sqlite.svg";
pub const LOGO_REDIS: &str = "icons/redis.svg";
pub const LOGO_MONGODB: &str = "icons/mongodb.svg";
pub const LOGO_COCKROACHDB: &str = "icons/cockroachdb.svg";
pub const LOGO_DUCKDB: &str = "icons/duckdb.svg";
pub const LOGO_ELASTICSEARCH: &str = "icons/elasticsearch.svg";
pub const LOGO_BIGQUERY: &str = "icons/bigquery.svg";
pub const LOGO_KAFKA: &str = "icons/kafka.svg";
pub const LOGO_TURSO: &str = "icons/turso.svg";
pub const LOGO_CLOUDFLARE_D1: &str = "icons/cloudflare-d1.svg";
pub const LOGO_CLICKHOUSE: &str = "icons/clickhouse.svg";
pub const LOGO_SNOWFLAKE: &str = "icons/snowflake.svg";
pub const LOGO_SQLSERVER: &str = "icons/sqlserver.svg";
pub const LOGO: &str = "logo.svg";
pub const LOGO_BYTES: &[u8] = include_bytes!("../../../logo.svg");

/// Compile-time UI assets, so packaged binaries never rely on the current
/// working directory to render their icons.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let asset = match path {
            ICON_DATABASE => include_bytes!("../assets/icons/database.svg").as_slice(),
            ICON_TABLE => include_bytes!("../assets/icons/table.svg").as_slice(),
            ICON_QUERY => include_bytes!("../assets/icons/query.svg").as_slice(),
            ICON_STRUCTURE => include_bytes!("../assets/icons/structure.svg").as_slice(),
            ICON_DIAGRAM => include_bytes!("../assets/icons/diagram.svg").as_slice(),
            ICON_SEARCH => include_bytes!("../assets/icons/search.svg").as_slice(),
            ICON_REFRESH => include_bytes!("../assets/icons/refresh.svg").as_slice(),
            ICON_SETTINGS => include_bytes!("../assets/icons/settings.svg").as_slice(),
            ICON_SUN => include_bytes!("../assets/icons/sun.svg").as_slice(),
            ICON_MOON => include_bytes!("../assets/icons/moon.svg").as_slice(),
            ICON_ADD => include_bytes!("../assets/icons/add.svg").as_slice(),
            ICON_CLOSE => include_bytes!("../assets/icons/close.svg").as_slice(),
            ICON_MORE => include_bytes!("../assets/icons/more.svg").as_slice(),
            ICON_ARROW_RIGHT => include_bytes!("../assets/icons/arrow-right.svg").as_slice(),
            ICON_MINIMIZE => include_bytes!("../assets/icons/minimize.svg").as_slice(),
            ICON_MAXIMIZE => include_bytes!("../assets/icons/maximize.svg").as_slice(),
            ICON_RESTORE => include_bytes!("../assets/icons/restore.svg").as_slice(),
            ICON_SIDEBAR => include_bytes!("../assets/icons/sidebar.svg").as_slice(),
            ICON_APPEARANCE => include_bytes!("../assets/icons/appearance.svg").as_slice(),
            ICON_LOCK => include_bytes!("../assets/icons/lock.svg").as_slice(),
            ICON_SPARKLES => include_bytes!("../assets/icons/sparkles.svg").as_slice(),
            ICON_TRASH => include_bytes!("../assets/icons/trash.svg").as_slice(),
            ICON_PENCIL => include_bytes!("../assets/icons/pencil.svg").as_slice(),
            ICON_TAG => include_bytes!("../assets/icons/tag.svg").as_slice(),
            ICON_DOWNLOAD => include_bytes!("../assets/icons/download.svg").as_slice(),
            ICON_CHEVRON_UP => include_bytes!("../assets/icons/chevron-up.svg").as_slice(),
            ICON_CHEVRON_DOWN => include_bytes!("../assets/icons/chevron-down.svg").as_slice(),
            LOGO_POSTGRESQL => include_bytes!("../assets/icons/postgresql.svg").as_slice(),
            LOGO_MYSQL => include_bytes!("../assets/icons/mysql.svg").as_slice(),
            LOGO_SQLITE => include_bytes!("../assets/icons/sqlite.svg").as_slice(),
            LOGO_REDIS => include_bytes!("../assets/icons/redis.svg").as_slice(),
            LOGO_MONGODB => include_bytes!("../assets/icons/mongodb.svg").as_slice(),
            LOGO_COCKROACHDB => include_bytes!("../assets/icons/cockroachdb.svg").as_slice(),
            LOGO_DUCKDB => include_bytes!("../assets/icons/duckdb.svg").as_slice(),
            LOGO_ELASTICSEARCH => include_bytes!("../assets/icons/elasticsearch.svg").as_slice(),
            LOGO_BIGQUERY => include_bytes!("../assets/icons/bigquery.svg").as_slice(),
            LOGO_KAFKA => include_bytes!("../assets/icons/kafka.svg").as_slice(),
            LOGO_TURSO => include_bytes!("../assets/icons/turso.svg").as_slice(),
            LOGO_CLOUDFLARE_D1 => include_bytes!("../assets/icons/cloudflare-d1.svg").as_slice(),
            LOGO_CLICKHOUSE => include_bytes!("../assets/icons/clickhouse.svg").as_slice(),
            LOGO_SNOWFLAKE => include_bytes!("../assets/icons/snowflake.svg").as_slice(),
            LOGO_SQLSERVER => include_bytes!("../assets/icons/sqlserver.svg").as_slice(),
            LOGO => LOGO_BYTES,
            // gpui-component draws its own glyphs (menu checkmarks, select
            // carets, dialog closes) from its bundled icon set.
            _ => return gpui_component_assets::Assets.load(path),
        };

        Ok(Some(Cow::Borrowed(asset)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path.is_empty() {
            Ok(vec![SharedString::from("icons"), SharedString::from(LOGO)])
        } else if path == "icons" {
            Ok([
                "database.svg",
                "table.svg",
                "query.svg",
                "structure.svg",
                "diagram.svg",
                "search.svg",
                "refresh.svg",
                "settings.svg",
                "sun.svg",
                "moon.svg",
                "add.svg",
                "close.svg",
                "more.svg",
                "arrow-right.svg",
                "chevron-up.svg",
                "chevron-down.svg",
                "minimize.svg",
                "maximize.svg",
                "restore.svg",
                "sidebar.svg",
                "appearance.svg",
                "lock.svg",
                "postgresql.svg",
                "mysql.svg",
                "sqlite.svg",
                "redis.svg",
                "mongodb.svg",
                "cockroachdb.svg",
                "duckdb.svg",
                "elasticsearch.svg",
                "bigquery.svg",
                "kafka.svg",
                "turso.svg",
                "cloudflare-d1.svg",
                "clickhouse.svg",
                "sqlserver.svg",
            ]
            .into_iter()
            .map(SharedString::from)
            .collect())
        } else {
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    use super::{Assets, ICON_DIAGRAM, ICON_MOON, ICON_SUN, LOGO, LOGO_BYTES};

    #[test]
    fn loads_and_lists_the_embedded_logo() {
        let assets = Assets;

        assert!(assets.load(LOGO).unwrap().is_some());
        assert!(
            assets
                .list("")
                .unwrap()
                .iter()
                .any(|asset| asset.as_ref() == LOGO)
        );
        assert!(assets.load(ICON_SUN).unwrap().is_some());
        assert!(assets.load(ICON_MOON).unwrap().is_some());
        assert!(assets.load(ICON_DIAGRAM).unwrap().is_some());
        assert!(
            assets
                .list("icons")
                .unwrap()
                .iter()
                .any(|asset| asset.as_ref() == "sun.svg")
        );
        assert!(
            assets
                .list("icons")
                .unwrap()
                .iter()
                .any(|asset| asset.as_ref() == "diagram.svg")
        );
        assert!(
            LOGO_BYTES
                .windows(b"<path".len())
                .any(|window| window == b"<path")
        );
        assert!(
            !LOGO_BYTES
                .windows(b"<image".len())
                .any(|window| window == b"<image")
        );
    }
}
