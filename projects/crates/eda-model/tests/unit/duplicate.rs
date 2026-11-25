use super::*;

fn open_design() -> u64 {
    create(CreateRequest::default()).expect("create")["handle"].as_u64().expect("handle")
}

fn place(handle: u64, reference: &str, value: &str, x: f64, y: f64) {
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: reference.to_string(),
            value: value.to_string(),
            x,
            y,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add component");
}

fn duplicate(handle: u64, reference: &str) -> Result<Value, ServiceError> {
    duplicate_component(handle, DuplicateComponentRequest { sheet: None, reference: reference.to_string(), dx: 0.0, dy: 0.0 })
}

#[test]
fn duplicate_takes_the_next_free_reference_and_keeps_the_body() {
    let handle = open_design();
    place(handle, "R1", "10k", 40.0, 30.0);
    place(handle, "R3", "1k", 80.0, 30.0);
    set_component_footprint(
        handle,
        SetFootprintRequest { sheet: None, reference: "R1".to_string(), footprint: "Resistor_SMD:R_0805".to_string() },
    )
    .expect("set footprint");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "R1".to_string(), pin: "1".to_string(), pin_type: Some(PinType::Passive) },
    )
    .expect("type pin");
    set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "MPN".to_string(),
            value: "RC0805".to_string(),
        },
    )
    .expect("set property");
    set_reference_prefix(handle, ReferencePrefixRequest { class: "resistor".to_string(), prefix: "R".to_string() })
        .expect("declare class prefix");
    set_component_class(
        handle,
        ComponentClassRequest { sheet: None, reference: "R1".to_string(), class: "resistor".to_string() },
    )
    .expect("set class");
    let before = list_components(handle, None).expect("list")["revision"].as_u64().expect("revision");

    let report = duplicate(handle, "R1").expect("duplicate");
    assert_eq!(report["source"], "R1");
    assert_eq!(report["reference"], "R2", "the smallest free number of the prefix wins");
    let component = &report["component"];
    assert_eq!(component["ref"], "R2");
    assert_eq!(component["id"], "component-3", "the duplicate receives a fresh identity");
    assert_eq!(component["value"], "10k");
    assert_eq!(component["footprint"], "Resistor_SMD:R_0805");
    assert_eq!(component["x"], 40.0);
    assert_eq!(component["y"], 30.0);
    assert_eq!(component["pins"], json!(["1", "2"]));
    assert_eq!(component["pinTypes"], json!({ "1": "passive" }));
    assert_eq!(component["properties"], json!({ "MPN": "RC0805" }));
    assert_eq!(report["revision"].as_u64().expect("revision"), before + 1, "one call adds one revision");
    assert!(report["canUndo"].as_bool().expect("canUndo"));

    let classes = list_component_classes(handle).expect("classes");
    let assigned = classes["componentClasses"]
        .as_array()
        .expect("assignments")
        .iter()
        .filter(|entry| entry["class"] == "resistor")
        .collect::<Vec<_>>();
    assert_eq!(assigned.len(), 2, "the class follows the duplicate");

    let listed = list_components(handle, None).expect("list");
    assert_eq!(listed["components"].as_array().expect("components").len(), 3);

    let undone = undo(handle).expect("undo");
    assert_eq!(
        undone["revision"].as_u64().expect("revision"),
        report["revision"].as_u64().expect("revision") + 1,
        "undo bumps the revision once"
    );
    let listed = list_components(handle, None).expect("list after undo");
    assert_eq!(listed["components"].as_array().expect("components").len(), 2);
    close(handle).expect("close");
}

#[test]
fn duplicate_never_copies_net_nodes_wires_or_no_connects() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0);
    place(handle, "C1", "100n", 20.0, 0.0);
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect VCC");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [20.0, 0.0] })
        .expect("add wire");
    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 0.0,
            y: 0.0,
            node: Some(Node { reference: "R1".to_string(), pin: "2".to_string() }),
        },
    )
    .expect("add no-connect");

    let report = duplicate(handle, "R1").expect("duplicate");
    assert_eq!(report["reference"], "R2");
    let nets = list_nets(handle, None).expect("nets");
    assert_eq!(nets["nets"][0]["nodes"].as_array().expect("nodes").len(), 2, "the net keeps only the source nodes");
    let wires = list_wires(handle, None).expect("wires");
    assert_eq!(wires["wires"].as_array().expect("wires").len(), 1, "no wire follows the duplicate");
    let flags = list_no_connects(handle, None).expect("flags");
    assert_eq!(flags["noConnects"].as_array().expect("flags").len(), 1, "no flag follows the duplicate");
    close(handle).expect("close");
}

#[test]
fn duplicate_counts_across_sheets_and_applies_the_offset() {
    let handle = open_design();
    place(handle, "R1", "10k", 10.0, 20.0);
    let sheet = add_sheet(handle, Some("Aux")).expect("add sheet")["sheet"]["id"].as_str().expect("sheet id").to_string();
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some(sheet.clone()),
            reference: "R2".to_string(),
            value: "4k7".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add on the child sheet");

    let report = duplicate_component(
        handle,
        DuplicateComponentRequest { sheet: Some(sheet), reference: "R2".to_string(), dx: 5.0, dy: -2.5 },
    )
    .expect("duplicate with offset");
    assert_eq!(report["reference"], "R3", "R1 and R2 are used design-wide");
    assert_eq!(report["component"]["x"], 5.0);
    assert_eq!(report["component"]["y"], -2.5);
    close(handle).expect("close");
}

#[test]
fn duplicate_refuses_unknown_references_prefixless_references_and_bad_offsets() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0);
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "#1".to_string(),
            value: "test".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add a prefixless reference");
    let before = list_components(handle, None).expect("list")["revision"].as_u64().expect("revision");

    let missing = duplicate(handle, "R9").expect_err("unknown reference");
    assert_eq!(missing.code, "object/not-found");
    let prefixless = duplicate(handle, "#1").expect_err("no prefix to derive");
    assert_eq!(prefixless.code, "session/argument-invalid");
    let infinite = duplicate_component(
        handle,
        DuplicateComponentRequest { sheet: None, reference: "R1".to_string(), dx: f64::INFINITY, dy: 0.0 },
    )
    .expect_err("non-finite offset");
    assert_eq!(infinite.code, "session/argument-invalid");
    assert_eq!(list_components(handle, None).expect("list")["revision"].as_u64().expect("revision"), before);
    assert_eq!(list_components(handle, None).expect("list")["components"].as_array().expect("components").len(), 2);
    close(handle).expect("close");
}

#[test]
fn a_duplicate_survives_save_and_reopen_and_the_matrix_reports_it() {
    let matrix = capabilities();
    assert_eq!(matrix["edits"]["duplicate"], "supported");

    let handle = open_design();
    place(handle, "U1", "MCU", 0.0, 0.0);
    duplicate(handle, "U1").expect("duplicate");
    let path = std::env::temp_dir().join(format!("open-eda-duplicate-{}.oeda", std::process::id()));
    save(handle, &path, true).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let handle = reopened["handle"].as_u64().expect("handle");
    let listed = list_components(handle, None).expect("list");
    let references: Vec<&str> =
        listed["components"].as_array().expect("components").iter().filter_map(|entry| entry["ref"].as_str()).collect();
    assert_eq!(references, vec!["U1", "U2"]);
    close(handle).expect("close");
    let _ = fs::remove_file(&path);
}
