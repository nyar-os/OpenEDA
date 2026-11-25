//! PNG raster projection of the schematic drawing.
//!
//! The rasterizer paints the same geometry as the SVG projection — the sheet
//! frame, wires, junctions, no-connect marks, label boxes and component
//! bodies: standard resistor, capacitor, inductor and diode artwork for
//! two-pin `R`/`C`/`L`/`D` parts, the enclosing ring for three-pin `Q`/`J`
//! parts and the generic box for everything else, all honouring the component
//! rotation, with pin stubs and connection dots. Text is not rasterized
//! because the model carries no font engine; the label outlines mark where
//! labels sit. The encoder writes a valid PNG from uncompressed deflate blocks
//! only, so the same document always produces byte-identical output.

use crate::{
    COMPONENT_HALF_WIDTH, Sheet, SymbolPrimitive, component_half_height, component_pins, drawing_bounds, rotate_about,
    symbol_artwork,
};

/// Width of the fixed raster canvas, in pixels.
pub(crate) const WIDTH: u32 = 1024;
/// Height of the fixed raster canvas, in pixels.
pub(crate) const HEIGHT: u32 = 768;

/// Sheet-frame padding around the drawing content, in design units.
const PADDING: f64 = 10.0;
/// Blank border kept inside the canvas, in pixels.
const MARGIN: f64 = 8.0;

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
    /// Fits the padded view rectangle into the canvas and centres it.
    fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        let view_x = min_x - PADDING;
        let view_y = min_y - PADDING;
        let view_width = (max_x - min_x + 2.0 * PADDING).max(1.0);
        let view_height = (max_y - min_y + 2.0 * PADDING).max(1.0);
        let scale = ((f64::from(WIDTH) - 2.0 * MARGIN) / view_width).min((f64::from(HEIGHT) - 2.0 * MARGIN) / view_height);
        View {
            origin_x: view_x - (f64::from(WIDTH) / scale - view_width) / 2.0,
            origin_y: view_y - (f64::from(HEIGHT) / scale - view_height) / 2.0,
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
        self.stamp((x, y), radius, colour);
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

/// Projects the given sheets onto a fixed 1024×768 PNG raster and returns the
/// encoded file bytes.
///
/// The export writer passes every sheet unless one is selected; the projection
/// always fits the union of the painted sheets, so a selected sheet fills the
/// canvas instead of shrinking into a corner of the design-wide view.
pub(crate) fn render_png(sheets: &[&Sheet]) -> Vec<u8> {
    let (min_x, min_y, max_x, max_y) = drawing_bounds(sheets.iter().copied());
    let view = View::new(min_x, min_y, max_x, max_y);
    let mut canvas = Canvas::new();
    // Sheet frame: the padded view rectangle, matching the SVG border.
    let frame = [
        [view.view_x, view.view_y],
        [view.view_x + view.view_width, view.view_y],
        [view.view_x + view.view_width, view.view_y + view.view_height],
        [view.view_x, view.view_y + view.view_height],
    ];
    canvas.polygon(&view, &frame, 0.4, FRAME);
    for sheet in sheets {
        for wire in &sheet.wires {
            canvas.line(&view, wire.from, wire.to, 0.5, WIRE);
        }
        for junction in &sheet.junctions {
            canvas.disc(&view, [junction.x, junction.y], 1.4, JUNCTION);
        }
        for flag in &sheet.no_connects {
            canvas.line(&view, [flag.x - 0.7, flag.y - 0.7], [flag.x + 0.7, flag.y + 0.7], 0.35, NO_CONNECT);
            canvas.line(&view, [flag.x - 0.7, flag.y + 0.7], [flag.x + 0.7, flag.y - 0.7], 0.35, NO_CONNECT);
        }
        for label in &sheet.labels {
            let width = f64::from(label.text.chars().count() as u32) * 2.8 * 0.6;
            match label.scope {
                crate::LabelScope::Local => {
                    canvas.line(&view, [label.x, label.y + 0.6], [label.x + width, label.y + 0.6], 0.3, LOCAL_LABEL)
                }
                crate::LabelScope::Global => {
                    canvas.polygon(
                        &view,
                        &[
                            [label.x - 0.6, label.y - 3.4],
                            [label.x - 0.6 + width + 1.2, label.y - 3.4],
                            [label.x - 0.6 + width + 1.2, label.y + 0.8],
                            [label.x - 0.6, label.y + 0.8],
                        ],
                        0.35,
                        GLOBAL_LABEL,
                    );
                }
                crate::LabelScope::Hierarchical => {
                    canvas.polygon(
                        &view,
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
                }
            }
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
                canvas.fill_polygon(&view, &corners, SYMBOL_FILL);
                canvas.polygon(&view, &corners, 0.5, SYMBOL);
            }
            for primitive in &artwork {
                let (source, closed) = match primitive {
                    SymbolPrimitive::Stroke(points) => (points, false),
                    SymbolPrimitive::Fill(points) => (points, true),
                };
                let points = source.iter().map(|point| rotate_about(component, *point)).collect::<Vec<_>>();
                if closed {
                    canvas.fill_polygon(&view, &points, SYMBOL);
                    canvas.polygon(&view, &points, 0.5, SYMBOL);
                }
                else {
                    canvas.polyline(&view, &points, 0.5, SYMBOL);
                }
            }
            for pin in component_pins(component) {
                let root = rotate_about(component, pin.root);
                let end = rotate_about(component, pin.end);
                canvas.line(&view, root, end, 0.35, SYMBOL);
                canvas.disc(&view, end, 0.45, SYMBOL);
            }
        }
    }
    encode_png(WIDTH, HEIGHT, &canvas.pixels)
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
