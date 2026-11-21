//! Read-only comparison of two design documents.
//!
//! The comparison is structural: sheets are matched by id, components by
//! reference, nets by name inside a sheet and buses by name. A sheet that exists
//! on one side only contributes all of its objects as added or removed, so a
//! deleted sheet never hides its contents. Every list is ordered by key, so one
//! pair of documents always produces the same report.
//!
//! The module only reads documents; it never mutates them and never writes a
//! file.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::{Bus, Component, Design, Junction, Net, NoConnect, Sheet, TextLabel, Wire};

/// Tag written into every diff report.
pub(crate) const DIFF_FORMAT: &str = "open-eda-diff";
/// Version written into every diff report.
pub(crate) const DIFF_VERSION: u64 = 1;

/// Added, removed and modified counters for one object family.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Counts {
    added: usize,
    removed: usize,
    modified: usize,
}

impl Counts {
    fn changed(&self) -> bool {
        self.added + self.removed + self.modified > 0
    }

    fn add(&mut self, other: &Counts) {
        self.added += other.added;
        self.removed += other.removed;
        self.modified += other.modified;
    }

    fn json(&self) -> Value {
        json!({ "added": self.added, "removed": self.removed, "modified": self.modified })
    }
}

/// Totals of one sheet comparison.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct SheetCounts {
    components: Counts,
    nets: Counts,
    wires: Counts,
    junctions: Counts,
    no_connects: Counts,
    texts: Counts,
}

impl SheetCounts {
    fn changed(&self) -> bool {
        self.components.changed()
            || self.nets.changed()
            || self.wires.changed()
            || self.junctions.changed()
            || self.no_connects.changed()
            || self.texts.changed()
    }
}

/// Compares two designs and returns the report the surfaces print.
pub(crate) fn compare(left_path: &str, left: &Design, right_path: &str, right: &Design) -> Value {
    let left_sheets = sheets_by_id(left);
    let right_sheets = sheets_by_id(right);
    let ids: BTreeSet<&str> = left_sheets.keys().copied().chain(right_sheets.keys().copied()).collect();

    let mut totals = SheetCounts::default();
    let mut sheets = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut modified = 0usize;
    let mut unchanged = 0usize;
    for id in ids {
        let (entry, counts) = compare_sheet(id, left_sheets.get(id).copied(), right_sheets.get(id).copied());
        match entry["status"].as_str().unwrap_or("unchanged") {
            "added" => added += 1,
            "removed" => removed += 1,
            "modified" => modified += 1,
            _ => unchanged += 1,
        }
        totals.components.add(&counts.components);
        totals.nets.add(&counts.nets);
        totals.wires.add(&counts.wires);
        totals.junctions.add(&counts.junctions);
        totals.no_connects.add(&counts.no_connects);
        totals.texts.add(&counts.texts);
        if counts.changed() || entry["status"] != "unchanged" {
            sheets.push(entry);
        }
    }

    let buses = compare_buses(left, right);
    let design = compare_design(left, right);
    let equal = added + removed + modified == 0 && !buses.changed() && design.is_none();

    json!({
        "format": DIFF_FORMAT,
        "version": DIFF_VERSION,
        "equal": equal,
        "left": side(left_path, left),
        "right": side(right_path, right),
        "summary": {
            "sheets": { "added": added, "removed": removed, "modified": modified, "unchanged": unchanged },
            "components": totals.components.json(),
            "nets": totals.nets.json(),
            "wires": totals.wires.json(),
            "junctions": totals.junctions.json(),
            "noConnects": totals.no_connects.json(),
            "texts": totals.texts.json(),
            "buses": buses.counts.json(),
            "design": { "modified": usize::from(design.is_some()) },
        },
        "design": { "changes": design },
        "sheets": sheets,
        "buses": {
            "added": buses.added,
            "removed": buses.removed,
            "modified": buses.modified,
        },
    })
}

