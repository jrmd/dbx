// DBX local extension of AccessKit's AT-SPI adapter.
// Licensed under MIT OR Apache-2.0, like the surrounding adapter.

use crate::atspi::{ObjectId, OwnedObjectAddress};
use accesskit_atspi_common::{NodeCacheInfo, NodeIdOrRoot, PlatformRoot};
use atspi::{Interface, InterfaceSet, Role, StateSet};
use serde::Serialize;
use zbus::{fdo, interface, names::OwnedUniqueName, object_server::SignalEmitter, zvariant::Type};

pub(crate) const CACHE_PATH: &str = "/org/a11y/atspi/cache";

#[derive(Serialize, Type)]
pub(crate) struct CacheItem {
    object: OwnedObjectAddress,
    app: OwnedObjectAddress,
    parent: OwnedObjectAddress,
    index: i32,
    children: i32,
    interfaces: InterfaceSet,
    name: String,
    role: Role,
    description: String,
    states: StateSet,
}

impl CacheItem {
    pub(crate) fn node(bus_name: &OwnedUniqueName, info: NodeCacheInfo) -> Self {
        let parent = match info.parent {
            NodeIdOrRoot::Root => ObjectId::Root,
            NodeIdOrRoot::Node(node) => ObjectId::Node {
                adapter: info.adapter,
                node,
            },
        };
        Self {
            object: ObjectId::Node {
                adapter: info.adapter,
                node: info.id,
            }
            .to_address(bus_name.inner()),
            app: ObjectId::Root.to_address(bus_name.inner()),
            parent: parent.to_address(bus_name.inner()),
            index: info.index,
            children: info.children,
            interfaces: info.interfaces,
            name: info.name,
            role: info.role,
            description: info.description,
            states: info.states,
        }
    }
}

pub(crate) struct CacheInterface {
    bus_name: OwnedUniqueName,
    root: PlatformRoot,
}
impl CacheInterface {
    pub(crate) fn new(bus_name: OwnedUniqueName, root: PlatformRoot) -> Self {
        Self { bus_name, root }
    }
}

#[interface(name = "org.a11y.atspi.Cache")]
impl CacheInterface {
    // Version 1 has GetItems/AddAccessible/RemoveAccessible. No asynchronous
    // initialization or version-2 Ready signal is advertised.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    fn get_items(&self) -> fdo::Result<Vec<CacheItem>> {
        let snapshot = self.root.cache_snapshot().map_err(super::map_root_error)?;
        let mut result = Vec::with_capacity(snapshot.nodes.len() + 1);
        result.push(CacheItem {
            object: ObjectId::Root.to_address(self.bus_name.inner()),
            app: ObjectId::Root.to_address(self.bus_name.inner()),
            parent: OwnedObjectAddress::null(),
            index: -1,
            children: snapshot.windows,
            interfaces: InterfaceSet::new(Interface::Accessible | Interface::Application),
            name: snapshot.name,
            role: Role::Application,
            description: String::new(),
            states: StateSet::empty(),
        });
        result.extend(
            snapshot
                .nodes
                .into_iter()
                .map(|node| CacheItem::node(&self.bus_name, node)),
        );
        Ok(result)
    }

    #[zbus(signal)]
    pub(crate) async fn add_accessible(
        emitter: &SignalEmitter<'_>,
        item: &CacheItem,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    pub(crate) async fn remove_accessible(
        emitter: &SignalEmitter<'_>,
        object: &OwnedObjectAddress,
    ) -> zbus::Result<()>;
}
