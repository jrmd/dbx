//! Keep labels, values and accessible actions on the library's original select
//! node. The retained UI patch supplies list names and a live-state toggle.

use gpui::{App, Entity, IntoElement, RenderOnce, SharedString, Window, prelude::*};
use gpui_component::select::{SearchableVec, Select, SelectState};

type TextSelect = Select<SearchableVec<SharedString>>;
type TextSelectState = Entity<SelectState<SearchableVec<SharedString>>>;

pub trait SelectAccessibility {
    fn accessible_select(
        self,
        name: impl Into<SharedString>,
        state: &TextSelectState,
    ) -> NamedSelect;
}

impl SelectAccessibility for TextSelect {
    fn accessible_select(
        self,
        name: impl Into<SharedString>,
        state: &TextSelectState,
    ) -> NamedSelect {
        NamedSelect {
            control: self,
            name: name.into(),
            state: state.clone(),
        }
    }
}

#[derive(IntoElement)]
pub struct NamedSelect {
    control: TextSelect,
    name: SharedString,
    state: TextSelectState,
}

impl RenderOnce for NamedSelect {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let prepared = self
            .control
            .accessibility_label(self.name.clone())
            .render(window, cx)
            .into_any_element();
        SelectElement {
            prepared,
            name: self.name,
            state: self.state,
        }
    }
}

/// The pinned styled select returns a stateless ViewElement<BaseSelect> whose
/// child is the original StatefulDiv. Add metadata after layout preparation and
/// before accessibility prepaint, without changing its identity or layout.
struct SelectElement {
    prepared: gpui::AnyElement,
    name: SharedString,
    state: TextSelectState,
}
impl SelectElement {
    fn original(&mut self) -> &mut gpui::ViewElement<gpui_base::Select> {
        self.prepared
            .downcast_mut()
            .expect("pinned Select wrapper changed; update the accessibility adapter")
    }
}
impl IntoElement for SelectElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl gpui::Element for SelectElement {
    type RequestLayoutState = Option<gpui::AnyElement>;
    type PrepaintState = Option<gpui::AnyElement>;
    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let (layout, mut root) = self.original().request_layout(id, inspector, window, cx);
        let root_element = root.as_mut().expect("pinned Select root was not rendered");
        let element = root_element
            .downcast_mut::<gpui::Stateful<gpui::Div>>()
            .expect("pinned Select root changed; update the accessibility adapter");
        let value = self.state.read(cx).selected_value().cloned();
        let state = self.state.clone();
        let original = std::mem::replace(element, gpui::div().id("unused-select-root"));
        *element = original
            .aria_label(self.name.clone())
            .aria_description(value.as_ref().map_or_else(
                || "No selection".to_owned(),
                |value| format!("Selected: {value}"),
            ))
            .a11y_synthetic_children(move |builder| {
                if let Some(value) = &value {
                    builder.parent_node().set_value(value.as_ref());
                }
            })
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                state.update(cx, |state, cx| state.toggle_menu_accessibly(window, cx));
            });
        // Keep the original prepared root and layout state; only metadata changed.
        (layout, root)
    }
    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.original()
            .prepaint(id, inspector, bounds, layout, window, cx)
    }
    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.original()
            .paint(id, inspector, bounds, layout, state, window, cx);
    }
}
