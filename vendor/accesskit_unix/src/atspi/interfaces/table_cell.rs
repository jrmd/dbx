// DBX local extension of AccessKit's AT-SPI adapter.
// Licensed under MIT OR Apache-2.0, like the surrounding adapter.

use crate::atspi::{ObjectId, OwnedObjectAddress};
use accesskit_atspi_common::PlatformNode;
use zbus::{fdo, interface, names::OwnedUniqueName};

pub(crate) struct TableCellInterface {
    bus_name: OwnedUniqueName,
    node: PlatformNode,
}

impl TableCellInterface {
    pub fn new(bus_name: OwnedUniqueName, node: PlatformNode) -> Self {
        Self { bus_name, node }
    }
    fn map_error(&self) -> impl '_ + FnOnce(accesskit_atspi_common::Error) -> fdo::Error {
        |error| crate::util::map_error_from_node(&self.node, error)
    }
    fn address(&self, id: Option<accesskit_atspi_common::NodeId>) -> (OwnedObjectAddress,) {
        super::optional_object_address(
            &self.bus_name,
            id.map(|node| ObjectId::Node {
                adapter: self.node.adapter_id(),
                node,
            }),
        )
    }
}

#[interface(name = "org.a11y.atspi.TableCell")]
impl TableCellInterface {
    #[zbus(property)]
    fn position(&self) -> fdo::Result<(i32, i32)> {
        self.node
            .table_cell_info()
            .map(|info| (info.row, info.column))
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn row_span(&self) -> fdo::Result<i32> {
        self.node
            .table_cell_info()
            .map(|info| info.row_span)
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn column_span(&self) -> fdo::Result<i32> {
        self.node
            .table_cell_info()
            .map(|info| info.column_span)
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn table(&self) -> fdo::Result<OwnedObjectAddress> {
        self.node
            .table_cell_table()
            .map(|id| self.address(Some(id)).0)
            .map_err(self.map_error())
    }
    // GNOME libatspi and the ATK bridge use four integers here. The upstream
    // XML/generated Rust proxy still includes an incompatible leading boolean.
    fn get_row_column_span(&self) -> fdo::Result<(i32, i32, i32, i32)> {
        self.node
            .table_cell_info()
            .map(|info| (info.row, info.column, info.row_span, info.column_span))
            .map_err(self.map_error())
    }
    fn get_row_header_cells(&self) -> fdo::Result<Vec<OwnedObjectAddress>> {
        self.node
            .table_cell_headers(true)
            .map(|headers| {
                headers
                    .into_iter()
                    .map(|id| self.address(Some(id)).0)
                    .collect()
            })
            .map_err(self.map_error())
    }
    fn get_column_header_cells(&self) -> fdo::Result<Vec<OwnedObjectAddress>> {
        self.node
            .table_cell_headers(false)
            .map(|headers| {
                headers
                    .into_iter()
                    .map(|id| self.address(Some(id)).0)
                    .collect()
            })
            .map_err(self.map_error())
    }
}