/// Compares the drawing properties that are visible on both documents.
///
/// The edit revision counter is deliberately ignored: two files written from the
/// same objects at different times would otherwise always differ. Title, drawing
/// revision, company, date and units are real content, so they are reported.
fn compare_design(left: &Design, right: &Design) -> Option<Value> {
    let mut changes = serde_json::Map::new();
    let mut field = |name: &str, from: Value, to: Value| {
        if from != to {
            changes.insert(name.to_string(), json!({ "from": from, "to": to }));
        }
    };
    field("title", json!(left.title), json!(right.title));
    field("rev", json!(left.rev), json!(right.rev));
    field("company", json!(left.company), json!(right.company));
    field("date", json!(left.date), json!(right.date));
    field("units", json!(left.units), json!(right.units));
    if changes.is_empty() { None } else { Some(Value::Object(changes)) }
}

fn side(path: &str, design: &Design) -> Value {
    json!({
        "path": path,
        "id": design.id,
        "revision": design.revision,
        "title": design.title,
    })
}

fn sheets_by_id(design: &Design) -> BTreeMap<&str, &Sheet> {
    design.sheets.iter().map(|sheet| (sheet.id.as_str(), sheet)).collect()
}

fn compare_sheet(id: &str, left: Option<&Sheet>, right: Option<&Sheet>) -> (Value, SheetCounts) {
    match (left, right) {
        (None, Some(sheet)) => {
            let counts = one_sided_counts(sheet, true);
            (one_sided_entry(id, None, Some(sheet), "added", true), counts)
        }
        (Some(sheet), None) => {
            let counts = one_sided_counts(sheet, false);
            (one_sided_entry(id, Some(sheet), None, "removed", false), counts)
        }
        (Some(left_sheet), Some(right_sheet)) => compare_shared_sheet(id, left_sheet, right_sheet),
        (None, None) => unreachable!("an id always comes from one of the two designs"),
    }
}

/// Counts every object of a sheet that exists on one side only.
fn one_sided_counts(sheet: &Sheet, added: bool) -> SheetCounts {
    let count = |size: usize| {
        if added { Counts { added: size, ..Counts::default() } } else { Counts { removed: size, ..Counts::default() } }
    };
    SheetCounts {
        components: count(sheet.components.len()),
        nets: count(sheet.nets.len()),
        wires: count(sheet.wires.len()),
        junctions: count(sheet.junctions.len()),
        no_connects: count(sheet.no_connects.len()),
        texts: count(sheet.texts.len()),
    }
}

/// Lists every object of a sheet as added or removed.
fn one_sided_entry(id: &str, left: Option<&Sheet>, right: Option<&Sheet>, status: &str, added: bool) -> Value {
    let sheet = right.or(left).expect("one side exists");
    let side_key = if added { "added" } else { "removed" };
    let empty = json!([]);
    let mut components = serde_json::Map::new();
    let mut nets = serde_json::Map::new();
    let mut wires = serde_json::Map::new();
    let mut junctions = serde_json::Map::new();
    let mut no_connects = serde_json::Map::new();
    let mut texts = serde_json::Map::new();
    components.insert(side_key.to_string(), json!(sheet.components.iter().map(component_view).collect::<Vec<_>>()));
    nets.insert(side_key.to_string(), json!(sheet.nets.iter().map(net_view).collect::<Vec<_>>()));
    wires.insert(side_key.to_string(), json!(sheet.wires.iter().map(wire_view).collect::<Vec<_>>()));
    junctions.insert(
        side_key.to_string(),
        json!(sheet.junctions.iter().map(|junction| json!({ "x": junction.x, "y": junction.y })).collect::<Vec<_>>()),
    );
    no_connects.insert(
        side_key.to_string(),
        json!(sheet.no_connects.iter().map(|flag| json!({ "x": flag.x, "y": flag.y })).collect::<Vec<_>>()),
    );
    texts.insert(
        side_key.to_string(),
        json!(sheet.texts.iter().map(|text| json!({ "text": text.text, "x": text.x, "y": text.y })).collect::<Vec<_>>()),
    );
    for map in [&mut components, &mut nets, &mut wires, &mut junctions, &mut no_connects, &mut texts] {
        for key in ["added", "removed"] {
            map.entry(key.to_string()).or_insert_with(|| empty.clone());
        }
    }
    let mut entry = json!({
        "id": id,
        "name": sheet.name,
        "leftName": left.map(|sheet| sheet.name.clone()),
        "rightName": right.map(|sheet| sheet.name.clone()),
        "status": status,
    });
    entry["components"] = Value::Object(components);
    entry["nets"] = Value::Object(nets);
    entry["wires"] = Value::Object(wires);
    entry["junctions"] = Value::Object(junctions);
    entry["noConnects"] = Value::Object(no_connects);
    entry["texts"] = Value::Object(texts);
    entry
}

