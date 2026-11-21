//! KiCad schematic exchange for the schematic subset this build models.
//!
//! The writer emits KiCad 6/7 S-expressions for the components, wires,
//! junctions, text annotations and net labels the document owns; the reader
//! parses the same subset back and reports every construct it cannot represent
//! as a structured loss instead of dropping it silently.
//!
//! Both directions are deterministic: identifiers are derived from the design
//! identity and the object ids with a fixed hash, so exporting one document
//! twice produces byte-identical files and no random source is involved.

use std::collections::{HashMap, HashSet};

use data_format_sexpr::{Node, parse_all};
use serde_json::{Value, json};

use crate::{Component, Design, Junction, Net, NoConnect, Node as NetNode, ServiceError, Sheet, TextLabel, Wire, invalid};

/// Loss code reported for a KiCad construct the model cannot represent.
pub(crate) const UNSUPPORTED_ELEMENT: &str = "kicad/unsupported-element";
/// Loss code reported when two symbols share one reference designator.
pub(crate) const DUPLICATE_REFERENCE: &str = "kicad/duplicate-reference";
/// Loss code reported when a symbol carries no reference property.
pub(crate) const MISSING_REFERENCE: &str = "kicad/missing-reference";
/// Loss code reported when a design-scoped label becomes a sheet-local net.
pub(crate) const LABEL_SCOPE: &str = "kicad/label-scope";

/// Root tags of the schematic file that carry no design object.
const STRUCTURAL_TAGS: &[&str] = &["version", "generator", "generator_version", "uuid", "paper", "lib_symbols", "title_block"];

/// One parsed S-expression node; atoms and quoted strings both carry text.
fn node_text(node: &Node) -> Option<&str> {
    node.as_atom().or_else(|| node.as_str())
}

/// Renders every sheet of one design as a KiCad schematic.
pub(crate) fn render(design: &Design) -> String {
    let mut out = String::new();
    out.push_str("(kicad_sch (version 20211123) (generator \"open-eda\")\n");
    out.push_str(&format!("  (uuid \"{}\")\n", uuid_for(&format!("{}:root", design.id))));
    out.push_str("  (paper \"A4\")\n");
    out.push_str("  (lib_symbols\n");
    for (lib_id, pins) in library_symbols(design) {
        render_library_symbol(&mut out, &lib_id, &pins);
    }
    out.push_str("  )\n");
    for sheet in &design.sheets {
        for component in &sheet.components {
            render_component(&mut out, design, component);
        }
    }
    for sheet in &design.sheets {
        for wire in &sheet.wires {
            render_wire(&mut out, design, wire);
        }
    }
    for sheet in &design.sheets {
        for (index, junction) in sheet.junctions.iter().enumerate() {
            render_junction(&mut out, design, sheet, index, junction);
        }
        for (index, no_connect) in sheet.no_connects.iter().enumerate() {
            render_no_connect(&mut out, design, sheet, index, no_connect);
        }
        for (index, text) in sheet.texts.iter().enumerate() {
            render_text(&mut out, design, sheet, index, text);
        }
    }
    for sheet in &design.sheets {
        for net in &sheet.nets {
            render_label(&mut out, design, sheet, net);
        }
    }
    out.push_str(")\n");
    out
}

/// Collects one library symbol per distinct `lib_id`, keyed for stable order.
fn library_symbols(design: &Design) -> Vec<(String, Vec<String>)> {
    let mut symbols: Vec<(String, Vec<String>)> = Vec::new();
    for sheet in &design.sheets {
        for component in &sheet.components {
            let lib_id = lib_id_for(component);
            if !symbols.iter().any(|(candidate, _)| *candidate == lib_id) {
                symbols.push((lib_id, component.pins.clone()));
            }
        }
    }
    symbols.sort_by(|left, right| left.0.cmp(&right.0));
    symbols
}

/// Builds a legal, stable library id from the component pins.
///
/// Components whose pins are the numeric sequence `1..n` share one symbol per
/// pin count; any other pin set gets a short hash suffix so the library symbol
/// declares exactly the pins the placed symbols reference.
pub(crate) fn lib_id_for(component: &Component) -> String {
    let numeric = component.pins.iter().enumerate().all(|(index, pin)| *pin == (index + 1).to_string());
    if numeric {
        format!("open-eda:{}pin", component.pins.len())
    }
    else {
        format!("open-eda:{}pin_{:08x}", component.pins.len(), hash64(&component.pins.join(",")) & 0xffff_ffff)
    }
}

