//! Additional visual drawings projected from one design.
//!
//! Every drawing shares one standard A4 landscape page: a white background, an
//! outer border, an eight-percent margin and a fixed title block whose cells
//! carry the drawing name, number, scale, unit, revision and sheet count. The
//! content is laid out at its natural size, then fitted uniformly into the
//! drawable area, so a small design fills the page instead of leaving most of it
//! empty. Text is monospace, never below the readable floor, measured before it
//! is painted, and truncated or wrapped to its own box. Wire labels are placed
//! by a candidate search that rejects any spot touching a box or another label.
//! Layouts are pure functions of the document, so one revision always paints
//! byte-identical SVG.

use crate::{
    Component, Design, PinType, Sheet, bom_rows, drawing_bounds, escape_xml, natural_compare, push_svg_component_marked,
    push_svg_sheet_contents,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Pixels per design unit for standalone drawings, matching the sheet SVG.
const PIXELS_PER_UNIT: f64 = 8.0;

/// Standard A4 landscape page in design units (millimetres).
const PAGE_WIDTH: f64 = 297.0;
/// Standard A4 landscape page in design units (millimetres).
const PAGE_HEIGHT: f64 = 210.0;
/// Horizontal page margin: eight percent of the page width.
const PAGE_MARGIN_X: f64 = PAGE_WIDTH * 0.08;
/// Vertical page margin: eight percent of the page height.
const PAGE_MARGIN_Y: f64 = PAGE_HEIGHT * 0.08;
/// Height of the fixed title block along the page bottom.
const TITLE_BLOCK_HEIGHT: f64 = 22.0;
/// Fixed title-block cell widths; the four cells always span the whole page.
const TITLE_BLOCK_COLUMNS: [f64; 4] = [132.0, 78.0, 40.0, 47.0];
/// Smallest readable font size on the page. At A4 width this is about 6.8 pt.
const MIN_FONT_SIZE: f64 = 2.4;
/// Font family used for every drawing.
const FONT_FAMILY: &str = "monospace";
/// Palette used to tell one net apart from another.
const NET_PALETTE: [&str; 8] = ["#0f766e", "#1d4ed8", "#b45309", "#7c3aed", "#be123c", "#0369a1", "#4d7c0f", "#a21caf"];

/// Opens one standalone SVG document sized as the shared A4 landscape page.
fn start_document() -> String {
    let mut svg = String::new();
    svg.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {PAGE_WIDTH:.2} {PAGE_HEIGHT:.2}\" width=\"{:.0}\" height=\"{:.0}\">\n",
        (PAGE_WIDTH * PIXELS_PER_UNIT).round(),
        (PAGE_HEIGHT * PIXELS_PER_UNIT).round(),
    ));
    svg.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{PAGE_WIDTH:.2}\" height=\"{PAGE_HEIGHT:.2}\" fill=\"#ffffff\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n"
    ));
    svg
}

/// Estimated painted width of a monospace run: ASCII takes 0.6 em, wide glyphs
/// such as CJK take one em.
fn measure_text(text: &str, size: f64) -> f64 {
    text.chars().map(|character| if character.is_ascii() { 0.6 } else { 1.0 }).sum::<f64>() * size
}

/// Truncates text so a monospace run at `size` stays within `max_units`, marking
/// a cut with an ASCII ellipsis so no label can paint over a neighbouring box or
/// leave the canvas.
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

/// Fits text into a fixed box by first trying to keep the preferred size, then a
/// smaller size, and only truncating at the readable floor. Returns the painted
/// text and its size, so no title-block cell can ever overflow.
fn fit_text_in_box(text: &str, max_units: f64, preferred: f64, floor: f64) -> (String, f64) {
    let floor = floor.max(MIN_FONT_SIZE);
    let preferred = preferred.max(floor);
    if measure_text(text, preferred) <= max_units {
        return (text.to_string(), preferred);
    }
    let mut size = preferred;
    while size > floor + 0.01 {
        size = (size - 0.2).max(floor);
        if measure_text(text, size) <= max_units {
            return (text.to_string(), size);
        }
    }
    (fit_text(text, max_units, floor), floor)
}

/// Wraps text to `max_units`, breaking at spaces, commas or hard character
/// boundaries so a long list stays inside its measured column.
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

/// The drawable area: the A4 page minus the eight-percent margins and the title
/// block.
fn drawable_size() -> (f64, f64) {
    (PAGE_WIDTH - 2.0 * PAGE_MARGIN_X, PAGE_HEIGHT - 2.0 * PAGE_MARGIN_Y - TITLE_BLOCK_HEIGHT)
}

/// Wraps one content group in the A4 page and fits it uniformly into the
/// drawable area, then paints the fixed title block.
///
/// The group carries its natural size, scale and offset so tests can check the
/// coverage and every layout can be audited without re-parsing the transforms.
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
) -> String {
    let (draw_width, draw_height) = drawable_size();
    let width = content_width.max(1.0);
    let height = content_height.max(1.0);
    let scale = (draw_width / width).min(draw_height / height).min(4.0);
    let used_width = width * scale;
    let used_height = height * scale;
    let offset_x = PAGE_MARGIN_X + (draw_width - used_width) / 2.0;
    let offset_y = PAGE_MARGIN_Y + (draw_height - used_height) / 2.0;
    let mut svg = start_document();
    svg.push_str(&format!(
        "  <g data-drawing=\"{}\" data-content=\"{width:.2} {height:.2}\" data-scale=\"{scale:.4}\" transform=\"translate({offset_x:.2} {offset_y:.2}) scale({scale:.4})\">\n",
        escape_xml(code)
    ));
    svg.push_str(content);
    svg.push_str("  </g>\n");
    paint_title_block(&mut svg, design, drawing, code, page_index, page_total);
    svg.push_str("</svg>\n");
    svg
}

/// Paints the fixed title block: one row of four fixed-width cells, each with a
/// primary and a secondary line fitted to its own cell.
fn paint_title_block(svg: &mut String, design: &Design, drawing: &str, code: &str, page_index: usize, page_total: usize) {
    let top = PAGE_HEIGHT - TITLE_BLOCK_HEIGHT;
    svg.push_str(&format!(
        "  <g data-title-block=\"true\">\n    <line x1=\"0.00\" y1=\"{top:.2}\" x2=\"{PAGE_WIDTH:.2}\" y2=\"{top:.2}\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n"
    ));
    let mut boundary = 0.0;
    for width in &TITLE_BLOCK_COLUMNS[..3] {
        boundary += width;
        svg.push_str(&format!(
            "    <line x1=\"{boundary:.2}\" y1=\"{top:.2}\" x2=\"{boundary:.2}\" y2=\"{PAGE_HEIGHT:.2}\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n"
        ));
    }
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
    let cells: [(String, f64, String, f64); 4] = [
        (title.to_string(), 4.0, subtitle, 2.8),
        (drawing.to_string(), 3.6, drawing_number, 2.8),
        ("SCALE 1:1".to_string(), 3.2, format!("UNIT {}", design.units), 2.8),
        (format!("SHEET {page_index}/{page_total}"), 3.2, format!("REV {rev}"), 2.8),
    ];
    let cell_widths = [
        TITLE_BLOCK_COLUMNS[0] - 4.0,
        TITLE_BLOCK_COLUMNS[1] - 4.0,
        TITLE_BLOCK_COLUMNS[2] - 4.0,
        TITLE_BLOCK_COLUMNS[3] - 4.0,
    ];
    let mut cell_x = 2.0;
    for (index, (primary, primary_size, secondary, secondary_size)) in cells.iter().enumerate() {
        let (primary_text, primary_painted) = fit_text_in_box(primary, cell_widths[index], *primary_size, MIN_FONT_SIZE);
        svg.push_str(&format!(
            "    <text x=\"{cell_x:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{primary_painted:.1}\" fill=\"#1f2933\">{}</text>\n",
            top + 9.0,
            escape_xml(&primary_text)
        ));
        if !secondary.is_empty() {
            let (secondary_text, secondary_painted) =
                fit_text_in_box(secondary, cell_widths[index], *secondary_size, MIN_FONT_SIZE);
            svg.push_str(&format!(
                "    <text x=\"{cell_x:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{secondary_painted:.1}\" fill=\"#64748b\">{}</text>\n",
                top + 17.0,
                escape_xml(&secondary_text)
            ));
        }
        cell_x += TITLE_BLOCK_COLUMNS[index];
    }
    svg.push_str("  </g>\n");
}