fn compare_shared_sheet(id: &str, left: &Sheet, right: &Sheet) -> (Value, SheetCounts) {
    let components = compare_components(left, right);
    let nets = compare_nets(left, right);
    let wires = compare_wires(left, right);
    let junctions = compare_junctions(left, right);
    let no_connects = compare_no_connects(left, right);
    let texts = compare_texts(left, right);
    let counts = SheetCounts {
        components: components.0,
        nets: nets.0,
        wires: wires.0,
        junctions: junctions.0,
        no_connects: no_connects.0,
        texts: texts.0,
    };
    let renamed = left.name != right.name;
    let status = if renamed || counts.changed() { "modified" } else { "unchanged" };
    let mut entry = json!({
        "id": id,
        "name": right.name,
        "leftName": left.name,
        "rightName": right.name,
        "status": status,
    });
    if renamed {
        entry["changes"] = json!({ "name": { "from": left.name, "to": right.name } });
    }
    entry["components"] = components.1;
    entry["nets"] = nets.1;
    entry["wires"] = wires.1;
    entry["junctions"] = junctions.1;
    entry["noConnects"] = no_connects.1;
    entry["texts"] = texts.1;
    (entry, counts)
}

fn compare_components(left: &Sheet, right: &Sheet) -> (Counts, Value) {
    let left_map: BTreeMap<&str, &Component> =
        left.components.iter().map(|component| (component.reference.as_str(), component)).collect();
    let right_map: BTreeMap<&str, &Component> =
        right.components.iter().map(|component| (component.reference.as_str(), component)).collect();
    let keys: BTreeSet<&str> = left_map.keys().copied().chain(right_map.keys().copied()).collect();

    let mut counts = Counts::default();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    for reference in keys {
        match (left_map.get(reference), right_map.get(reference)) {
            (None, Some(component)) => {
                counts.added += 1;
                added.push(component_view(component));
            }
            (Some(component), None) => {
                counts.removed += 1;
                removed.push(component_view(component));
            }
            (Some(before), Some(after)) => {
                if let Some(changes) = component_changes(before, after) {
                    counts.modified += 1;
                    modified.push(json!({ "ref": reference, "changes": changes }));
                }
            }
            (None, None) => {}
        }
    }
    (counts, json!({ "added": added, "removed": removed, "modified": modified }))
}

fn component_view(component: &Component) -> Value {
    json!({
        "id": component.id,
        "ref": component.reference,
        "value": component.value,
        "footprint": component.footprint,
        "x": component.x,
        "y": component.y,
        "rotation": component.rotation,
        "pins": component.pins,
        "pinTypes": component.pin_types,
    })
}

fn component_changes(before: &Component, after: &Component) -> Option<Value> {
    let mut changes = serde_json::Map::new();
    let mut field = |name: &str, from: Value, to: Value| {
        if from != to {
            changes.insert(name.to_string(), json!({ "from": from, "to": to }));
        }
    };
    field("value", json!(before.value), json!(after.value));
    field("footprint", json!(before.footprint), json!(after.footprint));
    field("x", json!(before.x), json!(after.x));
    field("y", json!(before.y), json!(after.y));
    field("rotation", json!(before.rotation), json!(after.rotation));
    field("pins", json!(before.pins), json!(after.pins));
    field("pinTypes", json!(before.pin_types), json!(after.pin_types));
    if changes.is_empty() { None } else { Some(Value::Object(changes)) }
}

