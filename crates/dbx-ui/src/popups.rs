//! App-owned dropdowns keep the component's menu lifecycle and focus behavior,
//! adding a scroll barrier behind every open popup. Keep this adapter aligned
//! with gpui-component's DropdownMenuPopover when upgrading the dependency.

use std::rc::Rc;

use gpui::{
    Anchor, Context, DismissEvent, ElementId, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, RenderOnce, SharedString, StyleRefinement, Styled, Window,
    prelude::FluentBuilder,
};

use gpui_component::{Selectable, button::Button, menu::PopupMenu, popover::Popover};

type MenuBuilder = dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu;

/// A dropdown menu trait for buttons.
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover;
}

impl DropdownMenu for Button {
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover {
        let style = self.style().clone();
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f).trigger_style(style)
    }
}

#[derive(IntoElement)]
pub struct DropdownMenuPopover {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    trigger: Button,
    builder: Rc<MenuBuilder>,
}

impl DropdownMenuPopover {
    fn new(
        id: ElementId,
        anchor: impl Into<Anchor>,
        trigger: Button,
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

impl RenderOnce for DropdownMenuPopover {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let builder = self.builder.clone();
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        let popover_id = SharedString::from(format!("popover:{}", self.id));
        let popover = Popover::new(popover_id.clone())
            .appearance(false)
            .overlay_closable(false)
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
                                    menu_state.update(cx, |state, cx| {
                                        state.menu = None;
                                        cx.notify();
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                }
            });
        ScrollBlockingPopover::new(popover_id, popover).trigger(self.trigger)
    }
}

/// Host the barrier outside the popup's deferred content, so it paints before
/// the popup surface and leaves the popup's own scroll regions interactive.
#[derive(IntoElement)]
pub(crate) struct ScrollBlockingPopover {
    id: ElementId,
    popover: Popover,
    trigger: Option<Button>,
    trap_focus: bool,
}

struct PopoverInteraction {
    open: bool,
    return_focus: Option<FocusHandle>,
    focus: FocusHandle,
}

impl ScrollBlockingPopover {
    pub(crate) fn new(id: impl Into<ElementId>, popover: Popover) -> Self {
        Self {
            id: id.into(),
            popover,
            trigger: None,
            trap_focus: false,
        }
    }

    pub(crate) fn trigger(mut self, trigger: Button) -> Self {
        self.trigger = Some(trigger);
        self
    }

    pub(crate) fn trap_focus(mut self) -> Self {
        self.trap_focus = true;
        self
    }
}

impl RenderOnce for ScrollBlockingPopover {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let state = window.use_keyed_state((self.id.clone(), "interaction"), cx, |_, cx| {
            PopoverInteraction {
                open: false,
                return_focus: None,
                focus: cx.focus_handle(),
            }
        });
        let open = state.read(cx).open;
        let trap_focus = self.trap_focus;
        let focus = state.read(cx).focus.clone();
        let popover = self
            .popover
            .when_some(self.trigger, |popover, trigger| {
                let state = state.clone();
                // BasePopover opens on mouse-down. Keyboard activation emits a
                // click, so use its controlled-open API and retain trigger focus.
                popover.trigger(
                    trigger
                        .when(open && trap_focus, |trigger| trigger.tab_stop(false))
                        .on_click(move |event, window, cx| {
                            if event.is_keyboard() {
                                cx.stop_propagation();
                                state.update(cx, |state, cx| {
                                    state.open = true;
                                    state.return_focus = window.focused(cx);
                                    state.focus.focus(window, cx);
                                    cx.notify();
                                });
                            }
                        }),
                )
            })
            .open(open)
            .on_open_change({
                let state = state.clone();
                move |open, window, cx| {
                    state.update(cx, |state, cx| {
                        state.open = *open;
                        if !open && let Some(focus) = state.return_focus.take() {
                            focus.focus(window, cx);
                        }
                        cx.notify();
                    });
                }
            });
        gpui::div()
            .id((self.id, "keyboard-popover"))
            .track_focus(&focus)
            .on_action({
                let state = state.clone();
                move |_: &crate::app::FocusNext, window, cx| {
                    if trap_focus && state.read(cx).open {
                        move_popover_focus(&state.read(cx).focus.clone(), true, window, cx);
                    } else {
                        cx.propagate();
                    }
                }
            })
            .on_action({
                let state = state.clone();
                move |_: &crate::app::FocusPrevious, window, cx| {
                    if trap_focus && state.read(cx).open {
                        move_popover_focus(&state.read(cx).focus.clone(), false, window, cx);
                    } else {
                        cx.propagate();
                    }
                }
            })
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key != "escape"
                    || !focus.is_focused(window)
                    || !state.read(cx).open
                {
                    return;
                }
                cx.stop_propagation();
                state.update(cx, |state, cx| {
                    state.open = false;
                    if let Some(focus) = state.return_focus.take() {
                        focus.focus(window, cx);
                    }
                    cx.notify();
                });
            })
            .when(open, |view| view.child(scroll_shield(window, 99)))
            .child(popover)
    }
}