/// One painted table column.
struct TableColumn {
    header: String,
    width: f64,
    align_right: bool,
}

/// One outlined table section before pagination.
struct TableSection {
    rows: Vec<Vec<String>>,
    row_heights: Vec<f64>,
}

/// Distributes a target width across measured columns.
///
/// Fixed columns keep their natural width; the leftover goes to the flexible
/// columns in proportion to their measured content, so a table always spans the
/// page instead of hugging its left edge.
fn distribute_widths(natural: &[f64], flexible: &[bool], target: f64) -> Vec<f64> {
    let total: f64 = natural.iter().sum();
    let flexible_count = flexible.iter().filter(|flex| **flex).count();
    if flexible_count == 0 {
        if total >= target {
            let factor = target / total;
            return natural.iter().map(|width| (width * factor).max(8.0)).collect();
        }
        return natural.to_vec();
    }
    let fixed_total: f64 = natural.iter().zip(flexible).filter(|(_, flex)| !**flex).map(|(width, _)| *width).sum();
    let flexible_total: f64 = natural.iter().zip(flexible).filter(|(_, flex)| **flex).map(|(width, _)| *width).sum();
    let available = (target - fixed_total).max(8.0 * flexible_count as f64);
    if total >= target && available < flexible_total {
        // Keep fixed columns at their measured width so their headers and values
        // never wrap, and take the deficit out of the flexible columns only.
        let factor = available / flexible_total;
        return natural
            .iter()
            .zip(flexible)
            .map(|(width, flex)| if *flex { (width * factor).max(16.0) } else { *width })
            .collect();
    }
    let extra = target - total;
    natural
        .iter()
        .zip(flexible)
        .map(|(width, flex)| {
            if !*flex {
                *width
            }
            else if flexible_total > 0.0 {
                width + extra * (width / flexible_total)
            }
            else {
                width + extra / flexible_count as f64
            }
        })
        .collect()
}

/// Paints ruled table sections and returns the content, its width and height.
fn paint_table(sections: &[TableSection], columns: &[TableColumn], size: f64, caption: &str) -> (String, f64, f64) {
    let width: f64 = columns.iter().map(|column| column.width).sum();
    let header_height = (size * 2.0).max(5.0);
    let line_height = size * 1.3;
    let mut content = String::from("  <g data-table=\"true\">\n");
    let mut y = 0.0;
    for (index, section) in sections.iter().enumerate() {
        y += size * 1.0;
        let caption = format!("{caption}  |  PAGE {} OF {}", index + 1, sections.len());
        content.push_str(&format!(
            "    <text x=\"0.00\" y=\"{y:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size:.1}\" fill=\"#1f2933\">{}</text>\n",
            escape_xml(&fit_text(&caption, width, size))
        ));
        y += size * 1.6;
        content.push_str(&format!(
            "    <rect x=\"0.00\" y=\"{y:.2}\" width=\"{width:.2}\" height=\"{header_height:.2}\" fill=\"#eef4ff\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n"
        ));
        let mut x = 0.0;
        for column in columns {
            let (label, painted) = fit_text_in_box(&column.header, column.width - 2.0, (size * 0.85).max(MIN_FONT_SIZE), MIN_FONT_SIZE);
            let text_x = if column.align_right { x + column.width - 1.5 } else { x + 1.5 };
            let anchor = if column.align_right { "end" } else { "start" };
            content.push_str(&format!(
                "    <text x=\"{text_x:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{painted:.1}\" text-anchor=\"{anchor}\" fill=\"#1f2933\">{}</text>\n",
                y + header_height * 0.7,
                escape_xml(&label)
            ));
            x += column.width;
        }
        y += header_height;
        let body_top = y;
        for (row, height) in section.rows.iter().zip(&section.row_heights) {
            let mut x = 0.0;
            for (index, column) in columns.iter().enumerate() {
                let cell = row.get(index).map(String::as_str).unwrap_or("");
                let text_x = if column.align_right { x + column.width - 1.5 } else { x + 1.5 };
                let anchor = if column.align_right { "end" } else { "start" };
                for (line, text) in cell.split('\n').enumerate() {
                    content.push_str(&format!(
                        "    <text x=\"{text_x:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size:.1}\" text-anchor=\"{anchor}\" fill=\"#334155\" data-table-cell=\"true\">{}</text>\n",
                        y + size + 0.2 + line as f64 * line_height,
                        escape_xml(text)
                    ));
                }
                x += column.width;
            }
            y += height;
            content.push_str(&format!(
                "    <line x1=\"0.00\" y1=\"{y:.2}\" x2=\"{width:.2}\" y2=\"{y:.2}\" stroke=\"#cbd5e1\" stroke-width=\"0.25\"/>\n"
            ));
        }
        let mut x = 0.0;
        for column in columns.iter().take(columns.len().saturating_sub(1)) {
            x += column.width;
            content.push_str(&format!(
                "    <line x1=\"{x:.2}\" y1=\"{body_top:.2}\" x2=\"{x:.2}\" y2=\"{y:.2}\" stroke=\"#1f6feb\" stroke-width=\"0.3\"/>\n"
            ));
        }
        content.push_str(&format!(
            "    <rect x=\"0.00\" y=\"{body_top:.2}\" width=\"{width:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n",
            y - body_top
        ));
        y += 3.0;
    }
    content.push_str("  </g>\n");
    (content, width, (y - 3.0).max(1.0))
}

/// A prepared table: headers, raw cell values, alignment and flexibility.
struct TableSpec {
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
    align_right: Vec<bool>,
    flexible: Vec<bool>,
    caption: String,
    cell_attribute: Option<&'static str>,
    font: f64,
}

