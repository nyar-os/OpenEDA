//! PNG raster projection of the schematic drawing.
//!
//! The rasterizer paints the same geometry as the SVG projection — the sheet
//! frame, wires, junctions, no-connect marks, label boxes and component
//! bodies: standard resistor, capacitor, inductor and diode artwork for
//! two-pin `R`/`C`/`L`/`D` parts, the enclosing ring for three-pin `Q`/`J`
//! parts and the generic box for everything else, all honouring the component
//! rotation, with pin stubs and round connection dots. Text is rasterized with
//! a built-in 5×7 bitmap font covering printable ASCII: component references
//! and values, net names, net labels, sheet texts and a title block.
//! Characters outside that range are written as `?`, mirroring the PDF
//! base-14 fallback. More than one sheet is stacked into separate labelled
//! bands so sheets that share design coordinates never overlap, matching the
//! paged SVG and PDF exports. The encoder writes a valid PNG from uncompressed
//! deflate blocks only, so the same document always produces byte-identical
//! output.

use crate::{
    COMPONENT_HALF_WIDTH, Design, Sheet, SymbolPrimitive, component_half_height, component_pins, drawing_bounds,
    net_name_anchors, net_name_point, rotate_about, symbol_artwork,
};

/// Width of the fixed raster canvas, in pixels.
pub(crate) const WIDTH: u32 = 1024;
/// Height of the fixed raster canvas, in pixels.
pub(crate) const HEIGHT: u32 = 768;

/// Sheet-frame padding around the drawing content, in design units.
const PADDING: f64 = 10.0;
/// Blank border kept inside the canvas, in pixels.
const MARGIN: f64 = 8.0;
/// Height of the title-block strip along the canvas bottom, in pixels.
const TITLE_STRIP: i32 = 44;
/// Header line reserved above every sheet band of a multi-sheet raster.
const BAND_HEADER: i32 = 14;
/// Blank gap between sheet bands, in pixels.
const BAND_GAP: i32 = 12;

/// One RGB pixel colour.
type Rgb = [u8; 3];

const BACKGROUND: Rgb = [255, 255, 255];
const FRAME: Rgb = [31, 41, 51];
const WIRE: Rgb = [15, 118, 110];
const JUNCTION: Rgb = [31, 41, 51];
const NO_CONNECT: Rgb = [185, 28, 28];
const SYMBOL_FILL: Rgb = [246, 250, 255];
const SYMBOL: Rgb = [31, 111, 235];
const LOCAL_LABEL: Rgb = [15, 118, 110];
const GLOBAL_LABEL: Rgb = [185, 28, 28];
const HIERARCHICAL_LABEL: Rgb = [21, 128, 61];
const NET_NAME: Rgb = [100, 116, 139];
const TEXT: Rgb = [51, 65, 85];

