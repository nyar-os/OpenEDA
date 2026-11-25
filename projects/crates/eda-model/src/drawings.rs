//! Additional visual drawings projected from one design.
//!
//! Every drawing shares one page frame: an outer border, at least eight
//! percent of the page width as content padding, and a title bar carrying the
//! drawing name, drawing number, scale, unit and sheet count. Layouts are pure
//! functions of the document, so one revision always paints byte-identical
//! SVG. Text never rotates with a symbol, carries a white backing when it sits
//! over geometry, and labels are placed on measured column widths.

use crate::{
    COMPONENT_HALF_WIDTH, Component, Design, PIN_LENGTH, PinType, Sheet, bom_rows, drawing_bounds, escape_xml, natural_compare,
    push_svg_component_marked, push_svg_sheet_contents,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Pixels per design unit for standalone drawings, matching the sheet SVG.
const PIXELS_PER_UNIT: f64 = 8.0;

/// Height of the shared drawing title bar in design units.
const DRAWING_TITLE_HEIGHT: f64 = 16.0;

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

/// Estimated painted width of a monospace run: ASCII takes 0.6 em, wide glyphs
/// such as CJK take one em.
fn measure_text(text: &str, size: f64) -> f64 {
    text.chars().map(|character| if character.is_ascii() { 0.6 } else { 1.0 }).sum::<f64>() * size
}

/// Truncates text so a monospace run at `size` design units stays within
/// `max_units`, marking a cut with an ASCII ellipsis so no label can paint
/// over a neighbouring box or leave the canvas.
pub(crate) fn fit_text(text: &str, max_units: f64, size: f64) -> String {
    let max_chars = (max_units / (size * 0.6)).floor().max(0.0) as usize;
    let characters = text.chars().count();
    if characters <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let mut cut: String = text.chars().take(max_chars - 3).collect();
    cut.push_str("...");
    cut
}

/// Wraps text to `max_units`, breaking at spaces, commas or hard character
/// boundaries so a long net list stays inside its measured column.
fn wrap_text(text: &str, max_units: f64, size: f64) -> Vec<String> {
    let max_chars = (max_units / (size * 0.6)).floor().max(1.0) as usize;
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_inclusive([' ', ',']) {
        let candidate_len = current.chars().count() + word.chars().count();
        if !current.is_empty() && candidate_len > max_chars {
            lines.push(current.trim_end().to_string());
            current.clear();
        }
        if word.chars().count() > max_chars {
            for character in word.chars() {
                if current.chars().count() == max_chars {
                    lines.push(current.trim_end().to_string());
                    current.clear();
                }
                current.push(character);
            }
        }
        else {
            current.push_str(word);
        }
    }
    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }
    lines
}

/// One page frame sized so the content keeps an eight-percent side margin.
struct Page {
    width: f64,
    height: f64,
    pad: f64,
    title_top: f64,
}

/// Computes the page that holds a `content_width` x `content_height` drawing.
fn page_for(content_width: f64, content_height: f64) -> Page {
    let content_width = content_width.max(80.0);
    let content_height = content_height.max(40.0);
    let pad = content_width * 0.08 / 0.84;
    Page {
        width: content_width + 2.0 * pad,
        height: pad + content_height + pad + DRAWING_TITLE_HEIGHT,
        pad,
        title_top: pad + content_height + pad,
    }
}

