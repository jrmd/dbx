//! Attach semantics to a rendered component without adding another focus
//! handle or layout container. In particular, GPUI's component focus must
//! remain on the same element that supplies the accessibility node.

mod select;
pub use select::SelectAccessibility;

use gpui::{
    A11ySubtreeBuilder, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Stateful, Window,
};

pub struct SemanticElement<E: Element> {
    element: E,
    metadata: Stateful<Div>,
}

pub fn with_semantics<E: IntoElement>(
    element: E,
    metadata: Stateful<Div>,
) -> SemanticElement<E::Element> {
    SemanticElement {
        element: element.into_element(),
        metadata,
    }
}

impl<E: Element> IntoElement for SemanticElement<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<E: Element> Element for SemanticElement<E> {
    type RequestLayoutState = (
        E::RequestLayoutState,
        <Stateful<Div> as Element>::RequestLayoutState,
    );
    type PrepaintState = (E::PrepaintState, <Stateful<Div> as Element>::PrepaintState);

    fn id(&self) -> Option<ElementId> {
        self.element.id().or_else(|| self.metadata.id())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.element.source_location()
    }

    fn a11y_role(&self) -> Option<gpui::Role> {
        self.metadata
            .a11y_role()
            .or_else(|| self.element.a11y_role())
    }

    fn write_a11y_info(&self, node: &mut gpui::accesskit::Node) {
        self.element.write_a11y_info(node);
        self.metadata.write_a11y_info(node);
    }

    fn a11y_synthetic_children(
        &mut self,
        state: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        self.element.a11y_synthetic_children(&mut state.0, builder);
        self.metadata.a11y_synthetic_children(&mut state.1, builder);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (layout, state) = self.element.request_layout(id, inspector, window, cx);
        let (_, metadata) = self.metadata.request_layout(id, inspector, window, cx);
        (layout, (state, metadata))
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        (
            self.element
                .prepaint(id, inspector, bounds, &mut state.0, window, cx),
            self.metadata
                .prepaint(id, inspector, bounds, &mut state.1, window, cx),
        )
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.paint(
            id,
            inspector,
            bounds,
            &mut layout.0,
            &mut state.0,
            window,
            cx,
        );
        self.metadata.paint(
            id,
            inspector,
            bounds,
            &mut layout.1,
            &mut state.1,
            window,
            cx,
        );
    }
}

/// Text that changes asynchronously should have a stable identity so that
/// assistive technology announces meaningful changes rather than new nodes.
pub fn announcement(
    id: impl Into<ElementId>,
    message: impl Into<gpui::SharedString>,
    error: bool,
) -> Stateful<Div> {
    use gpui::prelude::*;
    gpui::div()
        .id(id)
        .role(if error {
            gpui::Role::Alert
        } else {
            gpui::Role::Status
        })
        .aria_label(message)
        .a11y_synthetic_children(move |builder| {
            let node = builder.parent_node();
            node.set_live(if error {
                gpui::accesskit::Live::Assertive
            } else {
                gpui::accesskit::Live::Polite
            });
            node.set_live_atomic();
        })
}

/// FocusTrapContainer does not forward Element's accessibility hooks in the
/// pinned component release. Attach them to that same focus node.
pub trait DialogElement: IntoElement + Sized {
    fn accessible_dialog(
        self,
        id: impl Into<ElementId>,
        title: impl Into<gpui::SharedString>,
        description: impl Into<gpui::SharedString>,
        hidden: bool,
        alert: bool,
    ) -> SemanticElement<Self::Element> {
        use gpui::prelude::*;
        with_semantics(
            self,
            gpui::div()
                .id(id)
                .role(if alert {
                    gpui::Role::AlertDialog
                } else {
                    gpui::Role::Dialog
                })
                .aria_label(title)
                .aria_description(description)
                .a11y_synthetic_children(move |builder| {
                    let node = builder.parent_node();
                    node.set_modal();
                    if hidden {
                        node.set_hidden();
                    }
                }),
        )
    }
}
impl<T: IntoElement> DialogElement for T {}

struct FocusRevealState {
    focus: gpui::FocusHandle,
    was_focused: std::rc::Rc<std::cell::Cell<bool>>,
    last_scale: std::rc::Rc<std::cell::Cell<f32>>,
}

#[derive(gpui::IntoElement)]
pub struct FocusReveal {
    id: ElementId,
    scroll: gpui::ScrollHandle,
    child: gpui::AnyElement,
}

pub fn focus_reveal(
    id: impl Into<ElementId>,
    scroll: &gpui::ScrollHandle,
    child: impl IntoElement,
) -> FocusReveal {
    FocusReveal {
        id: id.into(),
        scroll: scroll.clone(),
        child: child.into_any_element(),
    }
}

impl gpui::RenderOnce for FocusReveal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui::prelude::*;
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            FocusRevealState {
                focus: cx.focus_handle(),
                was_focused: Default::default(),
                last_scale: Default::default(),
            } // Never a Tab stop.
        });
        let state = state.read(cx);
        let focus = state.focus.clone();
        let was_focused = state.was_focused.clone();
        let last_scale = state.last_scale.clone();
        let marker = focus.clone();
        gpui::div()
            .on_children_prepainted(move |bounds, window, cx| {
                let focused = marker.contains_focused(window, cx);
                let previous = was_focused.replace(focused);
                let entered = focused && !previous;
                let scale = crate::theme::ui_scale();
                let resized = last_scale.replace(scale) != scale;
                // Reveal focus changes and resizing, while allowing users to scroll
                // past a control that remains focused.
                if !focused || (!entered && !resized) {
                    return;
                }
                let Some(row) = bounds.first() else {
                    return;
                };
                let viewport = self.scroll.bounds();
                let delta = if row.size.height > viewport.size.height {
                    viewport.top() - row.top()
                } else if row.bottom() > viewport.bottom() {
                    viewport.bottom() - row.bottom()
                } else if row.top() < viewport.top() {
                    viewport.top() - row.top()
                } else {
                    gpui::px(0.)
                };
                if f32::from(delta).abs() < 0.5 {
                    return;
                }
                let scroll = self.scroll.clone();
                let offset = scroll.offset() + gpui::point(gpui::px(0.), delta);
                // Apply outside the ancestor's prepaint borrow, then redraw with
                // the new offset. Already-visible rows never move on mouse focus.
                window.on_next_frame(move |window, _| {
                    scroll.set_offset(offset);
                    window.refresh();
                });
            })
            .id(self.id)
            .track_focus(&focus)
            .child(self.child)
    }
}