fn compare_nets(left: &Sheet, right: &Sheet) -> (Counts, Value) {
    let left_map: BTreeMap<&str, &Net> = left.nets.iter().map(|net| (net.name.as_str(), net)).collect();
    let right_map: BTreeMap<&str, &Net> = right.nets.iter().map(|net| (net.name.as_str(), net)).collect();
    let keys: BTreeSet<&str> = left_map.keys().copied().chain(right_map.keys().copied()).collect();

    let mut counts = Counts::default();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    for name in keys {
        match (left_map.get(name), right_map.get(name)) {
            (None, Some(net)) => {
                counts.added += 1;
                added.push(net_view(net));
            }
            (Some(net), None) => {
                counts.removed += 1;
                removed.push(net_view(net));
            }
            (Some(before), Some(after)) => {
                let before_nodes = net_nodes(before);
                let after_nodes = net_nodes(after);
                if before_nodes != after_nodes {
                    counts.modified += 1;
                    modified.push(json!({
                        "name": name,
                        "changes": { "nodes": { "from": before_nodes, "to": after_nodes } },
                    }));
                }
            }
            (None, None) => {}
        }
    }
    (counts, json!({ "added": added, "removed": removed, "modified": modified }))
}

fn net_view(net: &Net) -> Value {
    json!({
        "id": net.id,
        "name": net.name,
        "nodes": net
            .nodes
            .iter()
            .map(|node| json!({ "ref": node.reference, "pin": node.pin }))
            .collect::<Vec<_>>(),
    })
}

/// Every node as a sorted `REF.PIN` string, so node order never reads as a change.
fn net_nodes(net: &Net) -> Vec<String> {
    let mut nodes: Vec<String> = net.nodes.iter().map(|node| format!("{}.{}", node.reference, node.pin)).collect();
    nodes.sort();
    nodes
}

fn compare_wires(left: &Sheet, right: &Sheet) -> (Counts, Value) {
    let left_map: BTreeMap<&str, &Wire> = left.wires.iter().map(|wire| (wire.id.as_str(), wire)).collect();
    let right_map: BTreeMap<&str, &Wire> = right.wires.iter().map(|wire| (wire.id.as_str(), wire)).collect();
    let keys: BTreeSet<&str> = left_map.keys().copied().chain(right_map.keys().copied()).collect();

    let mut counts = Counts::default();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    for id in keys {
        match (left_map.get(id), right_map.get(id)) {
            (None, Some(wire)) => {
                counts.added += 1;
                added.push(wire_view(wire));
            }
            (Some(wire), None) => {
                counts.removed += 1;
                removed.push(wire_view(wire));
            }
            (Some(before), Some(after)) => {
                if let Some(changes) = wire_changes(before, after) {
                    counts.modified += 1;
                    modified.push(json!({ "id": id, "changes": changes }));
                }
            }
            (None, None) => {}
        }
    }
    (counts, json!({ "added": added, "removed": removed, "modified": modified }))
}

fn wire_view(wire: &Wire) -> Value {
    json!({ "id": wire.id, "net": wire.net, "from": wire.from, "to": wire.to })
}

fn wire_changes(before: &Wire, after: &Wire) -> Option<Value> {
    let mut changes = serde_json::Map::new();
    if before.net != after.net {
        changes.insert("net".to_string(), json!({ "from": before.net, "to": after.net }));
    }
    if before.from != after.from {
        changes.insert("from".to_string(), json!({ "from": before.from, "to": after.from }));
    }
    if before.to != after.to {
        changes.insert("to".to_string(), json!({ "from": before.to, "to": after.to }));
    }
    if changes.is_empty() { None } else { Some(Value::Object(changes)) }
}

