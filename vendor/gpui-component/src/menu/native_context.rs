use super::PopupMenu;
use gpui::{App, Entity, Global, Pixels, Point, Window};
use serde_json::Value;
use std::collections::HashMap;

/// Application-provided native context-menu presenter. Returning false leaves
/// the existing GPUI menu in place (unsupported platform or material).
pub type NativeContextPresenter = fn(u64, &Value, Point<Pixels>, &mut Window, &mut App) -> bool;
struct NativeContextMenus {
    presenter: NativeContextPresenter,
    next_id: u64,
    pending: HashMap<u64, Entity<PopupMenu>>,
}
impl Global for NativeContextMenus {}

pub fn install_native_context_presenter(presenter: NativeContextPresenter, cx: &mut App) {
    cx.set_global(NativeContextMenus {
        presenter,
        next_id: 0,
        pending: HashMap::new(),
    });
}

pub fn present_native_context_menu(
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(adapter) = cx.try_global::<NativeContextMenus>() else {
        return false;
    };
    let presenter = adapter.presenter;
    let Some(items) = menu.read(cx).native_items(cx) else {
        return false;
    };
    let id = {
        let adapter = cx.global_mut::<NativeContextMenus>();
        adapter.next_id += 1;
        adapter.next_id
    };
    if !presenter(id, &items, position, window, cx) {
        return false;
    }
    cx.global_mut::<NativeContextMenus>()
        .pending
        .insert(id, menu);
    true
}

/// Called after the native menu closes; an empty path means cancellation.
pub fn complete_native_context_menu(id: u64, path: &[usize], window: &mut Window, cx: &mut App) {
    let Some(adapter) = cx.try_global::<NativeContextMenus>() else {
        return;
    };
    if !adapter.pending.contains_key(&id) {
        return;
    }
    let menu = cx
        .global_mut::<NativeContextMenus>()
        .pending
        .remove(&id)
        .unwrap();
    PopupMenu::activate_native(menu, path, window, cx);
}
