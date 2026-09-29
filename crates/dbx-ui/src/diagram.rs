//! A deterministic, exportable entity relationship diagram document.
//!
//! This module deliberately has no view state: callers load a
//! [`RelationalSchema`], retain the resulting [`DiagramDocument`], and use the
//! same vector scene for the native on-screen canvas and file export.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};
use dbx_core::{ColumnInfo, ForeignKeyInfo, RelationalSchema, TableInfo};
use gpui::SvgRenderer;

const NODE_WIDTH: f32 = 292.0;
pub(crate) const HEADER_HEIGHT: f32 = 42.0;
pub(crate) const ROW_HEIGHT: f32 = 23.0;
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
    /// Primary-key glyphs; foreign keys use `accent`.
    pub key: &'a str,
    pub relation: &'a str,
}

/// Corner radius for orthogonal relationship routes.
pub const EDGE_CORNER_RADIUS: f32 = 7.0;
/// Separation between relationships that share a routing corridor.
const LANE_SPACING: f32 = 7.0;

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
    /// Compact display spelling of the column type (`timestamptz`,
    /// `varchar(160)`), which keeps cards narrow without truncation.
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
    /// Orthogonal route points used by the native GPUI canvas. Keeping the
    /// geometry in the document avoids reparsing SVG in the render loop.
    pub points: Vec<(f32, f32)>,
    pub self_referential: bool,
    /// Every referencing column is nullable: a zero-or-many relationship.
    pub optional: bool,
}

/// Crow's-foot cardinality marks for one relationship, in document space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeMarkers {
    pub lines: Vec<[(f32, f32); 2]>,
    /// Centre and radius of the "zero" ring on optional relationships.
    pub ring: Option<((f32, f32), f32)>,
}