/// Measures, lays out, paginates and paints one table that fills the drawable
/// area. Columns whose values are all blank are removed before measuring, so an
/// unused field never leaves an empty column behind.
fn render_table(spec: &TableSpec) -> (String, f64, f64, usize) {
    let (draw_width, draw_height) = drawable_size();
    let font = spec.font;
    let keep: Vec<usize> = (0..spec.headers.len())
        .filter(|index| spec.rows.iter().any(|row| row.get(*index).is_some_and(|cell| !cell.trim().is_empty())))
        .collect();
    if keep.is_empty() {
        return (String::new(), 1.0, 1.0, 0);
    }
    let natural: Vec<f64> = keep
        .iter()
        .map(|index| {
            let longest = spec
                .rows
                .iter()
                .map(|row| measure_text(row.get(*index).map(String::as_str).unwrap_or(""), font))
                .fold(measure_text(spec.headers[*index], font * 0.8), f64::max);
            (longest + 4.0).max(8.0)
        })
        .collect();
    let flexible: Vec<bool> = keep.iter().map(|index| spec.flexible[*index]).collect();
    let widths = distribute_widths(&natural, &flexible, draw_width);
    let columns: Vec<TableColumn> = keep
        .iter()
        .zip(&widths)
        .map(|(index, width)| TableColumn {
            header: spec.headers[*index].to_string(),
            width: *width,
            align_right: spec.align_right[*index],
        })
        .collect();
    let cells: Vec<Vec<String>> = spec
        .rows
        .iter()
        .map(|row| {
            keep.iter()
                .zip(&widths)
                .map(|(index, width)| {
                    let value = row.get(*index).map(String::as_str).unwrap_or("");
                    wrap_text(value, width - 3.0, font).join("\n")
                })
                .collect()
        })
        .collect();
    let line_height = font * 1.3;
    let min_row = font * 1.9;
    let heights: Vec<f64> = cells
        .iter()
        .map(|row| {
            let lines = row.iter().map(|cell| cell.split('\n').count()).max().unwrap_or(1) as f64;
            (lines * line_height + font * 0.6).max(min_row)
        })
        .collect();
    let caption_height = font * 2.7;
    let header_height = (font * 2.0).max(5.0);
    let budget = draw_height * 0.88 - caption_height - header_height;
    let rows_per_page = rows_per_page(&heights, budget).max(1);
    let mut sections = Vec::new();
    let mut cursor = 0usize;
    while cursor < cells.len() {
        let end = (cursor + rows_per_page).min(cells.len());
        let mut section = TableSection { rows: cells[cursor..end].to_vec(), row_heights: heights[cursor..end].to_vec() };
        let used: f64 = section.row_heights.iter().sum();
        if used < budget {
            let add = ((budget - used) / section.row_heights.len().max(1) as f64).min(font * 2.2);
            for height in &mut section.row_heights {
                *height += add;
            }
        }
        sections.push(section);
        cursor = end;
    }
    if sections.is_empty() {
        sections.push(TableSection { rows: vec![vec![String::new(); columns.len()]], row_heights: vec![min_row] });
    }
    let (mut content, width, height) = paint_table(&sections, &columns, font, &spec.caption);
    if let Some(attribute) = spec.cell_attribute {
        content = content.replace("data-table-cell=\"true\"", &format!("data-table-cell=\"true\" {attribute}"));
    }
    (content, width, height, sections.len())
}

/// Number of table rows that fit one section budget.
fn rows_per_page(heights: &[f64], budget: f64) -> usize {
    let mut used = 0.0;
    let mut count = 0;
    for height in heights {
        if count > 0 && used + height > budget {
            break;
        }
        used += height;
        count += 1;
    }
    count.max(1)
}

/// Builds a measured table spec from headers, raw rows and layout hints.
#[allow(clippy::too_many_arguments)]
fn table_spec(
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
    align_right: Vec<bool>,
    flexible: Vec<bool>,
    caption: &str,
    cell_attribute: Option<&'static str>,
) -> TableSpec {
    TableSpec { headers, rows, align_right, flexible, caption: caption.to_string(), cell_attribute, font: 3.4 }
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
    row: usize,
    column: usize,
}

impl NodeBox {
    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }

    fn centre_y(self) -> f64 {
        self.y + self.height / 2.0
    }

    fn bounds(self) -> [f64; 4] {
        [self.x, self.y, self.right(), self.bottom()]
    }
}

/// Chooses the column count that makes a grid of `count` cells closest to the
/// page aspect while leaving as few cells empty as possible.
fn choose_grid(count: usize, cell_width: f64, cell_height: f64, gap_x: f64, gap_y: f64, target: f64) -> (usize, usize) {
    if count == 0 {
        return (1, 1);
    }
    let mut best = (1usize, count);
    let mut best_score = f64::INFINITY;
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        let empty = columns * rows - count;
        let width = columns as f64 * cell_width + columns.saturating_sub(1) as f64 * gap_x;
        let height = rows as f64 * cell_height + rows.saturating_sub(1) as f64 * gap_y;
        let score = (width / height - target).abs() + 2.0 * empty as f64 / count as f64;
        if score < best_score {
            best_score = score;
            best = (columns, rows);
        }
    }
    best
}

/// One placed wire label.
struct PlacedLabel {
    text: String,
    box_bounds: [f64; 4],
    x: f64,
    baseline: f64,
    leader: Option<([f64; 2], [f64; 2])>,
}

/// Tries eight directions around an anchor at growing distances and returns the
/// first spot that touches neither an obstacle nor an already placed label.
/// When every candidate collides, the label is moved further out and a leader
/// line is reported so the reader can still trace it.
fn place_label(
    text: &str,
    size: f64,
    anchor: [f64; 2],
    obstacles: &[[f64; 4]],
    labels: &[[f64; 4]],
    bounds: [f64; 4],
) -> PlacedLabel {
    let half = measure_text(text, size) / 2.0 + 0.4;
    let half_height = size * 0.55;
    let collides = |candidate: [f64; 4]| {
        candidate[0] < bounds[0]
            || candidate[1] < bounds[1]
            || candidate[2] > bounds[2]
            || candidate[3] > bounds[3]
            || obstacles.iter().chain(labels.iter()).any(|other| {
                candidate[0] < other[2] && other[0] < candidate[2] && candidate[1] < other[3] && other[1] < candidate[3]
            })
    };
    let directions = [(0.0, -1.0), (0.0, 1.0), (1.0, 0.0), (-1.0, 0.0), (1.0, -1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    for distance in [size * 0.8, size * 1.6, size * 2.6] {
        for (dx, dy) in directions {
            let x = anchor[0] + dx * (half + distance);
            let y = anchor[1] + dy * (half_height + distance);
            let baseline = y + size * 0.35;
            let candidate = [x - half, baseline - size * 0.8, x + half, baseline + size * 0.25];
            if !collides(candidate) {
                return PlacedLabel { text: text.to_string(), box_bounds: candidate, x, baseline, leader: None };
            }
        }
    }
    let x = anchor[0].clamp(bounds[0] + half, bounds[2] - half);
    let baseline = (anchor[1] - size * 4.0).max(bounds[1] + size);
    let candidate = [x - half, baseline - size * 0.8, x + half, baseline + size * 0.25];
    PlacedLabel { text: text.to_string(), box_bounds: candidate, x, baseline, leader: Some((anchor, [x, candidate[3]])) }
}

/// Paints one placed label with a white backing so it stays legible over a line.
fn push_label(svg: &mut String, label: &PlacedLabel, size: f64, colour: &str, marker: &str) {
    if let Some((from, to)) = label.leader {
        svg.push_str(&format!(
            "    <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{colour}\" stroke-width=\"0.25\"/>\n",
            from[0], from[1], to[0], to[1]
        ));
    }
    svg.push_str(&format!(
        "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#ffffff\"/>\n",
        label.box_bounds[0] - 0.2,
        label.box_bounds[1] - 0.2,
        label.box_bounds[2] - label.box_bounds[0] + 0.4,
        label.box_bounds[3] - label.box_bounds[1] + 0.4
    ));
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size:.1}\" text-anchor=\"middle\" fill=\"{colour}\" {marker}>{}</text>\n",
        label.x,
        label.baseline,
        escape_xml(&label.text)
    ));
}