fn render_library_symbol(out: &mut String, lib_id: &str, pins: &[String]) {
    out.push_str(&format!("    (symbol {} (pin_names (offset 1.016)) (in_bom yes) (on_board yes)\n", quote(lib_id)));
    out.push_str("      (property \"Reference\" \"U\" (at 0 2.54 0) (effects (font (size 1.27 1.27))))\n");
    out.push_str("      (property \"Value\" \"\" (at 0 -2.54 0) (effects (font (size 1.27 1.27))))\n");
    out.push_str(&format!("      (symbol {}\n", quote(&format!("{lib_id}_1_1"))));
    for (index, pin) in pins.iter().enumerate() {
        let y = -(index as f64) * 2.54;
        out.push_str(&format!(
            "        (pin passive line (at -5.08 {} 0) (length 2.54)\n          (name {} (effects (font (size 1.27 1.27))))\n          (number {} (effects (font (size 1.27 1.27)))))\n",
            num(y),
            quote(pin),
            quote(pin)
        ));
    }
    out.push_str("      )\n    )\n");
}

fn render_component(out: &mut String, design: &Design, component: &Component) {
    out.push_str(&format!(
        "  (symbol (lib_id {}) (at {} {} {}) (unit 1)\n    (in_bom yes) (on_board yes) (dnp no)\n    (uuid \"{}\")\n",
        quote(&lib_id_for(component)),
        num(component.x),
        num(component.y),
        num(component.rotation),
        uuid_for(&format!("{}:component:{}", design.id, component.id))
    ));
    out.push_str(&format!(
        "    (property \"Reference\" {} (at {} {} 0) (effects (font (size 1.27 1.27))))\n",
        quote(&component.reference),
        num(component.x),
        num(component.y - 2.54)
    ));
    out.push_str(&format!(
        "    (property \"Value\" {} (at {} {} 0) (effects (font (size 1.27 1.27))))\n",
        quote(&component.value),
        num(component.x),
        num(component.y + 2.54)
    ));
    out.push_str(&format!(
        "    (property \"Footprint\" {} (at {} {} 0) (effects (font (size 1.27 1.27)) hide))\n",
        quote(&component.footprint),
        num(component.x),
        num(component.y)
    ));
    for (index, pin) in component.pins.iter().enumerate() {
        out.push_str(&format!(
            "    (pin {} (uuid \"{}\"))\n",
            quote(pin),
            uuid_for(&format!("{}:pin:{}:{}:{}", design.id, component.id, index, pin))
        ));
    }
    out.push_str("  )\n");
}

fn render_wire(out: &mut String, design: &Design, wire: &Wire) {
    out.push_str(&format!(
        "  (wire (pts (xy {} {}) (xy {} {})) (stroke (width 0) (type default)) (uuid \"{}\"))\n",
        num(wire.from[0]),
        num(wire.from[1]),
        num(wire.to[0]),
        num(wire.to[1]),
        uuid_for(&format!("{}:wire:{}", design.id, wire.id))
    ));
}

fn render_junction(out: &mut String, design: &Design, sheet: &Sheet, index: usize, junction: &Junction) {
    out.push_str(&format!(
        "  (junction (at {} {}) (diameter 0) (uuid \"{}\"))\n",
        num(junction.x),
        num(junction.y),
        uuid_for(&format!("{}:junction:{}:{}", design.id, sheet.id, index))
    ));
}

fn render_no_connect(out: &mut String, design: &Design, sheet: &Sheet, index: usize, no_connect: &NoConnect) {
    out.push_str(&format!(
        "  (no_connect (at {} {}) (uuid \"{}\"))\n",
        num(no_connect.x),
        num(no_connect.y),
        uuid_for(&format!("{}:no-connect:{}:{}", design.id, sheet.id, index))
    ));
}

fn render_text(out: &mut String, design: &Design, sheet: &Sheet, index: usize, text: &TextLabel) {
    out.push_str(&format!(
        "  (text {} (at {} {} 0) (effects (font (size 1.27 1.27))) (uuid \"{}\"))\n",
        quote(&text.text),
        num(text.x),
        num(text.y),
        uuid_for(&format!("{}:text:{}:{}", design.id, sheet.id, index))
    ));
}

fn render_label(out: &mut String, design: &Design, sheet: &Sheet, net: &Net) {
    let (x, y) = net_position(sheet, net);
    out.push_str(&format!(
        "  (label {} (at {} {} 0) (effects (font (size 1.27 1.27)) (justify left bottom)) (uuid \"{}\"))\n",
        quote(&net.name),
        num(x),
        num(y),
        uuid_for(&format!("{}:net:{}", design.id, net.id))
    ));
}

