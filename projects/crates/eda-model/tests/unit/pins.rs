use super::*;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("open-eda-pins-{label}-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn join(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn add_resistor(handle: u64, reference: &str, pins: &[&str], sheet: Option<&str>) {
    add_component(
        handle,
        AddComponentRequest {
            sheet: sheet.map(str::to_string),
            reference: reference.to_string(),
            value: "10k".to_string(),
            x: 40.0,
            y: 30.0,
            rotation: 0.0,
            pins: pins.iter().map(|pin| pin.to_string()).collect(),
        },
    )
    .expect("add component");
}

#[test]
fn pins_can_be_added_and_refuse_duplicates_and_illegal_names() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_resistor(handle, "R1", &["1"], None);

    let added =
        add_pin(handle, AddPinRequest { sheet: None, reference: "R1".to_string(), pin: "2".to_string() }).expect("add pin");
    assert_eq!(added["component"]["pins"], json!(["1", "2"]));
    assert_eq!(added["pin"], "2");
    assert_eq!(added["canUndo"], true);
    let revision = added["revision"].as_u64().expect("revision");

    let duplicate =
        add_pin(handle, AddPinRequest { sheet: None, reference: "R1".to_string(), pin: "2".to_string() }).unwrap_err();
    assert_eq!(duplicate.code, "pin/duplicate");
    for illegal in ["", " ", "A B", "A.B"] {
        let refused =
            add_pin(handle, AddPinRequest { sheet: None, reference: "R1".to_string(), pin: illegal.to_string() }).unwrap_err();
        assert_eq!(refused.code, "session/argument-invalid", "pin {illegal:?}");
    }
    let missing_component =
        add_pin(handle, AddPinRequest { sheet: None, reference: "R9".to_string(), pin: "2".to_string() }).unwrap_err();
    assert_eq!(missing_component.code, "object/not-found");
    let missing_sheet = add_pin(
        handle,
        AddPinRequest { sheet: Some("missing".to_string()), reference: "R1".to_string(), pin: "2".to_string() },
    )
    .unwrap_err();
    assert_eq!(missing_sheet.code, "sheet/not-found");

    let summary = summary(handle).expect("summary");
    assert_eq!(summary["revision"], revision, "failed edits leave the revision alone");
    close(handle).expect("close");
}

#[test]
fn pin_removal_refuses_references_and_clears_the_erc_finding() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_resistor(handle, "R1", &["1", "2"], None);
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIGNAL".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect R1.1");
    let unconnected = |handle: u64| {
        let report =
            inspect_with_options(handle, InspectOptions { checks: Some(vec!["erc".to_string()]), ..InspectOptions::default() })
                .expect("inspect");
        report["issues"].as_array().expect("issues").iter().filter(|issue| issue["code"] == "erc/unconnected-pin").count()
    };
    assert_eq!(unconnected(handle), 1);

    let in_use =
        remove_pin(handle, RemovePinRequest { sheet: None, reference: "R1".to_string(), pin: "1".to_string() }).unwrap_err();
    assert_eq!(in_use.code, "pin/in-use");
    assert!(in_use.message.contains("SIGNAL"), "the message names the net: {}", in_use.message);

    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 30.0,
            node: Some(Node { reference: "R1".to_string(), pin: "2".to_string() }),
        },
    )
    .expect("flag R1.2");
    let flagged =
        remove_pin(handle, RemovePinRequest { sheet: None, reference: "R1".to_string(), pin: "2".to_string() }).unwrap_err();
    assert_eq!(flagged.code, "pin/in-use");
    assert!(flagged.message.contains("no-connect"), "the message names the flag: {}", flagged.message);
    delete_no_connect(handle, 0, None).expect("remove flag");

    let removed = remove_pin(handle, RemovePinRequest { sheet: None, reference: "R1".to_string(), pin: "2".to_string() })
        .expect("remove");
    assert_eq!(removed["component"]["pins"], json!(["1"]));
    assert_eq!(removed["deleted"], "2");
    assert_eq!(unconnected(handle), 0, "the removed pin no longer produces an ERC finding");

    let missing =
        remove_pin(handle, RemovePinRequest { sheet: None, reference: "R1".to_string(), pin: "2".to_string() }).unwrap_err();
    assert_eq!(missing.code, "pin/not-found");
    close(handle).expect("close");
}