/// Projects every component as a box and every multi-member net as a routed
/// chain through its members.
///
/// Components sit on a grid whose column count is chosen to match the page
/// aspect, so the drawing fills the sheet instead of forming one long strand.
/// Edges leave a box exactly on its edge, turn only in the gutters between
/// columns and rows, and never cross a component body. A net name is printed
/// once per chain beside its longest segment, avoiding boxes and other labels.
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

    let (draw_width, draw_height) = drawable_size();
    let node_width = components
        .iter()
        .map(|component| measure_text(&component.reference, 3.4).max(measure_text(&component.value, 2.8)))
        .fold(28.0, f64::max)
        .min(56.0)
        + 8.0;
    let node_height = 15.0;
    let gap_x = 24.0;
    let gap_y = 18.0;
    let target = draw_width / draw_height;
    let (columns, rows) = choose_grid(components.len(), node_width, node_height, gap_x, gap_y, target);
    let margin_x = gap_x * 0.5 + 3.0;
    let content_width = 2.0 * margin_x + columns as f64 * node_width + columns.saturating_sub(1) as f64 * gap_x;
    let content_height = gap_y + rows as f64 * node_height + rows.saturating_sub(1) as f64 * gap_y + gap_y;

    let mut ordered: Vec<&Component> = components.clone();
    ordered.sort_by(|left, right| {
        let left_depth = layer.get(left.reference.as_str()).copied().unwrap_or(0);
        let right_depth = layer.get(right.reference.as_str()).copied().unwrap_or(0);
        left_depth.cmp(&right_depth).then_with(|| natural_compare(&left.reference, &right.reference))
    });
    let mut node_box: BTreeMap<&str, NodeBox> = BTreeMap::new();
    for (index, component) in ordered.iter().enumerate() {
        let column = index % columns.max(1);
        let row = index / columns.max(1);
        node_box.insert(
            component.reference.as_str(),
            NodeBox {
                x: margin_x + column as f64 * (node_width + gap_x),
                y: gap_y + row as f64 * (node_height + gap_y),
                width: node_width,
                height: node_height,
                row,
                column,
            },
        );
    }

    let mut content = String::from("  <g data-block-diagram=\"true\">\n");
    let mut edges = 0usize;
    let mut nets = 0usize;
    let obstacles: Vec<[f64; 4]> = node_box.values().map(|node| node.bounds()).collect();
    let bounds = [0.0, 0.0, content_width, content_height];
    let mut label_boxes: Vec<[f64; 4]> = Vec::new();
    let mut edge_index = 0usize;
    for (net_index, (net, set)) in members.iter().enumerate() {
        let mut chain: Vec<&str> = set.iter().copied().collect();
        if chain.len() < 2 {
            continue;
        }
        chain.sort_by_key(|reference| node_box[reference].row * 4096 + node_box[reference].column);
        nets += 1;
        let colour = NET_PALETTE[net_index % NET_PALETTE.len()];
        for (segment, pair) in chain.windows(2).enumerate() {
            let from = node_box[pair[0]];
            let to = node_box[pair[1]];
            let points = route_edge(from, to, gap_x, gap_y, edge_index, content_height);
            edge_index += 1;
            let rendered = points.iter().map(|point| format!("{:.2},{:.2}", point[0], point[1])).collect::<Vec<_>>().join(" ");
            content.push_str(&format!(
                "    <polyline points=\"{rendered}\" fill=\"none\" stroke=\"{colour}\" stroke-width=\"0.55\" data-net=\"{}\"/>\n",
                escape_xml(net)
            ));
            edges += 1;
            if segment == 0 {
                let anchor = longest_horizontal_midpoint(&points).unwrap_or([points[0][0], points[0][1]]);
                let label = place_label(net, 3.0, [anchor[0], anchor[1] - 0.2], &obstacles, &label_boxes, bounds);
                label_boxes.push(label.box_bounds);
                push_label(&mut content, &label, 3.0, colour, &format!("data-net-name=\"{}\"", escape_xml(net)));
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
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"3.4\" text-anchor=\"middle\">{}</text>\n",
            node.x + node.width / 2.0,
            node.y + 6.2,
            escape_xml(&fit_text(&component.reference, node.width - 2.0, 3.4))
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.8\" text-anchor=\"middle\" fill=\"#64748b\">{}</text>\n",
            node.x + node.width / 2.0,
            node.y + 11.6,
            escape_xml(&fit_text(&component.value, node.width - 2.0, 2.8))
        ));
        content.push_str("    </g>\n");
    }
    if components.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"10.00\" font-family=\"{FONT_FAMILY}\" font-size=\"4.0\" text-anchor=\"middle\" fill=\"#64748b\">no components</text>\n",
            content_width / 2.0
        ));
    }
    content.push_str("  </g>\n");
    let svg = paint_page(&content, content_width, content_height, design, "BLOCK DIAGRAM", "BD", 1, 1);
    BlockDiagram { svg, nodes: components.len(), edges, nets }
}

/// Routes one block-diagram edge through the gutters, returning the polyline.
///
/// Horizontal runs always use the gutter above or below a row, so a segment that
/// spans several columns can never pass through an intermediate box.
fn route_edge(from: NodeBox, to: NodeBox, gap_x: f64, gap_y: f64, index: usize, content_height: f64) -> Vec<[f64; 2]> {
    let lane = (index % 5) as f64 * 1.5 - 3.0;
    if from.row == to.row && from.column.abs_diff(to.column) == 1 {
        let points = if from.x < to.x {
            vec![[from.right(), from.centre_y()], [to.x, to.centre_y()]]
        }
        else {
            vec![[from.x, from.centre_y()], [to.right(), to.centre_y()]]
        };
        return points;
    }
    if from.x == to.x {
        // Same column: use one gutter and one clean vertical run. A second
        // one-unit jog here would read as a dangling stub at the corridor.
        let gutter = from.right() + gap_x * 0.5 + lane;
        return vec![
            [from.right(), from.centre_y()],
            [gutter, from.centre_y()],
            [gutter, to.centre_y()],
            [to.right(), to.centre_y()],
        ];
    }
    let to_the_right = to.x > from.x || (to.x == from.x && to.row > from.row);
    let (start, first_gutter, end, second_gutter) = if to_the_right {
        ([from.right(), from.centre_y()], from.right() + gap_x * 0.5 + lane, [to.x, to.centre_y()], to.x - gap_x * 0.5 + lane)
    }
    else {
        ([from.x, from.centre_y()], from.x - gap_x * 0.5 - lane, [to.right(), to.centre_y()], to.right() + gap_x * 0.5 - lane)
    };
    let corridor =
        if to.centre_y() < from.centre_y() { to.y - gap_y * 0.5 - lane * 0.5 } else { to.bottom() + gap_y * 0.5 + lane * 0.5 }
            .clamp(gap_y * 0.2, content_height - gap_y * 0.2);
    let mut points = vec![
        start,
        [first_gutter, start[1]],
        [first_gutter, corridor],
        [second_gutter, corridor],
        [second_gutter, end[1]],
        end,
    ];
    let mut unique: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for point in points.drain(..) {
        if unique.last() != Some(&point) {
            unique.push(point);
        }
    }
    unique
}