/// One glyph column of the built-in 5×7 bitmap font: eight rows, bit 0 is the
/// top pixel. Indexed by `character - ' '`, covering printable ASCII.
const GLYPHS: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x5F, 0x00, 0x00],
    [0x00, 0x07, 0x00, 0x07, 0x00],
    [0x14, 0x7F, 0x14, 0x7F, 0x14],
    [0x24, 0x2A, 0x7F, 0x2A, 0x12],
    [0x23, 0x13, 0x08, 0x64, 0x62],
    [0x36, 0x49, 0x55, 0x22, 0x50],
    [0x00, 0x05, 0x03, 0x00, 0x00],
    [0x00, 0x1C, 0x22, 0x41, 0x00],
    [0x00, 0x41, 0x22, 0x1C, 0x00],
    [0x14, 0x08, 0x3E, 0x08, 0x14],
    [0x08, 0x08, 0x3E, 0x08, 0x08],
    [0x00, 0x50, 0x30, 0x00, 0x00],
    [0x08, 0x08, 0x08, 0x08, 0x08],
    [0x00, 0x60, 0x60, 0x00, 0x00],
    [0x20, 0x10, 0x08, 0x04, 0x02],
    [0x3E, 0x51, 0x49, 0x45, 0x3E],
    [0x00, 0x42, 0x7F, 0x40, 0x00],
    [0x42, 0x61, 0x51, 0x49, 0x46],
    [0x21, 0x41, 0x45, 0x4B, 0x31],
    [0x18, 0x14, 0x12, 0x7F, 0x10],
    [0x27, 0x45, 0x45, 0x45, 0x39],
    [0x3C, 0x4A, 0x49, 0x49, 0x30],
    [0x01, 0x71, 0x09, 0x05, 0x03],
    [0x36, 0x49, 0x49, 0x49, 0x36],
    [0x06, 0x49, 0x49, 0x29, 0x1E],
    [0x00, 0x36, 0x36, 0x00, 0x00],
    [0x00, 0x56, 0x36, 0x00, 0x00],
    [0x08, 0x14, 0x22, 0x41, 0x00],
    [0x14, 0x14, 0x14, 0x14, 0x14],
    [0x00, 0x41, 0x22, 0x14, 0x08],
    [0x02, 0x01, 0x51, 0x09, 0x06],
    [0x32, 0x49, 0x79, 0x41, 0x3E],
    [0x7E, 0x11, 0x11, 0x11, 0x7E],
    [0x7F, 0x49, 0x49, 0x49, 0x36],
    [0x3E, 0x41, 0x41, 0x41, 0x22],
    [0x7F, 0x41, 0x41, 0x22, 0x1C],
    [0x7F, 0x49, 0x49, 0x49, 0x41],
    [0x7F, 0x09, 0x09, 0x09, 0x01],
    [0x3E, 0x41, 0x49, 0x49, 0x7A],
    [0x7F, 0x08, 0x08, 0x08, 0x7F],
    [0x00, 0x41, 0x7F, 0x41, 0x00],
    [0x20, 0x40, 0x41, 0x3F, 0x01],
    [0x7F, 0x08, 0x14, 0x22, 0x41],
    [0x7F, 0x40, 0x40, 0x40, 0x40],
    [0x7F, 0x02, 0x0C, 0x02, 0x7F],
    [0x7F, 0x04, 0x08, 0x10, 0x7F],
    [0x3E, 0x41, 0x41, 0x41, 0x3E],
    [0x7F, 0x09, 0x09, 0x09, 0x06],
    [0x3E, 0x41, 0x51, 0x21, 0x5E],
    [0x7F, 0x09, 0x19, 0x29, 0x46],
    [0x46, 0x49, 0x49, 0x49, 0x31],
    [0x01, 0x01, 0x7F, 0x01, 0x01],
    [0x3F, 0x40, 0x40, 0x40, 0x3F],
    [0x1F, 0x20, 0x40, 0x20, 0x1F],
    [0x3F, 0x40, 0x38, 0x40, 0x3F],
    [0x63, 0x14, 0x08, 0x14, 0x63],
    [0x07, 0x08, 0x70, 0x08, 0x07],
    [0x61, 0x51, 0x49, 0x45, 0x43],
    [0x00, 0x7F, 0x41, 0x41, 0x00],
    [0x02, 0x04, 0x08, 0x10, 0x20],
    [0x00, 0x41, 0x41, 0x7F, 0x00],
    [0x04, 0x02, 0x01, 0x02, 0x04],
    [0x40, 0x40, 0x40, 0x40, 0x40],
    [0x00, 0x01, 0x02, 0x04, 0x00],
    [0x20, 0x54, 0x54, 0x54, 0x78],
    [0x7F, 0x48, 0x44, 0x44, 0x38],
    [0x38, 0x44, 0x44, 0x44, 0x20],
    [0x38, 0x44, 0x44, 0x48, 0x7F],
    [0x38, 0x54, 0x54, 0x54, 18],
    [0x08, 0x7E, 0x09, 0x01, 0x02],
    [0x0C, 0x52, 0x52, 0x52, 0x3E],
    [0x7F, 0x08, 0x04, 0x04, 0x78],
    [0x00, 0x44, 0x7D, 0x40, 0x00],
    [0x20, 0x40, 0x44, 0x3D, 0x00],
    [0x7F, 0x10, 0x28, 0x44, 0x00],
    [0x00, 0x41, 0x7F, 0x40, 0x00],
    [0x7C, 0x04, 0x18, 0x04, 0x78],
    [0x7C, 0x08, 0x04, 0x04, 0x78],
    [0x38, 0x44, 0x44, 0x44, 0x38],
    [0x7C, 0x14, 0x14, 0x14, 0x08],
    [0x08, 0x14, 0x14, 0x18, 0x7C],
    [0x7C, 0x08, 0x04, 0x04, 0x08],
    [0x48, 0x54, 0x54, 0x54, 0x20],
    [0x04, 0x3F, 0x44, 0x40, 0x20],
    [0x3C, 0x40, 0x40, 0x20, 0x1C],
    [0x1C, 0x20, 0x40, 0x20, 0x1C],
    [0x3C, 0x40, 0x30, 0x40, 0x3C],
    [0x44, 0x28, 0x10, 0x28, 0x44],
    [0x0C, 0x50, 0x50, 0x50, 0x3C],
    [0x44, 0x64, 0x54, 0x4C, 0x44],
    [0x00, 0x08, 0x36, 0x41, 0x00],
    [0x00, 0x00, 0x7F, 0x00, 0x00],
    [0x00, 0x41, 0x36, 0x08, 0x00],
    [0x08, 0x10, 0x20, 0x10, 0x08],
];

