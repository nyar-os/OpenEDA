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

use std::collections::{BTreeMap, HashMap, HashSet};

use data_format_sexpr::{Node, parse_all};
use serde_json::{Value, json};

use crate::{
    Component, Design, HierarchicalSheet, Junction, LabelScope, Net, NetLabel, NoConnect, Node as NetNode, PinType,
    PortDirection, ServiceError, Sheet, SheetPort, TextLabel, Wire, invalid,
};

/// The pins and pin roles one KiCad library symbol declares.
#[derive(Debug, Clone, Default)]
struct LibrarySymbol {
    pins: Vec<String>,
    pin_types: BTreeMap<String, PinType>,
}

/// KiCad spelling of one electrical pin type.
fn pin_type_name(pin_type: PinType) -> &'static str {
    match pin_type {
        PinType::Input => "input",
        PinType::Output => "output",
        PinType::Bidirectional => "bidirectional",
        PinType::TriState => "tri_state",
        PinType::Passive => "passive",
        PinType::PowerIn => "power_in",
        PinType::PowerOut => "power_out",
        PinType::OpenCollector => "open_collector",
        PinType::OpenEmitter => "open_emitter",
        PinType::Unspecified => "unspecified",
    }
}

/// Reads a KiCad pin type; `free` is KiCad's older name for `unspecified`.
fn read_pin_type(name: &str) -> Option<PinType> {
    Some(match name {
        "input" => PinType::Input,
        "output" => PinType::Output,
        "bidirectional" => PinType::Bidirectional,
        "tri_state" => PinType::TriState,
        "passive" => PinType::Passive,
        "power_in" => PinType::PowerIn,
        "power_out" => PinType::PowerOut,
        "open_collector" => PinType::OpenCollector,
        "open_emitter" => PinType::OpenEmitter,
        "free" | "unspecified" => PinType::Unspecified,
        _ => return None,
    })
}

/// Loss code reported for a KiCad construct the model cannot represent.
pub(crate) const UNSUPPORTED_ELEMENT: &str = "kicad/unsupported-element";
/// Loss code reported when two symbols share one reference designator.
pub(crate) const DUPLICATE_REFERENCE: &str = "kicad/duplicate-reference";
/// Loss code reported when a symbol carries no reference property.
pub(crate) const MISSING_REFERENCE: &str = "kicad/missing-reference";
/// Loss code reported when a design-scoped label becomes a sheet-local net.
pub(crate) const LABEL_SCOPE: &str = "kicad/label-scope";
/// Loss code reported for a `.kicad_sym` construct the reader skips.
pub(crate) const SYMBOL_UNSUPPORTED_ELEMENT: &str = "kicad_sym/unsupported-element";
/// Loss code reported for a netlist section the reader does not import.
pub(crate) const NET_UNSUPPORTED_ELEMENT: &str = "net/unsupported-element";
/// Loss code reported when a netlist node names a component the netlist does not declare.
pub(crate) const NET_MISSING_COMPONENT: &str = "net/missing-component";
/// Loss code reported when two netlist components share one reference.
pub(crate) const NET_DUPLICATE_REFERENCE: &str = "net/duplicate-reference";
/// Loss code reported when a netlist component carries no reference designator.
pub(crate) const NET_MISSING_REFERENCE: &str = "net/missing-reference";

/// Netlist root children that carry no placed design object.
const NET_STRUCTURAL_TAGS: &[&str] = &["version", "design", "libraries", "components", "nets"];

/// Root tags of the schematic file that carry no design object.
const STRUCTURAL_TAGS: &[&str] = &["version", "generator", "generator_version", "uuid", "paper", "lib_symbols", "title_block"];

/// Reads the drawing properties this model keeps from a `title_block`.
///
/// Title, date, revision and company map to the document. Any other child of
/// the block, such as a numbered comment, is reported as a loss instead of
/// being dropped silently.
fn read_title_block(root: &Node, losses: &mut Vec<Value>) -> (Option<String>, String, String, String) {
    let mut title = None;
    let mut rev = String::new();
    let mut company = String::new();
    let mut date = String::new();
    let Some(block) = root.find("title_block")
    else {
        return (title, rev, company, date);
    };
    for child in block.children() {
        let Some(head) = child.head()
        else {
            continue;
        };
        let text = child.children().first().and_then(node_text).unwrap_or("");
        match head {
            "title" => {
                if !text.trim().is_empty() {
                    title = Some(text.to_string());
                }
            }
            "rev" => rev = text.to_string(),
            "company" => company = text.to_string(),
            "date" => date = text.to_string(),
            other => losses.push(loss(
                UNSUPPORTED_ELEMENT,
                format!("`title_block` field `{other}` is outside the imported subset and was skipped"),
            )),
        }
    }
    (title, rev, company, date)
}

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
    render_title_block(&mut out, design);
    out.push_str("  (lib_symbols\n");
    for (lib_id, symbol) in library_symbols(design) {
        render_library_symbol(&mut out, &lib_id, &symbol);
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
            if !sheet.labels.iter().any(|label| label.text == net.name) {
                render_label(&mut out, design, sheet, net);
            }
        }
        for (index, label) in sheet.labels.iter().enumerate() {
            render_net_label(&mut out, design, sheet, index, label);
        }
    }
    for (index, sheet) in design.hierarchy.iter().enumerate() {
        render_hierarchical_sheet(&mut out, design, index, sheet);
    }
    out.push_str(")\n");
    out
}