/// Wraps one content group in a page frame and its title bar.
///
/// The title bar names the drawing and its number, scale, unit, revision and
/// the current sheet of the set. Every value comes from the document, so no
/// clock value can leak into the artifact.
#[allow(clippy::too_many_arguments)]
fn paint_page(
    content: &str,
    content_width: f64,
    content_height: f64,
    design: &Design,
    drawing: &str,
    code: &str,
    page_index: usize,
    page_total: usize,
) -> (String, f64, f64) {
    let page = page_for(content_width, content_height);
    let mut svg = start_document(page.width, page.height);
    svg.push_str(&format!(
        "  <g data-drawing=\"{}\" transform=\"translate({:.2} {:.2})\">\n",
        escape_xml(code),
        page.pad,
        page.pad
    ));
    svg.push_str(content);
    svg.push_str("  </g>\n");
    svg.push_str("  <g data-title-block=\"true\">\n");
    svg.push_str(&format!(
        "    <line x1=\"0.00\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
        page.title_top, page.width, page.title_top
    ));
    let columns = [0.42, 0.24, 0.16, 0.18];
    let mut cursor = 0.0;
    for fraction in &columns[..3] {
        cursor += page.width * fraction;
        svg.push_str(&format!(
            "    <line x1=\"{cursor:.2}\" y1=\"{:.2}\" x2=\"{cursor:.2}\" y2=\"{:.2}\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
            page.title_top, page.height
        ));
    }
    let cell_x = |index: usize| page.width * columns[..index].iter().sum::<f64>() + 1.6;
    let cell_width = |index: usize| page.width * columns[index] - 3.0;
    let title = design.title.as_deref().map(str::trim).filter(|text| !text.is_empty()).unwrap_or(design.id.as_str());
    let subtitle = {
        let mut parts = Vec::new();
        if !design.company.trim().is_empty() {
            parts.push(design.company.as_str());
        }
        if !design.date.trim().is_empty() {
            parts.push(design.date.as_str());
        }
        parts.join(" | ")
    };
    let rev = if design.rev.trim().is_empty() { "-" } else { design.rev.as_str() };
    let drawing_number = format!("DWG {}-{code}", design.id);
    let base = page.title_top;
    let mut text_at = |x: f64, y: f64, size: f64, weight: &str, value: &str| {
        svg.push_str(&format!(
            "    <text x=\"{x:.2}\" y=\"{y:.2}\" font-family=\"monospace\" font-size=\"{size:.1}\" fill=\"{weight}\">{}</text>\n",
            escape_xml(value)
        ));
    };
    text_at(cell_x(0), base + 5.2, 3.0, "#1f2933", &fit_text(title, cell_width(0), 3.0));
    if !subtitle.is_empty() {
        text_at(cell_x(0), base + 10.4, 2.2, "#64748b", &fit_text(&subtitle, cell_width(0), 2.2));
    }
    text_at(cell_x(1), base + 5.2, 2.8, "#1f2933", &fit_text(drawing, cell_width(1), 2.8));
    text_at(cell_x(1), base + 10.4, 2.2, "#64748b", &fit_text(&drawing_number, cell_width(1), 2.2));
    text_at(cell_x(2), base + 5.2, 2.4, "#1f2933", "SCALE 1:1");
    text_at(cell_x(2), base + 10.4, 2.2, "#64748b", &fit_text(&format!("UNIT {}", design.units), cell_width(2), 2.2));
    text_at(cell_x(3), base + 5.2, 2.4, "#1f2933", &format!("SHEET {page_index}/{page_total}"));
    text_at(cell_x(3), base + 10.4, 2.2, "#64748b", &fit_text(&format!("REV {rev}"), cell_width(3), 2.2));
    svg.push_str("  </g>\n");
    svg.push_str("</svg>\n");
    (svg, page.width, page.height)
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

/// One painted node box.
#[derive(Clone, Copy)]
struct NodeBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Projects every component as a box and every multi-member net as a routed
/// chain through its members.
///
/// Components are layered by breadth-first distance from the first unvisited
/// reference, then each layer is split into columns of at most six nodes so the
/// drawing stays landscape instead of one tall strand. Edges leave and enter a
/// box exactly on its edge and turn only in the channel between columns, so no
/// stroke crosses a node body. A net name is printed once per chain on the
/// first segment, beside the line, with a white backing.
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

    let node_width = components
        .iter()
        .map(|component| measure_text(&component.reference, 3.0).max(measure_text(&component.value, 2.4)) + 5.0)
        .fold(24.0, f64::max)
        .min(44.0);
    let node_height = 12.0;
    let gap_x = 20.0;
    let gap_y = 8.0;
    let max_rows = ((components.len() as f64).sqrt().ceil() as usize).clamp(3, 6);
    let mut columns: Vec<Vec<&str>> = Vec::new();
    let max_layer = layer.values().copied().max().unwrap_or(0);
    for depth in 0..=max_layer {
        let mut layer_nodes: Vec<&str> = components
            .iter()
            .filter(|component| layer[component.reference.as_str()] == depth)
            .map(|component| component.reference.as_str())
            .collect();
        layer_nodes.sort_by(|left, right| natural_compare(left, right));
        for chunk in layer_nodes.chunks(max_rows) {
            columns.push(chunk.to_vec());
        }
    }
    if columns.is_empty() && !components.is_empty() {
        columns.push(components.iter().map(|component| component.reference.as_str()).collect());
    }
    let column_count = columns.len().max(1);
    let row_count = columns.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let content_width = column_count as f64 * node_width + column_count.saturating_sub(1) as f64 * gap_x;
    let content_height = row_count as f64 * node_height + row_count.saturating_sub(1) as f64 * gap_y;

    let mut node_box: BTreeMap<&str, NodeBox> = BTreeMap::new();
    let mut node_column: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (column, references) in columns.iter().enumerate() {
        for (row, reference) in references.iter().enumerate() {
            node_box.insert(
                *reference,
                NodeBox {
                    x: column as f64 * (node_width + gap_x),
                    y: row as f64 * (node_height + gap_y),
                    width: node_width,
                    height: node_height,
                },
            );
            node_column.insert(*reference, (column, row));
        }
    }

    let mut content = String::new();
    content.push_str("  <g data-block-diagram=\"true\">\n");
    let mut edges = 0usize;
    let mut nets = 0usize;
    let obstacles: Vec<[f64; 4]> =
        node_box.values().map(|node| [node.x, node.y, node.x + node.width, node.y + node.height]).collect();
    let mut label_boxes: Vec<[f64; 4]> = Vec::new();
    for (net, set) in &members {
        let mut chain: Vec<&str> = set.iter().copied().collect();
        if chain.len() < 2 {
            continue;
        }
        chain.sort_by(|left, right| {
            node_column[left].0.cmp(&node_column[right].0).then_with(|| node_column[left].1.cmp(&node_column[right].1))
        });
        nets += 1;
        for (segment, pair) in chain.windows(2).enumerate() {
            let left = node_box[pair[0]];
            let right = node_box[pair[1]];
            let (left_column, _) = node_column[pair[0]];
            let (right_column, _) = node_column[pair[1]];
            let points: Vec<[f64; 2]> = if left_column == right_column {
                let (upper, lower) = if left.y <= right.y { (left, right) } else { (right, left) };
                vec![[upper.x + upper.width / 2.0, upper.y + upper.height], [lower.x + lower.width / 2.0, lower.y]]
            }
            else if left_column < right_column {
                let from = [left.x + left.width, left.y + left.height / 2.0];
                let to = [right.x, right.y + right.height / 2.0];
                let channel = left.x + left.width + gap_x / 2.0;
                vec![from, [channel, from[1]], [channel, to[1]], to]
            }
            else {
                let from = [left.x, left.y + left.height / 2.0];
                let to = [right.x + right.width, right.y + right.height / 2.0];
                let channel = left.x - gap_x / 2.0;
                vec![from, [channel, from[1]], [channel, to[1]], to]
            };
            let mut unique: Vec<[f64; 2]> = Vec::with_capacity(points.len());
            for point in points {
                if unique.last() != Some(&point) {
                    unique.push(point);
                }
            }
            let rendered = unique.iter().map(|point| format!("{:.2},{:.2}", point[0], point[1])).collect::<Vec<_>>().join(" ");
            content.push_str(&format!(
                "    <polyline points=\"{rendered}\" fill=\"none\" stroke=\"#0f766e\" stroke-width=\"0.5\" data-net=\"{}\"/>\n",
                escape_xml(net)
            ));
            edges += 1;
            if segment == 0
                && let Some(label_box) = place_wire_label(&unique, net, &obstacles, &label_boxes)
            {
                label_boxes.push(label_box.0);
                content.push_str(&format!(
                        "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#ffffff\" data-net-name-box=\"true\"/>\n",
                        label_box.0[0] - 0.4,
                        label_box.0[1] - 0.2,
                        label_box.0[2] - label_box.0[0] + 0.8,
                        label_box.0[3] - label_box.0[1] + 0.4
                    ));
                content.push_str(&format!(
                        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"middle\" fill=\"#0f766e\" data-net-name=\"{}\">{}</text>\n",
                        label_box.1,
                        label_box.2,
                        escape_xml(net),
                        escape_xml(net)
                    ));
            }
        }
    }
    for component in &components {
        let node = node_box[component.reference.as_str()];
        content.push_str(&format!("    <g data-block=\"{}\">\n", escape_xml(&component.reference)));
        content.push_str(&format!(
            "      <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"1.6\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n",
            node.x, node.y, node.width, node.height
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.0\" text-anchor=\"middle\">{}</text>\n",
            node.x + node.width / 2.0,
            node.y + 5.0,
            escape_xml(&fit_text(&component.reference, node.width - 2.0, 3.0))
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"middle\" fill=\"#64748b\">{}</text>\n",
            node.x + node.width / 2.0,
            node.y + 9.6,
            escape_xml(&fit_text(&component.value, node.width - 2.0, 2.4))
        ));
        content.push_str("    </g>\n");
    }
    if components.is_empty() {
        content.push_str(
            "    <text x=\"0.00\" y=\"8.00\" font-family=\"monospace\" font-size=\"3.0\" fill=\"#64748b\">no components</text>\n",
        );
    }
    content.push_str("  </g>\n");
    let (svg, _, _) =
        paint_page(&content, content_width.max(1.0), content_height.max(1.0), design, "BLOCK DIAGRAM", "BD", 1, 1);
    BlockDiagram { svg, nodes: components.len(), edges, nets }
}

