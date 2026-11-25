//! Additional visual drawings projected from one design.
//!
//! The module paints four schematic-adjacent drawings that reuse the shared
//! SVG primitives of the sheet projection: the net block diagram (components
//! as nodes, nets as routed polylines, layered left-to-right), the merged
//! multi-sheet overview (every sheet fitted into one grid cell with its own
//! separator frame and a single shared title block), the pinout chart (one
//! pin-to-net table block per component) and the symbol-library sheet (every
//! distinct symbol placed once on a grid). Every layout is a pure function of
//! the document: the same revision always paints byte-identical SVG.

use crate::{
    COMPONENT_HALF_WIDTH, Component, Design, PIN_LENGTH, Sheet, TITLE_BLOCK_HEIGHT, drawing_bounds, escape_xml,
    natural_compare, push_svg_component, push_svg_sheet_contents,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Pixels per design unit for standalone drawings, matching the sheet SVG.
const PIXELS_PER_UNIT: f64 = 8.0;

/// Opens one standalone SVG document sized in design units at the shared scale.
fn start_document(view_width: f64, view_height: f64) -> String {
    let mut svg = String::new();
    svg.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {:.2} {:.2}\" width=\"{:.0}\" height=\"{:.0}\">\n",
        view_width,
        view_height,
        (view_width * PIXELS_PER_UNIT).round(),
        (view_height * PIXELS_PER_UNIT).round(),
    ));
    svg.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#ffffff\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
        view_width, view_height
    ));
    svg
}

/// The net block diagram drawing and the counts it painted.
pub(crate) struct BlockDiagram {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Component nodes painted.
    pub(crate) nodes: usize,
    /// Net edge segments painted.
    pub(crate) edges: usize,
    /// Nets with at least two distinct component members.
    pub(crate) nets: usize,
}