/// Writes one hierarchical sheet symbol with its name, child file and ports.
fn render_hierarchical_sheet(out: &mut String, design: &Design, index: usize, sheet: &HierarchicalSheet) {
    out.push_str(&format!(
        "  (sheet (at {} {}) (size {} {}) (fields_autoplaced)\n",
        num(sheet.x),
        num(sheet.y),
        num(sheet.width),
        num(sheet.height)
    ));
    out.push_str("    (stroke (width 0.1524) (type solid))\n");
    out.push_str("    (fill (color 0 0 0 0.0000))\n");
    out.push_str(&format!("    (uuid \"{}\")\n", uuid_for(&format!("{}:hierarchy:{}:{}", design.id, sheet.id, index))));
    out.push_str(&format!(
        "    (property \"Sheetname\" {} (id 0) (at {} {} 0)\n",
        quote(&sheet.name),
        num(sheet.x),
        num(sheet.y)
    ));
    out.push_str("      (effects (font (size 1.27 1.27)) (justify left bottom))\n    )\n");
    out.push_str(&format!(
        "    (property \"Sheetfile\" {} (id 1) (at {} {} 0)\n",
        quote(&sheet.file),
        num(sheet.x),
        num(sheet.y + sheet.height)
    ));
    out.push_str("      (effects (font (size 1.27 1.27)) (justify left top))\n    )\n");
    for (port_index, port) in sheet.ports.iter().enumerate() {
        out.push_str(&format!(
            "    (pin {} {} (at {} {} 0)\n",
            quote(&port.name),
            port_direction_name(port.direction),
            num(port.x),
            num(port.y)
        ));
        out.push_str("      (effects (font (size 1.27 1.27)))\n");
        out.push_str(&format!(
            "      (uuid \"{}\")\n    )\n",
            uuid_for(&format!("{}:hierarchy:{}:port:{}:{}", design.id, sheet.id, port_index, port.name))
        ));
    }
    out.push_str("  )\n");
}

/// KiCad sheet-pin direction for one modelled port.
fn port_direction_name(direction: PortDirection) -> &'static str {
    match direction {
        PortDirection::Input => "input",
        PortDirection::Output => "output",
        PortDirection::Bidirectional => "bidirectional",
        PortDirection::TriState => "tri_state",
        PortDirection::Passive => "passive",
    }
}

/// Reads the direction of a KiCad sheet pin; unknown names stay unmapped.
fn read_port_direction(name: &str) -> Option<PortDirection> {
    match name {
        "input" => Some(PortDirection::Input),
        "output" => Some(PortDirection::Output),
        "bidirectional" => Some(PortDirection::Bidirectional),
        "tri_state" => Some(PortDirection::TriState),
        "passive" => Some(PortDirection::Passive),
        _ => None,
    }
}

/// Writes the drawing title block, omitting it when every property is empty.
fn render_title_block(out: &mut String, design: &Design) {
    let mut fields = Vec::new();
    if let Some(title) = design.title.as_deref().filter(|title| !title.is_empty()) {
        fields.push(("title", title));
    }
    if !design.date.is_empty() {
        fields.push(("date", design.date.as_str()));
    }
    if !design.rev.is_empty() {
        fields.push(("rev", design.rev.as_str()));
    }
    if !design.company.is_empty() {
        fields.push(("company", design.company.as_str()));
    }
    if fields.is_empty() {
        return;
    }
    out.push_str("  (title_block\n");
    for (key, value) in fields {
        out.push_str(&format!("    ({key} {})\n", quote(value)));
    }
    out.push_str("  )\n");
}

/// Collects one library symbol per distinct `lib_id`, keyed for stable order.
fn library_symbols(design: &Design) -> Vec<(String, LibrarySymbol)> {
    let mut symbols: Vec<(String, LibrarySymbol)> = Vec::new();
    for sheet in &design.sheets {
        for component in &sheet.components {
            let lib_id = lib_id_for(component);
            if !symbols.iter().any(|(candidate, _)| *candidate == lib_id) {
                symbols.push((lib_id, LibrarySymbol { pins: component.pins.clone(), pin_types: component.pin_types.clone() }));
            }
        }
    }
    symbols.sort_by(|left, right| left.0.cmp(&right.0));
    symbols
}