#[test]
fn pin_rename_rewrites_nets_no_connects_and_electrical_roles() {
    let dir = TempDir::new("rename");
    let path = dir.join("rename.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_resistor(handle, "R1", &["1", "2"], None);
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIGNAL".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect R1.1");
    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 30.0,
            node: Some(Node { reference: "R1".to_string(), pin: "2".to_string() }),
        },
    )
    .expect("flag R1.2");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "R1".to_string(), pin: "1".to_string(), pin_type: Some(PinType::Input) },
    )
    .expect("type R1.1");

    let renamed = rename_pin(
        handle,
        RenamePinRequest { sheet: None, reference: "R1".to_string(), from: "1".to_string(), to: "A".to_string() },
    )
    .expect("rename R1.1");
    assert_eq!(renamed["component"]["pins"], json!(["A", "2"]));
    assert_eq!(renamed["component"]["pinTypes"], json!({ "A": "input" }));
    assert_eq!(renamed["from"], "1");
    assert_eq!(renamed["to"], "A");
    let nets = list_nets(handle, None).expect("nets");
    assert_eq!(nets["nets"][0]["nodes"], json!([{ "ref": "R1", "pin": "A" }]));

    rename_pin(
        handle,
        RenamePinRequest { sheet: None, reference: "R1".to_string(), from: "2".to_string(), to: "B".to_string() },
    )
    .expect("rename R1.2");
    let flags = list_no_connects(handle, None).expect("no-connects");
    assert_eq!(flags["noConnects"][0]["node"], json!({ "ref": "R1", "pin": "B" }));

    let duplicate = rename_pin(
        handle,
        RenamePinRequest { sheet: None, reference: "R1".to_string(), from: "A".to_string(), to: "B".to_string() },
    )
    .unwrap_err();
    assert_eq!(duplicate.code, "pin/duplicate");
    let missing = rename_pin(
        handle,
        RenamePinRequest { sheet: None, reference: "R1".to_string(), from: "Z".to_string(), to: "Q".to_string() },
    )
    .unwrap_err();
    assert_eq!(missing.code, "pin/not-found");
    let illegal = rename_pin(
        handle,
        RenamePinRequest { sheet: None, reference: "R1".to_string(), from: "A".to_string(), to: "C D".to_string() },
    )
    .unwrap_err();
    assert_eq!(illegal.code, "session/argument-invalid");

    save(handle, &path, false).expect("a renamed pin keeps the document valid");
    close(handle).expect("close");
}

#[test]
fn pin_edits_resolve_sheets_and_walk_history() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_sheet(handle, Some("Power")).expect("add sheet");
    add_resistor(handle, "R1", &["1", "2"], Some("sheet-1"));
    add_resistor(handle, "R2", &["1"], Some("Power"));
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("Power".to_string()), net: "VCC".to_string(), nodes: vec!["R2.1".to_string()] },
    )
    .expect("connect R2.1");

    let added =
        add_pin(handle, AddPinRequest { sheet: Some("Power".to_string()), reference: "R2".to_string(), pin: "2".to_string() })
            .expect("add pin by sheet name");
    assert_eq!(added["component"]["pins"], json!(["1", "2"]));

    rename_pin(
        handle,
        RenamePinRequest {
            sheet: Some("sheet-2".to_string()),
            reference: "R2".to_string(),
            from: "1".to_string(),
            to: "VIN".to_string(),
        },
    )
    .expect("rename by sheet id");
    let nets = list_nets(handle, Some("Power")).expect("nets");
    assert_eq!(nets["nets"][0]["nodes"], json!([{ "ref": "R2", "pin": "VIN" }]));

    undo(handle).expect("undo rename");
    let restored = list_nets(handle, Some("Power")).expect("nets after undo");
    assert_eq!(restored["nets"][0]["nodes"], json!([{ "ref": "R2", "pin": "1" }]));
    redo(handle).expect("redo rename");
    let redone = list_nets(handle, Some("Power")).expect("nets after redo");
    assert_eq!(redone["nets"][0]["nodes"], json!([{ "ref": "R2", "pin": "VIN" }]));

    remove_pin(
        handle,
        RemovePinRequest { sheet: Some("Power".to_string()), reference: "R2".to_string(), pin: "2".to_string() },
    )
    .expect("remove R2.2");
    undo(handle).expect("undo removal");
    let components = list_components(handle, Some("Power")).expect("components");
    assert_eq!(components["components"][0]["pins"], json!(["VIN", "2"]));
    close(handle).expect("close");
}

