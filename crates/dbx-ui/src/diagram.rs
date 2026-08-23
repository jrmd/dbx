//! A deterministic, exportable entity relationship diagram document.
//!
//! This module deliberately has no view state: callers load a
//! [`RelationalSchema`], retain the resulting [`DiagramDocument`], and use the
//! same SVG for both the on-screen image and file export.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};
use dbx_core::{ColumnInfo, ForeignKeyInfo, RelationalSchema, TableInfo};
use gpui::SvgRenderer;

const NODE_WIDTH: f32 = 292.0;
const HEADER_HEIGHT: f32 = 42.0;
const ROW_HEIGHT: f32 = 23.0;
const COLUMN_LIMIT: usize = 18;
const HORIZONTAL_GAP: f32 = 116.0;
const VERTICAL_GAP: f32 = 54.0;
const PADDING: f32 = 44.0;
const COMPONENT_SHELF_WIDTH: f32 = NODE_WIDTH * 3.0 + HORIZONTAL_GAP * 2.0;

/// Colours used by the diagram on screen and in exported images.
#[derive(Clone, Copy, Debug)]
pub struct DiagramPalette<'a> {
    pub canvas: &'a str,
    pub surface: &'a str,
    pub surface_muted: &'a str,
    pub border: &'a str,
    pub text: &'a str,
    pub muted_text: &'a str,
    pub accent: &'a str,
    pub relation: &'a str,
}

/// A positioned table card in a diagram.
#[derive(Clone, Debug)]
pub struct DiagramNode {
    pub id: String,
    pub table: TableInfo,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub columns: Vec<DiagramColumn>,
    pub omitted_columns: usize,
}

/// A visible column row.
#[derive(Clone, Debug)]
pub struct DiagramColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub foreign_key: bool,
}

/// A relationship rendered between two diagram cards.
#[derive(Clone, Debug)]
pub struct DiagramEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub source_columns: Vec<String>,
    pub target_columns: Vec<String>,
    pub path: String,
    pub self_referential: bool,
}

/// A complete layout ready for rendering or export.
#[derive(Clone, Debug)]
pub struct DiagramDocument {
    pub database: String,
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    pub width: f32,
    pub height: f32,
}

impl DiagramDocument {
    /// Builds a stable layout. Tables and constraints can arrive in any order;
    /// IDs, placement and SVG output are nevertheless deterministic.
    pub fn from_schema(schema: &RelationalSchema) -> Self {
        Self::from_schema_selection(schema, None)
    }