/// Tries to place one net name beside a polyline without touching a node or an
/// already painted name. Returns the label box, its centre x and baseline y.
fn place_wire_label(
    points: &[[f64; 2]],
    text: &str,
    obstacles: &[[f64; 4]],
    labels: &[[f64; 4]],
) -> Option<([f64; 4], f64, f64)> {
    if points.len() < 2 {
        return None;
    }
    let (from, to) = (points[0], points[1]);
    let horizontal = (to[1] - from[1]).abs() < 0.01;
    let half = measure_text(text, 2.4) / 2.0;
    let candidate_box = |center_x: f64, baseline: f64| -> [f64; 4] {
        [center_x - half, baseline - 2.4 * 0.75, center_x + half, baseline + 2.4 * 0.25]
    };
    let overlaps = |candidate: [f64; 4]| {
        obstacles.iter().chain(labels.iter()).any(|box_min| {
            candidate[0] < box_min[2] && box_min[0] < candidate[2] && candidate[1] < box_min[3] && box_min[1] < candidate[3]
        })
    };
    let fractions = [0.5, 0.32, 0.68, 0.18, 0.82];
    for fraction in fractions {
        let (center_x, center_y) = if horizontal {
            (from[0] + (to[0] - from[0]) * fraction, from[1])
        }
        else {
            (from[0], from[1] + (to[1] - from[1]) * fraction)
        };
        let candidates = if horizontal {
            vec![(center_x, center_y - 1.0), (center_x, center_y + 2.8)]
        }
        else {
            vec![(center_x + half + 1.0, center_y + 0.8), (center_x - half - 1.0, center_y + 0.8)]
        };
        for (x, y) in candidates {
            let candidate = candidate_box(x, y);
            if !overlaps(candidate) && candidate[0] >= 0.0 {
                return Some((candidate, x, y));
            }
        }
    }
    // Keep the net named even in a dense drawing: the white backing keeps the
    // name legible, and the placement still sits beside the first segment.
    if horizontal {
        let x = from[0] + (to[0] - from[0]) * 0.5;
        let y = from[1] - 1.0;
        Some((candidate_box(x, y), x, y))
    }
    else {
        let x = from[0] + half + 1.0;
        let y = from[1] + (to[1] - from[1]) * 0.5 + 0.8;
        Some((candidate_box(x, y), x, y))
    }
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

/// Places every sheet on one drawing in a landscape grid.
///
/// Each cell carries a separator frame, the sheet name and its position in the
/// set. The sheet is painted through the shared sheet painter inside a
/// translate-and-scale group, so the geometry is identical to the single-sheet
/// SVG while staying inside its cell.
pub(crate) fn render_sheet_overview(design: &Design) -> SheetOverview {
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    let bounds: Vec<(f64, f64, f64, f64)> = sheets.iter().map(|sheet| drawing_bounds(std::iter::once(*sheet))).collect();
    let cell_padding = 7.0;
    let header_height = 6.0;
    let gap = 8.0;
    let span_width = bounds.iter().map(|bound| bound.2 - bound.0).fold(1.0, f64::max);
    let span_height = bounds.iter().map(|bound| bound.3 - bound.1 + 2.0).fold(1.0, f64::max);
    let cell_width = span_width + 2.0 * cell_padding;
    let cell_height = span_height + 2.0 * cell_padding + header_height;
    let columns = if sheets.len() <= 2 { sheets.len().max(1) } else { (sheets.len() as f64).sqrt().ceil() as usize };
    let rows = sheets.len().div_ceil(columns).max(1);
    let content_width = columns as f64 * cell_width + columns.saturating_sub(1) as f64 * gap;
    let content_height = rows as f64 * cell_height + rows.saturating_sub(1) as f64 * gap;

    let mut content = String::new();
    content.push_str("  <g data-sheet-overview=\"true\">\n");
    for (index, sheet) in sheets.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_x = column as f64 * (cell_width + gap);
        let cell_y = row as f64 * (cell_height + gap);
        let (min_x, min_y, max_x, max_y) = bounds[index];
        let span_x = (max_x - min_x).max(1.0);
        let span_y = (max_y - min_y + 2.0).max(1.0);
        let scale = ((cell_width - 2.0 * cell_padding) / span_x)
            .min((cell_height - header_height - 2.0 * cell_padding) / span_y)
            .min(1.0);
        let shift_x = cell_x + cell_padding + (cell_width - 2.0 * cell_padding - span_x * scale) / 2.0 - min_x * scale;
        let shift_y =
            cell_y + header_height + cell_padding + (cell_height - header_height - 2.0 * cell_padding - span_y * scale) / 2.0
                - min_y * scale;
        content.push_str(&format!(
            "    <rect x=\"{cell_x:.2}\" y=\"{cell_y:.2}\" width=\"{cell_width:.2}\" height=\"{cell_height:.2}\" fill=\"none\" stroke=\"#64748b\" stroke-width=\"0.4\" stroke-dasharray=\"2.4 1.6\"/>\n"
        ));
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" fill=\"#1f2933\">{}</text>\n",
            cell_x + 2.0,
            cell_y + 4.4,
            escape_xml(&fit_text(&sheet.name, cell_width - 20.0, 3.2))
        ));
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"end\" fill=\"#64748b\">{}/{}</text>\n",
            cell_x + cell_width - 2.0,
            cell_y + 4.4,
            index + 1,
            sheets.len()
        ));
        content.push_str(&format!(
            "    <g data-sheet=\"{}\" transform=\"translate({shift_x:.2} {shift_y:.2}) scale({scale:.4})\">\n",
            escape_xml(&sheet.id)
        ));
        push_svg_sheet_contents(&mut content, sheet);
        content.push_str("    </g>\n");
    }
    content.push_str("  </g>\n");
    let (svg, _, _) =
        paint_page(&content, content_width.max(1.0), content_height.max(1.0), design, "SHEET OVERVIEW", "SO", 1, 1);
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

/// One pin row prepared for layout.
struct PinRow {
    pin: String,
    nets: Vec<String>,
    no_connect: bool,
    height: f64,
}

