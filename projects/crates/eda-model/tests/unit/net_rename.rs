use super::*;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("open-eda-{label}-{}-{sequence}", std::process::id()));
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

/// Opens one design that spells `VCC` on two sheets and everywhere the rename
/// must follow: nodes, wires, a global label, a net class and a bus alias.
fn seeded_design(label: &str) -> (u64, TempDir) {
    let dir = TempDir::new(label);
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, value) in [("R1", "10k"), ("C1", "100n")] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: value.to_string(),
                x: 10.0,
                y: 10.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    add_sheet(handle, Some("Power")).expect("add sheet");
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("sheet-2".to_string()),
            reference: "U1".to_string(),
            value: "MCU".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add U1");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect VCC");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "GND".to_string(), nodes: vec!["R1.2".to_string()] })
        .expect("connect GND");
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("sheet-2".to_string()), net: "VCC".to_string(), nodes: vec!["U1.1".to_string()] },
    )
    .expect("connect VCC on sheet-2");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
        .expect("add wire");
    add_wire(
        handle,
        AddWireRequest { sheet: Some("sheet-2".to_string()), net: Some("VCC".to_string()), from: [0.0, 0.0], to: [0.0, 10.0] },
    )
    .expect("add wire on sheet-2");
    add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: "VCC".to_string(),
            x: 5.0,
            y: 0.0,
            scope: Some(LabelScope::Global),
            rotation: None,
            justify: None,
        },
    )
    .expect("add label");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 4, aliases: vec!["VCC".to_string()] }).expect("add bus");
    add_net_class(
        handle,
        NetClassRequest {
            name: "Power".to_string(),
            width: 0.2,
            clearance: 0.1,
            via: 0.3,
            nets: vec!["VCC".to_string(), "GND".to_string()],
            buses: Vec::new(),
        },
    )
    .expect("add net class");
    (handle, dir)
}

fn rename(handle: u64, net: &str, name: &str) -> Result<Value, ServiceError> {
    rename_net(handle, RenameNetRequest { sheet: None, net: net.to_string(), name: name.to_string() })
}

fn revision(handle: u64) -> u64 {
    list(handle).expect("list")["revision"].as_u64().expect("revision")
}

#[test]
fn rename_follows_declarations_wires_labels_classes_and_bus_aliases() {
    let (handle, _dir) = seeded_design("rename-propagate");
    let before = revision(handle);
    let renamed = rename(handle, "VCC", "VCC_3V3").expect("rename");
    assert_eq!(renamed["net"]["name"], "VCC_3V3");
    assert_eq!(renamed["previousName"], "VCC");
    assert_eq!(renamed["changed"], true);
    assert_eq!(renamed["revision"], before + 1, "a changed rename adds exactly one revision");

    let listed = list(handle).expect("list");
    let names: Vec<&str> =
        listed["nets"].as_array().expect("nets").iter().map(|net| net["name"].as_str().expect("name")).collect();
    assert_eq!(names, vec!["VCC_3V3", "GND", "VCC_3V3"], "every sheet's declaration follows in document order");
    assert_eq!(
        listed["nets"][0]["nodes"],
        json!([{"ref": "R1", "pin": "1"}, {"ref": "C1", "pin": "1"}]),
        "nodes follow their net"
    );
    assert_eq!(listed["nets"][2]["nodes"], json!([{"ref": "U1", "pin": "1"}]));
    assert_eq!(listed["wires"][0]["net"], "VCC_3V3");
    assert_eq!(listed["wires"][1]["net"], "VCC_3V3", "the wire on the second sheet follows");

    let labels = list_labels(handle, None).expect("labels");
    assert_eq!(labels["labels"][0]["text"], "VCC_3V3", "the label that spelled the old name follows");

    let classes = list_net_classes(handle).expect("classes");
    assert_eq!(classes["netClasses"][0]["nets"], json!(["GND", "VCC_3V3"]));

    let buses = list_buses(handle).expect("buses");
    assert_eq!(buses["buses"][0]["aliases"], json!(["VCC_3V3"]), "the bus alias that spelled the old name follows");
    close(handle).expect("close");
}

#[test]
fn rename_reports_the_documented_error_codes() {
    let (handle, _dir) = seeded_design("rename-errors");
    let before = revision(handle);
    assert_eq!(rename(handle, "VCC", "   ").unwrap_err().code, "session/argument-invalid");
    assert_eq!(rename(handle, "NOPE", "X").unwrap_err().code, "net/not-found");
    assert_eq!(rename(handle, "VCC", "GND").unwrap_err().code, "object/duplicate", "a declared name is refused");
    assert_eq!(rename(handle, "net-1", "GND").unwrap_err().code, "object/duplicate", "lookup by id keeps the same rules");
    assert_eq!(revision(handle), before, "a refused rename adds no revision");
    close(handle).expect("close");
}