/// Uses the placement of the component behind the first node, or the origin.
fn net_position(sheet: &Sheet, net: &Net) -> (f64, f64) {
    for node in &net.nodes {
        if let Some(component) = sheet.components.iter().find(|component| component.reference == node.reference) {
            return (component.x, component.y);
        }
    }
    (0.0, 0.0)
}

/// Parses one KiCad schematic into a design plus the constructs it lost.
pub(crate) fn import(text: &str) -> Result<(Design, Vec<Value>), ServiceError> {
    let nodes = parse(text)?;
    let root = nodes
        .iter()
        .find(|node| node.head() == Some("kicad_sch"))
        .ok_or_else(|| invalid("the file is not a KiCad schematic: no `(kicad_sch ...)` root was found"))?;

    let mut losses = Vec::new();
    let library = read_library(root);
    let mut components = Vec::new();
    let mut references = HashSet::new();
    for node in root.children().iter().filter(|node| node.head() == Some("symbol")) {
        let fallback = format!("U{}", components.len() + 1);
        let Some(component) = read_symbol(node, &library, &fallback, &mut losses)
        else {
            continue;
        };
        if !references.insert(component.reference.clone()) {
            losses.push(loss(
                DUPLICATE_REFERENCE,
                format!("a symbol repeats reference `{}` and was skipped", component.reference),
            ));
            continue;
        }
        components.push(component);
    }

    let mut nets: Vec<Net> = Vec::new();
    let mut wires = Vec::new();
    let mut junctions = Vec::new();
    let mut no_connects = Vec::new();
    let mut texts = Vec::new();
    for node in root.children() {
        match node.head() {
            Some("symbol") => {}
            Some("wire") => match read_wire(node, &mut losses) {
                Some((from, to)) => wires.push(Wire { id: String::new(), net: None, from, to }),
                None => losses.push(loss(UNSUPPORTED_ELEMENT, "a wire without two `(xy ...)` points was skipped".to_string())),
            },
            Some("junction") => read_junction(node, &mut junctions, &mut losses),
            Some("no_connect") => read_no_connect(node, &mut no_connects, &mut losses),
            Some("text") => read_text(node, &mut texts, &mut losses),
            Some("label") => {
                let _ = read_label(node, &components, &mut nets, &mut losses);
            }
            Some(tag @ ("global_label" | "hierarchical_label")) => {
                if let Some(name) = read_label(node, &components, &mut nets, &mut losses) {
                    losses.push(loss(
                        LABEL_SCOPE,
                        format!("`{tag}` `{name}` was imported as a sheet-local net label; cross-sheet scope is not modelled"),
                    ));
                }
            }
            Some(tag) if STRUCTURAL_TAGS.contains(&tag) => {}
            Some(tag) => {
                losses.push(loss(UNSUPPORTED_ELEMENT, format!("`{tag}` is outside the imported subset and was skipped")));
            }
            None => losses.push(loss(UNSUPPORTED_ELEMENT, "a root-level value without a tag was skipped".to_string())),
        }
    }

    for (index, component) in components.iter_mut().enumerate() {
        component.id = format!("component-{}", index + 1);
    }
    for (index, net) in nets.iter_mut().enumerate() {
        net.id = format!("net-{}", index + 1);
    }
    for (index, wire) in wires.iter_mut().enumerate() {
        wire.id = format!("wire-{}", index + 1);
    }

    let design = Design {
        id: "design-1".to_string(),
        title: None,
        revision: 1,
        units: "mm".to_string(),
        next_component: components.len() as u64 + 1,
        next_net: nets.len() as u64 + 1,
        next_wire: wires.len() as u64 + 1,
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components,
            nets,
            wires,
            junctions,
            no_connects,
            texts,
        }],
        buses: Vec::new(),
    };
    Ok((design, losses))
}

fn loss(code: &str, detail: String) -> Value {
    json!({ "code": code, "detail": detail })
}

/// Reads the pin numbers each library symbol declares, so placed symbols that
/// omit their `(pin ...)` lines still import with their pins.
fn read_library(root: &Node) -> HashMap<String, Vec<String>> {
    let mut library = HashMap::new();
    let Some(section) = root.find("lib_symbols")
    else {
        return library;
    };
    for symbol in section.children().iter().filter(|node| node.head() == Some("symbol")) {
        let Some(name) = symbol.children().first().and_then(node_text)
        else {
            continue;
        };
        let mut pins = Vec::new();
        collect_pins(symbol, &mut pins);
        library.insert(name.to_string(), pins);
    }
    library
}