#[test]
fn pin_edits_persist_across_save_and_reopen() {
    let dir = TempDir::new("persist");
    let path = dir.join("pins.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_resistor(handle, "R1", &["1", "2"], None);
    add_pin(handle, AddPinRequest { sheet: None, reference: "R1".to_string(), pin: "3".to_string() }).expect("add pin");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "R1".to_string(), pin: "3".to_string(), pin_type: Some(PinType::Passive) },
    )
    .expect("type pin 3");
    rename_pin(
        handle,
        RenamePinRequest { sheet: None, reference: "R1".to_string(), from: "2".to_string(), to: "B".to_string() },
    )
    .expect("rename pin");
    save(handle, &path, false).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let reopened_handle = reopened["handle"].as_u64().expect("reopened handle");
    let components = list_components(reopened_handle, None).expect("components");
    assert_eq!(components["components"][0]["pins"], json!(["1", "B", "3"]));
    assert_eq!(components["components"][0]["pinTypes"], json!({ "3": "passive" }));

    remove_pin(reopened_handle, RemovePinRequest { sheet: None, reference: "R1".to_string(), pin: "3".to_string() })
        .expect("remove typed pin");
    let components = list_components(reopened_handle, None).expect("components after removal");
    assert_eq!(components["components"][0]["pins"], json!(["1", "B"]));
    assert_eq!(components["components"][0]["pinTypes"], Value::Null);
    save(reopened_handle, &path, true).expect("save removal");
    close(reopened_handle).expect("close");

    let again = open(&path).expect("reopen after removal");
    let again_handle = again["handle"].as_u64().expect("handle");
    let components = list_components(again_handle, None).expect("components");
    assert_eq!(components["components"][0]["pins"], json!(["1", "B"]));
    close(again_handle).expect("close");
}

#[test]
fn svg_projection_draws_pin_stubs_and_names() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_resistor(handle, "U1", &["VCC", "GND", "IO1", "IO2"], None);

    let directory = TempDir::new("svg-pins");
    let target = directory.join("pins.svg");
    export(handle, "svg", &target).expect("export svg");
    let svg = fs::read_to_string(&target).expect("read svg");

    // The body spans 27.3 … 52.7 and every stub adds 2.54, so wires can end at 24.76 / 55.24.
    assert!(svg.contains("x2=\"24.76\""), "the left pin stub is drawn; got {svg}");
    assert!(svg.contains("x2=\"55.24\""), "the right pin stub is drawn; got {svg}");
    assert!(svg.contains(">VCC</text>"), "left pin names are drawn; got {svg}");
    assert!(svg.contains(">IO2</text>"), "right pin names are drawn; got {svg}");
    // Four pins split two per side on a 2.54 pitch around y = 30.
    assert!(svg.contains("y1=\"28.73\"") && svg.contains("y1=\"31.27\""), "pins use the fixed pitch; got {svg}");
    let view_box = svg.split("viewBox=\"").nth(1).expect("viewBox").split('"').next().expect("viewBox value");
    let min_x: f64 = view_box.split_whitespace().next().expect("min x").parse().expect("number");
    assert!(min_x <= 24.76, "the view box covers the left stubs; got {view_box}");
}
