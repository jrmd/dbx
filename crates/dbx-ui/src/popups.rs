//! App-owned dropdowns keep the component's menu lifecycle and focus behavior,
//! adding a scroll barrier behind every open popup. Keep this adapter aligned
//! with gpui-component's DropdownMenuPopover when upgrading the dependency.

use std::rc::Rc;

use gpui::{
    Anchor, Context, DismissEvent, ElementId, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, prelude::FluentBuilder,
};

use gpui_component::{
    ActiveTheme as _, Selectable, ThemeStyled as _, button::Button, menu::PopupMenu,
    popover::Popover,
};

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

/// Metrics shared by every context menu. `PopupMenu` draws the result-grid and
/// toolbar menus; app-owned menus that need their own keyboard model (the
/// explorer's) must wear the same surface and rows so the two stay identical.
const MENU_ITEM_HEIGHT: f32 = 26.;

/// The menu panel: popover fill and shadow, hairline edge, glass radius, 4px
/// padding and 2px between rows. Keep this in step with the surface
/// `PopupMenu` draws in gpui-component (`menu/popup_menu.rs`), so the
/// explorer's menu and every `context_menu` render identically.
pub(crate) fn menu_surface<T: Styled>(panel: T, cx: &gpui::App) -> T {
    panel
        .menu_style(cx)
        .flex()
        .flex_col()
        .p_1()
        .gap_y_0p5()
        .min_w(gpui::rems(8.))
        .max_w(gpui::px(500.))
}

/// One menu row. `highlighted` marks the keyboard cursor; hover is added for
/// enabled rows. `highlight` overrides the accent fill (and keeps the row's
/// own text colour), for rows with a warning or danger tone.
pub(crate) fn menu_item<T: Styled + StatefulInteractiveElement + FluentBuilder>(
    row: T,
    enabled: bool,
    highlighted: bool,
    highlight: Option<gpui::Rgba>,
    cx: &gpui::App,
) -> T {
    let theme = cx.theme();
    let background = highlight.map_or_else(
        || {
            if theme.native_glass_menus {
                theme.primary.alpha(0.22)
            } else {
                *theme.tokens.accent
            }
        },
        gpui::Hsla::from,
    );
    let foreground = if highlight.is_some() {
        None
    } else {
        Some(if theme.native_glass_menus {
            theme.popover_foreground
        } else {
            theme.accent_foreground
        })
    };
    row.h(gpui::px(if theme.native_glass_menus {
        28.
    } else {
        MENU_ITEM_HEIGHT
    }))
    .flex()
    .items_center()
    .px_2()
    .text_sm()
    .rounded(if theme.native_glass_menus {
        gpui::px(10.)
    } else {
        theme.radius_lg.min(gpui::px(8.))
    })
    .text_color(if enabled {
        theme.popover_foreground
    } else {
        theme.muted_foreground
    })
    .when(highlighted && enabled, |row| {
        row.bg(background)
            .when_some(foreground, |row, color| row.text_color(color))
    })
    .when(enabled, |row| {
        row.cursor_pointer().hover(move |style| {
            let style = style.bg(background);
            match foreground {
                Some(color) => style.text_color(color),
                None => style,
            }
        })
    })
}

/// A horizontal rule between menu groups, matching `PopupMenuItem::Separator`.
pub(crate) fn menu_separator(cx: &gpui::App) -> gpui::Div {
    gpui::div()
        .my_0p5()
        .when(!cx.theme().native_glass_menus, |line| line.mx_neg_1())
        .border_b(gpui::px(if cx.theme().native_glass_menus {
            1.
        } else {
            2.
        }))
        .border_color(cx.theme().border)
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
