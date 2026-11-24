use super::*;

fn open_design() -> u64 {
    create(CreateRequest::default()).expect("create")["handle"].as_u64().expect("handle")
}

fn add_segment(handle: u64, net: Option<&str>, from: [f64; 2], to: [f64; 2]) -> Value {
    add_wire(handle, AddWireRequest { sheet: None, net: net.map(str::to_string), from, to }).expect("add wire")
}

fn split(handle: u64, id: &str, x: f64, y: f64) -> Result<Value, ServiceError> {
    split_wire(handle, SplitWireRequest { sheet: None, id: id.to_string(), x, y })
}

#[test]
fn split_wire_keeps_identity_net_and_sheet_and_records_one_history_step() {
    let handle = open_design();
    let added = add_segment(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);
    let before = added["revision"].as_u64().expect("revision");

    let report = split(handle, "wire-1", 4.0, 0.0).expect("split");
    assert_eq!(report["first"]["id"], "wire-1");
    assert_eq!(report["first"]["from"], json!([0.0, 0.0]));
    assert_eq!(report["first"]["to"], json!([4.0, 0.0]));
    assert_eq!(report["first"]["net"], "VCC");
    assert_eq!(report["second"]["id"], "wire-2");
    assert_eq!(report["second"]["from"], json!([4.0, 0.0]));
    assert_eq!(report["second"]["to"], json!([10.0, 0.0]));
    assert_eq!(report["second"]["net"], "VCC");
    assert_eq!(report["revision"].as_u64().expect("revision"), before + 1);
    assert!(report["canUndo"].as_bool().expect("canUndo"));

    let listed = list_wires(handle, None).expect("list");
    assert_eq!(listed["sheetId"], "sheet-1");
    let wires = listed["wires"].as_array().expect("wires");
    assert_eq!(wires.len(), 2);
    assert_eq!(wires[0]["id"], "wire-1");
    assert_eq!(wires[1]["id"], "wire-2");

    let undone = undo(handle).expect("undo");
    assert_eq!(undone["revision"].as_u64().expect("revision"), before + 2, "undo bumps the revision once");
    let restored = list_wires(handle, None).expect("list");
    let wires = restored["wires"].as_array().expect("wires");
    assert_eq!(wires.len(), 1, "one undo restores the unsplit segment");
    assert_eq!(wires[0]["to"], json!([10.0, 0.0]));
    assert_eq!(wires[0]["net"], "VCC");

    let redone = redo(handle).expect("redo");
    assert_eq!(redone["revision"].as_u64().expect("revision"), before + 3);
    assert_eq!(list_wires(handle, None).expect("list")["wires"].as_array().expect("wires").len(), 2);
    close(handle).expect("close");
}

#[test]
fn split_wire_handles_a_diagonal_segment_and_allocates_the_next_free_id() {
    let handle = open_design();
    add_segment(handle, Some("A"), [0.0, 0.0], [8.0, 6.0]);
    add_segment(handle, Some("B"), [0.0, 10.0], [1.0, 10.0]);

    let report = split(handle, "wire-1", 4.0, 3.0).expect("split diagonal");
    assert_eq!(report["first"]["to"], json!([4.0, 3.0]));
    assert_eq!(report["second"]["id"], "wire-3", "the new id follows the document counter");
    assert_eq!(report["second"]["from"], json!([4.0, 3.0]));
    assert_eq!(report["second"]["to"], json!([8.0, 6.0]));
    assert_eq!(report["second"]["net"], "A");
    close(handle).expect("close");
}

#[test]
fn split_wire_refuses_both_endpoints_and_leaves_the_document_untouched() {
    let handle = open_design();
    let added = add_segment(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);
    let before = added["revision"].as_u64().expect("revision");

    for (x, y) in [(0.0, 0.0), (10.0, 0.0)] {
        let error = split(handle, "wire-1", x, y).expect_err("endpoint");
        assert_eq!(error.code, "wire/split-point-not-interior");
    }
    assert_eq!(list_wires(handle, None).expect("list")["revision"].as_u64().expect("revision"), before);
    assert_eq!(list_wires(handle, None).expect("list")["wires"].as_array().expect("wires").len(), 1);
    close(handle).expect("close");
}

#[test]
fn split_wire_refuses_points_off_the_segment() {
    let handle = open_design();
    add_segment(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);

    for (x, y) in [(5.0, 1.0), (5.0, -0.5), (12.0, 0.0), (-1.0, 0.0)] {
        let error = split(handle, "wire-1", x, y).expect_err("off segment");
        assert_eq!(error.code, "wire/split-point-not-interior", "point ({x}, {y})");
    }
    close(handle).expect("close");
}

#[test]
fn split_wire_refuses_unknown_ids_non_finite_points_and_zero_length_wires() {
    let handle = open_design();
    add_segment(handle, None, [2.0, 2.0], [2.0, 2.0]);

    let missing = split(handle, "wire-9", 5.0, 0.0).expect_err("unknown wire");
    assert_eq!(missing.code, "wire/not-found");
    let infinite = split(handle, "wire-1", f64::INFINITY, 0.0).expect_err("non-finite point");
    assert_eq!(infinite.code, "session/argument-invalid");
    let zero_length = split(handle, "wire-1", 2.0, 2.0).expect_err("zero-length wire");
    assert_eq!(zero_length.code, "wire/split-point-not-interior");
    close(handle).expect("close");
}

#[test]
fn split_wire_edits_only_the_named_sheet() {
    let handle = open_design();
    let sheet = add_sheet(handle, Some("Aux")).expect("add sheet")["sheet"]["id"].as_str().expect("sheet id").to_string();
    add_wire(
        handle,
        AddWireRequest { sheet: Some(sheet.clone()), net: Some("GND".to_string()), from: [0.0, 1.0], to: [6.0, 1.0] },
    )
    .expect("wire on the child sheet");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [6.0, 0.0] })
        .expect("wire on the root sheet");

    let report = split_wire(handle, SplitWireRequest { sheet: Some(sheet.clone()), id: "wire-1".to_string(), x: 2.0, y: 1.0 })
        .expect("split on the named sheet");
    assert_eq!(report["second"]["net"], "GND");

    let renamed = list_wires(handle, Some(sheet.as_str())).expect("list sheet");
    assert_eq!(renamed["wires"].as_array().expect("wires").len(), 2);
    let root = list_wires(handle, None).expect("list root");
    assert_eq!(root["wires"].as_array().expect("wires").len(), 1, "the root wire is untouched");
    close(handle).expect("close");
}

#[test]
fn a_split_survives_save_and_reopen() {
    let handle = open_design();
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect VCC");
    add_segment(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);
    split(handle, "wire-1", 4.0, 0.0).expect("split");
    let path = std::env::temp_dir().join(format!("open-eda-split-{}.oeda", std::process::id()));
    save(handle, &path, true).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let handle = reopened["handle"].as_u64().expect("handle");
    let listed = list_wires(handle, None).expect("list");
    let wires = listed["wires"].as_array().expect("wires");
    assert_eq!(wires.len(), 2);
    assert_eq!(wires[0]["id"], "wire-1");
    assert_eq!(wires[0]["to"], json!([4.0, 0.0]));
    assert_eq!(wires[1]["id"], "wire-2");
    assert_eq!(wires[1]["net"], "VCC");
    assert_eq!(wires[1]["to"], json!([10.0, 0.0]));
    close(handle).expect("close");
    let _ = fs::remove_file(&path);
}