/// Column bitmap of one character; anything outside printable ASCII falls
/// back to `?`, the same lossy choice the PDF base-14 font makes.
fn glyph(character: char) -> &'static [u8; 5] {
    match u32::from(character).checked_sub(u32::from(' ')) {
        Some(index) if (index as usize) < GLYPHS.len() => &GLYPHS[index as usize],
        _ => &GLYPHS[u32::from('?') as usize - u32::from(' ') as usize],
    }
}

/// Pixel width of one text run: five glyph columns plus one space column.
fn text_width(text: &str) -> i32 {
    text.chars().count() as i32 * 6 - 1
}

/// Truncates one text run so its bitmap width fits `max_width` pixels, marking a
/// cut with an ASCII ellipsis. Without this a long design title or sheet name
/// would run past the canvas edge and be silently clipped.
fn truncate_text(text: &str, max_width: i32) -> String {
    if text_width(text) <= max_width {
        return text.to_string();
    }
    let max_chars = ((max_width + 1) / 6).max(0) as usize;
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let mut cut: String = text.chars().take(max_chars - 3).collect();
    cut.push_str("...");
    cut
}

/// Maps design millimetres onto canvas pixels with one uniform scale.
struct View {
    origin_x: f64,
    origin_y: f64,
    scale: f64,
    /// Padded view rectangle: the sheet frame the SVG projection borders.
    view_x: f64,
    view_y: f64,
    view_width: f64,
    view_height: f64,
}

impl View {
    /// Fits the padded design bounds into one pixel rectangle and centres it.
    fn in_rect(bounds: (f64, f64, f64, f64), rect: (f64, f64, f64, f64)) -> Self {
        let (min_x, min_y, max_x, max_y) = bounds;
        let (rect_x, rect_y, rect_w, rect_h) = rect;
        let view_x = min_x - PADDING;
        let view_y = min_y - PADDING;
        let view_width = (max_x - min_x + 2.0 * PADDING).max(1.0);
        let view_height = (max_y - min_y + 2.0 * PADDING).max(1.0);
        let scale = (rect_w / view_width).min(rect_h / view_height);
        View {
            origin_x: view_x - (rect_w / scale - view_width) / 2.0 - rect_x / scale,
            origin_y: view_y - (rect_h / scale - view_height) / 2.0 - rect_y / scale,
            scale,
            view_x,
            view_y,
            view_width,
            view_height,
        }
    }

    /// Projects one design point onto canvas pixel coordinates.
    fn project(&self, point: [f64; 2]) -> (i32, i32) {
        let x = ((point[0] - self.origin_x) * self.scale).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        let y = ((point[1] - self.origin_y) * self.scale).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        (x, y)
    }