/// One component block prepared for layout.
struct PinBlock {
    reference: String,
    value: String,
    rows: Vec<PinRow>,
    height: f64,
}

/// Paints one pin-to-net table block per component on a measured layout.
///
/// Column widths come from the longest pin name and the longest wrapped net
/// list, not from a fixed block width. Blocks are then packed into the column
/// count that gives the most balanced landscape page. A pin with no net leaves
/// its cell blank instead of filling the column with placeholder dashes.
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

    let pin_width =
        components.iter().flat_map(|component| &component.pins).map(|pin| measure_text(pin, 2.6)).fold(8.0, f64::max);
    let longest_net = components
        .iter()
        .flat_map(|component| {
            component.pins.iter().map(|pin| {
                net_of.get(&(component.reference.as_str(), pin.as_str())).map(|nets| nets.join(", ")).unwrap_or_default()
            })
        })
        .map(|text| measure_text(&text, 2.4).min(52.0))
        .fold(18.0, f64::max);
    let header_size = 3.0;
    let block_width = (pin_width + longest_net + 10.0).max(measure_text("REF VALUE", header_size) + 3.0).max(
        components
            .iter()
            .map(|component| measure_text(&format!("{} {}", component.reference, component.value), header_size) + 7.0)
            .fold(44.0, f64::max),
    );
    let mut blocks: Vec<PinBlock> = Vec::new();
    let mut total_rows = 0usize;
    for component in &components {
        let mut rows = Vec::new();
        for pin in &component.pins {
            let key = (component.reference.as_str(), pin.as_str());
            let nets: Vec<String> =
                net_of.get(&key).map(|nets| nets.iter().map(|net| (*net).to_string()).collect()).unwrap_or_default();
            let lines = if no_connects.contains(&key) || nets.is_empty() {
                1
            }
            else {
                wrap_text(&nets.join(", "), block_width - pin_width - 7.0, 2.4).len()
            };
            rows.push(PinRow {
                pin: pin.clone(),
                nets,
                no_connect: no_connects.contains(&key),
                height: (lines as f64 * 3.0 + 1.8).max(4.4),
            });
            total_rows += 1;
        }
        let body: f64 = rows.iter().map(|row| row.height).sum();
        blocks.push(PinBlock {
            reference: component.reference.clone(),
            value: component.value.clone(),
            rows,
            height: 5.5 + 4.5 + body + 1.0,
        });
    }
    let best = (1..=blocks.len().clamp(1, 6))
        .map(|columns| {
            let mut heights = vec![0.0f64; columns];
            let mut counts = vec![0usize; columns];
            for block in &blocks {
                let target = (0..columns).min_by(|a, b| heights[*a].total_cmp(&heights[*b])).unwrap_or(0);
                heights[target] += block.height + 4.0;
                counts[target] += 1;
            }
            let width = columns as f64 * block_width + columns.saturating_sub(1) as f64 * 6.0;
            let height = heights.iter().copied().fold(1.0, f64::max) - 4.0;
            let score = (width / height - 1.4).abs();
            (columns, counts, width, height, score)
        })
        .min_by(|left, right| left.4.total_cmp(&right.4))
        .unwrap_or((1, vec![blocks.len()], block_width, 10.0, 0.0));
    let (columns, counts, content_width, content_height) = (best.0, best.1, best.2, best.3);
    let mut cursors = vec![0.0f64; columns];
    let mut content = String::new();
    content.push_str("  <g data-pinout-chart=\"true\">\n");
    let mut placed = 0usize;
    for (column, count) in counts.iter().enumerate() {
        for _ in 0..*count {
            let Some(block) = blocks.get(placed)
            else {
                break;
            };
            let block_x = column as f64 * (block_width + 6.0);
            let block_y = cursors[column];
            content.push_str(&format!("    <g data-ref=\"{}\">\n", escape_xml(&block.reference)));
            content.push_str(&format!(
                "      <rect x=\"{block_x:.2}\" y=\"{block_y:.2}\" width=\"{block_width:.2}\" height=\"5.5\" fill=\"#eef4ff\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n"
            ));
            content.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.0\">{}</text>\n",
                block_x + 2.0,
                block_y + 4.0,
                escape_xml(&fit_text(&format!("{} {}", block.reference, block.value), block_width - 4.0, 3.0))
            ));
            content.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"end\" fill=\"#64748b\">PIN</text>\n",
                block_x + 3.0 + pin_width,
                block_y + 8.8
            ));
            content.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" fill=\"#64748b\">NET</text>\n",
                block_x + 6.0 + pin_width,
                block_y + 8.8
            ));
            content.push_str(&format!(
                "      <line x1=\"{block_x:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#1f6feb\" stroke-width=\"0.3\"/>\n",
                block_y + 10.0,
                block_x + block_width,
                block_y + 10.0
            ));
            let mut row_y = block_y + 10.0;
            for row in &block.rows {
                content.push_str(&format!("      <g data-pin=\"{}\">\n", escape_xml(&row.pin)));
                content.push_str(&format!(
                    "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" text-anchor=\"end\">{}</text>\n",
                    block_x + 3.0 + pin_width,
                    row_y + 3.2,
                    escape_xml(&fit_text(&row.pin, pin_width, 2.6))
                ));
                if row.no_connect {
                    content.push_str(&format!(
                        "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" fill=\"#b91c1c\" data-no-connect=\"true\">NC</text>\n",
                        block_x + 6.0 + pin_width,
                        row_y + 3.2
                    ));
                }
                else if !row.nets.is_empty() {
                    let lines = wrap_text(&row.nets.join(", "), block_width - pin_width - 7.0, 2.4);
                    for (line, text) in lines.iter().enumerate() {
                        content.push_str(&format!(
                            "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" fill=\"#0f766e\">{}</text>\n",
                            block_x + 6.0 + pin_width,
                            row_y + 3.2 + line as f64 * 3.0,
                            escape_xml(text)
                        ));
                    }
                }
                content.push_str("      </g>\n");
                row_y += row.height;
                content.push_str(&format!(
                    "      <line x1=\"{block_x:.2}\" y1=\"{row_y:.2}\" x2=\"{:.2}\" y2=\"{row_y:.2}\" stroke=\"#cbd5e1\" stroke-width=\"0.25\"/>\n",
                    block_x + block_width
                ));
            }
            content.push_str(&format!(
                "      <rect x=\"{block_x:.2}\" y=\"{block_y:.2}\" width=\"{block_width:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n",
                block.height
            ));
            content.push_str("    </g>\n");
            cursors[column] += block.height + 4.0;
            placed += 1;
        }
    }
    content.push_str("  </g>\n");
    let (svg, _, _) = paint_page(&content, content_width.max(1.0), content_height.max(1.0), design, "PINOUT CHART", "PO", 1, 1);
    PinoutChart { svg, components: components.len(), rows: total_rows }
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
/// centre.
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

    let cell_width = COMPONENT_HALF_WIDTH * 2.0 + PIN_LENGTH * 2.0 + 22.0;
    let cell_height = 48.0;
    let gap = 6.0;
    let columns = (symbols.len() as f64).sqrt().ceil().max(1.0) as usize;
    let rows = symbols.len().div_ceil(columns).max(1);
    let content_width = columns as f64 * cell_width + columns.saturating_sub(1) as f64 * gap;
    let content_height = rows as f64 * cell_height + rows.saturating_sub(1) as f64 * gap;
    let no_dots = BTreeSet::new();

    let mut content = String::new();
    content.push_str("  <g data-symbol-library=\"true\">\n");
    for (index, (key, component)) in symbols.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_x = column as f64 * (cell_width + gap);
        let cell_y = row as f64 * (cell_height + gap);
        content.push_str(&format!("    <g data-symbol=\"{}\">\n", escape_xml(key)));
        content.push_str(&format!(
            "      <rect x=\"{cell_x:.2}\" y=\"{cell_y:.2}\" width=\"{cell_width:.2}\" height=\"{cell_height:.2}\" fill=\"none\" stroke=\"#cbd5e1\" stroke-width=\"0.3\"/>\n"
        ));
        let mut placed = (*component).clone();
        placed.x = cell_x + cell_width / 2.0;
        placed.y = cell_y + (cell_height - 8.0) / 2.0;
        placed.rotation = 0.0;
        placed.mirror = None;
        push_svg_component_marked(&mut content, &placed, Some(&no_dots));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"middle\" fill=\"#64748b\">{}</text>\n",
            cell_x + cell_width / 2.0,
            cell_y + cell_height - 2.0,
            escape_xml(&fit_text(&component.pins.join(", "), cell_width - 4.0, 2.2))
        ));
        content.push_str("    </g>\n");
    }
    content.push_str("  </g>\n");
    let (svg, _, _) =
        paint_page(&content, content_width.max(1.0), content_height.max(1.0), design, "SYMBOL LIBRARY", "SL", 1, 1);
    SymbolLibrary { svg, symbols: symbols.len(), components: components.len() }
}