/// The midpoint of the longest horizontal segment of a polyline.
fn longest_horizontal_midpoint(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    points
        .windows(2)
        .filter(|pair| (pair[0][1] - pair[1][1]).abs() < 0.05)
        .map(|pair| {
            let length = (pair[1][0] - pair[0][0]).abs();
            (length, [(pair[0][0] + pair[1][0]) / 2.0, pair[0][1]])
        })
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, point)| point)
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

/// Places every sheet on one page in a grid that fills the drawable area.
///
/// Each cell carries a separator frame, the sheet name and its position in the
/// set. The sheet is painted through the shared sheet painter inside a
/// translate-and-scale group, so its geometry is identical to the single-sheet
/// SVG while staying inside its cell.
pub(crate) fn render_sheet_overview(design: &Design) -> SheetOverview {
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    let bounds: Vec<(f64, f64, f64, f64)> = sheets.iter().map(|sheet| drawing_bounds(std::iter::once(*sheet))).collect();
    let (draw_width, draw_height) = drawable_size();
    let gap = 6.0;
    let header_height = 9.0;
    let cell_padding = 4.0;
    let columns = if sheets.len() <= 2 { sheets.len().max(1) } else { (sheets.len() as f64).sqrt().round() as usize };
    let rows = sheets.len().div_ceil(columns).max(1);
    let cell_width = (draw_width - (columns.saturating_sub(1)) as f64 * gap) / columns as f64;
    let cell_height = (draw_height - (rows.saturating_sub(1)) as f64 * gap) / rows as f64;

    let mut content = String::from("  <g data-sheet-overview=\"true\">\n");
    for (index, sheet) in sheets.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_x = column as f64 * (cell_width + gap);
        let cell_y = row as f64 * (cell_height + gap);
        let (min_x, min_y, max_x, max_y) = bounds[index];
        let span_x = (max_x - min_x).max(1.0);
        let span_y = (max_y - min_y + 2.0).max(1.0);
        let inner_width = cell_width - 2.0 * cell_padding;
        let inner_height = cell_height - header_height - 2.0 * cell_padding;
        let scale = (inner_width / span_x).min(inner_height / span_y);
        let shift_x = cell_x + cell_padding + (inner_width - span_x * scale) / 2.0 - min_x * scale;
        let shift_y = cell_y + header_height + cell_padding + (inner_height - span_y * scale) / 2.0 - min_y * scale;
        content.push_str(&format!(
            "    <rect x=\"{cell_x:.2}\" y=\"{cell_y:.2}\" width=\"{cell_width:.2}\" height=\"{cell_height:.2}\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"0.4\" stroke-dasharray=\"2.4 1.6\"/>\n"
        ));
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"3.6\" fill=\"#1f2933\">{}</text>\n",
            cell_x + 2.5,
            cell_y + 6.2,
            escape_xml(&fit_text(&sheet.name, cell_width - 22.0, 3.6))
        ));
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.8\" text-anchor=\"end\" fill=\"#64748b\">{}/{}</text>\n",
            cell_x + cell_width - 2.5,
            cell_y + 6.2,
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
    if sheets.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"4.0\" text-anchor=\"middle\" fill=\"#64748b\">no sheets</text>\n",
            draw_width / 2.0,
            draw_height / 2.0
        ));
    }
    let svg = paint_page(&content, draw_width, draw_height, design, "SHEET OVERVIEW", "SO", 1, 1);
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
/// count that gives the most balanced landscape page, and the whole page is
/// fitted so the blocks fill the sheet. A pin with no net leaves its cell blank
/// instead of filling the column with placeholder dashes.
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
        components.iter().flat_map(|component| &component.pins).map(|pin| measure_text(pin, 3.0)).fold(8.0, f64::max);
    let longest_net = components
        .iter()
        .flat_map(|component| {
            component.pins.iter().map(|pin| {
                net_of.get(&(component.reference.as_str(), pin.as_str())).map(|nets| nets.join(", ")).unwrap_or_default()
            })
        })
        .map(|text| measure_text(&text, 2.8).min(52.0))
        .fold(18.0, f64::max);
    let header_size = 3.4;
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
                wrap_text(&nets.join(", "), block_width - pin_width - 7.0, 2.8).len()
            };
            rows.push(PinRow {
                pin: pin.clone(),
                nets,
                no_connect: no_connects.contains(&key),
                height: (lines as f64 * 3.4 + 1.8).max(4.8),
            });
            total_rows += 1;
        }
        let body: f64 = rows.iter().map(|row| row.height).sum();
        blocks.push(PinBlock {
            reference: component.reference.clone(),
            value: component.value.clone(),
            rows,
            height: 6.0 + 5.0 + body + 1.0,
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
    let mut content = String::from("  <g data-pinout-chart=\"true\">\n");
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
                "      <rect x=\"{block_x:.2}\" y=\"{block_y:.2}\" width=\"{block_width:.2}\" height=\"6.0\" fill=\"#eef4ff\" stroke=\"#1f6feb\" stroke-width=\"0.4\"/>\n"
            ));
            content.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"3.4\">{}</text>\n",
                block_x + 2.0,
                block_y + 4.4,
                escape_xml(&fit_text(&format!("{} {}", block.reference, block.value), block_width - 4.0, 3.4))
            ));
            content.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.6\" text-anchor=\"end\" fill=\"#64748b\">PIN</text>\n",
                block_x + 3.0 + pin_width,
                block_y + 9.6
            ));
            content.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.6\" fill=\"#64748b\">NET</text>\n",
                block_x + 6.0 + pin_width,
                block_y + 9.6
            ));
            content.push_str(&format!(
                "      <line x1=\"{block_x:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#1f6feb\" stroke-width=\"0.3\"/>\n",
                block_y + 11.0,
                block_x + block_width,
                block_y + 11.0
            ));
            let mut row_y = block_y + 11.0;
            for row in &block.rows {
                content.push_str(&format!("      <g data-pin=\"{}\">\n", escape_xml(&row.pin)));
                content.push_str(&format!(
                    "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"3.0\" text-anchor=\"end\">{}</text>\n",
                    block_x + 3.0 + pin_width,
                    row_y + 3.4,
                    escape_xml(&fit_text(&row.pin, pin_width, 3.0))
                ));
                if row.no_connect {
                    content.push_str(&format!(
                        "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"3.0\" fill=\"#b91c1c\" data-no-connect=\"true\">NC</text>\n",
                        block_x + 6.0 + pin_width,
                        row_y + 3.4
                    ));
                }
                else if !row.nets.is_empty() {
                    let lines = wrap_text(&row.nets.join(", "), block_width - pin_width - 7.0, 2.8);
                    for (line, text) in lines.iter().enumerate() {
                        content.push_str(&format!(
                            "        <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.8\" fill=\"#0f766e\">{}</text>\n",
                            block_x + 6.0 + pin_width,
                            row_y + 3.4 + line as f64 * 3.4,
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
    let svg = paint_page(&content, content_width, content_height, design, "PINOUT CHART", "PO", 1, 1);
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

/// Paints every distinct symbol used by the design once, on a grid that fills
/// the page.
///
/// A symbol is identified by its reference prefix and its pin list, matching the
/// artwork the sheet projection selects; the first component in natural
/// reference order represents each symbol. The grid prefers a complete
/// factorization of the symbol count, so the last row is not left half empty
/// when an exact fit exists.
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

    let (draw_width, draw_height) = drawable_size();
    let gap = 5.0;
    let count = symbols.len().max(1);
    let exact = exact_grid(count, draw_width, draw_height, gap);
    let (columns, rows) = exact.unwrap_or_else(|| {
        let columns = (count as f64).sqrt().ceil() as usize;
        (columns, count.div_ceil(columns.max(1)))
    });
    let cell_width = (draw_width - (columns.saturating_sub(1)) as f64 * gap) / columns.max(1) as f64;
    let cell_height = (draw_height - (rows.saturating_sub(1)) as f64 * gap) / rows.max(1) as f64;
    let header_height = 7.0;
    let no_dots = BTreeSet::new();

    let mut content = String::from("  <g data-symbol-library=\"true\">\n");
    for (index, (key, component)) in symbols.iter().enumerate() {
        let column = index % columns.max(1);
        let row = index / columns.max(1);
        let cell_x = column as f64 * (cell_width + gap);
        let cell_y = row as f64 * (cell_height + gap);
        content.push_str(&format!("    <g data-symbol=\"{}\">\n", escape_xml(key)));
        content.push_str(&format!(
            "      <rect x=\"{cell_x:.2}\" y=\"{cell_y:.2}\" width=\"{cell_width:.2}\" height=\"{cell_height:.2}\" fill=\"#ffffff\" stroke=\"#cbd5e1\" stroke-width=\"0.35\"/>\n"
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"3.2\" fill=\"#1f2933\">{}</text>\n",
            cell_x + 2.5,
            cell_y + 4.8,
            escape_xml(&fit_text(&format!("{} {}", component.reference, component.value), cell_width - 5.0, 3.2))
        ));
        let mut placed = (*component).clone();
        placed.rotation = 0.0;
        placed.mirror = None;
        placed.x = 0.0;
        placed.y = 0.0;
        let (min, max) = crate::component_extents(&placed);
        let natural_width = (max[0] - min[0]).max(1.0);
        let natural_height = (max[1] - min[1]).max(1.0);
        let inner_width = cell_width - 8.0;
        let inner_height = cell_height - header_height - 12.0;
        let scale = (inner_width / natural_width).min(inner_height / natural_height).clamp(0.1, 3.0);
        let centre_x = cell_x + cell_width / 2.0;
        let centre_y = cell_y + header_height + inner_height / 2.0;
        placed.x = centre_x;
        placed.y = centre_y;
        content.push_str(&format!(
            "      <g transform=\"translate({centre_x:.2} {centre_y:.2}) scale({scale:.4}) translate({:.2} {:.2})\">\n",
            -centre_x, -centre_y
        ));
        push_svg_component_marked(&mut content, &placed, Some(&no_dots));
        content.push_str("      </g>\n");
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.8\" text-anchor=\"middle\" fill=\"#64748b\">{}</text>\n",
            centre_x,
            cell_y + cell_height - 3.0,
            escape_xml(&fit_text(&format!("PINS: {}", component.pins.join(", ")), cell_width - 5.0, 2.8))
        ));
        content.push_str("    </g>\n");
    }
    content.push_str("  </g>\n");
    if symbols.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"4.0\" text-anchor=\"middle\" fill=\"#64748b\">no symbols</text>\n",
            draw_width / 2.0,
            draw_height / 2.0
        ));
    }
    let svg = paint_page(&content, draw_width, draw_height, design, "SYMBOL LIBRARY", "SL", 1, 1);
    SymbolLibrary { svg, symbols: symbols.len(), components: components.len() }
}

