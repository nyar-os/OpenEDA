use super::*;

fn wire(id: &str, from: [f64; 2], to: [f64; 2]) -> Wire {
    Wire { id: id.to_string(), net: None, from, to }
}

fn component(id: &str, reference: &str, x: f64, y: f64) -> Component {
    Component {
        id: id.to_string(),
        reference: reference.to_string(),
        value: "10k".to_string(),
        footprint: String::new(),
        x,
        y,
        rotation: 0.0,
        pins: vec!["1".to_string()],
        pin_types: BTreeMap::new(),
        properties: BTreeMap::new(),
    }
}

fn blank_sheet(id: &str) -> Sheet {
    Sheet {
        id: id.to_string(),
        name: id.to_string(),
        components: Vec::new(),
        nets: Vec::new(),
        wires: Vec::new(),
        junctions: Vec::new(),
        no_connects: Vec::new(),
        texts: Vec::new(),
        labels: Vec::new(),
    }
}

fn design(sheets: Vec<Sheet>) -> Design {
    Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets,
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 1,
        next_net: 1,
        next_wire: 1,
    }
}

fn dangling(design: &Design) -> Vec<Value> {
    inspect_design(design, &["dangling".to_string()])
}

#[test]
fn one_warning_per_dangling_endpoint_in_wire_then_endpoint_order() {
    let mut sheet = blank_sheet("sheet-1");
    sheet.wires = vec![wire("wire-1", [0.0, 0.0], [10.0, 0.0]), wire("wire-2", [20.0, 0.0], [30.0, 0.0])];
    let design = design(vec![sheet]);

    let issues = dangling(&design);
    assert_eq!(issues.len(), 4);
    assert!(issues.iter().all(|issue| issue["code"] == "wire/dangling-endpoint"));
    assert!(issues.iter().all(|issue| issue["severity"] == "warning"));
    assert!(issues.iter().all(|issue| issue["recoverable"] == true));
    assert_eq!(issues[0]["sheetId"], "sheet-1");
    assert_eq!(issues[0]["objectId"], "wire-1");
    assert_eq!(issues[0]["endpoint"], 0);
    assert_eq!(issues[0]["x"], 0.0);
    assert_eq!(issues[0]["y"], 0.0);
    assert_eq!(issues[1]["endpoint"], 1);
    assert_eq!(issues[1]["x"], 10.0);
    assert_eq!(issues[2]["objectId"], "wire-2");
    assert_eq!(issues[2]["endpoint"], 0);
    assert_eq!(issues[3]["endpoint"], 1);
    assert!(issues[0]["message"].as_str().unwrap_or("").contains("`from`"));
    assert!(issues[1]["message"].as_str().unwrap_or("").contains("`to`"));
}

#[test]
fn junction_label_and_no_connect_anchors_attach_an_endpoint() {
    let mut sheet = blank_sheet("sheet-1");
    sheet.wires = vec![
        wire("wire-junction", [0.0, 0.0], [10.0, 0.0]),
        wire("wire-label", [20.0, 0.0], [30.0, 0.0]),
        wire("wire-no-connect", [40.0, 0.0], [50.0, 0.0]),
    ];
    sheet.junctions = vec![Junction { x: 0.0, y: 0.0 }, Junction { x: 10.0, y: 0.0 }];
    sheet.labels = vec![
        NetLabel {
            id: "label-1".to_string(),
            text: "A".to_string(),
            x: 20.0,
            y: 0.0,
            scope: LabelScope::Local,
            rotation: 0.0,
            justify: "left".to_string(),
        },
        NetLabel {
            id: "label-2".to_string(),
            text: "B".to_string(),
            x: 30.0,
            y: 0.0,
            scope: LabelScope::Global,
            rotation: 0.0,
            justify: "left".to_string(),
        },
    ];
    sheet.no_connects = vec![NoConnect { x: 40.0, y: 0.0, node: None }, NoConnect { x: 50.0, y: 0.0, node: None }];
    let design = design(vec![sheet]);

    assert!(dangling(&design).is_empty());
}

#[test]
fn a_component_anchors_through_its_placement_point() {
    let mut sheet = blank_sheet("sheet-1");
    sheet.wires = vec![wire("wire-1", [10.0, 10.0], [40.0, 10.0])];
    sheet.components = vec![component("component-1", "R1", 10.0, 10.0)];
    let design = design(vec![sheet]);

    let issues = dangling(&design);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["endpoint"], 1);
    assert_eq!(issues[0]["x"], 40.0);
}

#[test]
fn another_wire_attaches_at_a_shared_endpoint_or_its_interior() {
    let mut shared = blank_sheet("sheet-1");
    shared.wires = vec![wire("wire-1", [0.0, 0.0], [10.0, 0.0]), wire("wire-2", [10.0, 0.0], [20.0, 0.0])];
    shared.junctions = vec![Junction { x: 0.0, y: 0.0 }, Junction { x: 20.0, y: 0.0 }];
    assert!(dangling(&design(vec![shared])).is_empty());

    let mut interior = blank_sheet("sheet-1");
    interior.wires = vec![wire("wire-1", [0.0, 0.0], [10.0, 0.0]), wire("wire-2", [5.0, 0.0], [5.0, 5.0])];
    interior.junctions = vec![Junction { x: 0.0, y: 0.0 }, Junction { x: 10.0, y: 0.0 }, Junction { x: 5.0, y: 5.0 }];
    assert!(dangling(&design(vec![interior])).is_empty());
}