fn compare_junctions(left: &Sheet, right: &Sheet) -> (Counts, Value) {
    let (added, removed) = multiset_difference(&left.junctions, &right.junctions, junction_key);
    let counts = Counts { added: added.len(), removed: removed.len(), modified: 0 };
    (counts, json!({ "added": added, "removed": removed }))
}

fn compare_no_connects(left: &Sheet, right: &Sheet) -> (Counts, Value) {
    let (added, removed) = multiset_difference(&left.no_connects, &right.no_connects, flag_key);
    let counts = Counts { added: added.len(), removed: removed.len(), modified: 0 };
    (counts, json!({ "added": added, "removed": removed }))
}

fn compare_texts(left: &Sheet, right: &Sheet) -> (Counts, Value) {
    let (added, removed) = multiset_difference(&left.texts, &right.texts, text_key);
    let counts = Counts { added: added.len(), removed: removed.len(), modified: 0 };
    (counts, json!({ "added": added, "removed": removed }))
}

/// Returns the entries only on the right (added) and only on the left
/// (removed), each ordered by its key. Duplicate keys keep their multiplicity.
fn multiset_difference<T: Clone + ItemJson>(left: &[T], right: &[T], key: impl Fn(&T) -> String) -> (Vec<Value>, Vec<Value>) {
    let mut left_counts: BTreeMap<String, (usize, T)> = BTreeMap::new();
    for item in left {
        let entry = left_counts.entry(key(item)).or_insert_with(|| (0, item.clone()));
        entry.0 += 1;
    }
    let mut right_counts: BTreeMap<String, (usize, T)> = BTreeMap::new();
    for item in right {
        let entry = right_counts.entry(key(item)).or_insert_with(|| (0, item.clone()));
        entry.0 += 1;
    }

    let mut removed = Vec::new();
    for (text, (count, item)) in &left_counts {
        let matched = right_counts.get(text).map(|(right, _)| *right).unwrap_or(0);
        for _ in matched..*count {
            removed.push(item.to_json());
        }
    }
    let mut added = Vec::new();
    for (text, (count, item)) in &right_counts {
        let matched = left_counts.get(text).map(|(left, _)| *left).unwrap_or(0);
        for _ in matched..*count {
            added.push(item.to_json());
        }
    }
    (added, removed)
}

fn junction_key(junction: &Junction) -> String {
    format!("{:.4},{:.4}", junction.x, junction.y)
}

fn flag_key(flag: &NoConnect) -> String {
    format!("{:.4},{:.4}", flag.x, flag.y)
}

fn text_key(text: &TextLabel) -> String {
    format!("{}\u{1f}{:.4},{:.4}", text.text, text.x, text.y)
}

/// One sheet item that only carries placement or text.
trait ItemJson {
    fn to_json(&self) -> Value;
}

impl ItemJson for Junction {
    fn to_json(&self) -> Value {
        json!({ "x": self.x, "y": self.y })
    }
}

impl ItemJson for NoConnect {
    fn to_json(&self) -> Value {
        json!({ "x": self.x, "y": self.y })
    }
}

impl ItemJson for TextLabel {
    fn to_json(&self) -> Value {
        json!({ "text": self.text, "x": self.x, "y": self.y })
    }
}

/// Result of the design-level bus comparison.
struct BusDiff {
    counts: Counts,
    added: Vec<Value>,
    removed: Vec<Value>,
    modified: Vec<Value>,
}

impl BusDiff {
    fn changed(&self) -> bool {
        self.counts.changed()
    }
}