/// Projects every component as a box and every multi-member net as a routed
/// polyline chain through its members.
///
/// Components are layered by breadth-first distance from the first unvisited
/// reference in natural order, so connected parts land in adjacent columns and
/// each net chain runs left-to-right with one elbow per segment. Same-column
/// segments route below both boxes, so no edge ever crosses a node body. The
/// net name is printed once, on the first segment of its chain.
pub(crate) fn render_block_diagram(design: &Design) -> BlockDiagram {
    let mut components: Vec<&Component> = design.sheets.iter().flat_map(|sheet| &sheet.components).collect();
    components.sort_by(|left, right| natural_compare(&left.reference, &right.reference));
    let known: BTreeSet<&str> = components.iter().map(|component| component.reference.as_str()).collect();

    let mut members: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for sheet in &design.sheets {
        for net in &sheet.nets {
            for node in &net.nodes {
                if known.contains(node.reference.as_str()) {
                    members.entry(net.name.as_str()).or_default().insert(node.reference.as_str());
                }
            }
        }
    }

    let mut adjacency: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for set in members.values() {
        for left in set {
            for right in set {
                if left != right {
                    adjacency.entry(*left).or_default().insert(*right);
                }
            }
        }
    }

    let mut layer: BTreeMap<&str, usize> = BTreeMap::new();
    for component in &components {
        let reference = component.reference.as_str();
        if layer.contains_key(reference) {
            continue;
        }
        let mut queue = VecDeque::new();
        layer.insert(reference, 0);
        queue.push_back(reference);
        while let Some(head) = queue.pop_front() {
            let depth = layer[head] + 1;
            if let Some(neighbours) = adjacency.get(head) {
                for &next in neighbours {
                    if known.contains(next) && !layer.contains_key(next) {
                        layer.insert(next, depth);
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    let mut columns: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for component in &components {
        columns.entry(layer[component.reference.as_str()]).or_default().push(component.reference.as_str());
    }
    let node_width = 30.0;
    let node_height = 12.0;
    let gap_x = 26.0;
    let gap_y = 10.0;
    let margin = 10.0;
    let column_count = columns.len().max(1);
    let row_count = columns.values().map(Vec::len).max().unwrap_or(1);
    let view_width = margin * 2.0 + column_count as f64 * node_width + column_count.saturating_sub(1) as f64 * gap_x;
    let view_height =
        margin * 2.0 + row_count as f64 * node_height + row_count.saturating_sub(1) as f64 * gap_y + TITLE_BLOCK_HEIGHT;

    let mut position: BTreeMap<&str, [f64; 2]> = BTreeMap::new();
    for (index, column) in &columns {
        for (row, reference) in column.iter().enumerate() {
            position.insert(
                *reference,
                [margin + *index as f64 * (node_width + gap_x), margin + row as f64 * (node_height + gap_y)],
            );
        }
    }

    let mut svg = start_document(view_width, view_height);
    svg.push_str("  <g data-block-diagram=\"true\">\n");
    let mut edges = 0usize;
    let mut nets = 0usize;
    for (net, set) in &members {
        let mut chain: Vec<&str> = set.iter().copied().collect();
        if chain.len() < 2 {
            continue;
        }
        chain.sort_by(|left, right| layer[left].cmp(&layer[right]).then_with(|| natural_compare(left, right)));
        nets += 1;
        for (segment, pair) in chain.windows(2).enumerate() {
            edges += 1;
            let left = position[pair[0]];
            let right = position[pair[1]];
            let left_layer = layer[pair[0]];
            let right_layer = layer[pair[1]];
            let (points, label_at) = if left_layer == right_layer {
                let below = left[1].max(right[1]) + node_height + 6.0;
                let from = [left[0] + node_width / 2.0, left[1] + node_height];
                let to = [right[0] + node_width / 2.0, right[1] + node_height];
                ([from, [from[0], below], [to[0], below], to], [(from[0] + to[0]) / 2.0, below - 1.2])
            }
            else {
                let (source, target) = if left_layer < right_layer { (left, right) } else { (right, left) };
                let from = [source[0] + node_width, source[1] + node_height / 2.0];
                let to = [target[0], target[1] + node_height / 2.0];
                let mid_x = (from[0] + to[0]) / 2.0;
                ([from, [mid_x, from[1]], [mid_x, to[1]], to], [mid_x, (from[1] + to[1]) / 2.0 - 1.2])
            };
            let rendered = points.iter().map(|point| format!("{:.2},{:.2}", point[0], point[1])).collect::<Vec<_>>().join(" ");
            svg.push_str(&format!(
                "    <polyline points=\"{rendered}\" fill=\"none\" stroke=\"#0f766e\" stroke-width=\"0.5\" data-net=\"{}\"/>\n",
                escape_xml(net)
            ));
            if segment == 0 {
                svg.push_str(&format!(
                    "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"middle\" fill=\"#64748b\" data-net-name=\"{}\">{}</text>\n",
                    label_at[0],
                    label_at[1],
                    escape_xml(net),
                    escape_xml(net)
                ));
            }
        }
    }
    for component in &components {
        let top = position[component.reference.as_str()];
        svg.push_str(&format!("    <g data-block=\"{}\">\n", escape_xml(&component.reference)));
        svg.push_str(&format!(
            "      <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{node_width:.2}\" height=\"{node_height:.2}\" rx=\"1.6\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n",
            top[0], top[1]
        ));
        svg.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.0\" text-anchor=\"middle\">{}</text>\n",
            top[0] + node_width / 2.0,
            top[1] + 5.0,
            escape_xml(&component.reference)
        ));
        svg.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"middle\" fill=\"#64748b\">{}</text>\n",
            top[0] + node_width / 2.0,
            top[1] + 9.6,
            escape_xml(&component.value)
        ));
        svg.push_str("    </g>\n");
    }
    svg.push_str("  </g>\n");
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    crate::push_svg_title_block(&mut svg, design, &sheets, 0.0, 0.0, view_width, view_height);
    svg.push_str("</svg>\n");
    BlockDiagram { svg, nodes: components.len(), edges, nets }
}

/// The merged multi-sheet overview drawing and the grid it painted.
pub(crate) struct SheetOverview {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Sheets placed on the overview.
    pub(crate) sheets: usize,
    /// Grid columns.
    pub(crate) columns: usize,
    /// Grid rows.
    pub(crate) rows: usize,
}

/// Places every sheet on one drawing in a square-ish grid.
///
/// Each cell carries a separator frame and the sheet name, and the sheet is
/// painted through the shared sheet painter inside a translate-and-scale group,
/// so the geometry is identical to the single-sheet SVG. One shared title
/// block describes the whole design at the bottom.
pub(crate) fn render_sheet_overview(design: &Design) -> SheetOverview {
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    let bounds: Vec<(f64, f64, f64, f64)> = sheets.iter().map(|sheet| drawing_bounds(std::iter::once(*sheet))).collect();
    let cell_padding = 8.0;
    let header_height = 6.0;
    let gap = 10.0;
    let margin = 10.0;
    let span_width = bounds.iter().map(|bound| bound.2 - bound.0).fold(1.0, f64::max);
    let span_height = bounds.iter().map(|bound| bound.3 - bound.1).fold(1.0, f64::max);
    let cell_width = span_width + 2.0 * cell_padding;
    let cell_height = span_height + 2.0 * cell_padding + header_height;
    let columns = (sheets.len() as f64).sqrt().ceil().max(1.0) as usize;
    let rows = sheets.len().div_ceil(columns).max(1);
    let view_width = margin * 2.0 + columns as f64 * cell_width + columns.saturating_sub(1) as f64 * gap;
    let view_height = margin * 2.0 + rows as f64 * cell_height + rows.saturating_sub(1) as f64 * gap + TITLE_BLOCK_HEIGHT;

    let mut svg = start_document(view_width, view_height);
    svg.push_str("  <g data-sheet-overview=\"true\">\n");
    for (index, sheet) in sheets.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_x = margin + column as f64 * (cell_width + gap);
        let cell_y = margin + row as f64 * (cell_height + gap);
        let (min_x, min_y, max_x, max_y) = bounds[index];
        let scale = ((cell_width - 2.0 * cell_padding) / (max_x - min_x).max(1.0))
            .min((cell_height - header_height - 2.0 * cell_padding) / (max_y - min_y).max(1.0))
            .min(1.0);
        let content_width = (max_x - min_x) * scale;
        let content_height = (max_y - min_y) * scale;
        let shift_x = cell_x + cell_padding + (cell_width - 2.0 * cell_padding - content_width) / 2.0 - min_x * scale;
        let shift_y =
            cell_y + header_height + cell_padding + (cell_height - header_height - 2.0 * cell_padding - content_height) / 2.0
                - min_y * scale;
        svg.push_str(&format!(
            "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#64748b\" stroke-width=\"0.4\" stroke-dasharray=\"2.4 1.6\"/>\n",
            cell_x, cell_y, cell_width, cell_height
        ));
        svg.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" fill=\"#1f2933\">{}</text>\n",
            cell_x + 2.0,
            cell_y + 4.4,
            escape_xml(&sheet.name)
        ));
        svg.push_str(&format!(
            "    <g data-sheet=\"{}\" transform=\"translate({:.2} {:.2}) scale({:.4})\">\n",
            escape_xml(&sheet.id),
            shift_x,
            shift_y,
            scale
        ));
        push_svg_sheet_contents(&mut svg, sheet);
        svg.push_str("    </g>\n");
    }
    svg.push_str("  </g>\n");
    crate::push_svg_title_block(&mut svg, design, &sheets, 0.0, 0.0, view_width, view_height);
    svg.push_str("</svg>\n");
    SheetOverview { svg, sheets: sheets.len(), columns, rows }
}

