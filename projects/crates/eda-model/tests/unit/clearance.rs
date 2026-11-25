use super::*;

fn component(id: &str, reference: &str, x: f64, y: f64, rotation: f64) -> Component {
    Component {
        id: id.to_string(),
        reference: reference.to_string(),
        value: "10k".to_string(),
        footprint: String::new(),
        x,
        y,
        rotation,
        mirror: None,
        pins: vec!["1".to_string()],
        pin_types: BTreeMap::new(),
        properties: BTreeMap::new(),
    }
}

fn sheet(id: &str, name: &str, components: Vec<Component>) -> Sheet {
    Sheet {
        id: id.to_string(),
        name: name.to_string(),
        components,
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

fn clearance_issues(design: &Design) -> Vec<Value> {
    inspect_design(design, &["clearance".to_string()])
}

#[test]
fn overlapping_bodies_report_each_pair_once_in_reference_order() {
    let design = design(vec![sheet(
        "sheet-1",
        "Sheet 1",
        vec![
            component("component-1", "R1", 0.0, 0.0, 0.0),
            component("component-2", "R2", 10.0, 0.0, 0.0),
            component("component-3", "R3", 0.0, 10.0, 0.0),
        ],
    )]);

    let issues = clearance_issues(&design);
    assert_eq!(issues.len(), 3);
    assert!(issues.iter().all(|issue| issue["code"] == "clearance/component-overlap"));
    assert!(issues.iter().all(|issue| issue["severity"] == "warning"));

    let object_ids: Vec<&str> = issues.iter().map(|issue| issue["objectId"].as_str().unwrap_or("")).collect();
    assert_eq!(object_ids, vec!["component-1", "component-1", "component-2"]);

    for (left, right) in [("R1", "R2"), ("R1", "R3"), ("R2", "R3")] {
        let message = format!("components `{left}` and `{right}`");
        assert_eq!(issues.iter().filter(|issue| issue["message"].as_str().unwrap_or("").starts_with(&message)).count(), 1);
    }
}

#[test]
fn touching_edges_and_separated_bodies_do_not_report() {
    let touching_x = design(vec![sheet(
        "sheet-1",
        "Sheet 1",
        vec![
            component("component-1", "R1", 0.0, 0.0, 0.0),
            component("component-2", "R2", COMPONENT_HALF_WIDTH * 2.0, 0.0, 0.0),
        ],
    )]);
    assert!(clearance_issues(&touching_x).is_empty());

    let touching_y = design(vec![sheet(
        "sheet-1",
        "Sheet 1",
        vec![
            component("component-1", "R1", 0.0, 0.0, 0.0),
            component("component-2", "R2", 0.0, COMPONENT_HALF_HEIGHT * 2.0, 0.0),
        ],
    )]);
    assert!(clearance_issues(&touching_y).is_empty());

    let separated = design(vec![sheet(
        "sheet-1",
        "Sheet 1",
        vec![
            component("component-1", "R1", 0.0, 0.0, 0.0),
            component("component-2", "R2", COMPONENT_HALF_WIDTH * 2.0 + 1.0, 0.0, 0.0),
        ],
    )]);
    assert!(clearance_issues(&separated).is_empty());
}

#[test]
fn components_on_different_sheets_are_never_compared() {
    let design = design(vec![
        sheet("sheet-1", "Sheet 1", vec![component("component-1", "R1", 0.0, 0.0, 0.0)]),
        sheet("sheet-2", "Sheet 2", vec![component("component-2", "R2", 0.0, 0.0, 0.0)]),
    ]);

    assert!(clearance_issues(&design).is_empty());
}

#[test]
fn rotation_is_ignored_so_a_rotated_body_still_overlaps() {
    let rotated = design(vec![sheet(
        "sheet-1",
        "Sheet 1",
        vec![component("component-1", "R1", 0.0, 0.0, 0.0), component("component-2", "R2", 10.0, 0.0, 90.0)],
    )]);
    let unrotated = design(vec![sheet(
        "sheet-1",
        "Sheet 1",
        vec![component("component-1", "R1", 0.0, 0.0, 0.0), component("component-2", "R2", 10.0, 0.0, 0.0)],
    )]);

    assert_eq!(clearance_issues(&rotated).len(), 1);
    assert_eq!(clearance_issues(&rotated).len(), clearance_issues(&unrotated).len());
}

fn overlapping_design() -> u64 {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x) in [("R1", 0.0), ("R2", 10.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y: 0.0,
                rotation: 0.0,
                pins: Vec::new(),
            },
        )
        .expect("add component");
    }
    handle
}

#[test]
fn severity_override_to_error_fails_the_report() {
    let handle = overlapping_design();
    let checks = Some(vec!["clearance".to_string()]);

    let warning =
        inspect_with_options(handle, InspectOptions { checks: checks.clone(), ..InspectOptions::default() }).expect("inspect");
    assert_eq!(warning["passed"], true);
    assert_eq!(warning["issues"].as_array().expect("issues").len(), 1);

    let promoted = inspect_with_options(
        handle,
        InspectOptions {
            checks,
            severity: BTreeMap::from([("clearance/component-overlap".to_string(), "error".to_string())]),
            ..InspectOptions::default()
        },
    )
    .expect("inspect");
    assert_eq!(promoted["passed"], false);
    assert_eq!(promoted["issues"][0]["severity"], "error");

    close(handle).expect("close");
}

#[test]
fn waiver_accepts_the_overlap_and_no_overlap_leaves_issues_empty() {
    let handle = overlapping_design();
    let waived = inspect_with_options(
        handle,
        InspectOptions {
            checks: Some(vec!["clearance".to_string()]),
            waivers: vec![Waiver { code: "clearance/component-overlap".to_string(), sheet_id: None, object_id: None }],
            ..InspectOptions::default()
        },
    )
    .expect("inspect");
    assert_eq!(waived["passed"], true);
    assert_eq!(waived["issues"], json!([]));
    assert_eq!(waived["waived"].as_array().expect("waived").len(), 1);
    assert_eq!(waived["waived"][0]["waived"], true);

    move_component(handle, MoveComponentRequest { sheet: None, reference: "R2".to_string(), x: 100.0, y: 0.0, rotation: None })
        .expect("move component");
    let clear = inspect_with_options(
        handle,
        InspectOptions { checks: Some(vec!["clearance".to_string()]), ..InspectOptions::default() },
    )
    .expect("inspect");
    assert_eq!(clear["issues"], json!([]));
    assert_eq!(clear["passed"], true);

    close(handle).expect("close");
}
