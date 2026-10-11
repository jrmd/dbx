// DBX local extension of AccessKit's AT-SPI adapter.
// Licensed under MIT OR Apache-2.0, like the surrounding adapter.

use crate::atspi::{ObjectId, OwnedObjectAddress};
use accesskit_atspi_common::PlatformNode;
use zbus::{fdo, interface, names::OwnedUniqueName};

pub(crate) struct TableInterface {
    bus_name: OwnedUniqueName,
    node: PlatformNode,
}

impl TableInterface {
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

#[interface(name = "org.a11y.atspi.Table")]
impl TableInterface {
    #[zbus(property)]
    fn n_rows(&self) -> fdo::Result<i32> {
        self.node
            .table_dimensions()
            .map(|(rows, _)| rows)
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn n_columns(&self) -> fdo::Result<i32> {
        self.node
            .table_dimensions()
            .map(|(_, columns)| columns)
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn n_selected_rows(&self) -> fdo::Result<i32> {
        self.node
            .table_selected_axes(true)
            .map(|rows| rows.len() as i32)
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn n_selected_columns(&self) -> fdo::Result<i32> {
        self.node
            .table_selected_axes(false)
            .map(|columns| columns.len() as i32)
            .map_err(self.map_error())
    }
    #[zbus(property)]
    fn caption(&self) -> fdo::Result<OwnedObjectAddress> {
        self.node.table_dimensions().map_err(self.map_error())?;
        Ok(self.address(None).0)
    }
    #[zbus(property)]
    fn summary(&self) -> fdo::Result<OwnedObjectAddress> {
        self.node.table_dimensions().map_err(self.map_error())?;
        Ok(self.address(None).0)
    }
    fn get_accessible_at(&self, row: i32, column: i32) -> fdo::Result<(OwnedObjectAddress,)> {
        self.node
            .table_cell_at(row, column)
            .map(|cell| self.address(cell))
            .map_err(self.map_error())
    }
    fn get_index_at(&self, row: i32, column: i32) -> fdo::Result<i32> {
        self.node
            .table_index_at(row, column)
            .map_err(self.map_error())
    }
    fn get_row_at_index(&self, index: i32) -> fdo::Result<i32> {
        self.node
            .table_cell_at_index(index)
            .map(|cell| cell.map(|(info, _)| info.row).unwrap_or(-1))
            .map_err(self.map_error())
    }
    fn get_column_at_index(&self, index: i32) -> fdo::Result<i32> {
        self.node
            .table_cell_at_index(index)
            .map(|cell| cell.map(|(info, _)| info.column).unwrap_or(-1))
            .map_err(self.map_error())
    }
    fn get_row_column_extents_at_index(
        &self,
        index: i32,
    ) -> fdo::Result<(bool, i32, i32, i32, i32, bool)> {
        self.node
            .table_cell_at_index(index)
            .map(|cell| {
                cell.map(|(info, selected)| {
                    (
                        true,
                        info.row,
                        info.column,
                        info.row_span,
                        info.column_span,
                        selected,
                    )
                })
                .unwrap_or((false, -1, -1, 0, 0, false))
            })
            .map_err(self.map_error())
    }
    fn get_row_extent_at(&self, row: i32, column: i32) -> fdo::Result<i32> {
        self.node
            .table_cell_extent_at(row, column, true)
            .map_err(self.map_error())
    }
    fn get_column_extent_at(&self, row: i32, column: i32) -> fdo::Result<i32> {
        self.node
            .table_cell_extent_at(row, column, false)
            .map_err(self.map_error())
    }
    fn get_row_header(&self, row: i32) -> fdo::Result<(OwnedObjectAddress,)> {
        self.node
            .table_header(row, true)
            .map(|id| self.address(id))
            .map_err(self.map_error())
    }
    fn get_column_header(&self, column: i32) -> fdo::Result<(OwnedObjectAddress,)> {
        self.node
            .table_header(column, false)
            .map(|id| self.address(id))
            .map_err(self.map_error())
    }
    fn get_row_description(&self, row: i32) -> fdo::Result<String> {
        self.node
            .table_description(row, true)
            .map_err(self.map_error())
    }
    fn get_column_description(&self, column: i32) -> fdo::Result<String> {
        self.node
            .table_description(column, false)
            .map_err(self.map_error())
    }
    fn is_selected(&self, row: i32, column: i32) -> fdo::Result<bool> {
        self.node
            .table_is_selected(row, column)
            .map_err(self.map_error())
    }
    fn is_row_selected(&self, row: i32) -> fdo::Result<bool> {
        self.node
            .table_is_axis_selected(row, true)
            .map_err(self.map_error())
    }
    fn is_column_selected(&self, column: i32) -> fdo::Result<bool> {
        self.node
            .table_is_axis_selected(column, false)
            .map_err(self.map_error())
    }
    fn get_selected_rows(&self) -> fdo::Result<Vec<i32>> {
        self.node
            .table_selected_axes(true)
            .map_err(self.map_error())
    }
    fn get_selected_columns(&self) -> fdo::Result<Vec<i32>> {
        self.node
            .table_selected_axes(false)
            .map_err(self.map_error())
    }
    fn add_row_selection(&self, row: i32) -> fdo::Result<bool> {
        self.node
            .table_change_axis_selection(row, true)
            .map_err(self.map_error())
    }
    fn remove_row_selection(&self, row: i32) -> fdo::Result<bool> {
        self.node
            .table_change_axis_selection(row, true)
            .map_err(self.map_error())
    }
    fn add_column_selection(&self, column: i32) -> fdo::Result<bool> {
        self.node
            .table_change_axis_selection(column, false)
            .map_err(self.map_error())
    }
    fn remove_column_selection(&self, column: i32) -> fdo::Result<bool> {
        self.node
            .table_change_axis_selection(column, false)
            .map_err(self.map_error())
    }
}