/// The net topology drawing and the counts it painted.
pub(crate) struct NetTopology {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Component nodes painted.
    pub(crate) components: usize,
    /// Net nodes painted.
    pub(crate) nets: usize,
    /// Component-to-net incidence edges painted.
    pub(crate) edges: usize,
}

/// Paints the component/net incidence graph: every component box in a left
/// column, every net node in a right column, one elbow edge from each member
/// component to its net node.
pub(crate) fn render_net_topology(design: &Design) -> NetTopology {
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
    let mut net_names: Vec<&str> = members.keys().copied().collect();
    net_names.sort_by(|left, right| natural_compare(left, right));

    let comp_w = components.iter().map(|component| measure_text(&component.reference, 2.6)).fold(16.0, f64::max) + 6.0;
    let comp_h = 8.0;
    let comp_pitch = 12.0;
    let net_h = 6.0;
    let net_pitch = 10.0;
    let heading_height = 6.0;
    let net_gap = 44.0;
    let label_width = net_names.iter().map(|name| measure_text(name, 2.4)).fold(12.0, f64::max).min(48.0);
    let net_x = comp_w + net_gap;
    let net_w = 16.0;
    let content_width = net_x + net_w + 4.0 + label_width;
    let comp_bottom = components.len().saturating_sub(1) as f64 * comp_pitch + comp_h;
    let net_bottom = net_names.len().saturating_sub(1) as f64 * net_pitch + net_h;
    let content_height = heading_height + comp_bottom.max(net_bottom).max(20.0);

    let mut content = String::new();
    content.push_str("  <g data-net-topology=\"true\">\n");
    for (text, x) in [("COMPONENTS", 0.0), ("NETS", net_x)] {
        content.push_str(&format!(
            "    <text x=\"{x:.2}\" y=\"3.00\" font-family=\"monospace\" font-size=\"2.6\" fill=\"#64748b\">{text}</text>\n"
        ));
    }
    let mut edges = 0usize;
    for (net_index, name) in net_names.iter().enumerate() {
        let net_y = heading_height + net_index as f64 * net_pitch;
        let lane_x = net_x - 14.0 - (net_index % 8) as f64 * 1.4;
        for reference in members[*name].iter() {
            let row = components.iter().position(|component| component.reference == *reference).expect("known member");
            let from = [comp_w, heading_height + row as f64 * comp_pitch + comp_h / 2.0];
            let to = [net_x, net_y + net_h / 2.0];
            let points = [from, [lane_x, from[1]], [lane_x, to[1]], to];
            let rendered = points.iter().map(|point| format!("{:.2},{:.2}", point[0], point[1])).collect::<Vec<_>>().join(" ");
            content.push_str(&format!(
                "    <polyline points=\"{rendered}\" fill=\"none\" stroke=\"#0f766e\" stroke-width=\"0.4\" data-net=\"{}\"/>\n",
                escape_xml(name)
            ));
            edges += 1;
        }
        content.push_str(&format!(
            "    <rect x=\"{net_x:.2}\" y=\"{net_y:.2}\" width=\"{net_w:.2}\" height=\"{net_h:.2}\" rx=\"1.2\" fill=\"#f0fdfa\" stroke=\"#0f766e\" stroke-width=\"0.4\" data-net-node=\"{}\"/>\n",
            escape_xml(name)
        ));
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" fill=\"#0f766e\">{}</text>\n",
            net_x + net_w + 2.0,
            net_y + 4.4,
            escape_xml(&fit_text(name, label_width, 2.4))
        ));
    }
    for (index, component) in components.iter().enumerate() {
        let y = heading_height + index as f64 * comp_pitch;
        content.push_str(&format!("    <g data-block=\"{}\">\n", escape_xml(&component.reference)));
        content.push_str(&format!(
            "      <rect x=\"0.00\" y=\"{y:.2}\" width=\"{comp_w:.2}\" height=\"{comp_h:.2}\" rx=\"1.6\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n"
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" text-anchor=\"middle\">{}</text>\n",
            comp_w / 2.0,
            y + 5.4,
            escape_xml(&fit_text(&component.reference, comp_w - 2.0, 2.6))
        ));
        content.push_str("    </g>\n");
    }
    if components.is_empty() && net_names.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.0\" text-anchor=\"middle\" fill=\"#64748b\">no components or nets</text>\n",
            content_width / 2.0,
            heading_height + 8.0
        ));
    }
    content.push_str("  </g>\n");
    let (svg, _, _) = paint_page(&content, content_width.max(1.0), content_height.max(1.0), design, "NET TOPOLOGY", "NT", 1, 1);
    NetTopology { svg, components: components.len(), nets: net_names.len(), edges }
}