    /// Converts a stroke width in design units into a stamp radius in pixels.
    fn radius(&self, width: f64) -> i32 {
        (width * self.scale / 2.0).round().max(0.0) as i32
    }
}

/// Fixed-size RGB pixel buffer.
struct Canvas {
    pixels: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Canvas { pixels: vec![BACKGROUND[0]; (WIDTH as usize) * (HEIGHT as usize) * 3] }
    }

    fn set(&mut self, x: i32, y: i32, colour: Rgb) {
        if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
            return;
        }
        let offset = ((y as usize) * (WIDTH as usize) + x as usize) * 3;
        self.pixels[offset..offset + 3].copy_from_slice(&colour);
    }

    /// Stamps a filled square of the given radius around one pixel.
    fn stamp(&mut self, centre: (i32, i32), radius: i32, colour: Rgb) {
        for y in centre.1 - radius..=centre.1 + radius {
            for x in centre.0 - radius..=centre.0 + radius {
                self.set(x, y, colour);
            }
        }
    }

    /// Stamps a filled disc of the given radius around one pixel.
    fn stamp_round(&mut self, centre: (i32, i32), radius: i32, colour: Rgb) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    self.set(centre.0 + dx, centre.1 + dy, colour);
                }
            }
        }
    }

    /// Strokes one axis-aligned pixel rectangle outline.
    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, colour: Rgb) {
        for column in x..x + width {
            self.set(column, y, colour);
            self.set(column, y + height - 1, colour);
        }
        for row in y..y + height {
            self.set(x, row, colour);
            self.set(x + width - 1, row, colour);
        }
    }

    /// Draws one text run with the built-in 5×7 bitmap font, top-left anchored.
    fn text(&mut self, x: i32, y: i32, text: &str, colour: Rgb) {
        let mut cursor = x;
        for character in text.chars() {
            for (column, bits) in glyph(character).iter().enumerate() {
                for row in 0..7 {
                    if bits & (1 << row) != 0 {
                        self.set(cursor + column as i32, y + row, colour);
                    }
                }
            }
            cursor += 6;
        }
    }

    /// Draws one text run centred horizontally on a pixel column.
    fn text_centred(&mut self, centre_x: i32, top: i32, text: &str, colour: Rgb) {
        self.text(centre_x - text_width(text) / 2, top, text, colour);
    }

    /// Draws one line between two design points with the given stroke width.
    fn line(&mut self, view: &View, from: [f64; 2], to: [f64; 2], width: f64, colour: Rgb) {
        let (x0, y0) = view.project(from);
        let (x1, y1) = view.project(to);
        let radius = view.radius(width);
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for step in 0..=steps {
            let x = x0 + (i64::from(x1 - x0) * i64::from(step) / i64::from(steps)) as i32;
            let y = y0 + (i64::from(y1 - y0) * i64::from(step) / i64::from(steps)) as i32;
            self.stamp((x, y), radius, colour);
        }
    }

    /// Draws one filled disc at a design point.
    fn disc(&mut self, view: &View, centre: [f64; 2], radius: f64, colour: Rgb) {
        let (x, y) = view.project(centre);
        let radius = (radius * view.scale).round().max(0.0) as i32;
        self.stamp_round((x, y), radius, colour);
    }

    /// Draws an open polyline between consecutive design points.
    fn polyline(&mut self, view: &View, points: &[[f64; 2]], width: f64, colour: Rgb) {
        for edge in points.windows(2) {
            self.line(view, edge[0], edge[1], width, colour);
        }
    }

    /// Outlines a convex polygon given in design points.
    fn polygon(&mut self, view: &View, points: &[[f64; 2]], width: f64, colour: Rgb) {
        for edge in points.windows(2) {
            self.line(view, edge[0], edge[1], width, colour);
        }
        if points.len() > 2 {
            self.line(view, points[points.len() - 1], points[0], width, colour);
        }
    }

    /// Fills a convex polygon using an even-odd scanline over its edges.
    fn fill_polygon(&mut self, view: &View, points: &[[f64; 2]], colour: Rgb) {
        let projected = points.iter().map(|point| view.project(*point)).collect::<Vec<_>>();
        let min_y = projected.iter().map(|point| point.1).min().unwrap_or(0);
        let max_y = projected.iter().map(|point| point.1).max().unwrap_or(0);
        for y in min_y..=max_y {
            let mut crossings: Vec<i32> = Vec::new();
            for index in 0..projected.len() {
                let (x0, y0) = projected[index];
                let (x1, y1) = projected[(index + 1) % projected.len()];
                if y0 == y1 {
                    continue;
                }
                if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                    crossings.push(x0 + (i64::from(x1 - x0) * i64::from(y - y0) / i64::from(y1 - y0)) as i32);
                }
            }
            crossings.sort_unstable();
            for pair in crossings.chunks(2) {
                if pair.len() == 2 {
                    for x in pair[0]..=pair[1] {
                        self.set(x, y, colour);
                    }
                }
            }
        }
    }
}