/// Returns an exact column/row factorization of `count` whose cell aspect is
/// closest to the page aspect.
fn exact_grid(count: usize, draw_width: f64, draw_height: f64, gap: f64) -> Option<(usize, usize)> {
    if count == 0 {
        return None;
    }
    let target = draw_width / draw_height;
    (1..=count)
        .filter(|columns| count.is_multiple_of(*columns))
        .map(|columns| {
            let rows = count / columns;
            let cell_width = (draw_width - columns.saturating_sub(1) as f64 * gap) / columns as f64;
            let cell_height = (draw_height - rows.saturating_sub(1) as f64 * gap) / rows as f64;
            (columns, rows, (cell_width / cell_height - target).abs())
        })
        .min_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(columns, rows, _)| (columns, rows))
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

/// Paints the component/net incidence graph as a two-column component field
/// with one vertical trunk per net in the central channel.
///
/// Every net owns its own trunk at a distinct x and a named box at the top of
/// that trunk. A member component taps the trunk with a horizontal stub from its
/// own edge, so every net can be traced independently and the vertical runs
/// never collapse into a comb. The page is fitted so the columns fill the sheet.
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

    let comp_font = 3.2;
    let comp_width =
        components.iter().map(|component| measure_text(&component.reference, comp_font)).fold(10.0, f64::max).max(17.0) + 7.0;
    let comp_height = 10.0;
    let row_pitch = 13.0;
    let label_height = 7.0;
    let label_font = 2.8;
    let band_widths: Vec<f64> = net_names.iter().map(|name| (measure_text(name, label_font) + 4.0).max(12.5)).collect();
    let channel_width: f64 = band_widths.iter().sum();
    let channel_x = comp_width + 8.0;
    let right_x = channel_x + channel_width + 8.0;
    let left_count = components.len().div_ceil(2);
    let right_count = components.len() - left_count;
    let rows = left_count.max(right_count).max(1);
    let content_width = right_x + comp_width;
    let content_height = label_height + 6.0 + rows as f64 * row_pitch;

    let mut net_index: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, name) in net_names.iter().enumerate() {
        net_index.insert(*name, index);
    }
    let mut content = String::from("  <g data-net-topology=\"true\">\n");
    let mut edges = 0usize;
    for (index, component) in components.iter().enumerate() {
        let column_x = if index < left_count { 0.0 } else { right_x };
        let row = if index < left_count { index } else { index - left_count };
        let y = label_height + 6.0 + row as f64 * row_pitch;
        content.push_str(&format!(
            "    <g data-block=\"{}\">\n      <rect x=\"{column_x:.2}\" y=\"{y:.2}\" width=\"{comp_width:.2}\" height=\"{comp_height:.2}\" rx=\"1.6\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n",
            escape_xml(&component.reference)
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{comp_font:.1}\" text-anchor=\"middle\">{}</text>\n",
            column_x + comp_width / 2.0,
            y + 6.4,
            escape_xml(&fit_text(&component.reference, comp_width - 2.0, comp_font))
        ));
        content.push_str("    </g>\n");
    }
    let mut trunk_bottoms = vec![0.0f64; net_names.len()];
    for (index, component) in components.iter().enumerate() {
        let left = index < left_count;
        let column_x = if left { 0.0 } else { right_x };
        let row = if left { index } else { index - left_count };
        let y = label_height + 6.0 + row as f64 * row_pitch;
        let mut incident: Vec<&str> =
            net_names.iter().copied().filter(|name| members[*name].contains(component.reference.as_str())).collect();
        incident.sort_by_key(|name| net_index[name]);
        let step = (comp_height * 0.66 / net_names.len().max(1) as f64).min(0.8);
        let midpoint = (net_names.len() as f64 - 1.0) / 2.0;
        for name in &incident {
            let net = *net_index.get(name).expect("known net");
            // A fixed per-net offset keeps two different nets from sharing one
            // horizontal y, so a run across the channel is always traceable.
            let tap_y = y + comp_height * 0.5 + (net as f64 - midpoint) * step;
            let from_x = if left { column_x + comp_width } else { column_x };
            let trunk_x = channel_x + band_widths[..net].iter().sum::<f64>() + band_widths[net] / 2.0;
            let colour = NET_PALETTE[net % NET_PALETTE.len()];
            content.push_str(&format!(
                "    <line x1=\"{from_x:.2}\" y1=\"{tap_y:.2}\" x2=\"{trunk_x:.2}\" y2=\"{tap_y:.2}\" stroke=\"{colour}\" stroke-width=\"0.5\" data-net=\"{}\"/>\n",
                escape_xml(name)
            ));
            content.push_str(&format!("    <circle cx=\"{trunk_x:.2}\" cy=\"{tap_y:.2}\" r=\"0.4\" fill=\"{colour}\"/>\n"));
            trunk_bottoms[net] = trunk_bottoms[net].max(tap_y);
            edges += 1;
        }
    }
    for (net, name) in net_names.iter().enumerate() {
        let band_x = channel_x + band_widths[..net].iter().sum::<f64>();
        let trunk_x = band_x + band_widths[net] / 2.0;
        let colour = NET_PALETTE[net % NET_PALETTE.len()];
        content.push_str(&format!(
            "    <rect x=\"{:.2}\" y=\"0.00\" width=\"{:.2}\" height=\"{label_height:.2}\" rx=\"1.2\" fill=\"#f0fdfa\" stroke=\"{colour}\" stroke-width=\"0.45\" data-net-node=\"{}\"/>\n",
            band_x + 0.5,
            band_widths[net] - 1.0,
            escape_xml(name)
        ));
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{label_font:.1}\" fill=\"#0f172a\" data-net-name=\"{}\">{}</text>\n",
            band_x + 1.8,
            label_height - 2.2,
            escape_xml(name),
            escape_xml(&fit_text(name, band_widths[net] - 3.0, label_font))
        ));
        if trunk_bottoms[net] > 0.0 {
            content.push_str(&format!(
                "    <line x1=\"{trunk_x:.2}\" y1=\"{label_height:.2}\" x2=\"{trunk_x:.2}\" y2=\"{:.2}\" stroke=\"{colour}\" stroke-width=\"0.9\" data-trunk=\"true\"/>\n",
                trunk_bottoms[net]
            ));
        }
    }
    if components.is_empty() && net_names.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"4.0\" text-anchor=\"middle\" fill=\"#64748b\">no components or nets</text>\n",
            content_width / 2.0,
            content_height / 2.0
        ));
    }
    content.push_str("  </g>\n");
    let svg = paint_page(&content, content_width, content_height, design, "NET TOPOLOGY", "NT", 1, 1);
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