/// Which rail class a net belongs to in the power-distribution map.
enum NetRail {
    Power,
    Ground,
    Signal,
}

/// True for compact voltage-shaped names such as `5V`, `3V3` or `1V8`.
fn is_voltage_rail(text: &str) -> bool {
    let Some(at) = text.find('V')
    else {
        return false;
    };
    let (head, tail) = (&text[..at], &text[at + 1..]);
    !head.is_empty()
        && head.chars().all(|character| character.is_ascii_digit())
        && tail.chars().all(|character| character.is_ascii_digit())
}

/// Classifies a net by name convention, falling back to the power pin types
/// of its members. Ground names win over power names.
fn classify_net(name: &str, has_power_pin: bool) -> NetRail {
    let upper: String = name.chars().filter(|character| !character.is_whitespace()).collect::<String>().to_uppercase();
    if upper == "GND"
        || upper == "VSS"
        || upper == "VEE"
        || upper == "0V"
        || upper == "AGND"
        || upper == "PGND"
        || upper == "EARTH"
        || upper.ends_with("GND")
        || upper.ends_with("VSS")
        || upper.ends_with("VEE")
    {
        return NetRail::Ground;
    }
    if upper == "VCC"
        || upper == "VDD"
        || upper == "VAA"
        || upper == "VBAT"
        || upper == "VBUS"
        || upper == "VIN"
        || upper == "PWR"
        || upper.starts_with("VCC")
        || upper.starts_with("VDD")
        || is_voltage_rail(upper.trim_start_matches('+'))
    {
        return NetRail::Power;
    }
    if has_power_pin { NetRail::Power } else { NetRail::Signal }
}

/// The power-distribution drawing and the counts it painted.
pub(crate) struct PowerDistribution {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Power rails painted.
    pub(crate) power_nets: usize,
    /// Ground rails painted.
    pub(crate) ground_nets: usize,
    /// Component boxes hung on the rails.
    pub(crate) components: usize,
    /// Component-to-rail connections painted.
    pub(crate) connections: usize,
}

/// Paints every power and ground net as a horizontal rail bar and hangs each
/// member component between the rails with a lane-routed drop line.
///
/// Rail connections use a vertical lane beside the component column and a short
/// stub into the box edge, so a drop line never crosses a symbol body. Rails
/// come from the net-name convention (`VCC`, `3V3`, `GND`, `VSS`, voltages)
/// plus `PowerIn`/`PowerOut` pin types.
pub(crate) fn render_power_distribution(design: &Design) -> PowerDistribution {
    let mut components: Vec<&Component> = design.sheets.iter().flat_map(|sheet| &sheet.components).collect();
    components.sort_by(|left, right| natural_compare(&left.reference, &right.reference));
    let known: BTreeMap<&str, &Component> =
        components.iter().map(|component| (component.reference.as_str(), *component)).collect();

    let mut members: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut power_pins: BTreeSet<&str> = BTreeSet::new();
    for sheet in &design.sheets {
        for net in &sheet.nets {
            for node in &net.nodes {
                if let Some(component) = known.get(node.reference.as_str()) {
                    members.entry(net.name.as_str()).or_default().insert(node.reference.as_str());
                    if matches!(component.pin_types.get(&node.pin), Some(PinType::PowerIn | PinType::PowerOut)) {
                        power_pins.insert(net.name.as_str());
                    }
                }
            }
        }
    }
    let mut power: Vec<&str> = Vec::new();
    let mut ground: Vec<&str> = Vec::new();
    for name in members.keys() {
        match classify_net(name, power_pins.contains(name)) {
            NetRail::Power => power.push(name),
            NetRail::Ground => ground.push(name),
            NetRail::Signal => {}
        }
    }
    for list in [&mut power, &mut ground] {
        list.sort_by(|left, right| natural_compare(left, right));
    }

    let rail_h = 5.0;
    let rail_pitch = 8.0;
    let anchor_h = 6.0;
    let per_row = components.len().clamp(1, 5);
    let box_w = components.iter().map(|component| measure_text(&component.reference, 2.4)).fold(14.0, f64::max) + 6.0;
    let box_h = 8.0;
    let pitch_x = box_w + 8.0;
    let lane_room = 8.0;
    let content_width = per_row as f64 * pitch_x + lane_room;
    let rows = components.len().div_ceil(per_row).max(1);
    let comp_top = anchor_h + power.len() as f64 * rail_pitch + 4.0;
    let comp_bottom = comp_top + (rows - 1) as f64 * 15.0 + box_h;
    let ground_top = comp_bottom + 10.0;
    let content_height = ground_top + ground.len().saturating_sub(1) as f64 * rail_pitch + rail_h + 2.0;

    let mut content = String::new();
    content.push_str("  <g data-power-distribution=\"true\">\n");
    let mut connections = 0usize;
    for (class, list) in [("power", &power), ("ground", &ground)] {
        for (index, name) in list.iter().enumerate() {
            let rail_y =
                if class == "power" { anchor_h + index as f64 * rail_pitch } else { ground_top + index as f64 * rail_pitch };
            for reference in members[*name].iter() {
                let row = components.iter().position(|component| component.reference == *reference).expect("known member");
                let column = row % per_row;
                let grid_row = row / per_row;
                let box_x = column as f64 * pitch_x;
                let box_y = comp_top + grid_row as f64 * 15.0;
                let lane_x = box_x + box_w + 2.0 + (grid_row % 4) as f64 * 1.2;
                let points = if class == "power" {
                    vec![
                        [lane_x, rail_y + rail_h],
                        [lane_x, box_y - 2.0],
                        [box_x + box_w / 2.0, box_y - 2.0],
                        [box_x + box_w / 2.0, box_y],
                    ]
                }
                else {
                    vec![
                        [box_x + box_w / 2.0, box_y + box_h],
                        [box_x + box_w / 2.0, box_y + box_h + 2.0],
                        [lane_x, box_y + box_h + 2.0],
                        [lane_x, rail_y],
                    ]
                };
                let rendered =
                    points.iter().map(|point| format!("{:.2},{:.2}", point[0], point[1])).collect::<Vec<_>>().join(" ");
                content.push_str(&format!(
                    "    <polyline points=\"{rendered}\" fill=\"none\" stroke=\"#0f766e\" stroke-width=\"0.4\" data-rail=\"{class}\" data-net=\"{}\"/>\n",
                    escape_xml(name)
                ));
                connections += 1;
            }
        }
    }
    for (class, list, fill, stroke, text) in
        [("power", &power, "#fef3c7", "#b45309", "#92400e"), ("ground", &ground, "#e5e7eb", "#1f2933", "#1f2933")]
    {
        for (index, name) in list.iter().enumerate() {
            let y =
                if class == "power" { anchor_h + index as f64 * rail_pitch } else { ground_top + index as f64 * rail_pitch };
            content.push_str(&format!(
                "    <rect x=\"0.00\" y=\"{y:.2}\" width=\"{content_width:.2}\" height=\"{rail_h:.2}\" rx=\"1.2\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"0.4\" data-rail=\"{class}\" data-net=\"{}\"/>\n",
                escape_xml(name)
            ));
            content.push_str(&format!(
                "    <text x=\"2.00\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" fill=\"{text}\">{}</text>\n",
                y + 3.8,
                escape_xml(&fit_text(name, content_width - 4.0, 2.4))
            ));
        }
    }
    for (index, component) in components.iter().enumerate() {
        let x = (index % per_row) as f64 * pitch_x;
        let y = comp_top + (index / per_row) as f64 * 15.0;
        content.push_str(&format!("    <g data-block=\"{}\">\n", escape_xml(&component.reference)));
        content.push_str(&format!(
            "      <rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{box_w:.2}\" height=\"{box_h:.2}\" rx=\"1.4\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n"
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"middle\">{}</text>\n",
            x + box_w / 2.0,
            y + 5.2,
            escape_xml(&fit_text(&component.reference, box_w - 2.0, 2.4))
        ));
        content.push_str("    </g>\n");
    }
    if power.is_empty() && ground.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.0\" text-anchor=\"middle\" fill=\"#64748b\">no power or ground nets detected</text>\n",
            content_width / 2.0,
            comp_bottom + 6.0
        ));
    }
    content.push_str("  </g>\n");
    let (svg, _, _) =
        paint_page(&content, content_width.max(1.0), content_height.max(1.0), design, "POWER DISTRIBUTION", "PD", 1, 1);
    PowerDistribution { svg, power_nets: power.len(), ground_nets: ground.len(), components: components.len(), connections }
}

