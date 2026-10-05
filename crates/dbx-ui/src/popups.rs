//! App-owned dropdowns keep the component's menu lifecycle and focus behavior,
//! adding a scroll barrier behind every open popup. Keep this adapter aligned
//! with gpui-component's DropdownMenuPopover when upgrading the dependency.

use std::rc::Rc;

use gpui::{
    Anchor, Context, DismissEvent, ElementId, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StyleRefinement, Styled, Window,
    prelude::FluentBuilder,
};

use gpui_component::{Selectable, button::Button, menu::PopupMenu, popover::Popover};

type MenuBuilder = dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu;

/// A dropdown menu trait for buttons and other interactive elements
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        let style = self.style().clone();
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f).trigger_style(style)
    }
}

impl DropdownMenu for Button {}

#[derive(IntoElement)]
pub struct DropdownMenuPopover<T: Selectable + IntoElement + 'static> {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    trigger: T,
    builder: Rc<MenuBuilder>,
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn new(
        id: ElementId,
        anchor: impl Into<Anchor>,
        trigger: T,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: SharedString::from(format!("dropdown-menu:{:?}", id)).into(),
            style: StyleRefinement::default(),
            anchor: anchor.into(),
            trigger,
            builder: Rc::new(builder),
        }
    }

    /// Set the style refinement for the dropdown menu trigger.
    fn trigger_style(mut self, style: StyleRefinement) -> Self {
        self.style = style;
        self
    }
}

#[derive(Default)]
struct DropdownMenuState {
    menu: Option<Entity<PopupMenu>>,
}

impl<T> RenderOnce for DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let builder = self.builder.clone();
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        let popover_id = SharedString::from(format!("popover:{}", self.id));
        let popover = Popover::new(popover_id.clone())
            .appearance(false)
            .overlay_closable(false)
            .trigger(self.trigger)
            .trigger_style(self.style)
            .anchor(self.anchor)
            .content(move |_, window, cx| {
                // Reuse the menu while open; rebuild after dismissal so disabled
                // states and callbacks reflect the current workspace.
                match menu_state.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let builder = builder.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window, cx);

                        // Listen for dismiss events from the PopupMenu to close the popover.
                        let popover_state = cx.entity();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.clone();
                                move |_, _: &DismissEvent, window, cx| {
                                    popover_state.update(cx, |state, cx| {
                                        state.dismiss(window, cx);
                                    });
                                    menu_state.update(cx, |state, _| {
                                        state.menu = None;
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                }
            });
        ScrollBlockingPopover::new(popover_id, popover)
    }
}

/// Host the barrier outside the popup's deferred content, so it paints before
/// the popup surface and leaves the popup's own scroll regions interactive.
#[derive(IntoElement)]
pub(crate) struct ScrollBlockingPopover {
    id: ElementId,
    popover: Popover,
}

impl ScrollBlockingPopover {
    pub(crate) fn new(id: impl Into<ElementId>, popover: Popover) -> Self {
        Self {
            id: id.into(),
            popover,
        }
    }
}

impl RenderOnce for ScrollBlockingPopover {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let state = window.use_keyed_state((self.id, "scroll-blocker"), cx, |_, _| false);
        let open = *state.read(cx);
        let popover = self.popover.on_open_change(move |open, _, cx| {
            state.update(cx, |state, cx| {
                *state = *open;
                cx.notify();
            });
        });
        gpui::div()
            .when(open, |view| view.child(scroll_shield(window, 99)))
            .child(popover)
    }
}

/// Below the popup surface, above the workspace. It catches wheel and trackpad
/// events anywhere in the window without affecting scrolling inside the popup.
pub(crate) fn scroll_shield(window: &Window, priority: usize) -> impl IntoElement {
    gpui::deferred(
        gpui::anchored().child(
            gpui::div()
                .w(window.bounds().size.width)
                .h(window.bounds().size.height)
                .occlude()
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation()),
        ),
    )
    .with_priority(priority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Render, ScrollHandle, StatefulInteractiveElement, TestAppContext, div, point, px};
    use gpui_component::menu::PopupMenuItem;

    struct Harness {
        scroll: ScrollHandle,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("background-table")
                        .w(px(500.))
                        .h(px(300.))
                        .overflow_scroll()
                        .track_scroll(&self.scroll)
                        .child(div().w(px(1000.)).h(px(1000.)).flex_none()),
                )
                .child(
                    Button::new("export-menu")
                        .label("Export")
                        .dropdown_menu(|menu, _, _| menu.item(PopupMenuItem::new("JSON"))),
                )
        }
    }

    fn wheel(cx: &mut gpui::VisualTestContext, delta: gpui::Point<gpui::Pixels>) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: point(px(100.), px(100.)),
            delta: gpui::ScrollDelta::Pixels(delta),
            ..Default::default()
        });
    }

    #[gpui::test]
    fn dropdown_blocks_background_scroll_and_restores_it_on_dismiss(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (view, cx) = cx.add_window_view(|_, _| Harness {
            scroll: ScrollHandle::new(),
        });
        for delta in [point(px(0.), px(-50.)), point(px(-50.), px(0.))] {
            let initial = cx.update(|_, cx| view.read(cx).scroll.offset());
            wheel(cx, delta);
            let before = cx.update(|_, cx| view.read(cx).scroll.offset());
            assert_ne!(
                before, initial,
                "the background must scroll before the popup opens"
            );
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.simulate_click(point(px(25.), px(315.)), Default::default());
            wheel(cx, delta);
            assert_eq!(cx.update(|_, cx| view.read(cx).scroll.offset()), before);
            cx.simulate_keystrokes("escape");
            wheel(cx, delta);
            assert_ne!(cx.update(|_, cx| view.read(cx).scroll.offset()), before);
        }
    }

    struct ScrollablePopover {
        scroll: ScrollHandle,
    }
    impl Render for ScrollablePopover {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let popover = Popover::new("scrollable-popover")
                .trigger(Button::new("trigger").label("Open"))
                .child(
                    div()
                        .id("popup-scroll")
                        .debug_selector(|| "popup-scroll".into())
                        .w(px(200.))
                        .h(px(200.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(div().h(px(1000.)).flex_none()),
                );
            ScrollBlockingPopover::new("scrollable-popover", popover)
        }
    }

    #[gpui::test]
    fn scroll_shield_allows_scrolling_inside_the_popover(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (view, cx) = cx.add_window_view(|_, _| ScrollablePopover {
            scroll: ScrollHandle::new(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(20.), px(10.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx
            .debug_bounds("popup-scroll")
            .expect("popover content must be visible");
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-50.))),
            ..Default::default()
        });
        assert!(cx.update(|_, cx| view.read(cx).scroll.offset().y) < px(0.));
    }
}
