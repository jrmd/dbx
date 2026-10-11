// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

mod accessible;
mod action;
mod application;
mod cache;
mod component;
mod hyperlink;
mod selection;
mod table;
mod table_cell;
mod text;
mod value;

use crate::atspi::{ObjectId, OwnedObjectAddress};
use zbus::{fdo, names::UniqueName};

fn map_root_error(error: accesskit_atspi_common::Error) -> fdo::Error {
    crate::util::map_error(ObjectId::Root, error)
}

fn optional_object_address(
    bus_name: &UniqueName,
    object_id: Option<ObjectId>,
) -> (OwnedObjectAddress,) {
    match object_id {
        Some(id) => (id.to_address(bus_name),),
        None => (OwnedObjectAddress::null(),),
    }
}

pub(crate) use accessible::*;
pub(crate) use action::*;
pub(crate) use application::*;
pub(crate) use cache::*;
pub(crate) use component::*;
pub(crate) use hyperlink::*;
pub(crate) use selection::*;
pub(crate) use table::*;
pub(crate) use table_cell::*;
pub(crate) use text::*;
pub(crate) use value::*;

#[cfg(test)]
mod table_protocol_tests {
    use super::*;
    use accesskit::{ActionHandler, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
    use accesskit_atspi_common::{
        Adapter, AdapterCallback, AppContext, Event, InterfaceSet, PlatformNode, WindowBounds,
    };
    use zbus::object_server::Interface;

    struct NoActions;
    impl ActionHandler for NoActions {
        fn do_action(&mut self, _: ActionRequest) {}
    }
    struct NoCallbacks;
    impl AdapterCallback for NoCallbacks {
        fn register_interfaces(
            &self,
            _: &Adapter,
            _: accesskit_atspi_common::NodeId,
            _: InterfaceSet,
        ) {
        }
        fn unregister_interfaces(
            &self,
            _: &Adapter,
            _: accesskit_atspi_common::NodeId,
            _: InterfaceSet,
        ) {
        }
        fn emit_event(&self, _: &Adapter, _: Event) {}
    }

    fn fixture() -> (Adapter, PlatformNode) {
        let adapter = Adapter::new(
            &AppContext::new(None),
            NoCallbacks,
            TreeUpdate {
                nodes: vec![(NodeId(0), Node::new(Role::Grid))],
                tree: Some(Tree::new(NodeId(0))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
            WindowBounds::default(),
            NoActions,
        );
        let node = adapter.platform_node(adapter.root_id());
        (adapter, node)
    }

    fn xml(interface: &impl Interface) -> String {
        let mut xml = String::new();
        interface.introspect_to_writer(&mut xml, 0);
        xml
    }

    fn property(xml: &str, name: &str, signature: &str) {
        assert!(
            xml.lines().any(|line| line.contains("<property ")
                && line.contains(&format!("name=\"{name}\""))
                && line.contains(&format!("type=\"{signature}\""))),
            "Unexpected property schema for {name}: {xml}"
        );
    }

    fn method_outputs(xml: &str, method: &str) -> Vec<String> {
        let start = xml.find(&format!("<method name=\"{method}\"")).unwrap();
        let body = &xml[start..];
        let body = &body[..body.find("</method>").unwrap()];
        body.lines()
            .filter(|line| line.contains("direction=\"out\""))
            .map(|line| {
                line.split("type=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn cache_schema_uses_bulk_metadata_and_native_object_reference_signatures() {
        let (adapter, _) = fixture();
        let cache = CacheInterface::new(":1.42".try_into().unwrap(), adapter.platform_root());
        let xml = xml(&cache);
        property(&xml, "version", "u");
        assert_eq!(
            method_outputs(&xml, "GetItems"),
            ["a((so)(so)(so)iiassusau)"]
        );
        let add = &xml[xml.find("<signal name=\"AddAccessible\"").unwrap()..];
        assert!(add[..add.find("</signal>").unwrap()].contains("type=\"((so)(so)(so)iiassusau)\""));
        let remove = &xml[xml.find("<signal name=\"RemoveAccessible\"").unwrap()..];
        assert!(remove[..remove.find("</signal>").unwrap()].contains("type=\"(so)\""));
    }

    #[test]
    fn table_schema_matches_native_object_reference_and_scalar_contracts() {
        let (_adapter, node) = fixture();
        let table = TableInterface::new(":1.42".try_into().unwrap(), node);
        let xml = xml(&table);
        for name in ["Caption", "Summary"] {
            property(&xml, name, "(so)");
        }
        for name in ["NRows", "NColumns", "NSelectedRows", "NSelectedColumns"] {
            property(&xml, name, "i");
        }
        for name in ["GetAccessibleAt", "GetRowHeader", "GetColumnHeader"] {
            assert_eq!(method_outputs(&xml, name), ["(so)"]);
        }
        assert_eq!(
            method_outputs(&xml, "GetRowColumnExtentsAtIndex"),
            ["b", "i", "i", "i", "i", "b"]
        );
    }

    #[test]
    fn table_cell_schema_has_single_containing_table_struct_not_nested_wrapper() {
        let (_adapter, node) = fixture();
        let cell = TableCellInterface::new(":1.42".try_into().unwrap(), node);
        let xml = xml(&cell);
        // A ((so)) Table property causes libatspi's libdbus decoder to abort.
        property(&xml, "Table", "(so)");
        property(&xml, "Position", "(ii)");
        property(&xml, "RowSpan", "i");
        property(&xml, "ColumnSpan", "i");
        for name in ["GetRowHeaderCells", "GetColumnHeaderCells"] {
            assert_eq!(method_outputs(&xml, name), ["a(so)"]);
        }
        assert_eq!(
            method_outputs(&xml, "GetRowColumnSpan"),
            ["i", "i", "i", "i"]
        );
    }
}