/// The bill-of-materials table drawing and the rows it painted.
pub(crate) struct BomDrawing {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Grouped rows painted.
    pub(crate) rows: usize,
    /// Components covered by the rows.
    pub(crate) components: usize,
}

/// One outlined table section before pagination.
struct TableSection {
    caption: String,
    rows: Vec<Vec<String>>,
    row_heights: Vec<f64>,
}

/// Splits prepared rows into sections of at most `max_body` units.
fn paginate_rows(rows: Vec<Vec<String>>, heights: Vec<f64>, max_body: f64, caption: &str) -> Vec<TableSection> {
    let mut sections = Vec::new();
    let mut current_rows = Vec::new();
    let mut current_heights = Vec::new();
    let mut used = 0.0;
    for (row, height) in rows.into_iter().zip(heights) {
        if !current_rows.is_empty() && used + height > max_body {
            sections.push(TableSection {
                caption: String::new(),
                rows: std::mem::take(&mut current_rows),
                row_heights: std::mem::take(&mut current_heights),
            });
            used = 0.0;
        }
        used += height;
        current_rows.push(row);
        current_heights.push(height);
    }
    if current_rows.is_empty() {
        current_rows.push(vec![String::from("(no rows)")]);
        current_heights.push(6.0);
    }
    sections.push(TableSection { caption: String::new(), rows: current_rows, row_heights: current_heights });
    let total = sections.len();
    for (index, section) in sections.iter_mut().enumerate() {
        section.caption = if index == 0 {
            format!("{caption}  |  PAGE 1 OF {total}")
        }
        else {
            format!("{caption} (CONTINUED)  |  PAGE {} OF {total}", index + 1)
        };
    }
    sections
}

/// Paints a ruled table section list and returns the content and its size.
fn paint_table(
    sections: &[TableSection],
    columns: &[f64],
    headers: &[&str],
    size: f64,
    align_right: &[usize],
) -> (String, f64, f64) {
    let width: f64 = columns.iter().sum();
    let mut content = String::new();
    content.push_str("  <g data-table=\"true\">\n");
    let mut y = 0.0;
    for section in sections {
        content.push_str(&format!(
            "    <text x=\"0.00\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.8\" fill=\"#1f2933\">{}</text>\n",
            y + 4.0,
            escape_xml(&section.caption)
        ));
        y += 6.0;
        content.push_str(&format!(
            "    <rect x=\"0.00\" y=\"{y:.2}\" width=\"{width:.2}\" height=\"6.5\" fill=\"#eef4ff\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n"
        ));
        let mut x = 0.0;
        for (index, header) in headers.iter().enumerate() {
            let text_x = if align_right.contains(&index) { x + columns[index] - 1.5 } else { x + 1.5 };
            let anchor = if align_right.contains(&index) { "end" } else { "start" };
            content.push_str(&format!(
                "    <text x=\"{text_x:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"{anchor}\" fill=\"#1f2933\">{header}</text>\n",
                y + 4.4
            ));
            x += columns[index];
        }
        y += 6.5;
        let body_top = y;
        for (row, height) in section.rows.iter().zip(&section.row_heights) {
            let mut x = 0.0;
            for (index, cell) in row.iter().enumerate() {
                let text_x = if align_right.contains(&index) { x + columns[index] - 1.5 } else { x + 1.5 };
                let anchor = if align_right.contains(&index) { "end" } else { "start" };
                for (line, text) in cell.split('\n').enumerate() {
                    content.push_str(&format!(
                        "    <text x=\"{text_x:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"{size:.1}\" text-anchor=\"{anchor}\" fill=\"#334155\" data-table-cell=\"true\">{}</text>\n",
                        y + size + 0.4 + line as f64 * 3.2,
                        escape_xml(text)
                    ));
                }
                x += columns[index];
            }
            y += height;
            content.push_str(&format!(
                "    <line x1=\"0.00\" y1=\"{y:.2}\" x2=\"{width:.2}\" y2=\"{y:.2}\" stroke=\"#cbd5e1\" stroke-width=\"0.25\"/>\n"
            ));
        }
        let mut x = 0.0;
        for column in columns.iter().take(columns.len().saturating_sub(1)) {
            x += column;
            content.push_str(&format!(
                "    <line x1=\"{x:.2}\" y1=\"{body_top:.2}\" x2=\"{x:.2}\" y2=\"{y:.2}\" stroke=\"#1f6feb\" stroke-width=\"0.3\"/>\n"
            ));
        }
        content.push_str(&format!(
            "    <rect x=\"0.00\" y=\"{body_top:.2}\" width=\"{width:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n",
            y - body_top
        ));
        y += 4.0;
    }
    content.push_str("  </g>\n");
    (content, width, y - 4.0)
}