#[test]
fn a_zero_length_wire_is_reported_once_at_the_from_point() {
    let mut sheet = blank_sheet("sheet-1");
    sheet.wires = vec![wire("wire-1", [3.0, 4.0], [3.0, 4.0])];
    let floating = design(vec![sheet]);

    let issues = dangling(&floating);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["objectId"], "wire-1");
    assert_eq!(issues[0]["endpoint"], 0);
    assert_eq!(issues[0]["x"], 3.0);
    assert_eq!(issues[0]["y"], 4.0);
    assert!(issues[0]["message"].as_str().unwrap_or("").contains("zero-length"));

    let mut attached = blank_sheet("sheet-1");
    attached.wires = vec![wire("wire-1", [3.0, 4.0], [3.0, 4.0])];
    attached.junctions = vec![Junction { x: 3.0, y: 4.0 }];
    assert!(dangling(&design(vec![attached])).is_empty());
}

#[test]
fn every_sheet_is_checked_without_opening_child_files() {
    let mut first = blank_sheet("sheet-1");
    first.wires = vec![wire("wire-1", [2.0, 0.0], [3.0, 0.0])];
    let mut second = blank_sheet("sheet-2");
    second.wires = vec![wire("wire-2", [2.0, 0.0], [3.0, 0.0])];
    let mut design = design(vec![first, second]);
    design.hierarchy = vec![HierarchicalSheet {
        id: "hierarchy-1".to_string(),
        name: "Power".to_string(),
        file: "power.oeda".to_string(),
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
        ports: vec![SheetPort { name: "VCC".to_string(), direction: PortDirection::Input, x: 2.0, y: 0.0 }],
    }];

    let issues = dangling(&design);
    assert_eq!(issues.len(), 3);
    let sheets: Vec<&str> = issues.iter().map(|issue| issue["sheetId"].as_str().unwrap_or("")).collect();
    assert_eq!(sheets, vec!["sheet-1", "sheet-2", "sheet-2"]);
    assert_eq!(issues[0]["endpoint"], 1, "the root-sheet port anchor attaches the from point");
    assert_eq!(issues[1]["endpoint"], 0, "a root-sheet port anchor does not attach a child sheet");
}

#[test]
fn a_root_sheet_port_anchor_attaches_an_endpoint() {
    let mut root = blank_sheet("sheet-1");
    root.wires = vec![wire("wire-1", [2.0, 0.0], [3.0, 0.0])];
    root.junctions = vec![Junction { x: 3.0, y: 0.0 }];
    let mut design = design(vec![root]);
    design.hierarchy = vec![HierarchicalSheet {
        id: "hierarchy-1".to_string(),
        name: "Power".to_string(),
        file: "power.oeda".to_string(),
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
        ports: vec![SheetPort { name: "VCC".to_string(), direction: PortDirection::Input, x: 2.0, y: 0.0 }],
    }];

    assert!(dangling(&design).is_empty());
}

#[test]
fn severity_override_and_waiver_work_like_other_checks() {
    let mut sheet = blank_sheet("sheet-1");
    sheet.wires = vec![wire("wire-1", [0.0, 0.0], [10.0, 0.0])];
    let handle = insert_session(Session::new(design(vec![sheet])));
    let checks = Some(vec!["dangling".to_string()]);

    let warning =
        inspect_with_options(handle, InspectOptions { checks: checks.clone(), ..InspectOptions::default() }).expect("inspect");
    assert_eq!(warning["passed"], true);
    assert_eq!(warning["issues"].as_array().expect("issues").len(), 2);
    assert_eq!(warning["counts"], json!({ "active": 2, "waived": 0 }));

    let promoted = inspect_with_options(
        handle,
        InspectOptions {
            checks: checks.clone(),
            severity: BTreeMap::from([("wire/dangling-endpoint".to_string(), "error".to_string())]),
            ..InspectOptions::default()
        },
    )
    .expect("inspect");
    assert_eq!(promoted["passed"], false);
    assert_eq!(promoted["issues"][0]["severity"], "error");

    let waived = inspect_with_options(
        handle,
        InspectOptions {
            checks,
            waivers: vec![Waiver {
                code: "wire/dangling-endpoint".to_string(),
                sheet_id: None,
                object_id: Some("wire-1".to_string()),
            }],
            ..InspectOptions::default()
        },
    )
    .expect("inspect");
    assert_eq!(waived["passed"], true);
    assert_eq!(waived["issues"], json!([]));
    assert_eq!(waived["waived"].as_array().expect("waived").len(), 2);
    assert_eq!(waived["counts"], json!({ "active": 0, "waived": 2 }));

    close(handle).expect("close");
}

#[test]
fn repeated_inspection_is_byte_identical() {
    let mut sheet = blank_sheet("sheet-1");
    sheet.wires = vec![wire("wire-1", [0.0, 0.0], [10.0, 0.0]), wire("wire-2", [20.0, 0.0], [20.0, 0.0])];
    let design = design(vec![sheet]);

    let first = serde_json::to_string(&dangling(&design)).expect("serialize");
    let second = serde_json::to_string(&dangling(&design)).expect("serialize");
    assert_eq!(first, second);
}
