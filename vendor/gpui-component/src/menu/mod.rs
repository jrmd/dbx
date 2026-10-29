use gpui::App;

mod app_menu_bar;
mod context_menu;
mod dropdown_menu;
mod menu_item;
mod native_context;
mod popup_menu;

pub use app_menu_bar::AppMenuBar;
pub use context_menu::{ContextMenu, ContextMenuExt, ContextMenuState};
pub use dropdown_menu::DropdownMenu;
pub use native_context::{
    complete_native_context_menu, install_native_context_presenter, present_native_context_menu,
};
pub use popup_menu::{PopupMenu, PopupMenuItem};

pub(crate) fn init(cx: &mut App) {
    app_menu_bar::init(cx);
    popup_menu::init(cx);
}
