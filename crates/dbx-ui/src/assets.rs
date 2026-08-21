use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub const ICON_DATABASE: &str = "icons/database.svg";
pub const ICON_TABLE: &str = "icons/table.svg";
pub const ICON_QUERY: &str = "icons/query.svg";
pub const ICON_STRUCTURE: &str = "icons/structure.svg";
pub const ICON_SEARCH: &str = "icons/search.svg";
pub const ICON_REFRESH: &str = "icons/refresh.svg";
pub const ICON_SETTINGS: &str = "icons/settings.svg";
pub const ICON_ADD: &str = "icons/add.svg";
pub const ICON_CLOSE: &str = "icons/close.svg";
pub const ICON_MORE: &str = "icons/more.svg";
pub const ICON_ARROW_RIGHT: &str = "icons/arrow-right.svg";
pub const LOGO_POSTGRESQL: &str = "icons/postgresql.svg";
pub const LOGO_MYSQL: &str = "icons/mysql.svg";
pub const LOGO_SQLITE: &str = "icons/sqlite.svg";
pub const LOGO_REDIS: &str = "icons/redis.svg";
pub const LOGO: &str = "logo.png";
pub const LOGO_BYTES: &[u8] = include_bytes!("../assets/logo.png");

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
            ICON_SEARCH => include_bytes!("../assets/icons/search.svg").as_slice(),
            ICON_REFRESH => include_bytes!("../assets/icons/refresh.svg").as_slice(),
            ICON_SETTINGS => include_bytes!("../assets/icons/settings.svg").as_slice(),
            ICON_ADD => include_bytes!("../assets/icons/add.svg").as_slice(),
            ICON_CLOSE => include_bytes!("../assets/icons/close.svg").as_slice(),
            ICON_MORE => include_bytes!("../assets/icons/more.svg").as_slice(),
            ICON_ARROW_RIGHT => include_bytes!("../assets/icons/arrow-right.svg").as_slice(),
            LOGO_POSTGRESQL => include_bytes!("../assets/icons/postgresql.svg").as_slice(),
            LOGO_MYSQL => include_bytes!("../assets/icons/mysql.svg").as_slice(),
            LOGO_SQLITE => include_bytes!("../assets/icons/sqlite.svg").as_slice(),
            LOGO_REDIS => include_bytes!("../assets/icons/redis.svg").as_slice(),
            LOGO => LOGO_BYTES,
            _ => return Ok(None),
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
                "search.svg",
                "refresh.svg",
                "settings.svg",
                "add.svg",
                "close.svg",
                "more.svg",
                "arrow-right.svg",
                "postgresql.svg",
                "mysql.svg",
                "sqlite.svg",
                "redis.svg",
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

    use super::{Assets, LOGO};

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
    }
}