    /// Builds a stable layout for tables in the selected schemas.
    ///
    /// Passing `None` includes every table, while an empty set produces an
    /// empty document. Relationships are emitted only when both endpoints are
    /// included; the input schema and its foreign-key metadata are not changed.
    pub fn from_schema_selection(
        schema: &RelationalSchema,
        selected_schemas: Option<&BTreeSet<String>>,
    ) -> Self {
        let mut tables = schema
            .tables
            .iter()
            .filter(|entry| {
                selected_schemas.is_none_or(|selected| {
                    selected.contains(entry.table.schema.as_deref().unwrap_or_default())
                })
            })
            .collect::<Vec<_>>();
        tables.sort_by_key(|entry| table_id(&entry.table));

        let table_ids = tables
            .iter()
            .map(|entry| table_id(&entry.table))
            .collect::<HashSet<_>>();
        let foreign_columns = tables
            .iter()
            .map(|entry| {
                (
                    table_id(&entry.table),
                    entry
                        .structure
                        .foreign_keys
                        .iter()
                        .flat_map(|foreign_key| foreign_key.columns.iter().cloned())
                        .collect::<HashSet<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut relationship_columns = foreign_columns.clone();
        for entry in &tables {
            for foreign_key in &entry.structure.foreign_keys {
                let target = referenced_id(foreign_key, entry.table.schema.as_deref());
                if let Some(columns) = relationship_columns.get_mut(&target) {
                    columns.extend(foreign_key.referenced_columns.iter().cloned());
                }
            }
        }

        let node_columns = tables
            .iter()
            .map(|entry| {
                let id = table_id(&entry.table);
                let foreign = foreign_columns.get(&id).expect("foreign columns exist");
                let relationship = relationship_columns
                    .get(&id)
                    .expect("relationship columns exist");
                let mut columns = entry.structure.columns.clone();
                columns.sort_by_key(|column| column.ordinal);
                let visible = visible_columns(&columns, relationship, foreign);
                let omitted_columns = columns.len().saturating_sub(visible.len());
                (id, (visible, omitted_columns))
            })
            .collect::<HashMap<_, _>>();

        let levels = relationship_levels(&tables, &table_ids);
        let components = connected_components(&tables, &table_ids);
        let positions = position_tables(&node_columns, &levels, &components);
        let mut nodes = tables
            .iter()
            .map(|entry| {
                let id = table_id(&entry.table);
                let (columns, omitted_columns) = node_columns.get(&id).expect("node columns exist");
                let (x, y) = positions.get(&id).copied().expect("position exists");
                let rows = columns.len() + usize::from(*omitted_columns > 0);
                DiagramNode {
                    id,
                    table: entry.table.clone(),
                    x,
                    y,
                    width: NODE_WIDTH,
                    height: HEADER_HEIGHT + (rows as f32 * ROW_HEIGHT) + 10.0,
                    columns: columns.clone(),
                    omitted_columns: *omitted_columns,
                }
            })
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        let node_by_id = nodes
            .iter()
            .map(|node| (node.id.clone(), node))
            .collect::<HashMap<_, _>>();

        let mut edges = Vec::new();
        for entry in &tables {
            let source = table_id(&entry.table);
            for (index, foreign_key) in entry.structure.foreign_keys.iter().enumerate() {
                let target = referenced_id(foreign_key, entry.table.schema.as_deref());
                if !table_ids.contains(&target) {
                    continue;
                }
                let source_node = node_by_id[&source];
                let target_node = node_by_id[&target];
                let self_referential = source == target;
                let path = edge_path(source_node, target_node, foreign_key, self_referential);
                edges.push(DiagramEdge {
                    id: format!("{source}:{index}"),
                    source: source.clone(),
                    target,
                    source_columns: foreign_key.columns.clone(),
                    target_columns: foreign_key.referenced_columns.clone(),
                    path,
                    self_referential,
                });
            }
        }
        edges.sort_by(|left, right| left.id.cmp(&right.id));

        let width = nodes
            .iter()
            .map(|node| node.x + node.width + PADDING)
            .fold(PADDING * 2.0, f32::max);
        let height = nodes
            .iter()
            .map(|node| node.y + node.height + PADDING)
            .fold(PADDING * 2.0, f32::max);
        Self {
            database: schema.database.clone(),
            nodes,
            edges,
            width,
            height,
        }
    }

    /// Serializes the canonical vector representation. This is intentionally
    /// the single rendering source for the interactive surface and export.
    pub fn svg(&self, palette: DiagramPalette<'_>, selected: Option<&str>) -> String {
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {:.0} {:.0}" role="img" aria-label="Entity relationship diagram for {}"><defs><marker id="relation-arrow" markerWidth="8" markerHeight="8" refX="8" refY="4" viewBox="0 0 8 8" orient="auto"><path d="M 0 0 L 8 4 L 0 8 z" fill="{}"/></marker></defs><rect width="100%" height="100%" fill="{}"/>"#,
            self.width,
            self.height,
            self.width,
            self.height,
            escape(&self.database),
            palette.relation,
            palette.canvas
        );
        for edge in &self.edges {
            let column_pairs = edge
                .source_columns
                .iter()
                .zip(&edge.target_columns)
                .map(|(source, target)| format!("{} → {}", escape(source), escape(target)))
                .collect::<Vec<_>>()
                .join(", ");
            svg.push_str(&format!(r#"<path d="{}" fill="none" stroke="{}" stroke-width="1.5" stroke-linejoin="round" marker-end="url(#relation-arrow)" data-self-referential="{}"><title>{} → {} ({})</title></path>"#, edge.path, palette.relation, edge.self_referential, escape(&edge.source), escape(&edge.target), column_pairs));
        }
        for node in &self.nodes {
            let selected = selected == Some(node.id.as_str());
            let stroke = if selected {
                palette.accent
            } else {
                palette.border
            };
            let stroke_width = if selected { 2.0 } else { 1.0 };
            svg.push_str(&format!(r#"<g data-table="{}"><title>{}</title><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="8" fill="{}" stroke="{}" stroke-width="{}"/><path d="M {:.1} {:.1} h {:.1} v {:.1} h -{:.1} z" fill="{}"/><text x="{:.1}" y="{:.1}" fill="{}" font-family="system-ui, sans-serif" font-size="14" font-weight="600">{}</text><text x="{:.1}" y="{:.1}" fill="{}" font-family="system-ui, sans-serif" font-size="10">{}</text>"#,
                escape(&node.id), escape(&display_table(&node.table)), node.x, node.y, node.width, node.height, palette.surface, stroke, stroke_width,
                node.x, node.y, node.width, HEADER_HEIGHT, node.width, palette.surface_muted,
                node.x + 12.0, node.y + 18.0, palette.text, escape(&node.table.name),
                node.x + 12.0, node.y + 32.0, palette.muted_text, escape(node.table.schema.as_deref().unwrap_or("default"))
            ));
            for (index, column) in node.columns.iter().enumerate() {
                let y = node.y + HEADER_HEIGHT + 17.0 + (index as f32 * ROW_HEIGHT);
                let key = if column.primary_key {
                    "PK"
                } else if column.foreign_key {
                    "FK"
                } else {
                    ""
                };
                svg.push_str(&format!(r#"<text x="{:.1}" y="{:.1}" fill="{}" font-family="system-ui, sans-serif" font-size="10" font-weight="600">{}</text><text x="{:.1}" y="{:.1}" fill="{}" font-family="system-ui, sans-serif" font-size="12">{}</text><text x="{:.1}" y="{:.1}" text-anchor="end" fill="{}" font-family="system-ui, sans-serif" font-size="11">{}{}</text>"#,
                    node.x + 12.0, y, palette.accent, key, node.x + 38.0, y, palette.text, escape(&column.name), node.x + node.width - 12.0, y, palette.muted_text, escape(&column.data_type), if column.nullable { "" } else { " · not null" }
                ));
            }
            if node.omitted_columns > 0 {
                let y = node.y + HEADER_HEIGHT + 17.0 + (node.columns.len() as f32 * ROW_HEIGHT);
                svg.push_str(&format!(r#"<text x="{:.1}" y="{:.1}" fill="{}" font-family="system-ui, sans-serif" font-size="11">+{} more columns</text>"#, node.x + 12.0, y, palette.muted_text, node.omitted_columns));
            }
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        svg
    }

    /// Serializes a transparent, accent-coloured overlay for one table.
    ///
    /// The overlay contains every direct inbound, outbound, and self-referential
    /// relationship for `selected`, together with outlines for the selected
    /// table and its direct neighbours. `stroke` is used directly instead of
    /// relying on an alpha mask, so the overlay retains the base diagram's
    /// coordinate system even when the SVG renderer caps a large raster.
    pub fn selection_overlay_svg(&self, selected: Option<&str>, stroke: &str) -> Option<String> {
        let selected = selected?;
        if !self.nodes.iter().any(|node| node.id == selected) {
            return None;
        }
        let stroke = escape(stroke);

        let incident_edges = self
            .edges
            .iter()
            .filter(|edge| edge.source == selected || edge.target == selected)
            .collect::<Vec<_>>();
        let endpoint_ids =
            incident_edges
                .iter()
                .fold(BTreeSet::from([selected]), |mut endpoints, edge| {
                    endpoints.insert(edge.source.as_str());
                    endpoints.insert(edge.target.as_str());
                    endpoints
                });
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {:.0} {:.0}" aria-hidden="true"><defs><marker id="selection-relation-arrow" markerWidth="8" markerHeight="8" refX="8" refY="4" viewBox="0 0 8 8" orient="auto"><path d="M 0 0 L 8 4 L 0 8 z" fill="{}"/></marker></defs>"#,
            self.width, self.height, self.width, self.height, stroke,
        );
        for edge in incident_edges {
            svg.push_str(&format!(
                r#"<path d="{}" fill="none" stroke="{}" stroke-width="2" stroke-linejoin="round" marker-end="url(#selection-relation-arrow)" data-edge="{}" data-self-referential="{}"/>"#,
                edge.path,
                stroke,
                escape(&edge.id),
                edge.self_referential,
            ));
        }
        for node in self
            .nodes
            .iter()
            .filter(|node| endpoint_ids.contains(node.id.as_str()))
        {
            let (role, stroke_width) = if node.id == selected {
                ("selected", "3")
            } else {
                ("related", "1.5")
            };
            svg.push_str(&format!(
                r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="8" fill="none" stroke="{}" stroke-width="{}" data-role="{}" data-table="{}"/>"#,
                node.x,
                node.y,
                node.width,
                node.height,
                stroke,
                stroke_width,
                role,
                escape(&node.id),
            ));
        }
        svg.push_str("</svg>");
        Some(svg)
    }

    /// Rasterizes the canonical SVG without a second visual implementation.
    pub fn png(
        &self,
        renderer: &SvgRenderer,
        palette: DiagramPalette<'_>,
        selected: Option<&str>,
        scale: f32,
    ) -> Result<Vec<u8>> {
        let svg = self.svg(palette, selected);
        let image = renderer
            // GPUI doubles `ScaleFactor` rasterization internally for smooth
            // on-screen SVGs. Compensate here so the export API's scale is the
            // actual PNG scale requested by the caller.
            .render_single_frame(
                svg.as_bytes(),
                scale.max(0.1) / gpui::SMOOTH_SVG_SCALE_FACTOR,
            )
            .context("render database diagram SVG")?;
        let size = image.size(0);
        let width = u32::from(size.width);
        let height = u32::from(size.height);
        let bytes = image
            .as_bytes(0)
            .context("diagram renderer returned no frame")?;
        let expected_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("diagram PNG dimensions overflowed")?;
        ensure!(
            bytes.len() == expected_bytes,
            "diagram renderer returned {} bytes for a {width}×{height} BGRA frame; expected {expected_bytes}",
            bytes.len()
        );
        Ok(encode_bgra_png(width, height, bytes))
    }
}

fn visible_columns(
    columns: &[ColumnInfo],
    relationship: &HashSet<String>,
    foreign: &HashSet<String>,
) -> Vec<DiagramColumn> {
    // Relationship endpoints take priority, but the card remains bounded even
    // for unusual wide composite keys. Any endpoints beyond the cap attach to
    // the explicit omitted-columns row instead of growing the whole canvas.
    let important_columns = columns
        .iter()
        .filter(|column| column.primary_key || relationship.contains(&column.name))
        .take(COLUMN_LIMIT)
        .map(|column| column.name.as_str())
        .collect::<HashSet<_>>();
    let ordinary_budget = COLUMN_LIMIT.saturating_sub(important_columns.len());
    let mut ordinary_seen = 0;
    columns
        .iter()
        .filter_map(|column| {
            let important = important_columns.contains(column.name.as_str());
            if !important {
                if ordinary_seen >= ordinary_budget {
                    return None;
                }
                ordinary_seen += 1;
            }
            Some(DiagramColumn {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
                nullable: column.nullable,
                primary_key: column.primary_key,
                foreign_key: foreign.contains(&column.name),
            })
        })
        .collect()
}

fn relationship_levels(
    tables: &[&dbx_core::RelationalTable],
    known: &HashSet<String>,
) -> BTreeMap<String, usize> {
    let mut parents = BTreeMap::<String, BTreeSet<String>>::new();
    for entry in tables {
        let id = table_id(&entry.table);
        let set = parents.entry(id).or_default();
        for foreign_key in &entry.structure.foreign_keys {
            let parent = referenced_id(foreign_key, entry.table.schema.as_deref());
            if parent != table_id(&entry.table) && known.contains(&parent) {
                set.insert(parent);
            }
        }
    }
    fn depth(
        id: &str,
        parents: &BTreeMap<String, BTreeSet<String>>,
        visiting: &mut HashSet<String>,
        cache: &mut HashMap<String, usize>,
    ) -> usize {
        if let Some(value) = cache.get(id) {
            return *value;
        }
        if !visiting.insert(id.into()) {
            return 0;
        }
        let value = parents
            .get(id)
            .into_iter()
            .flatten()
            .map(|parent| depth(parent, parents, visiting, cache) + 1)
            .max()
            .unwrap_or(0);
        visiting.remove(id);
        cache.insert(id.into(), value);
        value
    }
    let mut cache = HashMap::new();
    for id in parents.keys() {
        depth(id, &parents, &mut HashSet::new(), &mut cache);
    }
    cache.into_iter().collect()
}

fn connected_components(
    tables: &[&dbx_core::RelationalTable],
    known: &HashSet<String>,
) -> Vec<Vec<String>> {
    let mut neighbors = tables
        .iter()
        .map(|entry| (table_id(&entry.table), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for entry in tables {
        let source = table_id(&entry.table);
        for foreign_key in &entry.structure.foreign_keys {
            let target = referenced_id(foreign_key, entry.table.schema.as_deref());
            if source != target && known.contains(&target) {
                neighbors
                    .get_mut(&source)
                    .expect("source table is known")
                    .insert(target.clone());
                neighbors
                    .get_mut(&target)
                    .expect("target table is known")
                    .insert(source.clone());
            }
        }
    }

    let mut remaining = neighbors.keys().cloned().collect::<BTreeSet<_>>();
    let mut components = Vec::new();
    while let Some(first) = remaining.pop_first() {
        let mut pending = vec![first];
        let mut component = Vec::new();
        while let Some(id) = pending.pop() {
            component.push(id.clone());
            for neighbor in neighbors.get(&id).into_iter().flatten().rev() {
                if remaining.remove(neighbor) {
                    pending.push(neighbor.clone());
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components
}

fn position_tables(
    columns: &HashMap<String, (Vec<DiagramColumn>, usize)>,
    levels: &BTreeMap<String, usize>,
    components: &[Vec<String>],
) -> HashMap<String, (f32, f32)> {
    let mut positions = HashMap::new();
    let mut shelf_x = 0.0;
    let mut shelf_y = 0.0;
    let mut shelf_height: f32 = 0.0;
    for component in components {
        let minimum_level = component
            .iter()
            .filter_map(|id| levels.get(id))
            .copied()
            .min()
            .unwrap_or(0);
        let mut ids_by_level = BTreeMap::<usize, Vec<&String>>::new();
        for id in component {
            ids_by_level
                .entry(levels.get(id).copied().unwrap_or(0) - minimum_level)
                .or_default()
                .push(id);
        }
        let maximum_level = ids_by_level.keys().copied().max().unwrap_or(0);
        let component_width = NODE_WIDTH + maximum_level as f32 * (NODE_WIDTH + HORIZONTAL_GAP);
        let component_height = ids_by_level
            .values()
            .map(|ids| {
                ids.iter()
                    .map(|id| node_height(&columns[id.as_str()]))
                    .sum::<f32>()
                    + VERTICAL_GAP * ids.len().saturating_sub(1) as f32
            })
            .fold(0.0, f32::max);

        if shelf_x > 0.0 && shelf_x + component_width > COMPONENT_SHELF_WIDTH {
            shelf_x = 0.0;
            shelf_y += shelf_height + VERTICAL_GAP;
            shelf_height = 0.0;
        }
        for (level, ids) in &mut ids_by_level {
            ids.sort();
            let x = PADDING + shelf_x + *level as f32 * (NODE_WIDTH + HORIZONTAL_GAP);
            let mut y = PADDING + shelf_y;
            for id in ids {
                positions.insert((*id).clone(), (x, y));
                y += node_height(&columns[id.as_str()]) + VERTICAL_GAP;
            }
        }
        shelf_x += component_width + HORIZONTAL_GAP;
        shelf_height = shelf_height.max(component_height);
    }
    positions
}

fn node_height(columns: &(Vec<DiagramColumn>, usize)) -> f32 {
    let (visible, omitted) = columns;
    HEADER_HEIGHT + ((visible.len() + usize::from(*omitted > 0)) as f32 * ROW_HEIGHT) + 10.0
}

fn edge_path(
    source: &DiagramNode,
    target: &DiagramNode,
    foreign_key: &ForeignKeyInfo,
    self_referential: bool,
) -> String {
    let from_y = row_y(source, foreign_key.columns.first().map(String::as_str));
    if self_referential {
        let right = source.x + source.width + 34.0;
        // Return to the top edge rather than stopping above the card. The
        // marker tip is therefore anchored to an actual card border.
        let return_x =
            (source.x + source.width - 24.0).clamp(source.x + 12.0, source.x + source.width - 12.0);
        return format!(
            "M {:.1} {:.1} H {:.1} V {:.1} H {:.1} V {:.1}",
            source.x + source.width,
            from_y,
            right,
            source.y - 18.0,
            return_x,
            source.y,
        );
    }
    let to_y = row_y(
        target,
        foreign_key.referenced_columns.first().map(String::as_str),
    );
    let source_bottom = source.y + source.height;
    let target_bottom = target.y + target.height;
    let source_right = source.x + source.width;
    let target_left = target.x;
    let target_right = target.x + target.width;
    let overlap_left = source.x.max(target.x);
    let overlap_right = source_right.min(target_right);

    // Cards in the same layout column overlap horizontally. Route through
    // their facing top/bottom borders instead of crossing their interiors.
    if overlap_left < overlap_right {
        let middle = (overlap_left + overlap_right) / 2.0;
        if source_bottom <= target.y {
            return format!("M {:.1} {:.1} V {:.1}", middle, source_bottom, target.y);
        }
        if target_bottom <= source.y {
            return format!("M {:.1} {:.1} V {:.1}", middle, source.y, target_bottom);
        }

        // A defensive route for manually-overlapping cards. Pick exposed
        // border points before travelling to an outer rail; row-centred side
        // ports can otherwise land inside the other card.
        if let (Some(source_y), Some(target_y)) = (
            exposed_vertical_port(source, target),
            exposed_vertical_port(target, source),
        ) {
            let rail = source.x.min(target.x) - 34.0;
            return format!(
                "M {:.1} {:.1} H {:.1} V {:.1} H {:.1}",
                source.x, source_y, rail, target_y, target.x
            );
        }
        if let (Some(source_x), Some(target_x)) = (
            exposed_horizontal_port(source, target),
            exposed_horizontal_port(target, source),
        ) {
            let rail = source.y.min(target.y) - 34.0;
            return format!(
                "M {:.1} {:.1} V {:.1} H {:.1} V {:.1}",
                source_x, source.y, rail, target_x, target.y
            );
        }

        // One card fully contains the other. Such rectangles cannot occur in
        // a document layout; no port on the inner card is exposed. Keep a
        // deterministic fallback for malformed caller-provided nodes.
        let rail = source_right.max(target_right) + 34.0;
        return format!(
            "M {:.1} {:.1} H {:.1} V {:.1} H {:.1}",
            source_right, from_y, rail, to_y, target_right
        );
    }
    if source_right <= target_left {
        let middle = (source_right + target_left) / 2.0;
        format!(
            "M {:.1} {:.1} H {:.1} V {:.1} H {:.1}",
            source_right, from_y, middle, to_y, target_left
        )
    } else {
        debug_assert!(target_right <= source.x);
        let middle = (target_right + source.x) / 2.0;
        format!(
            "M {:.1} {:.1} H {:.1} V {:.1} H {:.1}",
            source.x,
            from_y,
            middle,
            to_y,
            target.x + target.width
        )
    }
}

fn exposed_vertical_port(node: &DiagramNode, other: &DiagramNode) -> Option<f32> {
    [node.y, node.y + node.height]
        .into_iter()
        .find(|y| *y <= other.y || *y >= other.y + other.height)
}

fn exposed_horizontal_port(node: &DiagramNode, other: &DiagramNode) -> Option<f32> {
    [node.x, node.x + node.width]
        .into_iter()
        .find(|x| *x <= other.x || *x >= other.x + other.width)
}

fn row_y(node: &DiagramNode, name: Option<&str>) -> f32 {
    let index = name
        .and_then(|name| node.columns.iter().position(|column| column.name == name))
        .unwrap_or(node.columns.len());
    node.y + HEADER_HEIGHT + 11.5 + (index as f32 * ROW_HEIGHT)
}

fn table_id(table: &TableInfo) -> String {
    format!("{}.{}", table.schema.as_deref().unwrap_or(""), table.name)
}
fn referenced_id(key: &ForeignKeyInfo, fallback_schema: Option<&str>) -> String {
    format!(
        "{}.{}",
        key.referenced_schema
            .as_deref()
            .or(fallback_schema)
            .unwrap_or(""),
        key.referenced_table
    )
}
fn display_table(table: &TableInfo) -> String {
    table
        .schema
        .as_ref()
        .map(|schema| format!("{schema}.{}", table.name))
        .unwrap_or_else(|| table.name.clone())
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn encode_bgra_png(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * height as usize * 4) + height as usize);
    for row in bgra.chunks_exact(width as usize * 4) {
        raw.push(0);
        for pixel in row.chunks_exact(4) {
            raw.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &header);
    png_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    png_chunk(&mut png, b"IEND", &[]);
    png
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut result = vec![0x78, 0x01];
    for (index, chunk) in data.chunks(65_535).enumerate() {
        let final_block = index + 1 == data.chunks(65_535).len();
        result.push(u8::from(final_block));
        let length = chunk.len() as u16;
        result.extend_from_slice(&length.to_le_bytes());
        result.extend_from_slice(&(!length).to_le_bytes());
        result.extend_from_slice(chunk);
    }
    result.extend_from_slice(&adler32(data).to_be_bytes());
    result
}
fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_data = kind.to_vec();
    crc_data.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_data).to_be_bytes());
}
fn crc32(data: &[u8]) -> u32 {
    data.iter().fold(!0u32, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |value, _| {
            if value & 1 == 1 {
                (value >> 1) ^ 0xedb8_8320
            } else {
                value >> 1
            }
        })
    }) ^ !0
}
fn adler32(data: &[u8]) -> u32 {
    let (a, b) = data.iter().fold((1u32, 0u32), |(a, b), byte| {
        let a = (a + u32::from(*byte)) % 65_521;
        (a, (b + a) % 65_521)
    });
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::{EntityKind, ForeignKeyInfo, RelationalTable, TableStructure};
    use std::sync::Arc;

    fn table(
        name: &str,
        columns: Vec<ColumnInfo>,
        foreign_keys: Vec<ForeignKeyInfo>,
    ) -> RelationalTable {
        table_in_schema("public", name, columns, foreign_keys)
    }
    fn table_in_schema(
        schema: &str,
        name: &str,
        columns: Vec<ColumnInfo>,
        foreign_keys: Vec<ForeignKeyInfo>,
    ) -> RelationalTable {
        RelationalTable {
            table: TableInfo {
                name: name.into(),
                schema: Some(schema.into()),
                kind: EntityKind::Table,
            },
            structure: TableStructure {
                columns,
                foreign_keys,
            },
        }
    }
    fn column(name: &str, ordinal: usize, primary_key: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: "uuid".into(),
            enum_values: vec![],
            nullable: !primary_key,
            ordinal,
            primary_key,
        }
    }
    fn foreign(columns: &[&str], target: &str, target_columns: &[&str]) -> ForeignKeyInfo {
        foreign_in_schema("public", columns, target, target_columns)
    }
    fn foreign_in_schema(
        target_schema: &str,
        columns: &[&str],
        target: &str,
        target_columns: &[&str],
    ) -> ForeignKeyInfo {
        ForeignKeyInfo {
            constraint_name: None,
            columns: columns.iter().map(|value| (*value).into()).collect(),
            referenced_schema: Some(target_schema.into()),
            referenced_table: target.into(),
            referenced_columns: target_columns.iter().map(|value| (*value).into()).collect(),
            on_update: None,
            on_delete: None,
        }
    }
    fn palette() -> DiagramPalette<'static> {
        DiagramPalette {
            canvas: "#0a0c10",
            surface: "#111318",
            surface_muted: "#171a20",
            border: "#1f232b",
            text: "#f1f5f9",
            muted_text: "#94a3b8",
            accent: "#2563eb",
            relation: "#60a5fa",
        }
    }

    fn node(id: &str, x: f32, y: f32, width: f32, height: f32) -> DiagramNode {
        DiagramNode {
            id: id.into(),
            table: TableInfo {
                name: id.into(),
                schema: Some("public".into()),
                kind: EntityKind::Table,
            },
            x,
            y,
            width,
            height,
            columns: vec![],
            omitted_columns: 0,
        }
    }

    fn is_on_border(node: &DiagramNode, x: f32, y: f32) -> bool {
        let right = node.x + node.width;
        let bottom = node.y + node.height;
        ((x == node.x || x == right) && y >= node.y && y <= bottom)
            || ((y == node.y || y == bottom) && x >= node.x && x <= right)
    }

    fn crosses_interior(
        node: &DiagramNode,
        (start_x, start_y): (f32, f32),
        (end_x, end_y): (f32, f32),
    ) -> bool {
        let right = node.x + node.width;
        let bottom = node.y + node.height;
        if start_x == end_x {
            start_x > node.x
                && start_x < right
                && start_y.min(end_y) < bottom
                && start_y.max(end_y) > node.y
        } else {
            start_y > node.y
                && start_y < bottom
                && start_x.min(end_x) < right
                && start_x.max(end_x) > node.x
        }
    }

    #[test]
    fn layout_is_deterministic_and_nodes_do_not_overlap() {
        let users = table("users", vec![column("id", 0, true)], vec![]);
        let posts = table(
            "posts",
            vec![column("id", 0, true), column("user_id", 1, false)],
            vec![foreign(&["user_id"], "users", &["id"])],
        );
        let forward = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![users.clone(), posts.clone()],
        });
        let reverse = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![posts, users],
        });
        assert_eq!(forward.svg(palette(), None), reverse.svg(palette(), None));
        for (index, left) in forward.nodes.iter().enumerate() {
            for right in forward.nodes.iter().skip(index + 1) {
                assert!(
                    left.x + left.width <= right.x
                        || right.x + right.width <= left.x
                        || left.y + left.height <= right.y
                        || right.y + right.height <= left.y
                );
            }
        }
    }

    #[test]
    fn schema_projection_with_all_schemas_matches_from_schema() {
        let schema = RelationalSchema {
            database: "db".into(),
            tables: vec![
                table_in_schema("public", "users", vec![column("id", 0, true)], vec![]),
                table_in_schema("audit", "events", vec![column("id", 0, true)], vec![]),
            ],
        };

        let full = DiagramDocument::from_schema(&schema);
        let projected = DiagramDocument::from_schema_selection(&schema, None);

        assert_eq!(full.svg(palette(), None), projected.svg(palette(), None));
    }

    #[test]
    fn schema_projection_filters_nodes() {
        let schema = RelationalSchema {
            database: "db".into(),
            tables: vec![
                table_in_schema("public", "users", vec![column("id", 0, true)], vec![]),
                table_in_schema("audit", "events", vec![column("id", 0, true)], vec![]),
            ],
        };
        let selected = BTreeSet::from(["public".to_owned()]);

        let document = DiagramDocument::from_schema_selection(&schema, Some(&selected));

        assert_eq!(document.nodes.len(), 1);
        assert_eq!(document.nodes[0].id, "public.users");
        assert!(document.edges.is_empty());
    }

    #[test]
    fn schema_projection_retains_cross_schema_relationships() {
        let schema = RelationalSchema {
            database: "db".into(),
            tables: vec![
                table_in_schema("auth", "users", vec![column("id", 0, true)], vec![]),
                table_in_schema(
                    "public",
                    "posts",
                    vec![column("id", 0, true), column("author_id", 1, false)],
                    vec![foreign_in_schema("auth", &["author_id"], "users", &["id"])],
                ),
            ],
        };
        let original = schema.clone();
        let selected = BTreeSet::from(["auth".to_owned(), "public".to_owned()]);

        let document = DiagramDocument::from_schema_selection(&schema, Some(&selected));

        assert_eq!(document.nodes.len(), 2);
        assert_eq!(document.edges.len(), 1);
        assert_eq!(document.edges[0].source, "public.posts");
        assert_eq!(document.edges[0].target, "auth.users");
        assert_eq!(schema, original);
    }

    #[test]
    fn schema_projection_omits_relationships_with_hidden_endpoints() {
        let schema = RelationalSchema {
            database: "db".into(),
            tables: vec![
                table_in_schema("auth", "users", vec![column("id", 0, true)], vec![]),
                table_in_schema(
                    "public",
                    "posts",
                    vec![column("id", 0, true), column("author_id", 1, false)],
                    vec![foreign_in_schema("auth", &["author_id"], "users", &["id"])],
                ),
            ],
        };

        for selected_schema in ["auth", "public"] {
            let selected = BTreeSet::from([selected_schema.to_owned()]);
            let document = DiagramDocument::from_schema_selection(&schema, Some(&selected));
            assert_eq!(document.nodes.len(), 1);
            assert!(document.edges.is_empty());
        }
    }

    #[test]
    fn empty_schema_projection_has_an_empty_export() {
        let schema = RelationalSchema {
            database: "db".into(),
            tables: vec![table("users", vec![column("id", 0, true)], vec![])],
        };
        let selected = BTreeSet::new();

        let document = DiagramDocument::from_schema_selection(&schema, Some(&selected));
        let svg = document.svg(palette(), None);

        assert!(document.nodes.is_empty());
        assert!(document.edges.is_empty());
        assert!(!svg.contains("<g data-table="));
        assert!(!svg.contains("data-self-referential="));
    }

    #[test]
    fn disconnected_tables_are_shelf_packed_instead_of_one_tall_column() {
        let tables = (0..7)
            .map(|index| {
                table(
                    &format!("table_{index}"),
                    vec![column("id", 0, true)],
                    vec![],
                )
            })
            .collect();
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables,
        });
        let x_positions = document
            .nodes
            .iter()
            .map(|node| node.x as i32)
            .collect::<BTreeSet<_>>();