/// Paints one sheet's geometry and text through the given view.
fn paint_sheet(canvas: &mut Canvas, view: &View, sheet: &Sheet) {
    for wire in &sheet.wires {
        canvas.line(view, wire.from, wire.to, 0.5, WIRE);
    }
    for junction in &sheet.junctions {
        canvas.disc(view, [junction.x, junction.y], 1.4, JUNCTION);
    }
    for flag in &sheet.no_connects {
        canvas.line(view, [flag.x - 0.7, flag.y - 0.7], [flag.x + 0.7, flag.y + 0.7], 0.35, NO_CONNECT);
        canvas.line(view, [flag.x - 0.7, flag.y + 0.7], [flag.x + 0.7, flag.y - 0.7], 0.35, NO_CONNECT);
    }
    for (net, (from, to)) in net_name_anchors(sheet) {
        let (x, y) = view.project(net_name_point(sheet, from, to, &net));
        canvas.text_centred(x, y - 8, &net, NET_NAME);
    }
    for label in &sheet.labels {
        let width = f64::from(label.text.chars().count() as u32) * 2.8 * 0.6;
        match label.scope {
            crate::LabelScope::Local => {
                canvas.line(view, [label.x, label.y + 0.6], [label.x + width, label.y + 0.6], 0.3, LOCAL_LABEL);
                let (x, y) = view.project([label.x, label.y]);
                canvas.text(x, y - 8, &label.text, LOCAL_LABEL);
            }
            crate::LabelScope::Global => {
                canvas.polygon(
                    view,
                    &[
                        [label.x - 0.6, label.y - 3.4],
                        [label.x - 0.6 + width + 1.2, label.y - 3.4],
                        [label.x - 0.6 + width + 1.2, label.y + 0.8],
                        [label.x - 0.6, label.y + 0.8],
                    ],
                    0.35,
                    GLOBAL_LABEL,
                );
                let (x, y) = view.project([label.x, label.y]);
                canvas.text(x, y - 8, &label.text, GLOBAL_LABEL);
            }
            crate::LabelScope::Hierarchical => {
                canvas.polygon(
                    view,
                    &[
                        [label.x, label.y + 0.8],
                        [label.x + 1.6, label.y - 1.3],
                        [label.x, label.y - 3.4],
                        [label.x + width + 1.2, label.y - 3.4],
                        [label.x + width + 1.2, label.y + 0.8],
                    ],
                    0.35,
                    HIERARCHICAL_LABEL,
                );
                let (x, y) = view.project([label.x + 1.8, label.y]);
                canvas.text(x, y - 8, &label.text, HIERARCHICAL_LABEL);
            }
        }
    }
    for text in &sheet.texts {
        let (x, y) = view.project([text.x, text.y]);
        canvas.text(x, y - 8, &text.text, TEXT);
    }
    for component in &sheet.components {
        let half_height = component_half_height(component);
        let (artwork, keeps_box) = symbol_artwork(component);
        let inside_box = keeps_box || artwork.is_empty();
        if inside_box {
            let corners = [
                [component.x - COMPONENT_HALF_WIDTH, component.y - half_height],
                [component.x + COMPONENT_HALF_WIDTH, component.y - half_height],
                [component.x + COMPONENT_HALF_WIDTH, component.y + half_height],
                [component.x - COMPONENT_HALF_WIDTH, component.y + half_height],
            ]
            .map(|point| rotate_about(component, point));
            canvas.fill_polygon(view, &corners, SYMBOL_FILL);
            canvas.polygon(view, &corners, 0.5, SYMBOL);
        }
        for primitive in &artwork {
            let (source, closed) = match primitive {
                SymbolPrimitive::Stroke(points) => (points, false),
                SymbolPrimitive::Fill(points) => (points, true),
            };
            let points = source.iter().map(|point| rotate_about(component, *point)).collect::<Vec<_>>();
            if closed {
                canvas.fill_polygon(view, &points, SYMBOL);
                canvas.polygon(view, &points, 0.5, SYMBOL);
            }
            else {
                canvas.polyline(view, &points, 0.5, SYMBOL);
            }
        }
        for pin in component_pins(component) {
            let root = rotate_about(component, pin.root);
            let end = rotate_about(component, pin.end);
            canvas.line(view, root, end, 0.35, SYMBOL);
            canvas.disc(view, end, 0.45, SYMBOL);
        }
        let (reference_x, reference_y) = view.project([component.x, component.y - if inside_box { 0.6 } else { 4.5 }]);
        canvas.text_centred(reference_x, reference_y - 8, &component.reference, TEXT);
        if !component.value.is_empty() {
            let (value_x, value_y) = view.project([component.x, component.y + if inside_box { 3.0 } else { 4.5 }]);
            canvas.text_centred(value_x, value_y + 1, &component.value, TEXT);
        }
    }
}