fn collect_pins(node: &Node, pins: &mut Vec<String>) {
    for item in node.children() {
        match item.head() {
            Some("pin") => {
                if let Some(number) = child_text(item, "number")
                    && !pins.iter().any(|pin| pin == number)
                {
                    pins.push(number.to_string());
                }
            }
            Some("symbol") => collect_pins(item, pins),
            _ => {}
        }
    }
}

fn read_symbol(
    node: &Node,
    library: &HashMap<String, Vec<String>>,
    fallback: &str,
    losses: &mut Vec<Value>,
) -> Option<Component> {
    let lib_id = child_text(node, "lib_id").or_else(|| child_text(node, "lib_name")).unwrap_or("");
    let Some((x, y, rotation)) = read_at(node)
    else {
        losses.push(loss(UNSUPPORTED_ELEMENT, "a symbol without an `(at x y ...)` placement was skipped".to_string()));
        return None;
    };

    let mut reference = String::new();
    let mut value = String::new();
    let mut footprint = String::new();
    for property in node.children().iter().filter(|item| item.head() == Some("property")) {
        let Some(name) = property.children().first().and_then(node_text)
        else {
            continue;
        };
        let text = property.children().get(1).and_then(node_text).unwrap_or("");
        match name {
            "Reference" => reference = text.to_string(),
            "Value" => value = text.to_string(),
            "Footprint" => footprint = text.to_string(),
            _ => {}
        }
    }

    let mut pins: Vec<String> = node
        .children()
        .iter()
        .filter(|item| item.head() == Some("pin"))
        .filter_map(|pin| pin.children().first().and_then(node_text).map(str::to_string))
        .collect();
    if pins.is_empty() {
        pins = library.get(lib_id).cloned().unwrap_or_default();
    }
    if pins.is_empty() {
        losses.push(loss(
            UNSUPPORTED_ELEMENT,
            format!("symbol `{reference}` declares no pins, so its connectivity cannot be imported"),
        ));
    }
    if reference.trim().is_empty() {
        losses.push(loss(MISSING_REFERENCE, format!("a symbol without a reference property was named `{fallback}`")));
        reference = fallback.to_string();
    }
    Some(Component { id: String::new(), reference, value, footprint, x, y, rotation, pins })
}

fn read_wire(node: &Node, losses: &mut Vec<Value>) -> Option<([f64; 2], [f64; 2])> {
    let pts = node.find("pts")?;
    let points: Vec<[f64; 2]> = pts
        .children()
        .iter()
        .filter(|point| point.head() == Some("xy"))
        .filter_map(|point| {
            let mut values =
                point.children().iter().filter_map(|value| node_text(value).and_then(|text| text.parse::<f64>().ok()));
            Some([values.next()?, values.next()?])
        })
        .collect();
    if points.len() > 2 {
        losses.push(loss(UNSUPPORTED_ELEMENT, format!("a wire with {} points was reduced to its first segment", points.len())));
    }
    Some((*points.first()?, *points.get(1)?))
}

fn read_junction(node: &Node, junctions: &mut Vec<Junction>, losses: &mut Vec<Value>) {
    match read_at(node) {
        Some((x, y, _)) if x.is_finite() && y.is_finite() => junctions.push(Junction { x, y }),
        _ => losses.push(loss(UNSUPPORTED_ELEMENT, "a junction without an `(at x y)` placement was skipped".to_string())),
    }
}

fn read_text(node: &Node, texts: &mut Vec<TextLabel>, losses: &mut Vec<Value>) {
    let Some(text) = node.children().first().and_then(node_text)
    else {
        losses.push(loss(UNSUPPORTED_ELEMENT, "a text without content was skipped".to_string()));
        return;
    };
    match read_at(node) {
        Some((x, y, _)) if x.is_finite() && y.is_finite() => {
            texts.push(TextLabel { text: text.to_string(), x, y });
        }
        _ => losses.push(loss(UNSUPPORTED_ELEMENT, "a text without an `(at x y)` placement was skipped".to_string())),
    }
}

fn read_label(node: &Node, components: &[Component], nets: &mut Vec<Net>, losses: &mut Vec<Value>) -> Option<String> {
    let Some(name) = node.children().first().and_then(node_text)
    else {
        losses.push(loss(UNSUPPORTED_ELEMENT, "a label without a name was skipped".to_string()));
        return None;
    };
    if name.trim().is_empty() {
        losses.push(loss(UNSUPPORTED_ELEMENT, "a label with an empty name was skipped".to_string()));
        return None;
    }
    let name = name.to_string();
    let (x, y, _) = read_at(node).unwrap_or((0.0, 0.0, 0.0));
    let index = match nets.iter().position(|net| net.name == name) {
        Some(index) => index,
        None => {
            nets.push(Net { id: String::new(), name: name.clone(), nodes: Vec::new() });
            nets.len() - 1
        }
    };
    let Some(component) =
        components.iter().find(|component| (component.x - x).abs() < 1.0e-4 && (component.y - y).abs() < 1.0e-4)
    else {
        return Some(name);
    };
    if let Some(pin) = component.pins.first() {
        let node = NetNode { reference: component.reference.clone(), pin: pin.clone() };
        if !nets[index].nodes.contains(&node) {
            nets[index].nodes.push(node);
        }
    }
    Some(name)
}