/// The pinout chart drawing and the rows it painted.
pub(crate) struct PinoutChart {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Component blocks painted.
    pub(crate) components: usize,
    /// Pin rows painted.
    pub(crate) rows: usize,
}

/// Paints one pin-to-net table block per component on a grid.
///
/// The nets come from the declared net nodes, so a pin lists every net name
/// that references it; a pin carrying a no-connect flag is marked `NC` in the
/// flag colour and an unconnected pin prints an em dash. Blocks sort by
/// natural reference order and flow into a square-ish grid of equal cells.
pub(crate) fn render_pinout_chart(design: &Design) -> PinoutChart {
    let mut components: Vec<&Component> = design.sheets.iter().flat_map(|sheet| &sheet.components).collect();
    components.sort_by(|left, right| natural_compare(&left.reference, &right.reference));

    let mut net_of: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
    let mut no_connects: BTreeSet<(&str, &str)> = BTreeSet::new();
    for sheet in &design.sheets {
        for net in &sheet.nets {
            for node in &net.nodes {
                net_of.entry((node.reference.as_str(), node.pin.as_str())).or_default().push(net.name.as_str());
            }
        }
        for flag in &sheet.no_connects {
            if let Some(node) = &flag.node {
                no_connects.insert((node.reference.as_str(), node.pin.as_str()));
            }
        }
    }

    let block_width = 52.0;
    let row_height = 4.4;
    let header_height = 6.0;
    let pin_column = 14.0;
    let gap = 8.0;
    let margin = 10.0;
    let tallest = components.iter().map(|component| component.pins.len()).max().unwrap_or(0);
    let cell_height = header_height + tallest as f64 * row_height + 4.0;
    let columns = (components.len() as f64).sqrt().ceil().max(1.0) as usize;
    let rows = components.len().div_ceil(columns).max(1);
    let view_width = margin * 2.0 + columns as f64 * (block_width + gap) - gap;
    let view_height = margin * 2.0 + rows as f64 * (cell_height + gap) - gap + TITLE_BLOCK_HEIGHT;

    let mut svg = start_document(view_width, view_height);
    svg.push_str("  <g data-pinout-chart=\"true\">\n");
    let mut painted_rows = 0usize;
    for (index, component) in components.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let block_x = margin + column as f64 * (block_width + gap);
        let block_y = margin + row as f64 * (cell_height + gap);
        svg.push_str(&format!("    <g data-ref=\"{}\">\n", escape_xml(&component.reference)));
        svg.push_str(&format!(
            "      <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{block_width:.2}\" height=\"{header_height:.2}\" fill=\"#eef4ff\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n",
            block_x, block_y
        ));
        svg.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.0\">{} {}</text>\n",
            block_x + 2.0,
            block_y + 4.2,
            escape_xml(&component.reference),
            escape_xml(&component.value)
        ));
        for (pin_index, pin) in component.pins.iter().enumerate() {
            let line_y = block_y + header_height + pin_index as f64 * row_height;
            painted_rows += 1;
            svg.push_str(&format!(
                "      <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#cbd5e1\" stroke-width=\"0.25\"/>\n",
                block_x,
                line_y + row_height,
                block_x + block_width,
                line_y + row_height
            ));
            svg.push_str(&format!("      <g data-pin=\"{}\">\n", escape_xml(pin)));
            svg.push_str(&format!(
                "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" text-anchor=\"end\">{}</text>\n",
                block_x + pin_column,
                line_y + 3.0,
                escape_xml(pin)
            ));
            let key = (component.reference.as_str(), pin.as_str());
            if no_connects.contains(&key) {
                svg.push_str(&format!(
                    "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" fill=\"#b91c1c\" data-no-connect=\"true\">NC</text>\n",
                    block_x + pin_column + 4.0,
                    line_y + 3.0
                ));
            }
            else if let Some(nets) = net_of.get(&key) {
                svg.push_str(&format!(
                    "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" fill=\"#0f766e\">{}</text>\n",
                    block_x + pin_column + 4.0,
                    line_y + 3.0,
                    escape_xml(&nets.join(", "))
                ));
            }
            else {
                svg.push_str(&format!(
                    "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" fill=\"#94a3b8\">{}</text>\n",
                    block_x + pin_column + 4.0,
                    line_y + 3.0,
                    escape_xml("—")
                ));
            }
            svg.push_str("      </g>\n");
        }
        svg.push_str("    </g>\n");
    }
    svg.push_str("  </g>\n");
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    crate::push_svg_title_block(&mut svg, design, &sheets, 0.0, 0.0, view_width, view_height);
    svg.push_str("</svg>\n");
    PinoutChart { svg, components: components.len(), rows: painted_rows }
}