fn move_popover_focus(focus: &FocusHandle, forward: bool, window: &mut Window, cx: &mut gpui::App) {
    for _ in 0..2048 {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
        if focus.contains_focused(window, cx) && !focus.is_focused(window) {
            return;
        }
    }
    focus.focus(window, cx);
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
    use gpui::{
        AppContext, Render, ScrollHandle, StatefulInteractiveElement, TestAppContext, div, point,
        px,
    };
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

    struct KeyboardMenuHarness {
        builds: Rc<std::cell::Cell<usize>>,
    }

    impl Render for KeyboardMenuHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let builds = self.builds.clone();
            div().child(Button::new("keyboard-menu").label("Options").dropdown_menu(
                move |menu, _, _| {
                    builds.set(builds.get() + 1);
                    menu.item(PopupMenuItem::new("JSON"))
                },
            ))
        }
    }

    #[gpui::test]
    fn keyboard_opens_dropdown_and_escape_restores_trigger_focus(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let builds = Rc::new(std::cell::Cell::new(0));
        let (_, cx) = cx.add_window_view(|_, _| KeyboardMenuHarness {
            builds: builds.clone(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let trigger = cx.update(|window, cx| window.focused(cx)).unwrap();
        for key in ["enter", "space"] {
            let before = builds.get();
            cx.simulate_keystrokes(key);
            cx.simulate_event(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse(key).unwrap(),
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(
                builds.get(),
                before + 1,
                "{key} must open the focused dropdown"
            );
            assert_ne!(
                cx.update(|window, cx| window.focused(cx)),
                Some(trigger.clone())
            );
            cx.simulate_keystrokes("escape");
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(
                cx.update(|window, cx| window.focused(cx)),
                Some(trigger.clone())
            );
        }
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

            cx.update(|window, cx| window.focus_next(cx));
            cx.simulate_keystrokes("enter");
            cx.simulate_event(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("enter").unwrap(),
            });
            let before = cx.update(|_, cx| view.read(cx).scroll.offset());
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
            let popover = Popover::new("scrollable-popover").child(
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
                .trigger(Button::new("trigger").label("Open"))
        }
    }

    #[gpui::test]
    fn keyboard_opens_custom_popover_and_escape_restores_focus(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (_, cx) = cx.add_window_view(|_, _| ScrollablePopover {
            scroll: ScrollHandle::new(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let trigger = cx.update(|window, cx| window.focused(cx));
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-scroll").is_some());
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-scroll").is_none());
        assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger);
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

    struct CustomPopoverHarness;
    impl Render for CustomPopoverHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .on_action(|_: &crate::app::FocusNext, window, cx| window.focus_next(cx))
                .on_action(|_: &crate::app::FocusPrevious, window, cx| window.focus_prev(cx))
                .child(Button::new("before").label("Before"))
                .child(
                    ScrollBlockingPopover::new(
                        "custom-keyboard-popup",
                        Popover::new("custom-keyboard-popup").child(
                            gpui_component::checkbox::Checkbox::new("choice").label("Choice"),
                        ),
                    )
                    .trigger(Button::new("custom-trigger").label("Open"))
                    .trap_focus(),
                )
                .child(Button::new("after").label("After"))
        }
    }

    #[gpui::test]
    fn tab_enters_custom_popover_controls_and_wraps_inside(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(|cx| cx.bind_keys(crate::app::focus_key_bindings()));
        let (_, cx) = cx.add_window_view(|_, _| CustomPopoverHarness);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let before = cx.update(|window, cx| window.focused(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let trigger = cx.update(|window, cx| window.focused(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let after = cx.update(|window, cx| window.focused(cx));
        cx.update(|window, cx| trigger.as_ref().unwrap().focus(window, cx));
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for key in ["tab", "tab", "shift-tab"] {
            cx.simulate_keystrokes(key);
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let focused = cx.update(|window, cx| window.focused(cx));
            assert_ne!(
                focused, before,
                "Tab must reach popup content, not restart the app"
            );
            assert_ne!(focused, after);
            assert_ne!(focused, trigger);
        }
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger);
    }

    struct SelectHarness {
        select: Entity<
            gpui_component::select::SelectState<
                gpui_component::select::SearchableVec<SharedString>,
            >,
        >,
    }
    impl Render for SelectHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui_component::select::Select::new(&self.select).w(px(200.))
        }
    }

    #[gpui::test]
    fn select_opens_on_enter_and_returns_focus_after_escape_and_confirmation(
        cx: &mut TestAppContext,
    ) {
        use gpui_component::{
            IndexPath,
            select::{SearchableVec, SelectState},
        };
        cx.update(gpui_component::init);
        let (view, cx) = cx.add_window_view(|window, cx| SelectHarness {
            select: cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(vec![
                        SharedString::from("First"),
                        SharedString::from("Second"),
                    ]),
                    Some(IndexPath::new(0)),
                    window,
                    cx,
                )
            }),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let trigger = cx.update(|window, cx| window.focused(cx));
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_ne!(cx.update(|window, cx| window.focused(cx)), trigger);
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger);
        cx.simulate_keystrokes("enter down enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger);
        assert_eq!(
            view.read_with(cx, |view, cx| view
                .select
                .read(cx)
                .selected_value()
                .cloned()),
            Some("Second".into())
        );
    }
}