/// Reads one `(no_connect (at x y) ...)` flag.
fn read_no_connect(node: &Node, no_connects: &mut Vec<NoConnect>, losses: &mut Vec<Value>) {
    match read_at(node) {
        Some((x, y, _)) if x.is_finite() && y.is_finite() => no_connects.push(NoConnect { x, y }),
        _ => {
            losses.push(loss(UNSUPPORTED_ELEMENT, "a no-connect flag without an `(at x y)` placement was skipped".to_string()))
        }
    }
}

fn read_at(node: &Node) -> Option<(f64, f64, f64)> {
    let at = node.find("at")?;
    let mut values = at.children().iter().filter_map(|value| node_text(value).and_then(|text| text.parse::<f64>().ok()));
    let x = values.next()?;
    let y = values.next()?;
    Some((x, y, values.next().unwrap_or(0.0)))
}

fn child_text<'a>(node: &'a Node, head: &str) -> Option<&'a str> {
    node.find(head).and_then(|item| item.children().first()).and_then(node_text)
}

fn parse(text: &str) -> Result<Vec<Node>, ServiceError> {
    let nodes = parse_all(text.trim_start_matches('\u{feff}')).map_err(parse_error)?;
    if nodes.is_empty() {
        return Err(invalid("the KiCad schematic is empty"));
    }
    Ok(nodes)
}

/// Keeps the parse failure code stable and its wording close to the reader it replaced.
fn parse_error(error: data_format_sexpr::ParseError) -> ServiceError {
    let offset = error.offset();
    let message = match error.message() {
        "unexpected `)`" => format!("unexpected `)` at offset {offset}"),
        "unterminated list" => format!("unclosed `(` opened at offset {offset}"),
        "unterminated string" => "unterminated string in the KiCad schematic".to_string(),
        "unterminated string escape" => "unterminated string escape in the KiCad schematic".to_string(),
        other => format!("{other} at offset {offset}"),
    };
    invalid(message)
}

/// Formats a coordinate with four decimals and no redundant trailing zeros.
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let text = format!("{value:.4}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" { "0".to_string() } else { trimmed.to_string() }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

/// Derives a deterministic RFC 4122 shaped identifier from a seed.
fn uuid_for(seed: &str) -> String {
    format!("00000000-0000-0000-0000-{:012x}", hash64(seed) & 0x0000_ffff_ffff_ffff)
}

/// FNV-1a, so identifiers are stable across processes and platforms.
fn hash64(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_keep_four_decimals_and_drop_trailing_zeros() {
        assert_eq!(num(40.0), "40");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(12.34567), "12.3457");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(0.0000001), "0");
        assert_eq!(num(-2.25), "-2.25");
        assert_eq!(num(f64::NAN), "0");
    }

    #[test]
    fn uuids_and_library_ids_are_deterministic() {
        assert_eq!(uuid_for("design-1:component:component-1"), uuid_for("design-1:component:component-1"));
        assert_ne!(uuid_for("design-1:component:component-1"), uuid_for("design-1:component:component-2"));
        let uuid = uuid_for("seed");
        assert!(uuid.starts_with("00000000-0000-0000-0000-"));
        assert_eq!(uuid.len(), 36);

        let numeric = Component {
            id: "component-1".to_string(),
            reference: "R1".to_string(),
            value: "10k".to_string(),
            footprint: String::new(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        };
        assert_eq!(lib_id_for(&numeric), "open-eda:2pin");
        let named = Component { pins: vec!["A".to_string(), "K".to_string()], ..numeric };
        assert!(lib_id_for(&named).starts_with("open-eda:2pin_"));
        assert_eq!(lib_id_for(&named), lib_id_for(&named.clone()));
    }

    #[test]
    fn strings_escape_quotes_and_backslashes() {
        assert_eq!(quote("R1"), "\"R1\"");
        assert_eq!(quote("10k (1%)"), "\"10k (1%)\"");
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        let parsed = parse(&format!("(label {})", quote("a\"b\\c"))).expect("parse");
        assert_eq!(node_text(&parsed[0].children()[0]), Some("a\"b\\c"));
    }
}