/// Projects the given sheets onto a fixed 1024×768 PNG raster and returns the
/// encoded file bytes.
///
/// The export writer passes every sheet unless one is selected; a single sheet
/// always fits the canvas, and several sheets are stacked into separate
/// labelled bands so their geometry never overlaps, matching the paged PDF.
pub(crate) fn render_png(design: &Design, sheets: &[&Sheet]) -> Vec<u8> {
    let mut canvas = Canvas::new();
    // The buffer starts white and stays opaque RGB, so a viewer never shows a
    // transparent (black) background behind the drawing.
    if sheets.len() > 1 {
        let band_height =
            (i32::try_from(HEIGHT).expect("canvas height") - 2 * MARGIN as i32 - BAND_GAP * (sheets.len() - 1) as i32)
                / sheets.len() as i32;
        for (index, sheet) in sheets.iter().enumerate() {
            let top = MARGIN as i32 + index as i32 * (band_height + BAND_GAP);
            let bounds = drawing_bounds(std::iter::once(*sheet));
            let view = View::in_rect(
                bounds,
                (MARGIN, f64::from(top + BAND_HEADER), f64::from(WIDTH) - 2.0 * MARGIN, f64::from(band_height - BAND_HEADER)),
            );
            canvas.rect(MARGIN as i32, top, WIDTH as i32 - 2 * MARGIN as i32, band_height, FRAME);
            canvas.text(
                MARGIN as i32 + 3,
                top + 3,
                &truncate_text(
                    &format!("SHEET {} OF {} - {}", index + 1, sheets.len(), sheet.name),
                    WIDTH as i32 - 2 * MARGIN as i32 - 6,
                ),
                FRAME,
            );
            paint_sheet(&mut canvas, &view, sheet);
        }
    }
    else {
        let bounds = drawing_bounds(sheets.iter().copied());
        let view = View::in_rect(
            bounds,
            (MARGIN, MARGIN, f64::from(WIDTH) - 2.0 * MARGIN, f64::from(HEIGHT) - 2.0 * MARGIN - f64::from(TITLE_STRIP)),
        );
        // Sheet frame: the padded view rectangle, matching the SVG border.
        let frame = [
            [view.view_x, view.view_y],
            [view.view_x + view.view_width, view.view_y],
            [view.view_x + view.view_width, view.view_y + view.view_height],
            [view.view_x, view.view_y + view.view_height],
        ];
        canvas.polygon(&view, &frame, 0.4, FRAME);
        for sheet in sheets {
            paint_sheet(&mut canvas, &view, sheet);
        }
        paint_title_block(&mut canvas, design, sheets);
    }
    encode_png(WIDTH, HEIGHT, &canvas.pixels)
}

