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
        properties: BTreeMap::new(),
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
        labels: Vec::new(),
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
        variants: Vec::new(),
        hierarchy: Vec::new(),
        rules: crate::DesignRules::default(),
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
        vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4, aliases: Vec::new() }],
    );
    let right = design(
        "Blinker",
        3,
        vec![first],
        vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4, aliases: Vec::new() }],
    );
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
        vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4, aliases: Vec::new() }],
    );
    let right = design(
        "B",
        2,
        vec![sheet("sheet-1", "Core")],
        vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 8, aliases: Vec::new() }],
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

#[test]
fn bom_variant_changes_are_reported() {
    let mut left = design("A", 1, vec![sheet("sheet-1", "Main")], vec![]);
    left.variants.push(Variant {
        name: "lite".to_string(),
        dnp: vec!["R1".to_string()],
        values: BTreeMap::from([("R2".to_string(), "22k".to_string())]),
    });
    let mut right = left.clone();
    right.variants[0].dnp.push("R3".to_string());
    right.variants.push(Variant { name: "pro".to_string(), dnp: Vec::new(), values: BTreeMap::new() });

    let report = compare("a.oeda", &left, "b.oeda", &right);
    assert_eq!(report["equal"], false);
    assert_eq!(report["summary"]["variants"], json!({ "added": 1, "removed": 0, "modified": 1 }));
    assert_eq!(report["variants"]["added"][0]["name"], "pro");
    assert_eq!(report["variants"]["modified"][0]["name"], "lite");
    assert_eq!(report["variants"]["modified"][0]["changes"]["dnp"]["to"], json!(["R1", "R3"]));

    let unchanged = compare("a.oeda", &left, "b.oeda", &left.clone());
    assert_eq!(unchanged["equal"], true);
    assert_eq!(unchanged["summary"]["variants"], json!({ "added": 0, "removed": 0, "modified": 0 }));
}