        assert_eq!(x_positions.len(), 3);
        assert!(document.height < 600.0);
    }

    #[test]
    fn svg_escapes_identifiers_and_keeps_composite_relationships() {
        let parent = table(
            "parent<&",
            vec![column("first", 0, true), column("second", 1, true)],
            vec![],
        );
        let child = table(
            "child",
            vec![column("first", 0, false), column("second", 1, false)],
            vec![foreign(
                &["first", "second"],
                "parent<&",
                &["first", "second"],
            )],
        );
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![child, parent],
        });
        let svg = document.svg(palette(), None);
        assert!(svg.contains("parent&lt;&amp;"));
        assert_eq!(document.edges[0].source_columns, ["first", "second"]);
        assert_eq!(document.edges[0].target_columns, ["first", "second"]);
    }

    #[test]
    fn selection_overlay_contains_only_incident_relationships_and_endpoints() {
        let accounts = table("accounts", vec![column("id", 0, true)], vec![]);
        let orders = table(
            "orders",
            vec![column("id", 0, true), column("account_id", 1, false)],
            vec![foreign(&["account_id"], "accounts", &["id"])],
        );
        let payments = table(
            "payments",
            vec![column("id", 0, true), column("order_id", 1, false)],
            vec![foreign(&["order_id"], "orders", &["id"])],
        );
        let audits = table("audits", vec![column("id", 0, true)], vec![]);
        let logs = table(
            "logs",
            vec![column("id", 0, true), column("audit_id", 1, false)],
            vec![foreign(&["audit_id"], "audits", &["id"])],
        );
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![accounts, orders, payments, audits, logs],
        });

        let overlay = document
            .selection_overlay_svg(Some("public.orders"), "#c0ffee")
            .expect("known table has an overlay");

        assert!(overlay.contains("data-edge=\"public.orders:0\""));
        assert!(overlay.contains("data-edge=\"public.payments:0\""));
        assert!(!overlay.contains("data-edge=\"public.logs:0\""));
        for table in ["public.accounts", "public.orders", "public.payments"] {
            assert!(overlay.contains(&format!("data-table=\"{table}\"")));
        }
        assert!(
            overlay
                .contains("stroke-width=\"3\" data-role=\"selected\" data-table=\"public.orders\"")
        );
        for table in ["public.accounts", "public.payments"] {
            assert!(overlay.contains(&format!(
                "stroke-width=\"1.5\" data-role=\"related\" data-table=\"{table}\""
            )));
        }
        for table in ["public.audits", "public.logs"] {
            assert!(!overlay.contains(&format!("data-table=\"{table}\"")));
        }
        assert!(overlay.contains("marker-end=\"url(#selection-relation-arrow)\""));
        assert!(overlay.contains("stroke-width=\"2\" stroke-linejoin"));
        assert_eq!(overlay.matches("#c0ffee").count(), 6);
        assert!(
            !document
                .svg(palette(), None)
                .contains("selection-relation-arrow")
        );
    }

    #[test]
    fn selection_overlay_handles_self_relationships_and_is_deterministic() {
        let tree = table(
            "tree<&",
            vec![column("id", 0, true), column("parent_id", 1, false)],
            vec![foreign(&["parent_id"], "tree<&", &["id"])],
        );
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![tree.clone()],
        });
        let reversed = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![tree],
        });

        let overlay = document
            .selection_overlay_svg(Some("public.tree<&"), "#c0ffee")
            .expect("known table has an overlay");
        assert!(overlay.contains("data-edge=\"public.tree&lt;&amp;:0\""));
        assert!(overlay.contains("data-self-referential=\"true\""));
        assert_eq!(overlay.matches("data-table=").count(), 1);
        assert!(overlay.contains("stroke-width=\"3\" data-role=\"selected\""));
        assert_eq!(
            Some(overlay),
            reversed.selection_overlay_svg(Some("public.tree<&"), "#c0ffee")
        );
        assert_eq!(document.selection_overlay_svg(None, "#c0ffee"), None);
        assert_eq!(
            document.selection_overlay_svg(Some("public.missing"), "#c0ffee"),
            None
        );
    }

    #[test]
    fn selection_overlay_preserves_large_document_dimensions_and_stroke() {
        let document = DiagramDocument {
            database: "db".into(),
            nodes: vec![node("selected", 44.0, 44.0, NODE_WIDTH, 75.0)],
            edges: vec![],
            width: 4097.0,
            height: 256.0,
        };
        let dimensions = "width=\"4097\" height=\"256\" viewBox=\"0 0 4097 256\"";
        let canonical = document.svg(palette(), None);
        let overlay = document
            .selection_overlay_svg(Some("selected"), "#c0ffee")
            .expect("known table has an overlay");

        assert!(canonical.contains(dimensions));
        assert!(overlay.contains(dimensions));
        assert!(overlay.contains("fill=\"#c0ffee\""));
        assert!(overlay.contains("stroke=\"#c0ffee\""));

        let escaped_stroke = document
            .selection_overlay_svg(Some("selected"), r##"url(#accent<&")"##)
            .expect("known table has an overlay");
        assert!(escaped_stroke.contains("fill=\"url(#accent&lt;&amp;&quot;)\""));
    }

    #[test]
    fn selection_overlay_shares_the_capped_svg_raster_coordinate_space() {
        // GPUI renders smooth SVGs at 2x. This width therefore exceeds its
        // 8192px pixmap cap, while the short height keeps the test allocation
        // bounded to roughly 32 MiB across the two decoded frames.
        let document = DiagramDocument {
            database: "db".into(),
            nodes: vec![node("selected", 1_024.0, 44.0, NODE_WIDTH, 75.0)],
            edges: vec![],
            width: 4_097.0,
            height: 256.0,
        };
        let canonical = document.svg(palette(), None);
        let overlay = document
            .selection_overlay_svg(Some("selected"), "#c0ffee")
            .expect("known table has an overlay");
        let renderer = SvgRenderer::new(Arc::new(()));

        let canonical_image = renderer
            .render_single_frame(canonical.as_bytes(), 1.0)
            .expect("canonical SVG should decode");
        let overlay_image = renderer
            .render_single_frame(overlay.as_bytes(), 1.0)
            .expect("overlay SVG should decode");
        let canonical_size = canonical_image.size(0);
        let overlay_size = overlay_image.size(0);

        assert_eq!(canonical_size, overlay_size);
        assert_eq!(u32::from(canonical_size.width), 8_192);
        assert!(u32::from(canonical_size.height) < 512);
        assert!(canonical.contains("viewBox=\"0 0 4097 256\""));
        assert!(overlay.contains("viewBox=\"0 0 4097 256\""));
    }

    #[test]
    fn referenced_columns_remain_visible_without_becoming_foreign_keys() {
        let mut parent_columns = (0..20)
            .map(|index| column(&format!("column_{index}"), index, index == 0))
            .collect::<Vec<_>>();
        parent_columns.push(column("external_key", 20, false));
        let parent = table("parent", parent_columns, vec![]);
        let child = table(
            "child",
            vec![column("id", 0, true), column("parent_key", 1, false)],
            vec![foreign(&["parent_key"], "parent", &["external_key"])],
        );

        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![parent, child],
        });
        let parent = document
            .nodes
            .iter()
            .find(|node| node.table.name == "parent")
            .unwrap();
        let referenced = parent
            .columns
            .iter()
            .find(|column| column.name == "external_key")
            .expect("referenced column should be retained beyond the ordinary column limit");
        assert!(!referenced.foreign_key);
    }

    #[test]
    fn wide_composite_relationships_keep_cards_bounded() {
        let names = (0..25)
            .map(|index| format!("key_{index}"))
            .collect::<Vec<_>>();
        let parent = table(
            "parent",
            names
                .iter()
                .enumerate()
                .map(|(index, name)| column(name, index, true))
                .collect(),
            vec![],
        );
        let child = table(
            "child",
            names
                .iter()
                .enumerate()
                .map(|(index, name)| column(name, index, false))
                .collect(),
            vec![ForeignKeyInfo {
                constraint_name: Some("all_the_keys".into()),
                columns: names.clone(),
                referenced_schema: Some("public".into()),
                referenced_table: "parent".into(),
                referenced_columns: names,
                on_update: None,
                on_delete: None,
            }],
        );
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![parent, child],
        });

        for node in &document.nodes {
            assert_eq!(node.columns.len(), COLUMN_LIMIT);
            assert_eq!(node.omitted_columns, 7);
            assert_eq!(
                node.height,
                HEADER_HEIGHT + ((COLUMN_LIMIT + 1) as f32 * ROW_HEIGHT) + 10.0
            );
        }
    }

    #[test]
    fn vertically_stacked_cards_connect_through_their_facing_borders() {
        let source = node("child", 100.0, 100.0, 100.0, 100.0);
        let target = node("parent", 100.0, 300.0, 100.0, 100.0);

        assert_eq!(
            edge_path(
                &source,
                &target,
                &foreign(&["parent_id"], "parent", &["id"]),
                false
            ),
            "M 150.0 200.0 V 300.0"
        );
    }

    #[test]
    fn self_referential_loop_returns_to_a_card_border() {
        let table = node("tree", 100.0, 100.0, 100.0, 100.0);

        assert_eq!(
            edge_path(
                &table,
                &table,
                &foreign(&["parent_id"], "tree", &["id"]),
                true
            ),
            "M 200.0 153.5 H 234.0 V 82.0 H 176.0 V 100.0"
        );
    }

    #[test]
    fn overlapping_cards_use_exposed_border_ports_and_an_outer_rail() {
        // The target's horizontal span is contained by the source. Its top
        // edge overlaps the source, so the safe target port is its bottom.
        let source = node("source", 100.0, 100.0, 200.0, 200.0);
        let target = node("target", 150.0, 250.0, 100.0, 200.0);

        assert_eq!(
            edge_path(
                &source,
                &target,
                &foreign(&["target_id"], "target", &["id"]),
                false
            ),
            "M 100.0 100.0 H 66.0 V 450.0 H 150.0"
        );

        let segments = [
            ((100.0, 100.0), (66.0, 100.0)),
            ((66.0, 100.0), (66.0, 450.0)),
            ((66.0, 450.0), (150.0, 450.0)),
        ];
        for segment in segments {
            assert!(!crosses_interior(&source, segment.0, segment.1));
            assert!(!crosses_interior(&target, segment.0, segment.1));
        }
        assert!(is_on_border(&source, 100.0, 100.0));
        assert!(is_on_border(&target, 150.0, 450.0));
    }

    #[test]
    fn arrowhead_tip_is_anchored_at_the_connector_endpoint() {
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![table("users", vec![column("id", 0, true)], vec![])],
        });

        assert!(
            document
                .svg(palette(), None)
                .contains("markerWidth=\"8\" markerHeight=\"8\" refX=\"8\" refY=\"4\"")
        );
    }

    #[test]
    fn png_export_has_a_valid_signature() {
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![table("users", vec![column("id", 0, true)], vec![])],
        });
        let renderer = SvgRenderer::new(Arc::new(()));
        let png = document.png(&renderer, palette(), None, 1.0).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            document.width as u32
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
            document.height as u32
        );
        assert_eq!(&png[png.len() - 12..png.len() - 4], b"\0\0\0\0IEND");
    }
}