/// Paints the grouped bill of materials as ruled, paginated table sections.
///
/// Rows come from the shared BOM grouping: one row per value-and-footprint
/// pair, designators compressed into ranges. Column widths follow the measured
/// content, and a section too tall for one page continues under a `(CONTINUED)`
/// caption with its own page number.
pub(crate) fn render_bom_drawing(design: &Design) -> BomDrawing {
    let rows = bom_rows(design, None);
    let designators: Vec<String> = rows.iter().map(|row| crate::designator_ranges(&row.designators).join(", ")).collect();
    let comment_width = rows
        .iter()
        .map(|row| measure_text(&row.comment, 2.4))
        .chain(designators.iter().map(|text| measure_text(text, 2.4)))
        .fold(16.0, f64::max);
    let footprint_width = rows.iter().map(|row| measure_text(&row.footprint, 2.4)).fold(14.0, f64::max);
    let columns = [
        6.0,
        (comment_width + 4.0).clamp(16.0, 64.0),
        (footprint_width + 4.0).clamp(14.0, 46.0),
        (designators.iter().map(|text| measure_text(text, 2.4)).fold(20.0, f64::max) + 4.0).clamp(24.0, 96.0),
        10.0,
    ];
    let mut table_rows = Vec::new();
    let mut row_heights = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let designator_lines = wrap_text(&designators[index], columns[3] - 3.0, 2.4);
        let cells = [
            format!("{}", index + 1),
            fit_text(&row.comment, columns[1] - 3.0, 2.4),
            fit_text(&row.footprint, columns[2] - 3.0, 2.4),
            designator_lines.join("\n"),
            format!("{}", row.designators.len()),
        ];
        row_heights.push((designator_lines.len() as f64 * 3.2 + 3.0).max(6.0));
        table_rows.push(cells.to_vec());
    }
    let sections = paginate_rows(table_rows, row_heights, 132.0, "BILL OF MATERIALS");
    let (table, width, height) =
        paint_table(&sections, &columns, &["#", "COMMENT", "FOOTPRINT", "DESIGNATORS", "QTY"], 2.4, &[0, 4]);
    let table = table.replace("data-table-cell=\"true\"", "data-table-cell=\"true\" data-bom-cell=\"true\"");
    let content = format!("  <g data-bom-drawing=\"true\">\n{table}  </g>\n");
    let (svg, _, _) = paint_page(&content, width, height, design, "BILL OF MATERIALS", "BOM", 1, 1);
    BomDrawing { svg, rows: rows.len(), components: rows.iter().map(|row| row.designators.len()).sum() }
}

/// One row of the net cross-reference table.
pub(crate) struct NetXrefRow {
    /// Net name.
    pub(crate) name: String,
    /// `REFERENCE.PIN` members.
    pub(crate) nodes: Vec<String>,
    /// One-based sheet numbers that name the net.
    pub(crate) sheets: Vec<usize>,
}

/// Collects every net across the document with its nodes and sheet numbers.
///
/// Nets are grouped by name, nodes sort naturally and sheet numbers are the
/// one-based positions in the document. The rows feed both the SVG and PDF
/// cross-reference drawings.
pub(crate) fn net_xref_rows(design: &Design) -> Vec<NetXrefRow> {
    let mut rows: BTreeMap<String, (BTreeSet<String>, BTreeSet<usize>)> = BTreeMap::new();
    for (index, sheet) in design.sheets.iter().enumerate() {
        for net in &sheet.nets {
            let entry = rows.entry(net.name.clone()).or_default();
            entry.1.insert(index + 1);
            for node in &net.nodes {
                entry.0.insert(format!("{}.{}", node.reference, node.pin));
            }
        }
    }
    rows.into_iter()
        .map(|(name, (nodes, sheets))| {
            let mut nodes: Vec<String> = nodes.into_iter().collect();
            nodes.sort_by(|left, right| natural_compare(left, right));
            NetXrefRow { name, nodes, sheets: sheets.into_iter().collect() }
        })
        .collect()
}

/// The net cross-reference drawing and the counts it painted.
pub(crate) struct NetXref {
    /// The complete SVG document.
    pub(crate) svg: String,
    /// Nets listed.
    pub(crate) nets: usize,
    /// Node references listed.
    pub(crate) nodes: usize,
    /// Table sections (pages) painted.
    pub(crate) pages: usize,
}

/// Paints every net as a row of its `REFERENCE.PIN` members and sheet numbers.
///
/// The net column, node column and sheet column are measured from the widest
/// cell; a long node list wraps inside its column with the row growing in
/// height, keeping the monospace columns aligned. Sections continue with a
/// `(CONTINUED)` caption and a page number.
pub(crate) fn render_net_xref(design: &Design) -> NetXref {
    let rows = net_xref_rows(design);
    let node_count = rows.iter().map(|row| row.nodes.len()).sum();
    let net_width = rows.iter().map(|row| measure_text(&row.name, 2.4)).fold(12.0, f64::max).clamp(14.0, 40.0);
    let columns = [net_width + 4.0, 72.0, 14.0];
    let mut table_rows = Vec::new();
    let mut row_heights = Vec::new();
    for row in &rows {
        let nodes = row.nodes.join(", ");
        let lines = wrap_text(&nodes, columns[1] - 3.0, 2.4);
        let height = 3.4 * lines.len() as f64 + 3.0;
        table_rows.push(vec![
            fit_text(&row.name, columns[0] - 3.0, 2.4),
            lines.join("\n"),
            row.sheets.iter().map(usize::to_string).collect::<Vec<_>>().join(","),
        ]);
        row_heights.push(height);
    }
    let sections = paginate_rows(table_rows, row_heights, 140.0, "NET CROSS REFERENCE");
    let (table, width, height) = paint_table(&sections, &columns, &["NET", "NODES (REFERENCE.PIN)", "SHEET"], 2.4, &[2]);
    let content = format!("  <g data-net-xref=\"true\">\n{table}  </g>\n");
    let (svg, _, _) = paint_page(&content, width, height, design, "NET CROSS REFERENCE", "NX", 1, 1);
    NetXref { svg, nets: rows.len(), nodes: node_count, pages: sections.len() }
}