/// The symbol-library drawing and the counts it painted.
pub(crate) struct SymbolLibrary {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Distinct symbols painted.
    pub(crate) symbols: usize,
    /// Components the symbols were deduplicated from.
    pub(crate) components: usize,
}

/// Paints every distinct symbol used by the design once, on a grid.
///
/// A symbol is identified by its reference prefix and its pin list, matching
/// the artwork the sheet projection selects; the first component in natural
/// reference order represents each symbol and is drawn upright at its cell
/// centre through the shared component painter.
pub(crate) fn render_symbol_library(design: &Design) -> SymbolLibrary {
    let mut components: Vec<&Component> = design.sheets.iter().flat_map(|sheet| &sheet.components).collect();
    components.sort_by(|left, right| natural_compare(&left.reference, &right.reference));

    let mut symbols: BTreeMap<String, &Component> = BTreeMap::new();
    for component in &components {
        let prefix = component
            .reference
            .chars()
            .find(|character| character.is_alphabetic())
            .map(|character| character.to_ascii_uppercase())
            .unwrap_or('?');
        let key = format!("{prefix}|{}", component.pins.join("|"));
        symbols.entry(key).or_insert(component);
    }

    let cell_width = COMPONENT_HALF_WIDTH * 2.0 + PIN_LENGTH * 2.0 + 20.0;
    let cell_height = 44.0;
    let gap = 6.0;
    let margin = 10.0;
    let columns = (symbols.len() as f64).sqrt().ceil().max(1.0) as usize;
    let rows = symbols.len().div_ceil(columns).max(1);
    let view_width = margin * 2.0 + columns as f64 * (cell_width + gap) - gap;
    let view_height = margin * 2.0 + rows as f64 * (cell_height + gap) - gap + TITLE_BLOCK_HEIGHT;

    let mut svg = start_document(view_width, view_height);
    svg.push_str("  <g data-symbol-library=\"true\">\n");
    for (index, (key, component)) in symbols.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_x = margin + column as f64 * (cell_width + gap);
        let cell_y = margin + row as f64 * (cell_height + gap);
        svg.push_str(&format!("    <g data-symbol=\"{}\">\n", escape_xml(key)));
        svg.push_str(&format!(
            "      <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#cbd5e1\" stroke-width=\"0.3\"/>\n",
            cell_x, cell_y, cell_width, cell_height
        ));
        let mut placed = (*component).clone();
        placed.x = cell_x + cell_width / 2.0;
        placed.y = cell_y + (cell_height - 6.0) / 2.0;
        placed.rotation = 0.0;
        placed.mirror = None;
        push_svg_component(&mut svg, &placed);
        svg.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"middle\" fill=\"#64748b\">{}</text>\n",
            cell_x + cell_width / 2.0,
            cell_y + cell_height - 2.0,
            escape_xml(&component.pins.join(", "))
        ));
        svg.push_str("    </g>\n");
    }
    svg.push_str("  </g>\n");
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    crate::push_svg_title_block(&mut svg, design, &sheets, 0.0, 0.0, view_width, view_height);
    svg.push_str("</svg>\n");
    SymbolLibrary { svg, symbols: symbols.len(), components: components.len() }
}

#[cfg(test)]
#[path = "../tests/unit/drawings.rs"]
mod tests;