/// Builds a legal, stable library id from the component pins and pin roles.
///
/// Components whose pins are the numeric sequence `1..n` share one symbol per
/// pin count; any other pin set gets a short hash suffix so the library symbol
/// declares exactly the pins the placed symbols reference. A component with
/// typed pins adds a second suffix, so symbols that differ only in a pin role
/// never share one library entry.
pub(crate) fn lib_id_for(component: &Component) -> String {
    let numeric = component.pins.iter().enumerate().all(|(index, pin)| *pin == (index + 1).to_string());
    let base = if numeric {
        format!("open-eda:{}pin", component.pins.len())
    }
    else {
        format!("open-eda:{}pin_{:08x}", component.pins.len(), hash64(&component.pins.join(",")) & 0xffff_ffff)
    };
    if component.pin_types.is_empty() {
        return base;
    }
    let signature = component
        .pins
        .iter()
        .map(|pin| {
            let pin_type = component.pin_types.get(pin).copied().unwrap_or(PinType::Passive);
            format!("{pin}={}", pin_type_name(pin_type))
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{base}_{:08x}", hash64(&signature) & 0xffff_ffff)
}

fn render_library_symbol(out: &mut String, lib_id: &str, symbol: &LibrarySymbol) {
    out.push_str(&format!("    (symbol {} (pin_names (offset 1.016)) (in_bom yes) (on_board yes)\n", quote(lib_id)));
    out.push_str("      (property \"Reference\" \"U\" (at 0 2.54 0) (effects (font (size 1.27 1.27))))\n");
    out.push_str("      (property \"Value\" \"\" (at 0 -2.54 0) (effects (font (size 1.27 1.27))))\n");
    out.push_str(&format!("      (symbol {}\n", quote(&format!("{lib_id}_1_1"))));
    for (index, pin) in symbol.pins.iter().enumerate() {
        let y = -(index as f64) * 2.54;
        let pin_type = symbol.pin_types.get(pin).copied().unwrap_or(PinType::Passive);
        out.push_str(&format!(
            "        (pin {} line (at -5.08 {} 0) (length 2.54)\n          (name {} (effects (font (size 1.27 1.27))))\n          (number {} (effects (font (size 1.27 1.27)))))\n",
            pin_type_name(pin_type),
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
    // Custom properties are written as hidden KiCad symbol properties, so a
    // property assigned from the SDK or CLI round-trips through the schematic.
    for (key, value) in &component.properties {
        out.push_str(&format!(
            "    (property {} {} (at {} {} 0) (effects (font (size 1.27 1.27)) hide))\n",
            quote(key),
            quote(value),
            num(component.x),
            num(component.y)
        ));
    }
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

/// Writes one net label node, including its cross-sheet scope.
fn render_net_label(out: &mut String, design: &Design, sheet: &Sheet, index: usize, label: &NetLabel) {
    let tag = match label.scope {
        LabelScope::Local => "label",
        LabelScope::Global => "global_label",
        LabelScope::Hierarchical => "hierarchical_label",
    };
    let shape = match label.scope {
        LabelScope::Local => "",
        LabelScope::Global | LabelScope::Hierarchical => " (shape passive)",
    };
    out.push_str(&format!(
        "  ({} {}{} (at {} {} {}) (effects (font (size 1.27 1.27)) (justify {} bottom)) (uuid \"{}\"))\n",
        tag,
        quote(&label.text),
        shape,
        num(label.x),
        num(label.y),
        num(label.rotation),
        label.justify,
        uuid_for(&format!("{}:label:{}:{}:{}", design.id, sheet.id, index, label.id))
    ));
}

/// Anchors the synthesized label on the net's own wires: the wire end closest to
/// the component behind the first node, so the label sits on a pin connection
/// point. Falls back to that component's anchor, then the origin.
fn net_position(sheet: &Sheet, net: &Net) -> (f64, f64) {
    let anchor = net
        .nodes
        .first()
        .and_then(|node| sheet.components.iter().find(|component| component.reference == node.reference))
        .map(|component| (component.x, component.y));
    let mut best: Option<((f64, f64), f64)> = None;
    for wire in sheet.wires.iter().filter(|wire| wire.net.as_deref() == Some(net.name.as_str())) {
        for end in [wire.from, wire.to] {
            let point = (end[0], end[1]);
            let distance = anchor.map_or(0.0, |(x, y)| (point.0 - x).hypot(point.1 - y));
            let better = match best {
                None => true,
                Some((_, score)) => distance < score,
            };
            if better {
                best = Some((point, distance));
            }
        }
    }
    if let Some((point, _)) = best {
        return point;
    }
    if let Some(point) = anchor {
        return point;
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
    let library = read_library(root, &mut losses);
    let (title, rev, company, date) = read_title_block(root, &mut losses);
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
    let mut labels: Vec<NetLabel> = Vec::new();
    let mut hierarchy = Vec::new();
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
            Some("sheet") => read_hierarchical_sheet(node, &mut hierarchy, &mut losses),
            Some("label") => {
                let _ = read_label(node, &components, &mut nets, &mut labels, &mut losses, LabelScope::Local);
            }
            Some(tag @ ("global_label" | "hierarchical_label")) => {
                let scope = if tag == "global_label" { LabelScope::Global } else { LabelScope::Hierarchical };
                if let Some(name) = read_label(node, &components, &mut nets, &mut labels, &mut losses, scope) {
                    losses.push(loss(
                        LABEL_SCOPE,
                        format!(
                            "`{tag}` `{name}` keeps its scope on the label, but its net stays sheet-local; cross-sheet nets are not merged"
                        ),
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
    for (index, sheet) in hierarchy.iter_mut().enumerate() {
        sheet.id = format!("hierarchy-{}", index + 1);
    }
    for (index, label) in labels.iter_mut().enumerate() {
        label.id = format!("label-{}", index + 1);
    }

    let design = Design {
        id: "design-1".to_string(),
        title,
        rev,
        company,
        date,
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
            labels,
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: crate::DesignRules::default(),
        hierarchy,
    };
    Ok((design, losses))
}

/// Reads a KiCad 6/7 netlist and turns its components and nets into a design.
///
/// A netlist carries no placement, wires, junctions, labels or bus declarations,
/// so the imported components are laid out on a deterministic grid and a net's
/// nodes are the only evidence of its pins: an unconnected pin cannot appear in
/// a netlist and is therefore not invented. A node that names an undeclared
/// component, a duplicate reference and a component without a reference each add
/// a structured loss instead of being dropped silently.
pub(crate) fn import_net(text: &str) -> Result<(Design, Vec<Value>), ServiceError> {
    let nodes = parse(text)?;
    let root = nodes
        .iter()
        .find(|node| node.head() == Some("export"))
        .ok_or_else(|| invalid("the file is not a KiCad netlist: no `(export ...)` root was found"))?;

    let mut losses = Vec::new();
    let mut components: Vec<Component> = Vec::new();
    let mut references = HashSet::new();
    if let Some(section) = root.find("components") {
        for node in section.children().iter().filter(|node| node.head() == Some("comp")) {
            let reference = child_text(node, "ref").unwrap_or("").trim().to_string();
            if reference.is_empty() {
                losses.push(loss(NET_MISSING_REFERENCE, "a netlist component without `(ref ...)` was skipped".to_string()));
                continue;
            }
            if !references.insert(reference.clone()) {
                losses.push(loss(
                    NET_DUPLICATE_REFERENCE,
                    format!("a netlist component repeats reference `{reference}` and was skipped"),
                ));
                continue;
            }
            let index = components.len();
            components.push(Component {
                id: String::new(),
                reference,
                value: child_text(node, "value").unwrap_or("").to_string(),
                footprint: child_text(node, "footprint").unwrap_or("").to_string(),
                x: 25.0 + (index % 8) as f64 * 25.0,
                y: 25.0 + (index / 8) as f64 * 25.0,
                rotation: 0.0,
                pins: Vec::new(),
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            });
        }
    }

    let mut nets: Vec<Net> = Vec::new();
    if let Some(section) = root.find("nets") {
        for node in section.children().iter().filter(|node| node.head() == Some("net")) {
            let Some(name) = child_text(node, "name").map(str::trim).filter(|name| !name.is_empty())
            else {
                losses.push(loss(NET_UNSUPPORTED_ELEMENT, "a netlist net without a name was skipped".to_string()));
                continue;
            };
            let net_index = match nets.iter().position(|net| net.name == name) {
                Some(index) => index,
                None => {
                    nets.push(Net { id: String::new(), name: name.to_string(), nodes: Vec::new() });
                    nets.len() - 1
                }
            };
            for item in node.children().iter().filter(|item| item.head() == Some("node")) {
                let reference = child_text(item, "ref").unwrap_or("").trim().to_string();
                let pin = child_text(item, "pin").unwrap_or("").trim().to_string();
                if reference.is_empty() || pin.is_empty() {
                    losses.push(loss(
                        NET_UNSUPPORTED_ELEMENT,
                        format!("a node of net `{name}` without both `(ref ...)` and `(pin ...)` was skipped"),
                    ));
                    continue;
                }
                let Some(component) = components.iter_mut().find(|component| component.reference == reference)
                else {
                    losses.push(loss(
                        NET_MISSING_COMPONENT,
                        format!("net `{name}` references `{reference}`, which the netlist does not declare"),
                    ));
                    continue;
                };
                if !component.pins.iter().any(|entry| entry == &pin) {
                    component.pins.push(pin.clone());
                }
                if !nets[net_index].nodes.iter().any(|node| node.reference == reference && node.pin == pin) {
                    nets[net_index].nodes.push(NetNode { reference: reference.clone(), pin });
                }
            }
        }
    }

    for node in root.children() {
        match node.head() {
            Some(tag) if NET_STRUCTURAL_TAGS.contains(&tag) => {}
            Some(tag) => losses.push(loss(
                NET_UNSUPPORTED_ELEMENT,
                format!("netlist section `{tag}` is outside the imported subset and was skipped"),
            )),
            None => {}
        }
    }

    for (index, component) in components.iter_mut().enumerate() {
        component.id = format!("component-{}", index + 1);
    }
    for (index, net) in nets.iter_mut().enumerate() {
        net.id = format!("net-{}", index + 1);
    }

    let component_count = components.len();
    let net_count = nets.len();
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        next_component: component_count as u64 + 1,
        next_net: net_count as u64 + 1,
        next_wire: 1,
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components,
            nets,
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: crate::DesignRules::default(),
        hierarchy: Vec::new(),
    };
    Ok((design, losses))
}

/// Compares an open design with a netlist-derived design.
///
/// Components are keyed by reference and compared by value and footprint; nets
/// are keyed by name and compared by their `REF.PIN` node sets. The reference
/// side is the design, so `added` means present only in the netlist, `removed`
/// means present only in the design, and `modified` lists both values. Placement,
/// wires and pin ordering are not part of the comparison because a netlist does
/// not carry them.
pub(crate) fn compare_net(design: &Design, imported: &Design) -> Value {
    let left_components = net_components(design);
    let right_components = net_components(imported);
    let mut added_components = Vec::new();
    let mut removed_components = Vec::new();
    let mut modified_components = Vec::new();
    for (reference, (value, footprint)) in &right_components {
        match left_components.get(reference) {
            None => added_components.push(json!({ "reference": reference, "value": value, "footprint": footprint })),
            Some((left_value, left_footprint)) if left_value != value || left_footprint != footprint => {
                modified_components.push(json!({
                    "reference": reference,
                    "changes": {
                        "value": { "from": left_value, "to": value },
                        "footprint": { "from": left_footprint, "to": footprint },
                    },
                }));
            }
            Some(_) => {}
        }
    }
    for (reference, (value, footprint)) in &left_components {
        if !right_components.contains_key(reference) {
            removed_components.push(json!({ "reference": reference, "value": value, "footprint": footprint }));
        }
    }

    let left_nets = net_membership(design);
    let right_nets = net_membership(imported);
    let mut added_nets = Vec::new();
    let mut removed_nets = Vec::new();
    let mut modified_nets = Vec::new();
    for (name, nodes) in &right_nets {
        match left_nets.get(name) {
            None => added_nets.push(json!({ "name": name, "nodes": nodes })),
            Some(left_nodes) if left_nodes != nodes => modified_nets.push(json!({
                "name": name,
                "addedNodes": nodes.difference(left_nodes).collect::<Vec<_>>(),
                "removedNodes": left_nodes.difference(nodes).collect::<Vec<_>>(),
            })),
            Some(_) => {}
        }
    }
    for (name, nodes) in &left_nets {
        if !right_nets.contains_key(name) {
            removed_nets.push(json!({ "name": name, "nodes": nodes }));
        }
    }

    let equal = added_components.is_empty()
        && removed_components.is_empty()
        && modified_components.is_empty()
        && added_nets.is_empty()
        && removed_nets.is_empty()
        && modified_nets.is_empty();
    json!({
        "format": "open-eda-netlist-diff",
        "version": 1,
        "equal": equal,
        "components": {
            "added": added_components,
            "removed": removed_components,
            "modified": modified_components,
        },
        "nets": {
            "added": added_nets,
            "removed": removed_nets,
            "modified": modified_nets,
        },
    })
}

/// Flattens every sheet into reference -> (value, footprint) for comparison.
fn net_components(design: &Design) -> BTreeMap<String, (String, String)> {
    let mut components = BTreeMap::new();
    for component in design.sheets.iter().flat_map(|sheet| sheet.components.iter()) {
        components.insert(component.reference.clone(), (component.value.clone(), component.footprint.clone()));
    }
    components
}

/// Flattens every sheet into net name -> sorted `REF.PIN` node set.
fn net_membership(design: &Design) -> BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut nets: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for net in design.sheets.iter().flat_map(|sheet| sheet.nets.iter()) {
        let entry = nets.entry(net.name.clone()).or_default();
        for node in &net.nodes {
            entry.insert(format!("{}.{}", node.reference, node.pin));
        }
    }
    nets
}

fn loss(code: &str, detail: String) -> Value {
    json!({ "code": code, "detail": detail })
}

/// Reads the pin numbers each library symbol declares, so placed symbols that
/// omit their `(pin ...)` lines still import with their pins.
fn read_library(root: &Node, losses: &mut Vec<Value>) -> HashMap<String, LibrarySymbol> {
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
        let mut entry = LibrarySymbol::default();
        collect_pins(symbol, &mut entry, losses);
        library.insert(name.to_string(), entry);
    }
    library
}

fn collect_pins(node: &Node, symbol: &mut LibrarySymbol, losses: &mut Vec<Value>) {
    for item in node.children() {
        match item.head() {
            Some("pin") => {
                let Some(number) = child_text(item, "number")
                else {
                    continue;
                };
                if symbol.pins.iter().any(|pin| pin == number) {
                    continue;
                }
                symbol.pins.push(number.to_string());
                let type_name = item.children().first().and_then(node_text).unwrap_or("");
                match read_pin_type(type_name) {
                    Some(pin_type) => {
                        symbol.pin_types.insert(number.to_string(), pin_type);
                    }
                    None => losses.push(loss(
                        UNSUPPORTED_ELEMENT,
                        format!("pin `{number}` has electrical type `{type_name}`, which the model does not carry"),
                    )),
                }
            }
            Some("symbol") => collect_pins(item, symbol, losses),
            _ => {}
        }
    }
}

fn read_symbol(
    node: &Node,
    library: &HashMap<String, LibrarySymbol>,
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
    let mut properties = BTreeMap::new();
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
            _ => {
                let key = name.trim();
                // Custom properties use the same key rule as the user-facing
                // edit operation: non-empty and free of whitespace and
                // control characters, so every imported key can be addressed.
                if !key.is_empty() && !key.chars().any(|character| character.is_whitespace() || character.is_control()) {
                    properties.insert(key.to_string(), text.to_string());
                }
            }
        }
    }

    let mut pins: Vec<String> = node
        .children()
        .iter()
        .filter(|item| item.head() == Some("pin"))
        .filter_map(|pin| pin.children().first().and_then(node_text).map(str::to_string))
        .collect();
    let mut pin_types = BTreeMap::new();
    if let Some(symbol) = library.get(lib_id) {
        if pins.is_empty() {
            pins = symbol.pins.clone();
        }
        pin_types = symbol.pin_types.clone();
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
    Some(Component { id: String::new(), reference, value, footprint, x, y, rotation, pins, pin_types, properties })
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

fn read_label(
    node: &Node,
    components: &[Component],
    nets: &mut Vec<Net>,
    labels: &mut Vec<NetLabel>,
    losses: &mut Vec<Value>,
    scope: LabelScope,
) -> Option<String> {
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
    let (x, y, rotation) = read_at(node).unwrap_or((0.0, 0.0, 0.0));
    labels.push(NetLabel {
        id: String::new(),
        text: name.clone(),
        x,
        y,
        scope,
        rotation,
        justify: read_justify(node).unwrap_or_else(|| "left".to_string()),
    });
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

/// Reads the horizontal justification of one label, when the file sets it.
fn read_justify(node: &Node) -> Option<String> {
    let effects = node.find("effects")?;
    let justify = effects.find("justify")?;
    justify.children().iter().find_map(|child| child.as_atom().map(str::to_string))
}

/// Reads one `(no_connect (at x y) ...)` flag.
fn read_no_connect(node: &Node, no_connects: &mut Vec<NoConnect>, losses: &mut Vec<Value>) {
    match read_at(node) {
        Some((x, y, _)) if x.is_finite() && y.is_finite() => no_connects.push(NoConnect { x, y, node: None }),
        _ => {
            losses.push(loss(UNSUPPORTED_ELEMENT, "a no-connect flag without an `(at x y)` placement was skipped".to_string()))
        }
    }
}

/// Reads one `(sheet ...)` symbol: name, child file, geometry and ports.
///
/// The child schematic is a separate file, so this reader keeps the symbol as a
/// declaration instead of pretending to import that sheet's contents.
fn read_hierarchical_sheet(node: &Node, hierarchy: &mut Vec<HierarchicalSheet>, losses: &mut Vec<Value>) {
    let Some((x, y, _)) = read_at(node)
    else {
        losses.push(loss(UNSUPPORTED_ELEMENT, "a hierarchical sheet without an `(at x y)` placement was skipped".to_string()));
        return;
    };
    let (width, height) = node
        .find("size")
        .and_then(|size| {
            let mut values =
                size.children().iter().filter_map(|value| node_text(value).and_then(|text| text.parse::<f64>().ok()));
            Some((values.next()?, values.next()?))
        })
        .unwrap_or((0.0, 0.0));
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        losses
            .push(loss(UNSUPPORTED_ELEMENT, "a hierarchical sheet without a finite position and size was skipped".to_string()));
        return;
    }
    let mut name = String::new();
    let mut file = String::new();
    for property in node.children().iter().filter(|item| item.head() == Some("property")) {
        let Some(key) = property.children().first().and_then(node_text)
        else {
            continue;
        };
        let value = property.children().get(1).and_then(node_text).unwrap_or("");
        match key {
            "Sheetname" => name = value.to_string(),
            "Sheetfile" => file = value.to_string(),
            _ => {}
        }
    }
    if name.trim().is_empty() || file.trim().is_empty() {
        losses.push(loss(
            UNSUPPORTED_ELEMENT,
            "a hierarchical sheet without a `Sheetname` and `Sheetfile` property was skipped".to_string(),
        ));
        return;
    }
    let mut ports = Vec::new();
    for pin in node.children().iter().filter(|item| item.head() == Some("pin")) {
        let Some(port_name) = pin.children().first().and_then(node_text)
        else {
            losses.push(loss(UNSUPPORTED_ELEMENT, "a hierarchical sheet port without a name was skipped".to_string()));
            continue;
        };
        let direction_name = pin.children().get(1).and_then(node_text).unwrap_or("");
        let Some(direction) = read_port_direction(direction_name)
        else {
            losses.push(loss(
                UNSUPPORTED_ELEMENT,
                format!("hierarchical sheet port `{port_name}` has unknown direction `{direction_name}` and was skipped"),
            ));
            continue;
        };
        match read_at(pin) {
            Some((port_x, port_y, _)) if port_x.is_finite() && port_y.is_finite() => {
                ports.push(SheetPort { name: port_name.to_string(), direction, x: port_x, y: port_y });
            }
            _ => losses.push(loss(
                UNSUPPORTED_ELEMENT,
                format!("hierarchical sheet port `{port_name}` has no `(at x y)` placement and was skipped"),
            )),
        }
    }
    hierarchy.push(HierarchicalSheet { id: String::new(), name, file, x, y, width, height, ports });
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

/// Renders the component pins of one design as a KiCad 6/7 symbol library.
///
/// One symbol is written per distinct library id the schematic writer derives,
/// with one nested `_1_1` unit holding the pins. Only the pin interface is
/// exported: values, footprints and placement stay in the schematic writer, so
/// this projection is deterministic and never invents a graphic body.
pub(crate) fn render_symbol_library(design: &Design) -> String {
    let mut out = String::new();
    out.push_str("(kicad_symbol_lib (version 20231120) (generator \"open-eda\")\n");
    for (lib_id, symbol) in library_symbols(design) {
        let name = lib_id.replace(':', "_");
        out.push_str(&format!("  (symbol {}\n", quote(&name)));
        out.push_str("    (pin_names (offset 1.016))\n");
        out.push_str("    (exclude_from_sim no) (in_bom yes) (on_board yes)\n");
        out.push_str("    (property \"Reference\" \"U\" (at 0 2.54 0) (effects (font (size 1.27 1.27))))\n");
        out.push_str("    (property \"Value\" \"\" (at 0 -2.54 0) (effects (font (size 1.27 1.27))))\n");
        out.push_str(&format!("    (symbol {}\n", quote(&format!("{name}_1_1"))));
        for (index, pin) in symbol.pins.iter().enumerate() {
            let y = -(index as f64) * 2.54;
            let pin_type = symbol.pin_types.get(pin).copied().unwrap_or(PinType::Passive);
            out.push_str(&format!(
                "      (pin {} line (at -5.08 {} 0) (length 2.54)\n        (name {} (effects (font (size 1.27 1.27))))\n        (number {} (effects (font (size 1.27 1.27)))))\n",
                pin_type_name(pin_type),
                num(y),
                quote(pin),
                quote(pin)
            ));
        }
        out.push_str("    )\n  )\n");
    }
    out.push_str(")\n");
    out
}

/// Tag written into the deterministic symbol library check report.
pub(crate) const LIBRARY_CHECK_FORMAT: &str = "open-eda-library-check";
/// Version written into the symbol library check report.
pub(crate) const LIBRARY_CHECK_VERSION: u64 = 1;

/// Reads a KiCad 6/7 symbol library and reports the symbols it declares.
///
/// Only the declaration is read: each symbol reports its name, unit count and
/// pins with their number, name, electrical role and unit. A construct outside
/// that subset adds a `kicad_sym/unsupported-element` loss entry instead of
/// being dropped silently.
pub(crate) fn import_symbol_library(text: &str) -> Result<Value, ServiceError> {
    let nodes = parse(text)?;
    let root = nodes
        .iter()
        .find(|node| node.head() == Some("kicad_symbol_lib"))
        .ok_or_else(|| invalid("the file is not a KiCad symbol library: no `(kicad_symbol_lib ...)` root was found"))?;

    let mut losses = Vec::new();
    let mut symbols = Vec::new();
    let mut total_pins = 0usize;
    for node in root.children() {
        match node.head() {
            Some("symbol") => {
                if let Some((symbol, pin_count)) = read_library_symbol(node, &mut losses) {
                    total_pins += pin_count;
                    symbols.push(symbol);
                }
            }
            Some("version" | "generator" | "generator_version") => {}
            Some(tag) => losses.push(loss(
                SYMBOL_UNSUPPORTED_ELEMENT,
                format!("`{tag}` is outside the imported symbol subset and was skipped"),
            )),
            None => {}
        }
    }
    let symbol_count = symbols.len();

    Ok(json!({
        "kind": "kicad_sym",
        "symbols": symbols,
        "totals": { "symbols": symbol_count, "pins": total_pins },
        "losses": losses,
    }))
}

/// Reads a KiCad 6/7 symbol library and validates its declarations.
///
/// The check reads the same subset [`import_symbol_library`] reads and never
/// writes a file. It reports `library/duplicate-symbol` when two symbols share
/// a name, `library/symbol-without-pins` when a symbol declares no pin,
/// `library/duplicate-pin-number` when one unit declares a pin number twice and
/// `library/pin-without-name` when a pin carries an empty name. Symbols and
/// pins keep their declaration order, so one library always produces the same
/// report. `passed` is false only when a finding is an error; the
/// `kicad_sym/unsupported-element` losses of the reader are returned next to
/// the findings and never fail the check on their own.
pub(crate) fn check_symbol_library(text: &str) -> Result<Value, ServiceError> {
    let imported = import_symbol_library(text)?;
    let empty = Vec::new();
    let symbols = imported["symbols"].as_array().unwrap_or(&empty);
    let mut findings = Vec::new();
    let mut seen_symbols = HashSet::new();
    let mut pin_count = 0usize;
    for symbol in symbols {
        let name = symbol["name"].as_str().unwrap_or("");
        if !seen_symbols.insert(name) {
            findings.push(library_finding(
                "library/duplicate-symbol",
                "error",
                name,
                None,
                None,
                format!("symbol `{name}` is declared more than once"),
            ));
        }
        let pins = symbol["pins"].as_array().unwrap_or(&empty);
        if pins.is_empty() {
            findings.push(library_finding(
                "library/symbol-without-pins",
                "warning",
                name,
                None,
                None,
                format!("symbol `{name}` declares no pin"),
            ));
        }
        let mut seen_numbers: HashSet<(u64, &str)> = HashSet::new();
        for pin in pins {
            pin_count += 1;
            let number = pin["number"].as_str().unwrap_or("");
            let unit = pin["unit"].as_u64().unwrap_or(1);
            if !seen_numbers.insert((unit, number)) {
                findings.push(library_finding(
                    "library/duplicate-pin-number",
                    "error",
                    name,
                    Some(number),
                    Some(unit),
                    format!("symbol `{name}` declares pin number `{number}` twice in unit {unit}"),
                ));
            }
            if pin["name"].as_str().unwrap_or("").trim().is_empty() {
                findings.push(library_finding(
                    "library/pin-without-name",
                    "warning",
                    name,
                    Some(number),
                    Some(unit),
                    format!("pin `{number}` of symbol `{name}` has no name"),
                ));
            }
        }
    }
    let passed = !findings.iter().any(|finding| finding["severity"] == "error");
    Ok(json!({
        "kind": "kicad_sym",
        "format": LIBRARY_CHECK_FORMAT,
        "version": LIBRARY_CHECK_VERSION,
        "symbols": imported["totals"]["symbols"].as_u64().unwrap_or(0),
        "pins": pin_count,
        "findings": findings,
        "passed": passed,
        "losses": imported["losses"],
    }))
}

/// One finding of [`check_symbol_library`].
fn library_finding(code: &str, severity: &str, symbol: &str, pin: Option<&str>, unit: Option<u64>, detail: String) -> Value {
    json!({
        "code": code,
        "severity": severity,
        "symbol": symbol,
        "pin": pin,
        "unit": unit,
        "detail": detail,
    })
}

/// Reads one top-level library symbol and the pins of every nested unit symbol.
fn read_library_symbol(node: &Node, losses: &mut Vec<Value>) -> Option<(Value, usize)> {
    let Some(name) = node.children().first().and_then(node_text)
    else {
        losses.push(loss(SYMBOL_UNSUPPORTED_ELEMENT, "a symbol without a name was skipped".to_string()));
        return None;
    };
    let mut pins = Vec::new();
    let mut units = 1u64;
    collect_symbol_pins(node, 1, &mut units, &mut pins, losses);
    let pin_count = pins.len();
    Some((json!({ "name": name, "unitCount": units, "pins": pins }), pin_count))
}

/// Collects the pins of one symbol or nested unit symbol.
///
/// A nested symbol named `<name>_<unit>_<style>` marks the unit its pins belong
/// to; unit `0` is shared graphics and keeps the enclosing unit.
fn collect_symbol_pins(node: &Node, unit: u64, units: &mut u64, pins: &mut Vec<Value>, losses: &mut Vec<Value>) {
    for item in node.children() {
        match item.head() {
            Some("pin") => {
                let Some(number) = child_text(item, "number")
                else {
                    losses.push(loss(SYMBOL_UNSUPPORTED_ELEMENT, "a pin without a number was skipped".to_string()));
                    continue;
                };
                let name = child_text(item, "name").unwrap_or("");
                let type_name = item.children().first().and_then(node_text).unwrap_or("");
                let pin_type = match read_pin_type(type_name) {
                    Some(pin_type) => pin_type,
                    None => {
                        losses.push(loss(
                            SYMBOL_UNSUPPORTED_ELEMENT,
                            format!("pin `{number}` has electrical type `{type_name}`, which the model does not carry"),
                        ));
                        PinType::Unspecified
                    }
                };
                pins.push(json!({
                    "number": number,
                    "name": name,
                    "type": pin_type_name(pin_type),
                    "unit": unit,
                }));
            }
            Some("symbol") => {
                let nested = item
                    .children()
                    .first()
                    .and_then(node_text)
                    .and_then(unit_of_symbol_name)
                    .filter(|value| *value > 0)
                    .unwrap_or(unit);
                *units = (*units).max(nested);
                collect_symbol_pins(item, nested, units, pins, losses);
            }
            _ => {}
        }
    }
}

/// Reads the unit number out of a `<name>_<unit>_<style>` symbol name.
fn unit_of_symbol_name(name: &str) -> Option<u64> {
    let mut parts = name.rsplit('_');
    let _style = parts.next()?;
    parts.next()?.parse::<u64>().ok()
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
#[path = "../tests/unit/kicad.rs"]
mod tests;