fn compare_buses(left: &Design, right: &Design) -> BusDiff {
    let left_map: BTreeMap<&str, &Bus> = left.buses.iter().map(|bus| (bus.name.as_str(), bus)).collect();
    let right_map: BTreeMap<&str, &Bus> = right.buses.iter().map(|bus| (bus.name.as_str(), bus)).collect();
    let keys: BTreeSet<&str> = left_map.keys().copied().chain(right_map.keys().copied()).collect();

    let mut diff = BusDiff { counts: Counts::default(), added: Vec::new(), removed: Vec::new(), modified: Vec::new() };
    for name in keys {
        match (left_map.get(name), right_map.get(name)) {
            (None, Some(bus)) => {
                diff.counts.added += 1;
                diff.added.push(bus_view(bus));
            }
            (Some(bus), None) => {
                diff.counts.removed += 1;
                diff.removed.push(bus_view(bus));
            }
            (Some(before), Some(after)) => {
                if before.width != after.width {
                    diff.counts.modified += 1;
                    diff.modified.push(json!({
                        "name": name,
                        "changes": { "width": { "from": before.width, "to": after.width } },
                    }));
                }
            }
            (None, None) => {}
        }
    }
    diff
}

fn bus_view(bus: &Bus) -> Value {
    json!({ "id": bus.id, "name": bus.name, "width": bus.width })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Node as NetNode;

    fn component(id: &str, reference: &str, value: &str) -> Component {
        Component {
            id: id.to_string(),
            reference: reference.to_string(),
            value: value.to_string(),
            footprint: String::new(),
            x: 10.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
            pin_types: BTreeMap::new(),
        }
    }

    fn net(id: &str, name: &str, nodes: &[(&str, &str)]) -> Net {
        Net {
            id: id.to_string(),
            name: name.to_string(),
            nodes: nodes
                .iter()
                .map(|(reference, pin)| NetNode { reference: reference.to_string(), pin: pin.to_string() })
                .collect(),
        }
    }

    fn sheet(id: &str, name: &str) -> Sheet {
        Sheet {
            id: id.to_string(),
            name: name.to_string(),
            components: Vec::new(),
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
        }
    }

    fn design(title: &str, revision: u64, sheets: Vec<Sheet>, buses: Vec<Bus>) -> Design {
        Design {
            id: "design-1".to_string(),
            title: Some(title.to_string()),
            rev: String::new(),
            company: String::new(),
            date: String::new(),
            revision,
            units: "mm".to_string(),
            sheets,
            buses,
            next_component: 1,
            next_net: 1,
            next_wire: 1,
        }
    }

    #[test]
    fn identical_documents_are_equal() {
        let mut first = sheet("sheet-1", "Main");
        first.components.push(component("component-1", "R1", "10k"));
        first.nets.push(net("net-1", "VCC", &[("R1", "1")]));
        let left = design(
            "Blinker",
            3,
            vec![first.clone()],
            vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4 }],
        );
        let right =
            design("Blinker", 3, vec![first], vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4 }]);
        let report = compare("a.oeda", &left, "b.oeda", &right);
        assert_eq!(report["equal"], true);
        assert_eq!(report["sheets"], json!([]));
        assert_eq!(report["summary"]["components"]["modified"], 0);
        assert_eq!(report["summary"]["sheets"]["unchanged"], 1);
    }

    #[test]
    fn component_and_net_changes_are_reported() {
        let mut before = sheet("sheet-1", "Main");
        before.components.push(component("component-1", "R1", "10k"));
        before.components.push(component("component-2", "C1", "100n"));
        before.nets.push(net("net-1", "VCC", &[("R1", "1"), ("C1", "1")]));
        let mut after = sheet("sheet-1", "Main");
        after.components.push(component("component-1", "R1", "22k"));
        after.components.push(component("component-3", "R2", "1k"));
        after.nets.push(net("net-1", "VCC", &[("R1", "1"), ("R2", "1")]));
        after.nets.push(net("net-2", "GND", &[("C1", "2")]));

        let report = compare("a.oeda", &design("A", 1, vec![before], vec![]), "b.oeda", &design("B", 2, vec![after], vec![]));
        assert_eq!(report["equal"], false);
        assert_eq!(report["summary"]["components"], json!({ "added": 1, "removed": 1, "modified": 1 }));
        assert_eq!(report["summary"]["nets"], json!({ "added": 1, "removed": 0, "modified": 1 }));
        let entry = &report["sheets"][0];
        assert_eq!(entry["status"], "modified");
        assert_eq!(entry["components"]["added"][0]["ref"], "R2");
        assert_eq!(entry["components"]["removed"][0]["ref"], "C1");
        assert_eq!(entry["components"]["modified"][0]["ref"], "R1");
        assert_eq!(entry["components"]["modified"][0]["changes"]["value"]["from"], "10k");
        assert_eq!(entry["components"]["modified"][0]["changes"]["value"]["to"], "22k");
        assert_eq!(entry["nets"]["modified"][0]["changes"]["nodes"]["to"], json!(["R1.1", "R2.1"]));
    }

    #[test]
    fn added_and_removed_sheets_carry_their_objects() {
        let mut main = sheet("sheet-1", "Main");
        main.components.push(component("component-1", "R1", "10k"));
        let mut power = sheet("sheet-2", "Power");
        power.components.push(component("component-2", "U1", "LDO"));
        let report = compare("a.oeda", &design("A", 1, vec![main], vec![]), "b.oeda", &design("B", 1, vec![power], vec![]));
        assert_eq!(report["summary"]["sheets"], json!({ "added": 1, "removed": 1, "modified": 0, "unchanged": 0 }));
        assert_eq!(report["summary"]["components"], json!({ "added": 1, "removed": 1, "modified": 0 }));
        let sheets = report["sheets"].as_array().expect("sheets");
        let added = sheets.iter().find(|sheet| sheet["status"] == "added").expect("added sheet");
        assert_eq!(added["id"], "sheet-2");
        assert_eq!(added["components"]["added"][0]["ref"], "U1");
        let removed = sheets.iter().find(|sheet| sheet["status"] == "removed").expect("removed sheet");
        assert_eq!(removed["components"]["removed"][0]["ref"], "R1");
    }

    #[test]
    fn renamed_sheets_and_resized_buses_are_modified() {
        let left = design(
            "A",
            1,
            vec![sheet("sheet-1", "Main")],
            vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4 }],
        );
        let right = design(
            "B",
            2,
            vec![sheet("sheet-1", "Core")],
            vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 8 }],
        );
        let report = compare("a.oeda", &left, "b.oeda", &right);
        assert_eq!(report["sheets"][0]["changes"]["name"]["to"], "Core");
        assert_eq!(report["buses"]["modified"][0]["changes"]["width"]["to"], 8);
        assert_eq!(report["summary"]["buses"], json!({ "added": 0, "removed": 0, "modified": 1 }));
    }

    #[test]
    fn drawing_properties_are_compared_but_the_edit_counter_is_not() {
        let left = design("Blinker", 1, vec![sheet("sheet-1", "Main")], vec![]);
        let mut right = design("Blinker II", 2, vec![sheet("sheet-1", "Main")], vec![]);
        right.rev = "B".to_string();
        let report = compare("a.oeda", &left, "b.oeda", &right);
        assert_eq!(report["equal"], false);
        assert_eq!(report["summary"]["design"]["modified"], 1);
        assert_eq!(report["design"]["changes"]["title"]["to"], "Blinker II");
        assert_eq!(report["design"]["changes"]["rev"]["to"], "B");

        let mut same_objects = left.clone();
        same_objects.revision = 9;
        let unchanged = compare("a.oeda", &left, "b.oeda", &same_objects);
        assert_eq!(unchanged["equal"], true);
        assert_eq!(unchanged["summary"]["design"]["modified"], 0);
    }

    #[test]
    fn node_order_does_not_read_as_a_change() {
        let mut before = sheet("sheet-1", "Main");
        before.components.push(component("component-1", "R1", "10k"));
        before.components.push(component("component-2", "C1", "100n"));
        before.nets.push(net("net-1", "VCC", &[("R1", "1"), ("C1", "1")]));
        let mut after = before.clone();
        after.nets[0].nodes.reverse();
        let report = compare("a.oeda", &design("A", 1, vec![before], vec![]), "b.oeda", &design("A", 1, vec![after], vec![]));
        assert_eq!(report["equal"], true);
    }
}