/// Classifies a net by name convention, falling back to the power pin types of
/// its members. Ground names win over power names.
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

/// Paints every power and ground net as a full-width rail bar and connects each
/// member component from its own left or right side.
///
/// Power members leave the right edge of a box into a gutter riser that runs up
/// to its power rail; ground members leave the left edge into a gutter riser
/// that runs down to its ground rail. Every riser starts and ends on a rail, and
/// every tap starts on a box edge, so no line dangles and no line crosses a
/// component body.
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

    let (draw_width, draw_height) = drawable_size();
    let box_font = 3.2;
    let box_width = components.iter().map(|component| measure_text(&component.reference, box_font)).fold(12.0, f64::max) + 8.0;
    let box_height = 12.0;
    let gap_y = 8.0;
    let risers = power.len().max(ground.len());
    let gutter = 10.0 + 2.6 * risers as f64;
    let rail_height = 5.5;
    let rail_pitch = 7.0;
    let top_rails = power.len() as f64 * rail_pitch + 10.0;
    let bottom_rails = ground.len() as f64 * rail_pitch + 8.0;
    let target = draw_width / draw_height;
    let (mut columns, mut rows) = (1usize, components.len().max(1));
    let mut best_score = f64::INFINITY;
    for candidate in 1..=components.len().max(1) {
        let candidate_rows = components.len().div_ceil(candidate).max(1);
        let width = candidate as f64 * box_width + (candidate + 1) as f64 * gutter;
        let height =
            top_rails + candidate_rows as f64 * box_height + (candidate_rows.saturating_sub(1)) as f64 * gap_y + bottom_rails;
        let score = (width / height - target).abs();
        if score < best_score {
            best_score = score;
            columns = candidate;
            rows = candidate_rows;
        }
    }
    let content_width = columns as f64 * box_width + (columns + 1) as f64 * gutter;
    let component_top = top_rails;
    let component_bottom = component_top + rows as f64 * box_height + (rows.saturating_sub(1)) as f64 * gap_y;
    let ground_top = component_bottom + 9.0;
    let content_height =
        if ground.is_empty() { component_bottom + 6.0 } else { ground_top + ground.len() as f64 * rail_pitch + 2.0 };

    let mut content = String::from("  <g data-power-distribution=\"true\">\n");
    let mut connections = 0usize;
    let mut position = BTreeMap::new();
    for (index, component) in components.iter().enumerate() {
        let column = index % columns.max(1);
        let row = index / columns.max(1);
        position.insert(
            component.reference.as_str(),
            (gutter + column as f64 * (box_width + gutter), component_top + row as f64 * (box_height + gap_y)),
        );
    }
    for (rail_index, name) in power.iter().enumerate() {
        let rail_y = 4.0 + rail_index as f64 * rail_pitch;
        for column in 0..columns {
            let members_in_column: Vec<(&str, f64)> = members[*name]
                .iter()
                .filter_map(|reference| {
                    let (x, y) = position[*reference];
                    let member_column = (x - gutter) / (box_width + gutter);
                    ((member_column.round() as usize) == column).then_some((*reference, y + box_height / 2.0))
                })
                .collect();
            if members_in_column.is_empty() {
                continue;
            }
            let x = gutter + column as f64 * (box_width + gutter) + box_width + 2.2 + rail_index as f64 * 2.6;
            let lowest = members_in_column.iter().map(|(_, y)| *y).fold(f64::NEG_INFINITY, f64::max);
            content.push_str(&format!(
                "    <line x1=\"{x:.2}\" y1=\"{:.2}\" x2=\"{x:.2}\" y2=\"{lowest:.2}\" stroke=\"#b45309\" stroke-width=\"0.6\" data-net=\"{}\" data-riser=\"power\"/>\n",
                rail_y + rail_height,
                escape_xml(name)
            ));
            for (reference, tap_y) in &members_in_column {
                let (box_x, _) = position[reference];
                let tap_y = tap_y + (rail_index as f64 - (power.len() as f64 - 1.0) / 2.0) * 0.9;
                content.push_str(&format!(
                    "    <line x1=\"{:.2}\" y1=\"{tap_y:.2}\" x2=\"{x:.2}\" y2=\"{tap_y:.2}\" stroke=\"#b45309\" stroke-width=\"0.5\" data-net=\"{}\" data-tap=\"power\"/>\n",
                    box_x + box_width,
                    escape_xml(name)
                ));
                connections += 1;
            }
        }
    }
    for (rail_index, name) in ground.iter().enumerate() {
        let rail_y = ground_top + rail_index as f64 * rail_pitch;
        for column in 0..columns {
            let members_in_column: Vec<(&str, f64)> = members[*name]
                .iter()
                .filter_map(|reference| {
                    let (x, y) = position[*reference];
                    let member_column = (x - gutter) / (box_width + gutter);
                    ((member_column.round() as usize) == column).then_some((*reference, y + box_height / 2.0))
                })
                .collect();
            if members_in_column.is_empty() {
                continue;
            }
            let x = gutter + column as f64 * (box_width + gutter) - 2.2 - rail_index as f64 * 2.6;
            let highest = members_in_column.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
            content.push_str(&format!(
                "    <line x1=\"{x:.2}\" y1=\"{highest:.2}\" x2=\"{x:.2}\" y2=\"{rail_y:.2}\" stroke=\"#1f2933\" stroke-width=\"0.6\" data-net=\"{}\" data-riser=\"ground\"/>\n",
                escape_xml(name)
            ));
            for (reference, tap_y) in &members_in_column {
                let (box_x, _) = position[reference];
                let tap_y = tap_y + (rail_index as f64 - (ground.len() as f64 - 1.0) / 2.0) * 0.9;
                content.push_str(&format!(
                    "    <line x1=\"{x:.2}\" y1=\"{tap_y:.2}\" x2=\"{box_x:.2}\" y2=\"{tap_y:.2}\" stroke=\"#1f2933\" stroke-width=\"0.5\" data-net=\"{}\" data-tap=\"ground\"/>\n",
                    escape_xml(name)
                ));
                connections += 1;
            }
        }
    }
    for (rail_index, name) in power.iter().enumerate() {
        let rail_y = 4.0 + rail_index as f64 * rail_pitch;
        content.push_str(&format!(
            "    <rect x=\"0.00\" y=\"{rail_y:.2}\" width=\"{content_width:.2}\" height=\"{rail_height:.2}\" rx=\"1.2\" fill=\"#fef3c7\" stroke=\"#b45309\" stroke-width=\"0.4\" data-rail=\"power\" data-net=\"{}\"/>\n",
            escape_xml(name)
        ));
        content.push_str(&format!(
            "    <text x=\"2.00\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.9\" fill=\"#92400e\">{}</text>\n",
            rail_y + 4.0,
            escape_xml(&fit_text(name, content_width - 4.0, 2.9))
        ));
    }
    for (rail_index, name) in ground.iter().enumerate() {
        let rail_y = ground_top + rail_index as f64 * rail_pitch;
        content.push_str(&format!(
            "    <rect x=\"0.00\" y=\"{rail_y:.2}\" width=\"{content_width:.2}\" height=\"{rail_height:.2}\" rx=\"1.2\" fill=\"#e5e7eb\" stroke=\"#1f2933\" stroke-width=\"0.4\" data-rail=\"ground\" data-net=\"{}\"/>\n",
            escape_xml(name)
        ));
        content.push_str(&format!(
            "    <text x=\"2.00\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"2.9\" fill=\"#1f2933\">{}</text>\n",
            rail_y + 4.0,
            escape_xml(&fit_text(name, content_width - 4.0, 2.9))
        ));
    }
    for component in &components {
        let (x, y) = position[component.reference.as_str()];
        content.push_str(&format!("    <g data-block=\"{}\">\n", escape_xml(&component.reference)));
        content.push_str(&format!(
            "      <rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{box_width:.2}\" height=\"{box_height:.2}\" rx=\"1.4\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n"
        ));
        content.push_str(&format!(
            "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"{box_font:.1}\" text-anchor=\"middle\">{}</text>\n",
            x + box_width / 2.0,
            y + 7.2,
            escape_xml(&fit_text(&component.reference, box_width - 2.0, box_font))
        ));
        content.push_str("    </g>\n");
    }
    if power.is_empty() && ground.is_empty() {
        content.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_FAMILY}\" font-size=\"4.0\" text-anchor=\"middle\" fill=\"#64748b\">no power or ground nets detected</text>\n",
            content_width / 2.0,
            content_height - 4.0
        ));
    }
    content.push_str("  </g>\n");
    let svg = paint_page(&content, content_width, content_height, design, "POWER DISTRIBUTION", "PD", 1, 1);
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