#[test]
fn an_already_matching_name_reports_changed_false_and_adds_no_revision() {
    let (handle, _dir) = seeded_design("rename-noop");
    let before = revision(handle);
    let unchanged = rename(handle, "VCC", "VCC").expect("no-op rename");
    assert_eq!(unchanged["changed"], false);
    assert_eq!(unchanged["previousName"], "VCC");
    assert_eq!(unchanged["net"]["name"], "VCC");
    assert_eq!(unchanged["revision"], before, "a no-op adds no revision");
    assert_eq!(revision(handle), before);
    close(handle).expect("close");
}

#[test]
fn rename_undoes_and_redoes_as_one_history_step() {
    let (handle, _dir) = seeded_design("rename-history");
    let before = revision(handle);
    rename(handle, "VCC", "VCC_3V3").expect("rename");
    assert_eq!(revision(handle), before + 1);

    let undone = undo(handle).expect("undo");
    assert_eq!(undone["revision"].as_u64().expect("revision"), before + 2, "undo restores the content and bumps the revision");
    let listed = list(handle).expect("list");
    let names: Vec<&str> =
        listed["nets"].as_array().expect("nets").iter().map(|net| net["name"].as_str().expect("name")).collect();
    assert_eq!(names, vec!["VCC", "GND", "VCC"], "one undo restores the whole rename");
    assert_eq!(listed["wires"][0]["net"], "VCC");
    assert_eq!(list_labels(handle, None).expect("labels")["labels"][0]["text"], "VCC");
    assert_eq!(
        list_net_classes(handle).expect("classes")["netClasses"][0]["nets"],
        json!(["VCC", "GND"]),
        "the class list returns to its original order"
    );
    assert_eq!(list_buses(handle).expect("buses")["buses"][0]["aliases"], json!(["VCC"]));

    let redone = redo(handle).expect("redo");
    assert_eq!(redone["revision"].as_u64().expect("revision"), before + 3);
    let listed = list(handle).expect("list");
    assert_eq!(listed["nets"][0]["name"], "VCC_3V3");
    assert_eq!(listed["nets"][2]["name"], "VCC_3V3");
    close(handle).expect("close");
}

#[test]
fn renamed_nets_wires_labels_classes_and_aliases_persist_across_reopen() {
    let (handle, dir) = seeded_design("rename-reopen");
    rename(handle, "VCC", "VCC_3V3").expect("rename");
    let path = dir.join("renamed.oeda");
    save(handle, &path, false).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let handle = reopened["handle"].as_u64().expect("handle");
    let listed = list(handle).expect("list");
    let names: Vec<&str> =
        listed["nets"].as_array().expect("nets").iter().map(|net| net["name"].as_str().expect("name")).collect();
    assert_eq!(names, vec!["VCC_3V3", "GND", "VCC_3V3"]);
    assert_eq!(listed["wires"][0]["net"], "VCC_3V3");
    assert_eq!(listed["wires"][1]["net"], "VCC_3V3");
    assert_eq!(list_labels(handle, None).expect("labels")["labels"][0]["text"], "VCC_3V3");
    assert_eq!(list_net_classes(handle).expect("classes")["netClasses"][0]["nets"], json!(["GND", "VCC_3V3"]));
    assert_eq!(list_buses(handle).expect("buses")["buses"][0]["aliases"], json!(["VCC_3V3"]));
    close(handle).expect("close");
}

#[test]
fn identical_renames_produce_byte_identical_documents() {
    let (first, first_dir) = seeded_design("rename-determinism-a");
    let (second, second_dir) = seeded_design("rename-determinism-b");
    rename(first, "VCC", "VCC_3V3").expect("rename first");
    rename(second, "VCC", "VCC_3V3").expect("rename second");
    let first_path = first_dir.join("a.oeda");
    let second_path = second_dir.join("b.oeda");
    save(first, &first_path, false).expect("save first");
    save(second, &second_path, false).expect("save second");
    let first_bytes = fs::read(&first_path).expect("read first");
    let second_bytes = fs::read(&second_path).expect("read second");
    assert_eq!(first_bytes, second_bytes, "the same document and command produce byte-identical output");
    assert_eq!(
        serde_json::to_string(&list(first).expect("list")).expect("json"),
        serde_json::to_string(&list(second).expect("json")).expect("json")
    );
    close(first).expect("close first");
    close(second).expect("close second");
}