/// Strokes the title-block strip along the canvas bottom and prints the
/// design identity, sheet names, revision and counts in ASCII.
fn paint_title_block(canvas: &mut Canvas, design: &Design, sheets: &[&Sheet]) {
    let strip_top = HEIGHT as i32 - TITLE_STRIP;
    for column in MARGIN as i32..WIDTH as i32 - MARGIN as i32 {
        canvas.set(column, strip_top, FRAME);
    }
    let title = design.title.as_deref().filter(|title| !title.trim().is_empty()).unwrap_or(&design.id);
    let names = sheets.iter().map(|sheet| sheet.name.as_str()).collect::<Vec<_>>().join(", ");
    let rev = if design.rev.trim().is_empty() { "-" } else { design.rev.as_str() };
    let components: usize = sheets.iter().map(|sheet| sheet.components.len()).sum();
    let wires: usize = sheets.iter().map(|sheet| sheet.wires.len()).sum();
    let available = WIDTH as i32 - 2 * MARGIN as i32 - 6;
    canvas.text(MARGIN as i32 + 3, strip_top + 4, &truncate_text(title, available), FRAME);
    canvas.text(
        MARGIN as i32 + 3,
        strip_top + 15,
        &truncate_text(&format!("{names} | {} sheet(s)", sheets.len()), available),
        TEXT,
    );
    let right_line_1 = format!("REV {rev} | units: {}", design.units);
    let right_line_2 = format!("{components} parts / {wires} wires | OpenEDA rev {}", design.revision);
    let right_line_1 = truncate_text(&right_line_1, available);
    let right_line_2 = truncate_text(&right_line_2, available);
    canvas.text(WIDTH as i32 - MARGIN as i32 - 3 - text_width(&right_line_1), strip_top + 4, &right_line_1, TEXT);
    canvas.text(WIDTH as i32 - MARGIN as i32 - 3 - text_width(&right_line_2), strip_top + 15, &right_line_2, TEXT);
}

/// Encodes an RGB byte buffer as a PNG file with uncompressed deflate blocks.
///
/// The encoder is intentionally minimal: filter byte 0 per scanline, stored
/// (uncompressed) deflate blocks, and hand-written CRC-32 and Adler-32. The
/// result is a valid PNG that any decoder accepts, and it is deterministic for
/// identical pixel data.
fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let stride = (width as usize) * 3;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgb[row * stride..(row + 1) * stride]);
    }
    let mut idat = vec![0x78, 0x01];
    let mut offset = 0;
    while offset < raw.len() {
        let end = (offset + 65_535).min(raw.len());
        let block = &raw[offset..end];
        idat.push(if end == raw.len() { 1 } else { 0 });
        idat.extend_from_slice(&(block.len() as u16).to_le_bytes());
        idat.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        idat.extend_from_slice(block);
        offset = end;
    }
    idat.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &idat);
    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// Appends one PNG chunk: length, type, data and CRC over type and data.
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// CRC-32 as specified by ITU-T V.42 and used by PNG chunks.
fn crc32(data: &[u8]) -> u32 {
    let mut value = 0xFFFF_FFFF;
    for byte in data {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = if value & 1 != 0 { (value >> 1) ^ 0xEDB8_8320 } else { value >> 1 };
        }
    }
    !value
}

/// Adler-32 checksum used by zlib streams.
fn adler32(data: &[u8]) -> u32 {
    let (mut low, mut high) = (1u32, 0u32);
    for byte in data {
        low = (low + u32::from(*byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    (high << 16) | low
}

#[cfg(test)]
#[path = "../tests/unit/png.rs"]
mod tests;