/// Paints the grouped bill of materials as a full-width ruled table.
///
/// Rows come from the shared BOM grouping: one row per value-and-footprint
/// pair, designators compressed into ranges. Columns are measured from their
/// content and then stretched to the page width; a column whose every value is
/// blank (for example an unset footprint) is not drawn at all.
pub(crate) fn render_bom_drawing(design: &Design) -> BomDrawing {
    let rows = bom_rows(design, None);
    let designators: Vec<String> = rows.iter().map(|row| crate::designator_ranges(&row.designators).join(", ")).collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            vec![
                (index + 1).to_string(),
                row.comment.clone(),
                row.footprint.clone(),
                designators[index].clone(),
                row.designators.len().to_string(),
            ]
        })
        .collect();
    let spec = table_spec(
        vec!["#", "COMMENT", "FOOTPRINT", "DESIGNATORS", "QTY"],
        table_rows,
        vec![true, false, false, false, true],
        vec![false, true, true, true, false],
        "BILL OF MATERIALS",
        Some("data-bom-cell=\"true\""),
    );
    let (table, width, height, _) = render_table(&spec);
    let content = format!("  <g data-bom-drawing=\"true\">\n{table}  </g>\n");
    let svg = paint_page(&content, width, height, design, "BILL OF MATERIALS", "BOM", 1, 1);
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
/// The net and sheet columns are measured from their widest cell; the node
/// column takes the remaining page width and wraps a long member list, so rows
/// grow in height instead of overlapping their neighbour.
pub(crate) fn render_net_xref(design: &Design) -> NetXref {
    let rows = net_xref_rows(design);
    let node_count = rows.iter().map(|row| row.nodes.len()).sum();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![row.name.clone(), row.nodes.join(", "), row.sheets.iter().map(usize::to_string).collect::<Vec<_>>().join(",")]
        })
        .collect();
    let spec = table_spec(
        vec!["NET", "NODES (REFERENCE.PIN)", "SHEET"],
        table_rows,
        vec![false, false, true],
        vec![false, true, false],
        "NET CROSS REFERENCE",
        None,
    );
    let (table, width, height, pages) = render_table(&spec);
    let content = format!("  <g data-net-xref=\"true\">\n{table}  </g>\n");
    let svg = paint_page(&content, width, height, design, "NET CROSS REFERENCE", "NX", 1, 1);
    NetXref { svg, nets: rows.len(), nodes: node_count, pages }
}