/// One drawing step of a rounded orthogonal route.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteStep {
    Move((f32, f32)),
    Line((f32, f32)),
    /// Quadratic curve to the first point, controlled by the second.
    Curve((f32, f32), (f32, f32)),
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
        let mut edges = Vec::new();
        for entry in &tables {
            let source = table_id(&entry.table);
            for (index, foreign_key) in entry.structure.foreign_keys.iter().enumerate() {
                let target = referenced_id(foreign_key, entry.table.schema.as_deref());
                if !table_ids.contains(&target) {
                    continue;
                }
                let optional = !foreign_key.columns.is_empty()
                    && foreign_key.columns.iter().all(|name| {
                        entry
                            .structure
                            .columns
                            .iter()
                            .find(|column| column.name == *name)
                            .is_some_and(|column| column.nullable)
                    });
                edges.push(DiagramEdge {
                    id: format!("{source}:{index}"),
                    self_referential: source == target,
                    source: source.clone(),
                    target,
                    source_columns: foreign_key.columns.clone(),
                    target_columns: foreign_key.referenced_columns.clone(),
                    path: String::new(),
                    points: Vec::new(),
                    optional,
                });
            }
        }
        edges.sort_by(|left, right| left.id.cmp(&right.id));

        let mut document = Self {
            database: schema.database.clone(),
            nodes,
            edges,
            width: 0.0,
            height: 0.0,
        };
        document.route();
        document
    }

    /// Move one card to a new document position and reroute every
    /// relationship. Positions are clamped so a card cannot leave the scene.
    pub fn move_node(&mut self, id: &str, x: f32, y: f32) -> bool {
        let Some(node) = self.nodes.iter_mut().find(|node| node.id == id) else {
            return false;
        };
        let (x, y) = (x.max(PADDING / 2.0).round(), y.max(PADDING / 2.0).round());
        if node.x == x && node.y == y {
            return false;
        }
        node.x = x;
        node.y = y;
        self.route();
        true
    }

    /// Restore user-arranged card positions after a rebuild. Unknown IDs are
    /// ignored so a schema filter can hide arranged tables.
    pub fn place_nodes(&mut self, positions: &HashMap<String, (f32, f32)>) {
        if positions.is_empty() {
            return;
        }
        for node in &mut self.nodes {
            if let Some(&(x, y)) = positions.get(&node.id) {
                node.x = x.max(PADDING / 2.0);
                node.y = y.max(PADDING / 2.0);
            }
        }
        self.route();
    }

    /// The top-left corner of a card, if it exists.
    pub fn node_position(&self, id: &str) -> Option<(f32, f32)> {
        self.nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| (node.x, node.y))
    }

    /// Recompute relationship geometry and scene bounds from card positions.
    fn route(&mut self) {
        let node_by_id = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<HashMap<_, _>>();
        for edge in &mut self.edges {
            let (Some(source), Some(target)) = (
                node_by_id.get(edge.source.as_str()),
                node_by_id.get(edge.target.as_str()),
            ) else {
                continue;
            };
            let path = edge_route(
                source,
                target,
                edge.source_columns.first().map(String::as_str),
                edge.target_columns.first().map(String::as_str),
                edge.self_referential,
            );
            edge.points = edge_path_points(&path);
        }
        separate_lanes(&mut self.edges);
        for edge in &mut self.edges {
            edge.path = rounded_svg_path(&edge.points, EDGE_CORNER_RADIUS);
        }

        self.width = self
            .nodes
            .iter()
            .map(|node| node.x + node.width + PADDING)
            .fold(PADDING * 2.0, f32::max);
        self.height = self
            .nodes
            .iter()
            .map(|node| node.y + node.height + PADDING)
            .fold(PADDING * 2.0, f32::max);
    }

    /// Serializes the canonical vector representation. This is intentionally
    /// the export representation of the same geometry used by the native view.
    pub fn svg(&self, palette: DiagramPalette<'_>, selected: Option<&str>) -> String {
        const FONT: &str = r#"font-family="system-ui, -apple-system, 'Segoe UI', sans-serif""#;
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {:.0} {:.0}" role="img" aria-label="Entity relationship diagram for {}"><defs><pattern id="dbx-grid" width="24" height="24" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="{}" fill-opacity="0.45"/></pattern></defs><rect width="100%" height="100%" fill="{}"/><rect width="100%" height="100%" fill="url(#dbx-grid)"/>"#,
            self.width,
            self.height,
            self.width,
            self.height,
            escape(&self.database),
            palette.border,
            palette.canvas,
        );
        for edge in &self.edges {
            let column_pairs = edge
                .source_columns
                .iter()
                .zip(&edge.target_columns)
                .map(|(source, target)| format!("{} → {}", escape(source), escape(target)))
                .collect::<Vec<_>>()
                .join(", ");
            let markers = edge_markers(&edge.points, edge.optional);
            let mut marks = String::new();
            for [from, to] in &markers.lines {
                marks.push_str(&format!(
                    "M {:.1} {:.1} L {:.1} {:.1} ",
                    from.0, from.1, to.0, to.1
                ));
            }
            svg.push_str(&format!(
                r#"<g data-relationship="{}" data-self-referential="{}"><title>{} → {} ({})</title><path d="{}" fill="none" stroke="{}" stroke-width="1.5" stroke-linejoin="round"/><path d="{}" fill="none" stroke="{}" stroke-width="1.5" stroke-linecap="round"/>"#,
                escape(&edge.id),
                edge.self_referential,
                escape(&edge.source),
                escape(&edge.target),
                column_pairs,
                edge.path,
                palette.relation,
                marks.trim_end(),
                palette.relation,
            ));
            if let Some(((x, y), radius)) = markers.ring {
                svg.push_str(&format!(
                    r#"<circle cx="{x:.1}" cy="{y:.1}" r="{radius:.1}" fill="{}" stroke="{}" stroke-width="1.5"/>"#,
                    palette.canvas, palette.relation
                ));
            }
            svg.push_str("</g>");
        }
        for node in &self.nodes {
            let is_selected = selected == Some(node.id.as_str());
            let stroke = if is_selected {
                palette.accent
            } else {
                palette.border
            };
            let stroke_width = if is_selected { 2.0 } else { 1.0 };
            svg.push_str(&format!(
                r#"<g data-table="{}"><title>{}</title><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="10" fill="{}" stroke="{}" stroke-width="{}"/><path d="M {:.1} {:.1} a 10 10 0 0 1 10 -10 h {:.1} a 10 10 0 0 1 10 10 v {:.1} h -{:.1} z" fill="{}"/><line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}"/><text x="{:.1}" y="{:.1}" fill="{}" {FONT} font-size="14" font-weight="600">{}</text><text x="{:.1}" y="{:.1}" fill="{}" {FONT} font-size="10">{}</text>"#,
                escape(&node.id),
                escape(&display_table(&node.table)),
                node.x,
                node.y,
                node.width,
                node.height,
                palette.surface,
                stroke,
                stroke_width,
                node.x,
                node.y + 10.0,
                node.width - 20.0,
                HEADER_HEIGHT - 10.0,
                node.width,
                palette.surface_muted,
                node.x,
                node.y + HEADER_HEIGHT,
                node.x + node.width,
                node.y + HEADER_HEIGHT,
                palette.border,
                node.x + 12.0,
                node.y + 19.0,
                palette.text,
                escape(&node.table.name),
                node.x + 12.0,
                node.y + 33.0,
                palette.muted_text,
                escape(node.table.schema.as_deref().unwrap_or("default")),
            ));
            for (index, column) in node.columns.iter().enumerate() {
                let y = node.y + HEADER_HEIGHT + 17.0 + (index as f32 * ROW_HEIGHT);
                let (key, key_color) = if column.primary_key {
                    ("PK", palette.key)
                } else if column.foreign_key {
                    ("FK", palette.accent)
                } else {
                    ("", palette.muted_text)
                };
                svg.push_str(&format!(
                    r#"<text x="{:.1}" y="{:.1}" fill="{}" {FONT} font-size="9" font-weight="700">{}</text><text x="{:.1}" y="{:.1}" fill="{}" {FONT} font-size="12"{}>{}</text><text x="{:.1}" y="{:.1}" text-anchor="end" fill="{}" {FONT} font-size="11">{}{}</text>"#,
                    node.x + 12.0,
                    y,
                    key_color,
                    key,
                    node.x + 38.0,
                    y,
                    palette.text,
                    if column.primary_key {
                        r#" font-weight="600""#
                    } else {
                        ""
                    },
                    escape(&column.name),
                    node.x + node.width - 12.0,
                    y,
                    palette.muted_text,
                    escape(&column.data_type),
                    if column.nullable { "?" } else { "" }
                ));
            }
            if node.omitted_columns > 0 {
                let y = node.y + HEADER_HEIGHT + 17.0 + (node.columns.len() as f32 * ROW_HEIGHT);
                svg.push_str(&format!(
                    r#"<text x="{:.1}" y="{:.1}" fill="{}" {FONT} font-size="11">+{} more columns</text>"#,
                    node.x + 12.0,
                    y,
                    palette.muted_text,
                    node.omitted_columns
                ));
            }
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        svg
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
                data_type: short_type(&column.data_type),
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

#[cfg(test)]
fn edge_path(
    source: &DiagramNode,
    target: &DiagramNode,
    foreign_key: &ForeignKeyInfo,
    self_referential: bool,
) -> String {
    edge_route(
        source,
        target,
        foreign_key.columns.first().map(String::as_str),
        foreign_key.referenced_columns.first().map(String::as_str),
        self_referential,
    )
}

/// An orthogonal route from the referencing column's row on `source` to the
/// referenced column's row on `target`, as an `M/H/V` path.
fn edge_route(
    source: &DiagramNode,
    target: &DiagramNode,
    source_column: Option<&str>,
    target_column: Option<&str>,
    self_referential: bool,
) -> String {
    let from_y = row_y(source, source_column);
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
    let to_y = row_y(target, target_column);
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
    // The vertical run sits in the gap beside the target. Relationships that
    // skip a layout column then never run hidden behind an intermediate card,
    // and every relationship into one table shares a corridor that
    // `separate_lanes` can fan out.
    if source_right <= target_left {
        let middle = target_left - (target_left - source_right).min(HORIZONTAL_GAP) / 2.0;
        format!(
            "M {:.1} {:.1} H {:.1} V {:.1} H {:.1}",
            source_right, from_y, middle, to_y, target_left
        )
    } else {
        debug_assert!(target_right <= source.x);
        let middle = target_right + (source.x - target_right).min(HORIZONTAL_GAP) / 2.0;
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

/// Fan out relationships whose vertical runs share a corridor so parallel
/// lines never draw on top of one another.
fn separate_lanes(edges: &mut [DiagramEdge]) {
    let mut corridors = BTreeMap::<i64, Vec<usize>>::new();
    for (index, edge) in edges.iter().enumerate() {
        let points = &edge.points;
        if edge.self_referential
            || points.len() != 4
            || points[1].0 != points[2].0
            || points[0].1 != points[1].1
        {
            continue;
        }
        corridors
            .entry((points[1].0 * 10.0).round() as i64)
            .or_default()
            .push(index);
    }
    for members in corridors.into_values() {
        if members.len() < 2 {
            continue;
        }
        // Space available on either side of the corridor, bounded by the
        // narrowest gap between facing card borders in this group.
        let half_width = members
            .iter()
            .map(|&index| {
                let points = &edges[index].points;
                let corridor = points[1].0;
                (corridor - points[0].0)
                    .abs()
                    .min((points[3].0 - corridor).abs())
            })
            .fold(f32::INFINITY, f32::min)
            - 10.0;
        if half_width <= 0.0 {
            continue;
        }
        let mut ordered = members;
        ordered.sort_by(|&left, &right| {
            let (left, right) = (&edges[left].points, &edges[right].points);
            left[3]
                .1
                .total_cmp(&right[3].1)
                .then(left[0].1.total_cmp(&right[0].1))
        });
        let count = ordered.len() as f32;
        let spacing = LANE_SPACING.min(half_width * 2.0 / (count - 1.0));
        for (lane, index) in ordered.into_iter().enumerate() {
            let offset = (lane as f32 - (count - 1.0) / 2.0) * spacing;
            let points = &mut edges[index].points;
            points[1].0 += offset;
            points[2].0 += offset;
        }
    }
}

/// Draw an orthogonal polyline with rounded corners. Radii shrink on short
/// segments so a curve never overshoots its neighbours.
pub fn rounded_route(points: &[(f32, f32)], radius: f32) -> Vec<RouteStep> {
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut steps = vec![RouteStep::Move(first)];
    for index in 1..points.len() {
        let corner = points[index];
        let Some(&next) = points.get(index + 1) else {
            steps.push(RouteStep::Line(corner));
            break;
        };
        let previous = points[index - 1];
        let incoming = distance(previous, corner);
        let outgoing = distance(corner, next);
        let radius = radius.min(incoming / 2.0).min(outgoing / 2.0);
        if radius <= 0.5 {
            steps.push(RouteStep::Line(corner));
            continue;
        }
        steps.push(RouteStep::Line(toward(corner, previous, radius)));
        steps.push(RouteStep::Curve(toward(corner, next, radius), corner));
    }
    steps
}

fn rounded_svg_path(points: &[(f32, f32)], radius: f32) -> String {
    rounded_route(points, radius)
        .into_iter()
        .map(|step| match step {
            RouteStep::Move((x, y)) => format!("M {x:.1} {y:.1}"),
            RouteStep::Line((x, y)) => format!("L {x:.1} {y:.1}"),
            RouteStep::Curve((x, y), (cx, cy)) => format!("Q {cx:.1} {cy:.1} {x:.1} {y:.1}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn distance(from: (f32, f32), to: (f32, f32)) -> f32 {
    (to.0 - from.0).hypot(to.1 - from.1)
}

/// The point `length` along the segment from `from` toward `to`.
fn toward(from: (f32, f32), to: (f32, f32), length: f32) -> (f32, f32) {
    let total = distance(from, to);
    if total <= f32::EPSILON {
        return from;
    }
    let ratio = length / total;
    (
        from.0 + (to.0 - from.0) * ratio,
        from.1 + (to.1 - from.1) * ratio,
    )
}

/// Crow's-foot notation: a fork on the referencing (many) end, a double bar
/// on the referenced (exactly one) end, and a bar or ring beside the fork for
/// mandatory or optional participation.
pub fn edge_markers(points: &[(f32, f32)], optional: bool) -> EdgeMarkers {
    let mut markers = EdgeMarkers::default();
    let unit = |from: (f32, f32), to: (f32, f32)| {
        let length = distance(from, to);
        (length > f32::EPSILON).then(|| ((to.0 - from.0) / length, (to.1 - from.1) / length))
    };
    let along = |origin: (f32, f32), direction: (f32, f32), length: f32| {
        (
            origin.0 + direction.0 * length,
            origin.1 + direction.1 * length,
        )
    };
    let across = |origin: (f32, f32), direction: (f32, f32), half: f32| {
        let normal = (-direction.1, direction.0);
        [
            (origin.0 + normal.0 * half, origin.1 + normal.1 * half),
            (origin.0 - normal.0 * half, origin.1 - normal.1 * half),
        ]
    };

    if let (Some(&start), Some(&next)) = (points.first(), points.get(1))
        && let Some(direction) = unit(start, next)
    {
        let apex = along(start, direction, 11.0);
        let [left, right] = across(start, direction, 6.0);
        markers.lines.push([apex, left]);
        markers.lines.push([apex, right]);
        markers.lines.push([apex, start]);
        if optional {
            markers.ring = Some((along(start, direction, 16.0), 3.5));
        } else {
            let bar = along(start, direction, 15.0);
            markers.lines.push(across(bar, direction, 6.0));
        }
    }
    if points.len() >= 2 {
        let tip = points[points.len() - 1];
        let previous = points[points.len() - 2];
        if let Some(direction) = unit(tip, previous) {
            for offset in [6.0, 10.0] {
                markers
                    .lines
                    .push(across(along(tip, direction, offset), direction, 6.0));
            }
        }
    }
    markers
}

/// Compact, conventional spellings for verbose catalog type names.
pub fn short_type(data_type: &str) -> String {
    let trimmed = data_type.trim();
    let lower = trimmed.to_ascii_lowercase();
    const REPLACEMENTS: [(&str, &str); 9] = [
        ("timestamp with time zone", "timestamptz"),
        ("timestamp without time zone", "timestamp"),
        ("time with time zone", "timetz"),
        ("time without time zone", "time"),
        ("character varying", "varchar"),
        ("double precision", "float8"),
        ("bit varying", "varbit"),
        ("character", "char"),
        ("integer", "int"),
    ];
    for (verbose, short) in REPLACEMENTS {
        if let Some(rest) = lower.strip_prefix(verbose)
            && (rest.is_empty() || rest.starts_with(['(', '[', ' ']))
        {
            return format!("{short}{}", &trimmed[verbose.len()..]);
        }
    }
    trimmed.to_owned()
}

fn edge_path_points(path: &str) -> Vec<(f32, f32)> {
    let mut tokens = path.split_whitespace();
    let mut points = Vec::new();
    let mut current = (0.0, 0.0);

    while let Some(command) = tokens.next() {
        match command {
            "M" => {
                let Some(x) = tokens.next().and_then(|value| value.parse::<f32>().ok()) else {
                    break;
                };
                let Some(y) = tokens.next().and_then(|value| value.parse::<f32>().ok()) else {
                    break;
                };
                current = (x, y);
                points.push(current);
            }
            "H" => {
                let Some(x) = tokens.next().and_then(|value| value.parse::<f32>().ok()) else {
                    break;
                };
                current.0 = x;
                points.push(current);
            }
            "V" => {
                let Some(y) = tokens.next().and_then(|value| value.parse::<f32>().ok()) else {
                    break;
                };
                current.1 = y;
                points.push(current);
            }
            _ => break,
        }
    }

    points
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
        for pixel in row.as_chunks::<4>().0 {
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
            key: "#f59e0b",
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
        let path = edge_path(
            &source,
            &target,
            &foreign(&["parent_id"], "parent", &["id"]),
            false,
        );

        assert_eq!(path, "M 150.0 200.0 V 300.0");
        assert_eq!(
            edge_path_points(&path),
            vec![(150.0, 200.0), (150.0, 300.0)]
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
    fn crows_foot_marks_the_many_end_and_bars_the_one_end() {
        let points = [(100.0, 50.0), (60.0, 50.0), (60.0, 10.0), (0.0, 10.0)];
        let mandatory = edge_markers(&points, false);
        // Fork (three lines) plus a participation bar, then two "one" bars.
        assert_eq!(mandatory.lines.len(), 6);
        assert!(mandatory.ring.is_none());
        assert!(
            mandatory
                .lines
                .iter()
                .take(3)
                .all(|[apex, _]| *apex == (89.0, 50.0))
        );
        assert!(
            mandatory.lines[4..]
                .iter()
                .all(|[from, to]| from.0 == to.0 && from.0 < 11.0)
        );
        let optional = edge_markers(&points, true);
        assert_eq!(optional.lines.len(), 5);
        assert_eq!(optional.ring, Some(((84.0, 50.0), 3.5)));
    }

    #[test]
    fn shared_corridors_fan_out_into_separate_lanes() {
        let parent = table("parent", vec![column("id", 0, true)], vec![]);
        let children = ["a", "b", "c"].map(|name| {
            table(
                name,
                vec![column("id", 0, true), column("parent_id", 1, false)],
                vec![foreign(&["parent_id"], "parent", &["id"])],
            )
        });
        let mut tables = vec![parent];
        tables.extend(children);
        let document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables,
        });
        let corridors = document
            .edges
            .iter()
            .map(|edge| edge.points[1].0)
            .collect::<Vec<_>>();
        let mut distinct = corridors.clone();
        distinct.sort_by(f32::total_cmp);
        distinct.dedup();
        assert_eq!(corridors.len(), 3);
        assert_eq!(distinct.len(), 3, "{corridors:?}");
        assert!(document.edges.iter().all(|edge| edge.path.contains('Q')));
    }

    #[test]
    fn moving_a_card_reroutes_its_relationships() {
        let mut document = DiagramDocument::from_schema(&RelationalSchema {
            database: "db".into(),
            tables: vec![
                table("parent", vec![column("id", 0, true)], vec![]),
                table(
                    "child",
                    vec![column("id", 0, true), column("parent_id", 1, false)],
                    vec![foreign(&["parent_id"], "parent", &["id"])],
                ),
            ],
        });
        let before = document.edges[0].points.clone();
        assert!(document.move_node("public.parent", 900.0, 700.0));
        assert_eq!(
            document.node_position("public.parent"),
            Some((900.0, 700.0))
        );
        assert_ne!(document.edges[0].points, before);
        assert!(document.width >= 900.0 && document.height >= 700.0);
        assert!(!document.move_node("public.missing", 0.0, 0.0));
    }

    #[test]
    fn verbose_catalog_types_are_shortened_for_cards() {
        assert_eq!(short_type("timestamp with time zone"), "timestamptz");
        assert_eq!(short_type("character varying(160)"), "varchar(160)");
        assert_eq!(short_type("character(2)[]"), "char(2)[]");
        assert_eq!(short_type("integer[]"), "int[]");
        assert_eq!(short_type("interval"), "interval");
        assert_eq!(short_type("characters"), "characters");
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
