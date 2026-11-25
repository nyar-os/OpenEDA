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

#[test]
fn capability_matrix_reports_only_the_implemented_entries() {
    let matrix = capabilities();
    assert_eq!(matrix["engine"]["package"], env!("CARGO_PKG_NAME"));
    assert_eq!(matrix["project"]["import"], "partial");
    assert_eq!(matrix["project"]["edit"], "partial");
    assert_eq!(matrix["project"]["export"], "partial");
    for &format in DESIGN_FORMATS {
        let expected_export = if matches!(format, "net" | "spice" | "svg" | "pdf" | "png" | "bom" | "kicad_sch") {
            "supported"
        }
        else {
            "unsupported"
        };
        let expected_import = if matches!(format, "kicad_sch" | "net") { "supported" } else { "unsupported" };
        let expected_losses = match format {
            "kicad_sch" => json!(KICAD_LOSS_CODES),
            "net" => json!(NET_LOSS_CODES),
            _ => json!([]),
        };
        assert_eq!(matrix["formats"][format]["import"], expected_import, "format {format}");
        assert_eq!(matrix["formats"][format]["edit"], "unsupported");
        assert_eq!(matrix["formats"][format]["export"], expected_export, "format {format}");
        assert_eq!(matrix["formats"][format]["lossCodes"], expected_losses, "format {format}");
    }
    assert_eq!(matrix["checks"]["erc"], "supported");
    assert_eq!(matrix["checks"]["electrical"], "supported");
    assert_eq!(matrix["checks"]["drivers"], "supported");
    assert_eq!(matrix["checks"]["pin_types"], "supported");
    assert_eq!(matrix["checks"]["bus"], "supported");
    assert_eq!(matrix["checks"]["nets"], "supported");
    assert_eq!(matrix["checks"]["connectivity"], "supported");
    assert_eq!(matrix["checks"]["footprints"], "supported");
    assert_eq!(matrix["checks"]["power"], "supported");
    assert_eq!(matrix["checks"]["hierarchy"], "supported");
    assert_eq!(matrix["checks"]["drc"], "unsupported");
    assert_eq!(matrix["checks"]["clearance"], "supported");
    assert_eq!(matrix["renderers"]["svg"], "supported");
    assert_eq!(matrix["renderers"]["png"], "unsupported");
    assert_eq!(matrix["renderers"]["pdf"], "unsupported");
    assert_eq!(matrix["exportTargets"]["net"], "supported");
    assert_eq!(matrix["exportTargets"]["net_csv"], "supported");
    assert_eq!(matrix["exportTargets"]["net_hier"], "supported");
    assert_eq!(matrix["exportTargets"]["net_class"], "supported");
    assert_eq!(matrix["exportTargets"]["net_length"], "supported");
    assert_eq!(matrix["exportTargets"]["spice"], "supported");
    assert_eq!(matrix["exportTargets"]["spice_subckt"], "supported");
    assert_eq!(matrix["exportTargets"]["svg"], "supported");
    assert_eq!(matrix["exportTargets"]["pdf"], "supported");
    assert_eq!(matrix["exportTargets"]["bom"], "supported");
    assert_eq!(matrix["exportTargets"]["bom_json"], "supported");
    assert_eq!(matrix["exportTargets"]["bom_html"], "supported");
    assert_eq!(matrix["exportTargets"]["report"], "supported");
    assert_eq!(matrix["exportTargets"]["json"], "supported");
    assert_eq!(matrix["exportTargets"]["kicad_sch"], "supported");
    assert_eq!(matrix["exportTargets"]["kicad_sym"], "supported");
    assert_eq!(matrix["exportTargets"]["kicad_net"], "supported");
    assert_eq!(matrix["exportTargets"]["png"], "supported");
    assert_eq!(matrix["formats"]["png"]["export"], "supported");
    assert_eq!(matrix["exportTargets"]["kicad_pcb"], "unsupported");
    assert_eq!(matrix["exportTargets"]["gbr"], "unsupported");
    assert_eq!(matrix["edits"]["components"], "supported");
    assert_eq!(matrix["edits"]["junctions"], "supported");
    assert_eq!(matrix["edits"]["noConnects"], "supported");
    assert_eq!(matrix["edits"]["pins"], "supported");
    assert_eq!(matrix["edits"]["pinSwap"], "supported");
    assert_eq!(matrix["edits"]["annotate"], "supported");
    assert_eq!(matrix["edits"]["netRename"], "supported");
    assert_eq!(matrix["edits"]["wireSplit"], "supported");
    assert_eq!(matrix["edits"]["duplicate"], "supported");
    assert_eq!(matrix["queries"]["component"], "supported");
    assert_eq!(matrix["queries"]["net"], "supported");
    assert_eq!(matrix["queries"]["trace"], "supported");
}

#[test]
fn empty_design_passes_inspection() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    let report = inspect(handle, None).expect("inspect");
    assert_eq!(report["passed"], true);
    let codes: Vec<&str> =
        report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert_eq!(codes, vec!["sheet-empty"]);
    close(handle).expect("close");
}

#[test]
fn net_class_export_groups_rows_by_declared_class_and_reports_pin_roles() {
    let dir = TempDir::new("net-class");
    let path = dir.join("blinker-classes.csv");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, value, pins) in [
        ("R1", "10k", vec!["1".to_string(), "2".to_string()]),
        ("U1", "Buffer", vec!["VCC".to_string(), "OUT".to_string()]),
        ("C1", "100n", vec!["1".to_string(), "2".to_string()]),
    ] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: value.to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins,
            },
        )
        .expect("add component");
    }
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: vec!["D".to_string()] }).expect("add bus");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "U1.VCC".to_string()] },
    )
    .expect("connect VCC");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "DATA".to_string(), nodes: vec!["U1.OUT".to_string()] })
        .expect("connect DATA");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "SIG".to_string(), nodes: vec!["C1.1".to_string(), "C1.2".to_string()] },
    )
    .expect("connect SIG");
    set_pin_type(
        handle,
        SetPinTypeRequest {
            sheet: None,
            reference: "U1".to_string(),
            pin: "VCC".to_string(),
            pin_type: Some(PinType::PowerIn),
        },
    )
    .expect("type VCC pin");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "U1".to_string(), pin: "OUT".to_string(), pin_type: Some(PinType::Output) },
    )
    .expect("type OUT pin");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "R1".to_string(), pin: "1".to_string(), pin_type: Some(PinType::Passive) },
    )
    .expect("type R1 pin");
    add_net_class(
        handle,
        NetClassRequest {
            name: "HighSpeed".to_string(),
            width: 0.25,
            clearance: 0.2,
            via: 0.6,
            nets: vec!["VCC".to_string()],
            buses: vec!["DATA".to_string()],
        },
    )
    .expect("add net class");

    let report = export(handle, "net_class", &path).expect("export net_class");
    assert_eq!(report["format"], "net_class");
    assert_eq!(report["artifact"]["kind"], "net_class");
    assert_eq!(report["artifact"]["classes"], 1);
    assert_eq!(report["artifact"]["rows"], 6);
    assert_eq!(report["artifact"]["unclassifiedRows"], 2);
    assert_eq!(
        fs::read_to_string(&path).expect("read net class csv"),
        "Class,Sheet,Net,Ref,Pin,Role\n\
         HighSpeed,Sheet 1,VCC,R1,1,passive\n\
         HighSpeed,Sheet 1,VCC,U1,VCC,power_in\n\
         HighSpeed,Sheet 1,DATA0,U1,OUT,output\n\
         HighSpeed,Sheet 1,DATA1,U1,OUT,output\n\
         ,Sheet 1,SIG,C1,1,\n\
         ,Sheet 1,SIG,C1,2,\n"
    );
    assert_eq!(export_with_variant(handle, "net_class", &path, Some("lite")).unwrap_err().code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn net_length_export_reports_per_net_wire_length_and_unrouted_nets() {
    let dir = TempDir::new("net-length");
    let path = dir.join("lengths.csv");
    let second = dir.join("lengths-again.csv");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    .expect("add component");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "R1.2".to_string()] },
    )
    .expect("connect VCC");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "SIG".to_string(), nodes: vec!["R1.1".to_string(), "R1.2".to_string()] },
    )
    .expect("connect SIG");
    for (from, to) in [([0.0, 0.0], [10.0, 0.0]), ([10.0, 0.0], [10.0, 20.0]), ([10.0, 20.0], [13.0, 24.0])] {
        add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from, to }).expect("add wire");
    }

    let report = export(handle, "net_length", &path).expect("export net_length");
    assert_eq!(report["format"], "net_length");
    assert_eq!(report["artifact"]["kind"], "net_length");
    assert_eq!(report["artifact"]["nets"], 2);
    assert_eq!(report["artifact"]["wires"], 3);
    assert_eq!(report["artifact"]["unrouted"], 1);
    assert_eq!(report["artifact"]["length"], "35");
    let text = fs::read_to_string(&path).expect("read net length csv");
    assert_eq!(
        text,
        "Sheet,Net,Length,Wires,Unrouted\n\
         Sheet 1,VCC,35,3,no\n\
         Sheet 1,SIG,0,0,yes\n"
    );
    export(handle, "net_length", &second).expect("export again");
    assert_eq!(text, fs::read_to_string(&second).expect("read second csv"), "export must be byte-stable");
    close(handle).expect("close");
}

#[test]
fn pin_types_check_reports_role_conflicts_the_other_checks_do_not() {
    let mut u1 = inspection_component("component-1", "U1", 0.0, 0.0);
    u1.pins = vec!["OUT".to_string(), "VCC".to_string(), "BUS".to_string()];
    u1.pin_types.insert("OUT".to_string(), PinType::Output);
    u1.pin_types.insert("VCC".to_string(), PinType::PowerIn);
    u1.pin_types.insert("BUS".to_string(), PinType::TriState);
    let mut u2 = inspection_component("component-2", "U2", 10.0, 0.0);
    u2.pins = vec!["PWR".to_string()];
    u2.pin_types.insert("PWR".to_string(), PinType::PowerOut);
    let node = |reference: &str, pin: &str| Node { reference: reference.to_string(), pin: pin.to_string() };
    let mut sheet = inspection_sheet("sheet-1", vec![u1, u2]);
    sheet.nets = vec![
        Net { id: "net-1".to_string(), name: "SUPPLY_IN".to_string(), nodes: vec![node("U1", "OUT"), node("U1", "VCC")] },
        Net { id: "net-2".to_string(), name: "BUS_DRIVE".to_string(), nodes: vec![node("U2", "PWR"), node("U1", "BUS")] },
        Net { id: "net-3".to_string(), name: "REGULATED".to_string(), nodes: vec![node("U2", "PWR"), node("U1", "OUT")] },
        Net { id: "net-4".to_string(), name: "CLEAN".to_string(), nodes: vec![node("U2", "PWR"), node("U1", "VCC")] },
    ];
    let design = inspection_design(vec![sheet], 4);

    let issues = inspect_design(&design, &["pin_types".to_string()]);
    let codes: Vec<&str> = issues.iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert_eq!(codes, vec!["erc/output-power-in-conflict", "erc/power-output-driver-conflict"]);
    assert_eq!(issues[0]["severity"], "error");
    assert_eq!(issues[0]["sheetId"], "sheet-1");
    assert_eq!(issues[0]["objectId"], "net-1");
    assert!(issues[0]["message"].as_str().unwrap_or("").contains("U1.OUT"), "{}", issues[0]["message"]);
    assert_eq!(issues[1]["objectId"], "net-2");
    assert!(issues[1]["message"].as_str().unwrap_or("").contains("U1.BUS"), "{}", issues[1]["message"]);
    assert_eq!(design.revision, 4, "inspection must not advance the revision");

    let clean = inspection_design(
        vec![inspection_sheet(
            "sheet-1",
            vec![
                {
                    let mut u1 = inspection_component("component-1", "U1", 0.0, 0.0);
                    u1.pins = vec!["VCC".to_string()];
                    u1.pin_types.insert("VCC".to_string(), PinType::PowerIn);
                    u1
                },
                {
                    let mut u2 = inspection_component("component-2", "U2", 10.0, 0.0);
                    u2.pins = vec!["PWR".to_string()];
                    u2.pin_types.insert("PWR".to_string(), PinType::PowerOut);
                    u2
                },
            ],
        )],
        1,
    );
    assert!(inspect_design(&clean, &["pin_types".to_string()]).is_empty(), "a power output feeding a power input is legal");
}

#[test]
fn bom_export_sorts_designators_numerically_before_compressing_ranges() {
    let dir = TempDir::new("bom-natural");
    let path = dir.join("resistors.csv");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for index in 1..=12 {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: format!("R{index}"),
                value: "10k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add resistor");
    }
    export(handle, "bom", &path).expect("export bom");
    assert_eq!(fs::read_to_string(&path).expect("read bom"), "Comment,Footprint,Designator,Quantity\n10k,,R1-R12,12\n");
    close(handle).expect("close");
}

#[test]
fn create_edit_save_open_round_trip_and_exports() {
    let dir = TempDir::new("round-trip");
    let document = dir.join("blinker.oeda");
    let netlist = dir.join("blinker.net");
    let drawing = dir.join("blinker.svg");

    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    assert_eq!(created["design"]["sheets"][0]["title"], "Sheet 1");

    let first = add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 40.0,
            y: 30.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    assert_eq!(first["component"]["id"], "component-1");

    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "C1".to_string(),
            value: "100n".to_string(),
            x: 70.0,
            y: 30.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add C1");

    let vcc = connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect VCC");
    assert_eq!(vcc["net"]["id"], "net-1");
    assert_eq!(vcc["net"]["nodes"].as_array().expect("nodes").len(), 2);

    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "GND".to_string(), nodes: vec!["R1.2".to_string(), "C1.2".to_string()] },
    )
    .expect("connect GND");

    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [40.0, 30.0], to: [70.0, 30.0] })
        .expect("add wire");

    let report = inspect(handle, None).expect("inspect");
    assert_eq!(report["passed"], true);
    assert_eq!(report["issues"], json!([]));

    let saved = save(handle, &document, false).expect("save");
    assert!(saved["bytes"].as_u64().expect("bytes") > 0);
    let text = fs::read_to_string(&document).expect("read document");
    assert!(text.contains("\"format\": \"open-eda-design\""));
    assert!(text.contains("\"version\": 1"));

    let opened = open(&document).expect("open");
    let reopened = opened["handle"].as_u64().expect("reopened handle");
    assert_eq!(opened["design"]["revision"], report["revision"]);
    let listed = list(reopened).expect("list");
    assert_eq!(listed["components"].as_array().expect("components").len(), 2);
    assert_eq!(listed["nets"].as_array().expect("nets").len(), 2);
    assert_eq!(listed["wires"].as_array().expect("wires").len(), 1);

    let net_report = export(handle, "net", &netlist).expect("export net");
    assert_eq!(net_report["format"], "net");
    let net_text = fs::read_to_string(&netlist).expect("read netlist");
    assert!(net_text.starts_with("# OpenEDA netlist v1\n"));
    assert!(net_text.contains("NET VCC"));
    assert!(net_text.contains("NODE R1.1"));
    assert!(net_text.contains("NODE C1.1"));

    let svg_report = export(handle, "svg", &drawing).expect("export svg");
    assert_eq!(svg_report["format"], "svg");
    let svg_text = fs::read_to_string(&drawing).expect("read svg");
    assert!(svg_text.contains("<svg"));
    assert!(svg_text.contains("viewBox="));
    assert!(svg_text.contains("R1"));
    assert!(svg_text.contains("10k"));

    let kicad_path = dir.join("blinker.kicad_sch");
    let kicad_report = export(handle, "kicad_sch", &kicad_path).expect("export kicad_sch");
    assert_eq!(kicad_report["format"], "kicad_sch");
    assert_eq!(kicad_report["artifact"]["symbols"], 2);
    assert!(fs::read_to_string(&kicad_path).expect("read kicad_sch").starts_with("(kicad_sch "));

    close(handle).expect("close created");
    close(reopened).expect("close reopened");
    assert_eq!(summary(handle).unwrap_err().code, "design/not-found");
}

#[test]
fn inspection_reports_real_findings_and_rejects_unsupported_checks() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");

    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "1k".to_string(),
            x: 10.0,
            y: 10.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R2".to_string(),
            value: String::new(),
            x: 20.0,
            y: 10.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add R2");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIG".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("single node net");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("NOPE".to_string()), from: [0.0, 0.0], to: [5.0, 0.0] })
        .expect("wire with undeclared net");

    let report = inspect(handle, None).expect("inspect");
    assert_eq!(report["passed"], false);
    let codes: Vec<&str> =
        report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert!(codes.contains(&"net-fewer-than-two-nodes"));
    assert!(codes.contains(&"wire-unknown-net"));
    assert!(codes.contains(&"component-empty-value"));
    assert!(codes.contains(&"unreferenced-pin"));

    let erc_only = inspect(handle, Some(vec!["erc".to_string()])).expect("inspect erc");
    let erc_codes: Vec<&str> =
        erc_only["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert!(erc_codes.contains(&"unreferenced-pin"));
    assert!(!erc_codes.contains(&"wire-unknown-net"));
    assert!(!erc_codes.contains(&"component-empty-value"));

    let unsupported = inspect(handle, Some(vec!["drc".to_string()])).unwrap_err();
    assert_eq!(unsupported.code, "capability/not-implemented");

    close(handle).expect("close");
}

#[test]
fn editing_rejects_duplicate_references_and_missing_pins() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "1k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add R1");

    let duplicate_error = add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "2k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .unwrap_err();
    assert_eq!(duplicate_error.code, "object/duplicate");

    let missing_component =
        connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["C9.1".to_string()] })
            .unwrap_err();
    assert_eq!(missing_component.code, "object/not-found");

    let missing_pin =
        connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.2".to_string()] })
            .unwrap_err();
    assert_eq!(missing_pin.code, "object/not-found");

    let deleted = delete_component(handle, "R1", None).expect("delete");
    assert_eq!(deleted["deleted"], "R1");
    assert_eq!(list_components(handle, None).expect("list")["components"], json!([]));

    close(handle).expect("close");
}

#[test]
fn diff_documents_reads_two_files_and_reports_changes() {
    let dir = TempDir::new("diff");
    let left = dir.join("left.oeda");
    let right = dir.join("right.oeda");

    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 10.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    save(handle, &left, false).expect("save left");

    update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: Some("22k".to_string()),
            footprint: None,
            x: None,
            y: None,
            rotation: None,
        },
    )
    .expect("update R1");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "C1".to_string(),
            value: "100n".to_string(),
            x: 30.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add C1");
    save(handle, &right, false).expect("save right");

    let report = diff_documents(&left, &right).expect("diff");
    assert_eq!(report["format"], "open-eda-diff");
    assert_eq!(report["equal"], false);
    assert_eq!(report["summary"]["components"], json!({ "added": 1, "removed": 0, "modified": 1 }));
    assert_eq!(report["left"]["path"], left.display().to_string());
    assert_eq!(report["right"]["path"], right.display().to_string());
    assert!(report["right"]["revision"].as_u64().expect("revision") > report["left"]["revision"].as_u64().expect("revision"));

    let unchanged = diff_documents(&left, &left).expect("same file");
    assert_eq!(unchanged["equal"], true);
    assert_eq!(unchanged["sheets"], json!([]));

    close(handle).expect("close");
}

#[test]
fn duplicate_references_are_an_inspection_finding() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 3,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![
                Component {
                    id: "component-1".to_string(),
                    reference: "R1".to_string(),
                    value: "1k".to_string(),
                    footprint: String::new(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string()],
                    pin_types: BTreeMap::new(),
                    properties: BTreeMap::new(),
                },
                Component {
                    id: "component-2".to_string(),
                    reference: "R1".to_string(),
                    value: "2k".to_string(),
                    footprint: String::new(),
                    x: 10.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string()],
                    pin_types: BTreeMap::new(),
                    properties: BTreeMap::new(),
                },
            ],
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 3,
        next_net: 1,
        next_wire: 1,
    };
    let issues = inspect_design(&design, &["erc".to_string()]);
    assert!(issues.iter().any(|issue| issue["code"] == "duplicate-reference"));
}

fn inspection_component(id: &str, reference: &str, x: f64, y: f64) -> Component {
    Component {
        id: id.to_string(),
        reference: reference.to_string(),
        value: "1k".to_string(),
        footprint: String::new(),
        x,
        y,
        rotation: 0.0,
        pins: vec!["1".to_string()],
        pin_types: BTreeMap::new(),
        properties: BTreeMap::new(),
    }
}

fn inspection_sheet(id: &str, components: Vec<Component>) -> Sheet {
    Sheet {
        id: id.to_string(),
        name: id.to_string(),
        components,
        nets: Vec::new(),
        wires: Vec::new(),
        junctions: Vec::new(),
        no_connects: Vec::new(),
        texts: Vec::new(),
        labels: Vec::new(),
    }
}

fn inspection_design(sheets: Vec<Sheet>, revision: u64) -> Design {
    Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision,
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

#[test]
fn duplicate_references_across_sheets_report_one_grouped_finding() {
    let design = inspection_design(
        vec![
            inspection_sheet("sheet-1", vec![inspection_component("component-1", "R1", 10.0, 20.0)]),
            inspection_sheet("sheet-2", vec![inspection_component("component-2", "R1", 30.0, 40.0)]),
        ],
        7,
    );

    let issues = inspect_design(&design, &["erc".to_string()]);
    let grouped: Vec<&Value> = issues.iter().filter(|issue| issue["code"] == "erc/duplicate-reference").collect();
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0]["severity"], "error");
    assert_eq!(grouped[0]["revision"], 7);
    assert_eq!(grouped[0]["sheetId"], "sheet-1");
    assert_eq!(grouped[0]["objectId"], "component-1");
    let message = grouped[0]["message"].as_str().expect("message");
    assert!(message.contains("shared by 2 components"), "{message}");
    assert!(message.contains("`component-1`, `component-2`"), "{message}");
    assert_eq!(design.revision, 7, "inspection must not advance the revision");
}

#[test]
fn unique_references_report_no_duplicate_reference() {
    let design = inspection_design(
        vec![inspection_sheet(
            "sheet-1",
            vec![inspection_component("component-1", "R1", 0.0, 0.0), inspection_component("component-2", "R2", 10.0, 0.0)],
        )],
        1,
    );

    let issues = inspect_design(&design, &["erc".to_string()]);
    assert!(issues.iter().all(|issue| issue["code"] != "erc/duplicate-reference"));
}

#[test]
fn duplicate_reference_groups_report_once_in_deterministic_order() {
    let design = inspection_design(
        vec![
            inspection_sheet(
                "sheet-1",
                vec![inspection_component("component-1", "Z1", 0.0, 0.0), inspection_component("component-2", "A1", 10.0, 0.0)],
            ),
            inspection_sheet(
                "sheet-2",
                vec![
                    inspection_component("component-3", "Z1", 0.0, 0.0),
                    inspection_component("component-4", "Z1", 10.0, 0.0),
                    inspection_component("component-5", "A1", 20.0, 0.0),
                ],
            ),
        ],
        2,
    );

    let issues = inspect_design(&design, &["erc".to_string()]);
    let grouped: Vec<&Value> = issues.iter().filter(|issue| issue["code"] == "erc/duplicate-reference").collect();
    assert_eq!(grouped.len(), 2, "one finding per shared reference");

    let messages: Vec<&str> = grouped.iter().map(|issue| issue["message"].as_str().unwrap_or("")).collect();
    assert_eq!(
        messages,
        vec![
            "reference `A1` is shared by 2 components: `component-2`, `component-5`",
            "reference `Z1` is shared by 3 components: `component-1`, `component-3`, `component-4`",
        ]
    );
    assert_eq!(grouped[0]["objectId"], "component-2");

    let repeated = inspect_design(&design, &["erc".to_string()]);
    assert_eq!(issues, repeated, "the grouped findings are deterministic");
}

#[test]
fn duplicate_net_names_are_an_inspection_finding() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 2,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: Vec::new(),
            nets: vec![
                Net { id: "net-1".to_string(), name: "VCC".to_string(), nodes: Vec::new() },
                Net { id: "net-2".to_string(), name: "VCC".to_string(), nodes: Vec::new() },
            ],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 1,
        next_net: 3,
        next_wire: 1,
    };
    let issues = inspect_design(&design, &["erc".to_string()]);
    assert!(issues.iter().any(|issue| issue["code"] == "duplicate-net-name" && issue["severity"] == "error"));
}

#[test]
fn inspection_applies_severity_overrides_and_waivers() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 2,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "U1".to_string(),
                value: "buffer".to_string(),
                footprint: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string(), "3".to_string(), "4".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: vec![
                Net {
                    id: "net-1".to_string(),
                    name: "VCC".to_string(),
                    nodes: vec![
                        Node { reference: "U1".to_string(), pin: "1".to_string() },
                        Node { reference: "U1".to_string(), pin: "3".to_string() },
                    ],
                },
                Net {
                    id: "net-2".to_string(),
                    name: "VCC".to_string(),
                    nodes: vec![
                        Node { reference: "U1".to_string(), pin: "2".to_string() },
                        Node { reference: "U1".to_string(), pin: "4".to_string() },
                    ],
                },
            ],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 3,
        next_wire: 1,
    };
    let handle = insert_session(Session::new(design));
    let checks = Some(vec!["erc".to_string()]);

    let demoted = inspect_with_options(
        handle,
        InspectOptions {
            checks: checks.clone(),
            severity: BTreeMap::from([
                ("duplicate-net-name".to_string(), "warning".to_string()),
                ("erc/duplicate-net-name".to_string(), "off".to_string()),
            ]),
            waivers: Vec::new(),
        },
    )
    .expect("demote duplicate nets");
    assert_eq!(demoted["passed"], true);
    let active = demoted["issues"].as_array().expect("issues");
    assert_eq!(active.len(), 1);
    assert!(active.iter().all(|issue| issue["code"] == "duplicate-net-name"));
    assert!(active.iter().all(|issue| issue["severity"] == "warning"));
    assert_eq!(demoted["waived"], json!([]));
    assert_eq!(demoted["counts"], json!({ "active": 1, "waived": 0 }));

    let waived = inspect_with_options(
        handle,
        InspectOptions {
            checks: checks.clone(),
            severity: BTreeMap::new(),
            waivers: vec![
                Waiver { code: "duplicate-net-name".to_string(), sheet_id: None, object_id: Some("net-2".to_string()) },
                Waiver { code: "erc/duplicate-net-name".to_string(), sheet_id: None, object_id: Some("net-2".to_string()) },
            ],
        },
    )
    .expect("waive one object");
    assert_eq!(waived["issues"], json!([]));
    assert_eq!(waived["passed"], true);
    let accepted = waived["waived"].as_array().expect("waived");
    assert_eq!(accepted.len(), 2);
    assert!(accepted.iter().all(|issue| issue["objectId"] == "net-2"));
    assert!(accepted.iter().all(|issue| issue["waived"] == true));
    assert_eq!(waived["counts"], json!({ "active": 0, "waived": 2 }));

    let silenced = inspect_with_options(
        handle,
        InspectOptions {
            checks: checks.clone(),
            severity: BTreeMap::from([
                ("duplicate-net-name".to_string(), "off".to_string()),
                ("erc/duplicate-net-name".to_string(), "off".to_string()),
            ]),
            waivers: Vec::new(),
        },
    )
    .expect("silence duplicate nets");
    assert_eq!(silenced["issues"], json!([]));
    assert_eq!(silenced["waived"], json!([]));
    assert_eq!(silenced["counts"], json!({ "active": 0, "waived": 0 }));
    assert_eq!(silenced["passed"], true);

    let invalid = inspect_with_options(
        handle,
        InspectOptions {
            checks,
            severity: BTreeMap::from([("duplicate-net-name".to_string(), "loud".to_string())]),
            waivers: Vec::new(),
        },
    )
    .unwrap_err();
    assert_eq!(invalid.code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn undo_and_redo_walk_the_per_handle_history() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    assert_eq!(undo(handle).unwrap_err().code, "history/empty");
    assert_eq!(redo(handle).unwrap_err().code, "history/empty");

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
        .expect("add");
    }
    let before = summary(handle).expect("summary");
    assert_eq!(before["canUndo"], true);
    assert_eq!(before["canRedo"], false);
    assert_eq!(before["history"]["undo"], 2);

    let undone = undo(handle).expect("undo");
    assert_eq!(undone["canUndo"], true);
    assert_eq!(undone["canRedo"], true);
    assert_eq!(undone["design"]["counts"]["components"], 1);
    assert_eq!(list_components(handle, None).expect("list")["components"].as_array().expect("components").len(), 1);

    let redone = redo(handle).expect("redo");
    assert_eq!(redone["canRedo"], false);
    assert_eq!(redone["design"]["counts"]["components"], 2);
    close(handle).expect("close");
}

#[test]
fn new_edits_clear_redo_and_save_adds_no_step() {
    let dir = TempDir::new("history-save");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add R1");
    undo(handle).expect("undo");
    assert_eq!(summary(handle).expect("summary")["canRedo"], true);

    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "C1".to_string(),
            value: "100n".to_string(),
            x: 5.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add C1");
    let after_edit = summary(handle).expect("summary");
    assert_eq!(after_edit["canRedo"], false);
    assert_eq!(after_edit["history"]["undo"], 1);

    save(handle, &dir.join("history.oeda"), false).expect("save");
    let after_save = summary(handle).expect("summary");
    assert_eq!(after_save["history"]["undo"], 1);
    assert_eq!(after_save["canRedo"], false);
    close(handle).expect("close");
}

#[test]
fn history_is_bounded_to_the_newest_snapshots() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for index in 1..=40 {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: format!("R{index}"),
                value: "1k".to_string(),
                x: index as f64,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }
    assert_eq!(summary(handle).expect("summary")["history"]["undo"], 32);
    for _ in 0..32 {
        undo(handle).expect("undo");
    }
    assert_eq!(summary(handle).expect("summary")["canUndo"], false);
    assert_eq!(undo(handle).unwrap_err().code, "history/empty");
    close(handle).expect("close");
}

#[test]
fn sheets_can_be_added_renamed_and_deleted_with_integrity_rules() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    let added = add_sheet(handle, Some("Power")).expect("add sheet");
    assert_eq!(added["sheet"]["id"], "sheet-2");

    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("sheet-1".to_string()),
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add R1");

    assert_eq!(delete_sheet(handle, "sheet-1").unwrap_err().code, "sheet/not-empty");
    let renamed = rename_sheet(handle, "sheet-2", "Main").expect("rename sheet");
    assert_eq!(renamed["sheet"]["name"], "Main");
    let report = summary(handle).expect("summary");
    assert_eq!(report["sheets"][1]["name"], "Main");
    assert_eq!(report["counts"]["sheets"], 2);

    delete_sheet(handle, "sheet-2").expect("delete empty sheet");
    assert_eq!(delete_sheet(handle, "sheet-1").unwrap_err().code, "sheet/last-sheet");
    add_sheet(handle, Some("Main")).expect("add sheet again");
    assert_eq!(add_sheet(handle, Some("Sheet 1")).unwrap_err().code, "object/duplicate");
    close(handle).expect("close");
}

#[test]
fn deep_edits_keep_nets_and_wires_referentially_sound() {
    let dir = TempDir::new("deep-edits");
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
        .expect("add");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
        .expect("add wire");

    let moved = move_component(
        handle,
        MoveComponentRequest { sheet: None, reference: "R1".to_string(), x: 50.0, y: 60.0, rotation: Some(90.0) },
    )
    .expect("move");
    assert_eq!(moved["component"]["x"], 50.0);
    assert_eq!(moved["component"]["rotation"], 90.0);
    assert_eq!(
        move_component(
            handle,
            MoveComponentRequest { sheet: None, reference: "R9".to_string(), x: 0.0, y: 0.0, rotation: None }
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );

    let updated = update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: Some("22k".to_string()),
            footprint: None,
            x: None,
            y: None,
            rotation: None,
        },
    )
    .expect("update");
    assert_eq!(updated["component"]["value"], "22k");

    let renamed = rename_net(handle, RenameNetRequest { sheet: None, net: "VCC".to_string(), name: "VIN".to_string() })
        .expect("rename net");
    assert_eq!(renamed["net"]["name"], "VIN");
    let wires = list(handle).expect("list")["wires"].clone();
    assert_eq!(wires[0]["net"], "VIN");
    assert_eq!(
        rename_net(handle, RenameNetRequest { sheet: None, net: "VIN".to_string(), name: "VIN".to_string() })
            .expect("no-op rename")["net"]["name"],
        "VIN"
    );

    delete_wire(handle, "wire-1", None).expect("delete wire");
    assert_eq!(list(handle).expect("list")["wires"], json!([]));
    assert_eq!(delete_wire(handle, "wire-1", None).unwrap_err().code, "object/not-found");

    add_wire(handle, AddWireRequest { sheet: None, net: Some("VIN".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
        .expect("add wire again");
    delete_component(handle, "C1", None).expect("delete C1");
    assert_eq!(list(handle).expect("list")["wires"][0]["net"], "VIN");
    delete_component(handle, "R1", None).expect("delete R1");
    assert_eq!(list(handle).expect("list")["nets"], json!([]));
    assert_eq!(list(handle).expect("list")["wires"][0]["net"], Value::Null);
    save(handle, &dir.join("deep.oeda"), false).expect("the detached wire stays valid");
    close(handle).expect("close");
}

#[test]
fn component_update_changes_only_the_named_fields() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 10.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    set_component_footprint(
        handle,
        SetFootprintRequest { sheet: None, reference: "R1".to_string(), footprint: "0603".to_string() },
    )
    .expect("assign footprint");
    set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "MPN".to_string(),
            value: "ABC-123".to_string(),
        },
    )
    .expect("assign property");
    let before = summary(handle).expect("summary")["revision"].as_u64().expect("revision");

    let value_only = update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: Some("22k".to_string()),
            footprint: None,
            x: None,
            y: None,
            rotation: None,
        },
    )
    .expect("update value");
    assert_eq!(value_only["component"]["value"], "22k");
    assert_eq!(value_only["component"]["footprint"], "0603", "an unlisted footprint keeps its text");
    assert_eq!(value_only["component"]["x"], 10.0);
    assert_eq!(value_only["component"]["y"], 20.0);
    assert_eq!(value_only["component"]["rotation"], 0.0);
    assert_eq!(value_only["revision"].as_u64(), Some(before + 1));

    let multi = update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: None,
            footprint: Some("  R_0805_2012Metric  ".to_string()),
            x: Some(50.0),
            y: Some(60.0),
            rotation: Some(450.0),
        },
    )
    .expect("update placement");
    assert_eq!(multi["component"]["value"], "22k", "an unlisted value keeps its text");
    assert_eq!(multi["component"]["footprint"], "R_0805_2012Metric", "the footprint is trimmed");
    assert_eq!(multi["component"]["x"], 50.0);
    assert_eq!(multi["component"]["y"], 60.0);
    assert_eq!(multi["component"]["rotation"], 450.0, "rotation is stored as passed without wrapping");
    assert_eq!(multi["component"]["properties"]["MPN"], "ABC-123", "properties are never rewritten");
    assert_eq!(multi["revision"].as_u64(), Some(before + 2), "one update adds one revision");

    let repeated = update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: None,
            footprint: Some("  R_0805_2012Metric  ".to_string()),
            x: Some(50.0),
            y: Some(60.0),
            rotation: Some(450.0),
        },
    )
    .expect("repeat update");
    assert_eq!(repeated["component"], multi["component"], "the same request yields the same component");
    assert_eq!(repeated["revision"].as_u64(), Some(before + 3));

    let missing = update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R9".to_string(),
            value: Some("22k".to_string()),
            footprint: None,
            x: None,
            y: None,
            rotation: None,
        },
    )
    .expect_err("unknown reference");
    assert_eq!(missing.code, "object/not-found");
    assert_eq!(summary(handle).expect("summary")["revision"].as_u64(), Some(before + 3));

    for request in [
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: None,
            footprint: None,
            x: Some(f64::INFINITY),
            y: None,
            rotation: None,
        },
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: None,
            footprint: None,
            x: None,
            y: None,
            rotation: Some(f64::NAN),
        },
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: None,
            footprint: None,
            x: None,
            y: None,
            rotation: None,
        },
    ] {
        assert_eq!(update_component(handle, request).expect_err("invalid request").code, "session/argument-invalid");
    }
    close(handle).expect("close");
}

#[test]
fn wires_can_be_listed_and_updated_in_place() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
        .expect("add wire");

    let listed = list_wires(handle, None).expect("list wires");
    assert_eq!(listed["sheetId"], "sheet-1");
    assert_eq!(listed["wires"].as_array().expect("wires").len(), 1);
    assert_eq!(listed["wires"][0]["id"], "wire-1");

    let updated = update_wire(
        handle,
        UpdateWireRequest {
            sheet: None,
            id: "wire-1".to_string(),
            net: Some("GND".to_string()),
            from: Some([1.0, 2.0]),
            to: None,
        },
    )
    .expect("update wire");
    assert_eq!(updated["wire"]["net"], "GND");
    assert_eq!(updated["wire"]["from"], json!([1.0, 2.0]));
    assert_eq!(updated["wire"]["to"], json!([10.0, 0.0]));

    let cleared = update_wire(
        handle,
        UpdateWireRequest {
            sheet: None,
            id: "wire-1".to_string(),
            net: Some(String::new()),
            from: None,
            to: Some([11.0, 0.0]),
        },
    )
    .expect("clear net");
    assert_eq!(cleared["wire"]["net"], Value::Null);
    assert_eq!(cleared["wire"]["to"], json!([11.0, 0.0]));

    let no_change =
        update_wire(handle, UpdateWireRequest { sheet: None, id: "wire-1".to_string(), net: None, from: None, to: None })
            .expect_err("nothing to update");
    assert_eq!(no_change.code, "session/argument-invalid");
    let missing = update_wire(
        handle,
        UpdateWireRequest { sheet: None, id: "wire-9".to_string(), net: Some("GND".to_string()), from: None, to: None },
    )
    .expect_err("unknown wire");
    assert_eq!(missing.code, "object/not-found");
    let infinite = update_wire(
        handle,
        UpdateWireRequest { sheet: None, id: "wire-1".to_string(), net: None, from: Some([f64::INFINITY, 0.0]), to: None },
    )
    .expect_err("non-finite point");
    assert_eq!(infinite.code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn wires_merge_end_to_end_and_keep_the_first_identity() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
        .expect("first wire");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [10.0, 0.0], to: [30.0, 0.0] })
        .expect("second wire");

    let merged = merge_wires(handle, "sheet-1", 0, 1).expect("merge");
    assert_eq!(merged["wire"]["id"], "wire-1");
    assert_eq!(merged["wire"]["net"], "VCC");
    assert_eq!(merged["wire"]["from"], json!([0.0, 0.0]));
    assert_eq!(merged["wire"]["to"], json!([30.0, 0.0]));
    assert_eq!(merged["removed"], "wire-2");
    assert_eq!(merged["revision"], 4);
    let listed = list_wires(handle, None).expect("list");
    assert_eq!(listed["wires"].as_array().expect("wires").len(), 1);
    assert_eq!(listed["wires"][0]["id"], "wire-1");

    let undone = undo(handle).expect("undo");
    assert_eq!(undone["revision"], 5);
    assert_eq!(list_wires(handle, None).expect("list")["wires"].as_array().expect("wires").len(), 2);

    let reversed = merge_wires(handle, "sheet-1", 1, 0).expect("reverse merge");
    assert_eq!(reversed["wire"]["id"], "wire-2");
    assert_eq!(reversed["wire"]["from"], json!([0.0, 0.0]));
    assert_eq!(reversed["wire"]["to"], json!([30.0, 0.0]));
    assert_eq!(reversed["removed"], "wire-1");
    assert_eq!(list_wires(handle, None).expect("list")["wires"].as_array().expect("wires").len(), 1);
    close(handle).expect("close");
}

#[test]
fn wire_merge_rejects_mismatched_pairs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (net, from, to) in [
        (Some("VCC"), [0.0, 0.0], [10.0, 0.0]),
        (Some("VCC"), [10.0, 0.0], [20.0, 0.0]),
        (Some("GND"), [20.0, 0.0], [30.0, 0.0]),
        (Some("VCC"), [10.0, 0.0], [10.0, 10.0]),
        (Some("VCC"), [40.0, 0.0], [50.0, 0.0]),
        (Some("VCC"), [0.0, 0.0], [5.0, 0.0]),
        (Some("VCC"), [60.0, 0.0], [60.0, 0.0]),
        (None, [0.0, 20.0], [10.0, 20.0]),
        (None, [10.0, 20.0], [20.0, 20.0]),
    ] {
        add_wire(handle, AddWireRequest { sheet: None, net: net.map(str::to_string), from, to }).expect("add wire");
    }

    assert_eq!(merge_wires(handle, "sheet-1", 0, 2).expect_err("different net").code, "wire/merge-mismatch");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 3).expect_err("non-collinear").code, "wire/merge-mismatch");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 4).expect_err("not touching").code, "wire/merge-mismatch");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 5).expect_err("overlap").code, "wire/merge-mismatch");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 0).expect_err("same wire").code, "wire/merge-mismatch");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 6).expect_err("zero-length").code, "session/argument-invalid");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 99).expect_err("unknown index").code, "object/not-found");
    assert_eq!(merge_wires(handle, "sheet-1", 99, 0).expect_err("unknown first index").code, "object/not-found");
    assert_eq!(merge_wires(handle, "missing", 0, 1).expect_err("unknown sheet").code, "sheet/not-found");

    add_sheet(handle, Some("Power")).expect("add sheet");
    add_wire(
        handle,
        AddWireRequest { sheet: Some("Power".to_string()), net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] },
    )
    .expect("wire on the second sheet");
    assert_eq!(merge_wires(handle, "sheet-2", 0, 1).expect_err("index only exists on another sheet").code, "object/not-found");

    add_junction(handle, AddJunctionRequest { sheet: None, x: 10.0, y: 0.0 }).expect("junction");
    assert_eq!(merge_wires(handle, "sheet-1", 0, 1).expect_err("junction at shared point").code, "wire/merge-junction");

    let netless = merge_wires(handle, "sheet-1", 7, 8).expect("net-less merge");
    assert_eq!(netless["wire"]["net"], Value::Null);
    assert_eq!(netless["wire"]["from"], json!([0.0, 20.0]));
    assert_eq!(netless["wire"]["to"], json!([20.0, 20.0]));
    close(handle).expect("close");
}

#[test]
fn wire_merge_is_deterministic_and_survives_a_save() {
    let dir = TempDir::new("wire-merge");
    let path = dir.join("merge.oeda");
    let seed = |handle: u64| {
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
        .expect("component");
        connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
            .expect("declare VCC");
        add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
            .expect("first wire");
        add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [10.0, 0.0], to: [25.0, 0.0] })
            .expect("second wire");
    };
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    seed(handle);
    let first = merge_wires(handle, "sheet-1", 0, 1).expect("merge");

    let other = create(CreateRequest::default()).expect("create");
    let other_handle = other["handle"].as_u64().expect("handle");
    seed(other_handle);
    let second = merge_wires(other_handle, "sheet-1", 0, 1).expect("merge again");
    assert_eq!(first["wire"], second["wire"]);
    assert_eq!(first["removed"], second["removed"]);

    save(handle, &path, false).expect("save");
    close(handle).expect("close");
    close(other_handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let reopened_handle = reopened["handle"].as_u64().expect("handle");
    let wired = list_wires(reopened_handle, None).expect("list reopened");
    assert_eq!(wired["wires"].as_array().expect("wires").len(), 1);
    assert_eq!(wired["wires"][0]["id"], "wire-1");
    assert_eq!(wired["wires"][0]["net"], "VCC");
    assert_eq!(wired["wires"][0]["from"], json!([0.0, 0.0]));
    assert_eq!(wired["wires"][0]["to"], json!([25.0, 0.0]));
    close(reopened_handle).expect("close");
}

#[test]
fn selected_components_and_wires_translate_all_or_nothing() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x) in [("R1", 10.0), ("C1", 20.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y: 10.0,
                rotation: 90.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    let wire =
        add_wire(handle, AddWireRequest { sheet: None, net: None, from: [10.0, 10.0], to: [20.0, 10.0] }).expect("add wire");
    let wire_id = wire["wire"]["id"].as_str().expect("wire id").to_string();

    let translated = translate_objects(
        handle,
        TranslateRequest {
            sheet: None,
            references: vec!["R1".to_string(), "R1".to_string()],
            wires: vec![wire_id.clone()],
            dx: 5.0,
            dy: -2.0,
        },
    )
    .expect("translate");
    assert_eq!(translated["components"], 1);
    assert_eq!(translated["wires"], 1);
    assert_eq!(translated["dx"], 5.0);
    let listed = list_components(handle, None).expect("list components");
    assert_eq!(listed["components"][0]["x"], 15.0);
    assert_eq!(listed["components"][0]["y"], 8.0);
    assert_eq!(listed["components"][0]["rotation"], 90.0);
    assert_eq!(listed["components"][1]["x"], 20.0);
    assert_eq!(listed["components"][1]["y"], 10.0);
    let listed_wires = list_wires(handle, None).expect("list wires");
    assert_eq!(listed_wires["wires"][0]["from"], json!([15.0, 8.0]));
    assert_eq!(listed_wires["wires"][0]["to"], json!([25.0, 8.0]));

    let missing = translate_objects(
        handle,
        TranslateRequest {
            sheet: None,
            references: vec!["R1".to_string(), "R9".to_string()],
            wires: Vec::new(),
            dx: 1.0,
            dy: 0.0,
        },
    )
    .expect_err("missing reference");
    assert_eq!(missing.code, "object/not-found");
    assert_eq!(list_components(handle, None).expect("unchanged")["components"][0]["x"], 15.0);

    let empty = translate_objects(
        handle,
        TranslateRequest { sheet: None, references: Vec::new(), wires: Vec::new(), dx: 1.0, dy: 0.0 },
    )
    .expect_err("nothing selected");
    assert_eq!(empty.code, "session/argument-invalid");
    let still = translate_objects(
        handle,
        TranslateRequest { sheet: None, references: vec!["R1".to_string()], wires: Vec::new(), dx: 0.0, dy: 0.0 },
    )
    .expect_err("zero offset");
    assert_eq!(still.code, "session/argument-invalid");
    let infinite = translate_objects(
        handle,
        TranslateRequest { sheet: None, references: vec!["R1".to_string()], wires: Vec::new(), dx: f64::INFINITY, dy: 0.0 },
    )
    .expect_err("non-finite offset");
    assert_eq!(infinite.code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn bom_export_groups_by_value_and_footprint_and_compresses_designators() {
    let dir = TempDir::new("bom");
    let path = dir.join("blinker.csv");
    let json_path = dir.join("blinker-bom.json");
    let html_path = dir.join("blinker-bom.html");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for reference in ["R1", "R2", "R3", "R5", "R6", "R7", "C1"] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }
    set_component_footprint(
        handle,
        SetFootprintRequest {
            sheet: None,
            reference: "R1".to_string(),
            footprint: "Resistor_SMD:R_0805_2012Metric".to_string(),
        },
    )
    .expect("assign R1 footprint");
    let assigned = set_component_footprint(
        handle,
        SetFootprintRequest {
            sheet: None,
            reference: "C1".to_string(),
            footprint: " Capacitor_SMD:C_0603_1608Metric ".to_string(),
        },
    )
    .expect("assign C1 footprint");
    assert_eq!(assigned["component"]["footprint"], "Capacitor_SMD:C_0603_1608Metric");

    let report = export(handle, "bom", &path).expect("export bom");
    assert_eq!(report["format"], "bom");
    assert_eq!(report["artifact"]["rows"], 3);
    assert_eq!(report["artifact"]["components"], 7);
    assert_eq!(report["bytes"], fs::metadata(&path).expect("metadata").len() as usize);
    let text = fs::read_to_string(&path).expect("read bom");
    assert_eq!(
        text,
        "Comment,Footprint,Designator,Quantity\n\
             10k,,\"R2,R3,R5-R7\",5\n\
             10k,Capacitor_SMD:C_0603_1608Metric,C1,1\n\
             10k,Resistor_SMD:R_0805_2012Metric,R1,1\n"
    );

    let json_report = export(handle, "bom_json", &json_path).expect("export bom_json");
    assert_eq!(json_report["format"], "bom_json");
    assert_eq!(json_report["artifact"]["rows"], 3);
    let projection: Value = serde_json::from_str(&fs::read_to_string(&json_path).expect("read bom json")).expect("bom json");
    assert_eq!(projection["format"], "open-eda-bom");
    assert_eq!(projection["version"], 1);
    assert_eq!(projection["design"], "design-1");
    let items = projection["items"].as_array().expect("items");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["comment"], "10k");
    assert_eq!(items[0]["footprint"], "");
    assert_eq!(items[0]["quantity"], 5);
    assert_eq!(items[0]["designators"], json!(["R2", "R3", "R5", "R6", "R7"]));
    assert_eq!(items[0]["designatorRanges"], json!(["R2", "R3", "R5-R7"]));
    assert_eq!(items[1]["footprint"], "Capacitor_SMD:C_0603_1608Metric");
    assert_eq!(items[2]["designatorRanges"], json!(["R1"]));

    let html_report = export(handle, "bom_html", &html_path).expect("export bom_html");
    assert_eq!(html_report["format"], "bom_html");
    assert_eq!(html_report["artifact"]["rows"], 3);
    assert_eq!(html_report["artifact"]["components"], 7);
    let html = fs::read_to_string(&html_path).expect("read bom html");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<th>Comment</th><th>Footprint</th><th>Designator</th><th>Quantity</th>"));
    assert!(html.contains("<td>10k</td><td></td><td>R2, R3, R5-R7</td><td>5</td>"));
    assert!(html.contains("Resistor_SMD:R_0805_2012Metric"));
    assert!(html.contains("</table>"));
    assert_eq!(
        set_component_footprint(
            handle,
            SetFootprintRequest { sheet: None, reference: "R9".to_string(), footprint: String::new() }
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );
    close(handle).expect("close");
}

#[test]
fn symbols_report_groups_pins_references_and_missing_footprints() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, pins) in [
        ("R2", vec!["1".to_string(), "2".to_string()]),
        ("R1", vec!["1".to_string(), "2".to_string()]),
        ("U1", vec!["1".to_string(), "2".to_string(), "3".to_string()]),
    ] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins,
            },
        )
        .expect("add");
    }
    set_component_footprint(
        handle,
        SetFootprintRequest {
            sheet: None,
            reference: "R1".to_string(),
            footprint: "Resistor_SMD:R_0805_2012Metric".to_string(),
        },
    )
    .expect("assign footprint");

    let library = symbols(handle).expect("symbols");
    assert_eq!(library["revision"], summary(handle).expect("summary")["revision"]);
    assert_eq!(library["totals"]["symbols"], 2);
    assert_eq!(library["totals"]["components"], 3);
    assert_eq!(library["totals"]["missingFootprint"], 2);

    let entries = library["symbols"].as_array().expect("symbols");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["libId"], "open-eda:2pin");
    assert_eq!(entries[0]["pins"], 2);
    assert_eq!(entries[0]["components"], 2);
    assert_eq!(entries[0]["references"], json!(["R1", "R2"]));
    assert_eq!(entries[0]["footprints"], json!(["Resistor_SMD:R_0805_2012Metric"]));
    assert_eq!(entries[0]["missingFootprint"], 1);
    assert_eq!(entries[1]["libId"], "open-eda:3pin");
    assert_eq!(entries[1]["references"], json!(["U1"]));
    assert_eq!(entries[1]["footprints"], json!([]));
    assert_eq!(entries[1]["missingFootprint"], 1);
    close(handle).expect("close");
}

#[test]
fn pdf_export_writes_a_complete_vector_document() {
    let dir = TempDir::new("pdf");
    let path = dir.join("blinker.pdf");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k (1%)".to_string(),
            x: 20.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    add_wire(handle, AddWireRequest { sheet: None, net: None, from: [0.0, 0.0], to: [20.0, 20.0] }).expect("add wire");

    let report = export(handle, "pdf", &path).expect("export pdf");
    assert_eq!(report["format"], "pdf");
    assert_eq!(report["artifact"]["pages"], 1);
    assert_eq!(report["bytes"], fs::metadata(&path).expect("metadata").len() as usize);
    let text = fs::read_to_string(&path).expect("read pdf");
    assert!(text.starts_with("%PDF-1.4\n"));
    assert!(text.ends_with("%%EOF\n"));
    assert!(text.contains("startxref"));
    assert!(text.contains("/MediaBox [0 0 842 595]"));
    assert!(text.contains(r"10k \(1%\)"));
    assert!(text.contains("(R1) Tj"));
    close(handle).expect("close");
}

#[test]
fn pdf_export_reports_characters_the_base_font_cannot_encode() {
    let dir = TempDir::new("pdf-loss");
    let path = dir.join("unicode.pdf");
    let created =
        create(CreateRequest { title: Some("电源模块".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 20.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");

    let report = export(handle, "pdf", &path).expect("export pdf");
    assert_eq!(report["losses"][0]["code"], "pdf/text-encoding");
    assert!(report["losses"][0]["detail"].as_str().expect("detail").starts_with("4 character(s)"));
    let text = fs::read_to_string(&path).expect("read pdf");
    assert!(text.contains("(????) Tj"), "the Chinese title is written as question marks");

    close(handle).expect("close");
}

/// Creates a design with one component per sheet and returns its handle.
fn two_sheet_design() -> u64 {
    let created =
        create(CreateRequest { title: Some("Probe".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "ALPHAONLY".to_string(),
            x: 20.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    let second = add_sheet(handle, Some("Power")).expect("add sheet");
    assert_eq!(second["sheet"]["id"], "sheet-2");
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("sheet-2".to_string()),
            reference: "R2".to_string(),
            value: "BETAONLY".to_string(),
            x: 120.0,
            y: 90.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R2");
    handle
}

#[test]
fn pdf_export_writes_one_page_per_sheet_without_crossing_sheet_content() {
    let dir = TempDir::new("pdf-sheets");
    let path = dir.join("two-sheets.pdf");
    let handle = two_sheet_design();

    let report = export(handle, "pdf", &path).expect("export pdf");
    assert_eq!(report["artifact"]["pages"], 2);
    assert_eq!(report["artifact"]["components"], 2);
    let text = fs::read_to_string(&path).expect("read pdf");
    assert_eq!(text.matches("/Type /Page /Parent").count(), 2);
    let streams: Vec<&str> = text
        .split("<< /Length ")
        .skip(1)
        .map(|chunk| chunk.split("stream\n").nth(1).expect("content stream"))
        .map(|body| body.split("endstream").next().expect("stream body"))
        .collect();
    assert_eq!(streams.len(), 2);
    assert!(streams[0].contains("(ALPHAONLY)") && !streams[0].contains("(BETAONLY)"), "page 1:\n{}", streams[0]);
    assert!(streams[1].contains("(BETAONLY)") && !streams[1].contains("(ALPHAONLY)"), "page 2:\n{}", streams[1]);
    assert!(!streams[0].contains("(Power)"), "page 1 must not name the second sheet");
    assert!(!streams[1].contains("(Sheet 1)"), "page 2 must not name the first sheet");

    close(handle).expect("close");
}

#[test]
fn export_sheet_option_limits_the_drawing_to_one_sheet() {
    let dir = TempDir::new("export-sheet");
    let handle = two_sheet_design();

    let svg_path = dir.join("power.svg");
    let report = export_with_options(handle, "svg", &svg_path, None, Some("sheet-2")).expect("export svg");
    assert_eq!(report["artifact"]["components"], 1);
    let svg = fs::read_to_string(&svg_path).expect("read svg");
    assert!(svg.contains(r#"data-sheet="sheet-2""#), "{svg}");
    assert!(!svg.contains(r#"data-sheet="sheet-1""#), "a selected SVG must not paint the first sheet");
    assert!(svg.contains("BETAONLY") && !svg.contains("ALPHAONLY"));

    let pdf_path = dir.join("power.pdf");
    let report = export_with_options(handle, "pdf", &pdf_path, None, Some("Power")).expect("export pdf by name");
    assert_eq!(report["artifact"]["pages"], 1);
    assert_eq!(report["artifact"]["components"], 1);
    let pdf = fs::read_to_string(&pdf_path).expect("read pdf");
    assert_eq!(pdf.matches("/Type /Page /Parent").count(), 1);
    assert!(pdf.contains("(BETAONLY)") && !pdf.contains("(ALPHAONLY)"));

    assert_eq!(
        export_with_options(handle, "svg", &dir.join("x.svg"), None, Some("missing")).unwrap_err().code,
        "sheet/not-found"
    );
    assert_eq!(
        export_with_options(handle, "net", &dir.join("net.txt"), None, Some("sheet-1")).unwrap_err().code,
        "session/argument-invalid"
    );

    close(handle).expect("close");
}

#[test]
fn pdf_layout_centres_the_drawing_inside_the_printable_area() {
    // A wide drawing is limited by the printable width and keeps the left and
    // right margins, but its height is centred above the title block.
    let wide = pdf_layout(30.48, 15.24);
    assert!((wide.offset_x - PDF_MARGIN).abs() < 1e-9);
    let wide_center = wide.offset_y + 15.24 * wide.scale / 2.0;
    assert!((wide_center - (PDF_CONTENT_BOTTOM + PDF_CONTENT_TOP) / 2.0).abs() < 1e-6, "vertical centre {wide_center}");
    assert!(wide.offset_y + 15.24 * wide.scale <= PDF_CONTENT_TOP + 1e-9);

    // A tall drawing is limited by the printable height and is centred
    // horizontally instead of hugging the left margin.
    let tall = pdf_layout(10.0, 200.0);
    let tall_center = tall.offset_x + 10.0 * tall.scale / 2.0;
    assert!((tall_center - PDF_PAGE_WIDTH / 2.0).abs() < 1e-6, "horizontal centre {tall_center}");
    assert!(tall.offset_x >= PDF_MARGIN - 1e-9);
    assert!(tall.offset_x + 10.0 * tall.scale <= PDF_PAGE_WIDTH - PDF_MARGIN + 1e-9);
    assert!(tall.offset_y >= PDF_CONTENT_BOTTOM - 1e-9);
    assert!(tall.offset_y + 200.0 * tall.scale <= PDF_CONTENT_TOP + 1e-9);
}

#[test]
fn open_rejects_unknown_tags_versions_and_inconsistent_documents() {
    let dir = TempDir::new("invalid");
    let path = dir.join("broken.oeda");

    fs::write(&path, "not json").expect("write");
    assert_eq!(open(&path).unwrap_err().code, "format/invalid");

    fs::write(&path, json!({ "format": "other", "version": 1, "document": {} }).to_string()).expect("write");
    assert_eq!(open(&path).unwrap_err().code, "format/invalid");

    fs::write(&path, json!({ "format": "open-eda-design", "version": 2, "document": {} }).to_string()).expect("write");
    assert_eq!(open(&path).unwrap_err().code, "format/unsupported-version");

    let mut design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "R1".to_string(),
                value: "1k".to_string(),
                footprint: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 1,
        next_wire: 1,
    };
    let write = |design: &Design| {
        fs::write(&path, json!({ "format": "open-eda-design", "version": 1, "document": design }).to_string()).expect("write");
    };

    write(&design);
    assert!(open(&path).is_ok());

    design.units = "inch".to_string();
    write(&design);
    assert_eq!(open(&path).unwrap_err().code, "format/invalid");

    design.units = "mm".to_string();
    design.sheets[0].nets.push(Net {
        id: "net-1".to_string(),
        name: "VCC".to_string(),
        nodes: vec![Node { reference: "C9".to_string(), pin: "1".to_string() }],
    });
    write(&design);
    assert_eq!(open(&path).unwrap_err().code, "format/invalid");
}

#[test]
fn save_refuses_to_overwrite_by_default() {
    let dir = TempDir::new("overwrite");
    let path = dir.join("design.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");

    save(handle, &path, false).expect("first save");
    assert_eq!(save(handle, &path, false).unwrap_err().code, "format/invalid");
    save(handle, &path, true).expect("overwrite save");
    assert!(path.is_file());

    close(handle).expect("close");
}

fn blinker(handle: u64) {
    for (reference, value, x) in [("R1", "10k", 40.0), ("C1", "100n", 70.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: value.to_string(),
                x,
                y: 30.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect VCC");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "GND".to_string(), nodes: vec!["R1.2".to_string(), "C1.2".to_string()] },
    )
    .expect("connect GND");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [40.0, 30.0], to: [70.0, 30.0] })
        .expect("add wire");
}

#[test]
fn spice_export_maps_two_pin_elements_is_byte_stable_and_ends_with_end() {
    let dir = TempDir::new("spice");
    let first = dir.join("blinker.cir");
    let second = dir.join("blinker-again.cir");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);

    let report = export(handle, "spice", &first).expect("export spice");
    assert_eq!(report["format"], "spice");
    assert_eq!(report["editable"], false);
    assert_eq!(report["losses"], json!([]));
    assert_eq!(report["bytes"], fs::metadata(&first).expect("metadata").len() as usize);

    let text = fs::read_to_string(&first).expect("read spice");
    assert!(text.starts_with("* OpenEDA SPICE netlist v1\n"));
    assert!(text.trim_end().ends_with(".END"));
    assert!(text.contains("R1 VCC GND 10k\n"));
    assert!(text.contains("C1 VCC GND 100n\n"));

    export(handle, "spice", &second).expect("export spice again");
    assert_eq!(text, fs::read_to_string(&second).expect("read second spice"), "export must be byte-stable");
    close(handle).expect("close");
}

#[test]
fn spice_export_keeps_open_pins_as_nc_nodes_and_reports_losses() {
    let dir = TempDir::new("spice-open-pin");
    let path = dir.join("open.cir");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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

    let report = export(handle, "spice", &path).expect("export spice");
    let losses = report["losses"].as_array().expect("losses");
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0]["code"], "spice/open-pin");
    assert!(losses[0]["detail"].as_str().expect("detail").contains("R1"));

    let text = fs::read_to_string(&path).expect("read spice");
    assert!(text.contains("R1 VCC NC_R1_2 10k\n"));
    assert!(text.contains("* design design-1 revision"));
    close(handle).expect("close");
}

#[test]
fn spice_subcircuit_export_wraps_each_sheet_in_a_subckt() {
    let dir = TempDir::new("spice-subckt");
    let first = dir.join("library.cir");
    let second = dir.join("library-again.cir");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    rename_sheet(handle, "sheet-1", "Main").expect("rename sheet");
    add_sheet(handle, Some("Power")).expect("add sheet");
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("Power".to_string()),
            reference: "R2".to_string(),
            value: "1k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R2");
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("Power".to_string()), net: "VIN".to_string(), nodes: vec!["R2.1".to_string()] },
    )
    .expect("connect VIN");

    let report = export(handle, "spice_subckt", &first).expect("export spice_subckt");
    assert_eq!(report["format"], "spice_subckt");
    assert_eq!(report["artifact"]["kind"], "spice_subckt");
    assert_eq!(report["artifact"]["subcircuits"], 2);
    let losses = report["losses"].as_array().expect("losses");
    assert_eq!(losses.len(), 1, "the open R2.2 pin is reported");
    assert_eq!(losses[0]["code"], "spice/open-pin");

    let text = fs::read_to_string(&first).expect("read spice subckt");
    assert!(text.starts_with("* OpenEDA SPICE subcircuits v1\n"));
    assert!(text.contains(".SUBCKT Main VCC GND\n"), "{text}");
    assert!(text.contains("R1 VCC GND 10k\n"));
    assert!(text.contains(".ENDS Main\n"));
    assert!(text.contains(".SUBCKT Power VIN\n"));
    assert!(text.contains("R2 VIN NC_R2_2 1k\n"));
    assert!(text.contains(".ENDS Power\n"));
    assert!(text.trim_end().ends_with(".END"));

    export(handle, "spice_subckt", &second).expect("export again");
    assert_eq!(text, fs::read_to_string(&second).expect("read second"), "export must be byte-stable");
    close(handle).expect("close");
}

#[test]
fn spice_subcircuit_names_and_ports_stay_unique() {
    let dir = TempDir::new("spice-subckt-duplicate");
    let path = dir.join("duplicate.cir");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    rename_sheet(handle, "sheet-1", "A-B").expect("rename first sheet");
    add_sheet(handle, Some("A_B")).expect("add second sheet");
    for (sheet, reference) in [(None, "R1"), (Some("sheet-2".to_string()), "R2")] {
        add_component(
            handle,
            AddComponentRequest {
                sheet,
                reference: reference.to_string(),
                value: "1k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    connect_net(handle, ConnectNetRequest { sheet: None, net: "A-B".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect first net");
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("sheet-2".to_string()), net: "A_B".to_string(), nodes: vec!["R2.1".to_string()] },
    )
    .expect("connect second net");

    export(handle, "spice_subckt", &path).expect("export");
    let text = fs::read_to_string(&path).expect("read");
    assert!(text.contains(".SUBCKT A_B A_B\n"), "{text}");
    assert!(text.contains(".SUBCKT A_B_2 A_B\n"), "{text}");
    close(handle).expect("close");
}

#[test]
fn junction_and_text_edits_render_in_the_svg_projection() {
    let dir = TempDir::new("junction-text");
    let path = dir.join("annotated.svg");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");

    let junction = add_junction(handle, AddJunctionRequest { sheet: None, x: 10.0, y: 20.0 }).expect("add junction");
    assert_eq!(junction["junction"], json!({ "x": 10.0, "y": 20.0 }));
    assert_eq!(junction["revision"], 2);
    let text =
        add_text(handle, AddTextRequest { sheet: None, text: "NOTE <A>".to_string(), x: 30.0, y: 40.0 }).expect("add text");
    assert_eq!(text["text"]["text"], "NOTE <A>");
    assert_eq!(text["revision"], 3);

    let report = export(handle, "svg", &path).expect("export svg");
    assert_eq!(report["artifact"]["components"], 0);
    let svg = fs::read_to_string(&path).expect("read svg");
    assert!(svg.contains("<circle cx=\"10.00\" cy=\"20.00\""));
    assert!(svg.contains(">NOTE &lt;A&gt;</text>"));

    let rejected = add_junction(handle, AddJunctionRequest { sheet: None, x: f64::NAN, y: 0.0 }).unwrap_err();
    assert_eq!(rejected.code, "session/argument-invalid");
    let rejected_text =
        add_text(handle, AddTextRequest { sheet: None, text: "x".to_string(), x: 0.0, y: f64::INFINITY }).unwrap_err();
    assert_eq!(rejected_text.code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn svg_export_annotates_nets_labels_and_draws_a_title_block() {
    let dir = TempDir::new("svg-annotation");
    let path = dir.join("annotated.svg");
    let created = create(CreateRequest {
        title: Some("Sensor board".into()),
        rev: Some("B".into()),
        company: Some("OpenLab".into()),
        date: None,
        template: Some("blank".into()),
    })
    .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".into(),
            value: "10k".into(),
            x: 40.0,
            y: 40.0,
            rotation: 0.0,
            pins: vec!["1".into(), "2".into()],
        },
    )
    .expect("add component");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".into()), from: [10.0, 40.0], to: [27.30, 40.0] })
        .expect("add wire");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".into()), from: [10.0, 40.0], to: [10.0, 20.0] })
        .expect("add wire");
    add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: "N_IN".into(),
            x: 55.0,
            y: 40.0,
            scope: Some(LabelScope::Global),
            rotation: None,
            justify: None,
        },
    )
    .expect("add label");
    add_no_connect(handle, AddNoConnectRequest { sheet: None, x: 52.70, y: 42.54, node: None }).expect("add no-connect");

    export(handle, "svg", &path).expect("export svg");
    let svg = fs::read_to_string(&path).expect("read svg");
    // The net name is painted once, on the longest VCC segment (the vertical one).
    assert_eq!(svg.matches("data-net=\"VCC\"").count(), 1, "{svg}");
    assert!(svg.contains(">VCC</text>"), "{svg}");
    // Global labels get their boxed treatment; no-connect flags get the red cross.
    assert!(svg.contains(">N_IN</text>"), "{svg}");
    assert!(svg.contains("data-no-connect=\"true\""), "{svg}");
    // The title block carries the design identity without inventing values.
    assert!(svg.contains("data-title-block=\"true\""), "{svg}");
    assert!(svg.contains(">Sensor board</text>"), "{svg}");
    assert!(svg.contains("REV B</text>"), "{svg}");
    assert!(svg.contains("OpenLab"), "{svg}");
    assert!(svg.contains("1 parts / 2 wires"), "{svg}");
    close(handle).expect("close");
}

#[test]
fn validation_rejects_non_finite_junction_and_text_coordinates() {
    let mut design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: Vec::new(),
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: vec![Junction { x: f64::NAN, y: 0.0 }],
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 1,
        next_net: 1,
        next_wire: 1,
    };
    assert_eq!(validate_design(&design).unwrap_err().code, "format/invalid");
    design.sheets[0].junctions.clear();
    design.sheets[0].texts.push(TextLabel { text: "note".to_string(), x: 0.0, y: f64::INFINITY });
    assert_eq!(validate_design(&design).unwrap_err().code, "format/invalid");
}

#[test]
fn kicad_round_trip_preserves_junctions_and_texts_without_losses() {
    let dir = TempDir::new("kicad-junction-text");
    let path = dir.join("annotated.kicad_sch");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    add_junction(handle, AddJunctionRequest { sheet: None, x: 12.5, y: 7.5 }).expect("add junction");
    add_text(handle, AddTextRequest { sheet: None, text: "Revision A".to_string(), x: 15.0, y: 9.0 }).expect("add text");

    export(handle, "kicad_sch", &path).expect("export kicad_sch");
    let text = fs::read_to_string(&path).expect("read kicad_sch");
    assert!(text.contains("(junction (at 12.5 7.5)"));
    assert!(text.contains("(text \"Revision A\" (at 15 9 0)"));

    let imported = import_kicad_sch(&text).expect("import kicad_sch");
    assert_eq!(imported["losses"], json!([]));
    let sheet = &imported["design"]["sheets"][0];
    assert_eq!(sheet["junctions"], json!([{ "x": 12.5, "y": 7.5 }]));
    assert_eq!(sheet["texts"], json!([{ "text": "Revision A", "x": 15.0, "y": 9.0 }]));
    close(handle).expect("close");
}

#[test]
fn no_connect_flags_mark_pins_and_erc_ignores_them() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 40.0,
            y: 30.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIGNAL".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect R1.1");

    let unconnected = |value: &Value| {
        value["issues"].as_array().expect("issues").iter().filter(|issue| issue["code"] == "erc/unconnected-pin").count()
    };
    let before =
        inspect_with_options(handle, InspectOptions { checks: Some(vec!["erc".to_string()]), ..InspectOptions::default() })
            .expect("inspect");
    assert_eq!(unconnected(&before), 1);
    let opened = connectivity_report(handle).expect("connectivity");
    assert_eq!(opened["totals"]["unconnectedPins"], 1);

    let flag = add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 30.0,
            node: Some(Node { reference: "R1".to_string(), pin: "2".to_string() }),
        },
    )
    .expect("add no-connect");
    assert_eq!(flag["noConnect"]["node"], json!({ "ref": "R1", "pin": "2" }));

    let after =
        inspect_with_options(handle, InspectOptions { checks: Some(vec!["erc".to_string()]), ..InspectOptions::default() })
            .expect("inspect");
    assert_eq!(unconnected(&after), 0);
    let report = connectivity_report(handle).expect("connectivity");
    assert_eq!(report["totals"]["unconnectedPins"], 0);
    assert_eq!(report["sheets"][0]["noConnects"], 1);

    let duplicate = add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 30.0,
            node: Some(Node { reference: "R1".to_string(), pin: "2".to_string() }),
        },
    )
    .expect_err("duplicate flag");
    assert_eq!(duplicate.code, "no-connect/duplicate");
    let missing = add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 30.0,
            node: Some(Node { reference: "R1".to_string(), pin: "9".to_string() }),
        },
    )
    .expect_err("missing pin");
    assert_eq!(missing.code, "no-connect/pin-missing");

    let removed = delete_no_connect(handle, 0, None).expect("delete flag");
    assert_eq!(removed["remaining"], 0);
    let restored =
        inspect_with_options(handle, InspectOptions { checks: Some(vec!["erc".to_string()]), ..InspectOptions::default() })
            .expect("inspect");
    assert_eq!(unconnected(&restored), 1);
    assert_eq!(delete_no_connect(handle, 0, None).expect_err("out of range").code, "no-connect/not-found");
    close(handle).expect("close");
}

#[test]
fn junction_edits_refuse_duplicates_and_keep_indices_stable() {
    let dir = TempDir::new("junction-edits");
    let path = dir.join("junctions.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");

    let first = add_junction(handle, AddJunctionRequest { sheet: None, x: 10.0, y: 20.0 }).expect("first junction");
    assert_eq!(first["revision"], 2);
    let duplicate = add_junction(handle, AddJunctionRequest { sheet: None, x: 10.0, y: 20.0 }).unwrap_err();
    assert_eq!(duplicate.code, "junction/duplicate");
    let second = add_junction(handle, AddJunctionRequest { sheet: None, x: 30.0, y: 40.0 }).expect("second junction");
    assert_eq!(second["revision"], 3);
    let unknown_sheet =
        add_junction(handle, AddJunctionRequest { sheet: Some("missing".to_string()), x: 1.0, y: 2.0 }).unwrap_err();
    assert_eq!(unknown_sheet.code, "sheet/not-found");
    assert_eq!(move_junction(handle, 1, None, 10.0, 20.0).unwrap_err().code, "junction/duplicate");

    let listed = list_junctions(handle, None).expect("list junctions");
    assert_eq!(listed["junctions"], json!([{ "index": 0, "x": 10.0, "y": 20.0 }, { "index": 1, "x": 30.0, "y": 40.0 }]));

    let removed = delete_junction(handle, 0, None).expect("remove first");
    assert_eq!(removed["index"], 0);
    assert_eq!(removed["remaining"], 1);
    let relisted = list_junctions(handle, None).expect("relist");
    assert_eq!(relisted["junctions"], json!([{ "index": 0, "x": 30.0, "y": 40.0 }]));
    assert_eq!(delete_junction(handle, 1, None).unwrap_err().code, "junction/not-found");
    assert_eq!(delete_junction(handle, 0, Some("missing")).unwrap_err().code, "sheet/not-found");

    add_junction(handle, AddJunctionRequest { sheet: None, x: 55.0, y: 30.0 }).expect("persist junction");
    save(handle, &path, false).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let reopened_handle = reopened["handle"].as_u64().expect("reopened handle");
    let persisted = list_junctions(reopened_handle, None).expect("list reopened");
    assert_eq!(persisted["junctions"], json!([{ "index": 0, "x": 30.0, "y": 40.0 }, { "index": 1, "x": 55.0, "y": 30.0 }]));
    delete_junction(reopened_handle, 0, None).expect("remove after reopen");
    save(reopened_handle, &path, true).expect("save removal");
    close(reopened_handle).expect("close");
}

#[test]
fn no_connect_edits_suppress_erc_only_for_pin_flags() {
    let dir = TempDir::new("no-connect-edits");
    let path = dir.join("no-connects.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 40.0,
            y: 30.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIGNAL".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect R1.1");

    let unconnected = |handle: u64| {
        let report =
            inspect_with_options(handle, InspectOptions { checks: Some(vec!["erc".to_string()]), ..InspectOptions::default() })
                .expect("inspect");
        report["issues"].as_array().expect("issues").iter().filter(|issue| issue["code"] == "erc/unconnected-pin").count()
    };

    assert_eq!(unconnected(handle), 1);
    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 30.0,
            node: Some(Node { reference: "R1".to_string(), pin: "2".to_string() }),
        },
    )
    .expect("pin flag");
    assert_eq!(unconnected(handle), 0);

    add_no_connect(handle, AddNoConnectRequest { sheet: None, x: 0.0, y: 0.0, node: None }).expect("coordinate mark");
    assert_eq!(unconnected(handle), 0, "a coordinate-only mark suppresses nothing");

    save(handle, &path, false).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let reopened_handle = reopened["handle"].as_u64().expect("reopened handle");
    let listed = list_no_connects(reopened_handle, None).expect("list reopened");
    assert_eq!(listed["noConnects"][0]["node"], json!({ "ref": "R1", "pin": "2" }));
    assert_eq!(listed["noConnects"][1], json!({ "index": 1, "x": 0.0, "y": 0.0 }));
    assert_eq!(unconnected(reopened_handle), 0);

    let removed = delete_no_connect(reopened_handle, 0, None).expect("remove pin flag");
    assert_eq!(removed["remaining"], 1);
    assert_eq!(unconnected(reopened_handle), 1, "removing the flag restores the finding");
    assert_eq!(delete_no_connect(reopened_handle, 1, None).unwrap_err().code, "no-connect/not-found");
    assert_eq!(delete_no_connect(reopened_handle, 0, Some("missing")).unwrap_err().code, "sheet/not-found");
    close(reopened_handle).expect("close");
}

#[test]
fn kicad_hierarchical_sheets_and_ports_round_trip() {
    let text = "(kicad_sch (version 20211123) (generator \"open-eda\")\n\
                    \x20 (sheet (at 30 40) (size 20 15)\n\
                    \x20   (property \"Sheetname\" \"Power\") (property \"Sheetfile\" \"power.kicad_sch\")\n\
                    \x20   (pin \"VIN\" input (at 30 45 180)) (pin \"VOUT\" output (at 50 45 0))\n\
                    \x20 )\n)\n";
    let imported = import_kicad_sch(text).expect("import hierarchy");
    assert_eq!(imported["losses"], json!([]));
    let hierarchy = &imported["design"]["hierarchy"][0];
    assert_eq!(hierarchy["id"], "hierarchy-1");
    assert_eq!(hierarchy["name"], "Power");
    assert_eq!(hierarchy["file"], "power.kicad_sch");
    assert_eq!(hierarchy["x"], 30.0);
    assert_eq!(hierarchy["y"], 40.0);
    assert_eq!(hierarchy["width"], 20.0);
    assert_eq!(hierarchy["height"], 15.0);
    assert_eq!(
        hierarchy["ports"],
        json!([
            { "name": "VIN", "direction": "input", "x": 30.0, "y": 45.0 },
            { "name": "VOUT", "direction": "output", "x": 50.0, "y": 45.0 },
        ])
    );

    let design: Design = serde_json::from_value(imported["design"].clone()).expect("decode design");
    validate_design(&design).expect("validate");
    let rendered = crate::kicad::render(&design);
    assert!(rendered.contains("(sheet (at 30 40) (size 20 15)"));
    assert!(rendered.contains("(property \"Sheetname\" \"Power\""));
    assert!(rendered.contains("(property \"Sheetfile\" \"power.kicad_sch\""));
    assert!(rendered.contains("(pin \"VIN\" input (at 30 45 0)"));

    let again = import_kicad_sch(&rendered).expect("re-import hierarchy");
    assert_eq!(again["losses"], json!([]));
    assert_eq!(again["design"]["hierarchy"], imported["design"]["hierarchy"]);

    let mut broken = design.clone();
    broken.hierarchy[0].width = 0.0;
    assert_eq!(validate_design(&broken).expect_err("non-positive size").code, "format/invalid");
}

#[test]
fn erc_reports_hierarchical_ports_that_no_net_names() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: Vec::new(),
            nets: vec![Net { id: "net-1".to_string(), name: "VIN".to_string(), nodes: Vec::new() }],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: vec![HierarchicalSheet {
            id: "hierarchy-1".to_string(),
            name: "Power".to_string(),
            file: "power.kicad_sch".to_string(),
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 15.0,
            ports: vec![
                SheetPort { name: "VIN".to_string(), direction: PortDirection::Input, x: 0.0, y: 0.0 },
                SheetPort { name: "VOUT".to_string(), direction: PortDirection::Output, x: 0.0, y: 0.0 },
            ],
        }],
        next_component: 1,
        next_net: 2,
        next_wire: 1,
    };

    let issues = inspect_design(&design, &["erc".to_string()]);
    let ports: Vec<&Value> = issues.iter().filter(|issue| issue["code"] == "erc/unconnected-hierarchical-port").collect();
    assert_eq!(ports.len(), 1, "only VOUT matches no net");
    assert_eq!(ports[0]["severity"], "warning");
    assert_eq!(ports[0]["sheetId"], "hierarchy-1");
    assert_eq!(ports[0]["objectId"], "hierarchy-1:port:VOUT");
    assert!(ports[0]["message"].as_str().expect("message").contains("VOUT"));
}

#[test]
fn electrical_pin_types_report_conflicts_and_unpowered_inputs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x) in [("U1", 0.0), ("U2", 20.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "gate".to_string(),
                x,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "SIGNAL".to_string(), nodes: vec!["U1.1".to_string(), "U2.1".to_string()] },
    )
    .expect("connect signal");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["U1.2".to_string(), "U2.2".to_string()] },
    )
    .expect("connect rail");

    let set = |reference: &str, pin: &str, pin_type: Option<PinType>| {
        set_pin_type(
            handle,
            SetPinTypeRequest { sheet: None, reference: reference.to_string(), pin: pin.to_string(), pin_type },
        )
        .expect("set pin type")
    };
    set("U1", "1", Some(PinType::Output));
    set("U2", "1", Some(PinType::Output));

    let codes = |report: &Value| {
        report["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|issue| issue["code"].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    let conflict = inspect_with_options(
        handle,
        InspectOptions { checks: Some(vec!["electrical".to_string()]), ..InspectOptions::default() },
    )
    .expect("inspect");
    assert!(codes(&conflict).iter().any(|code| code == "erc/output-output-conflict"));
    let listed = list_components(handle, None).expect("list components");
    let u1 =
        listed["components"].as_array().expect("components").iter().find(|component| component["ref"] == "U1").expect("U1");
    assert_eq!(u1["pinTypes"]["1"], "output");
    assert_eq!(u1["pinTypes"].get("2"), None);

    set("U2", "1", Some(PinType::Input));
    set("U1", "2", Some(PinType::PowerIn));
    set("U2", "2", Some(PinType::PowerOut));
    let unpowered_cleared = inspect_with_options(
        handle,
        InspectOptions { checks: Some(vec!["electrical".to_string()]), ..InspectOptions::default() },
    )
    .expect("inspect");
    assert!(!codes(&unpowered_cleared).iter().any(|code| code == "erc/output-output-conflict"));
    assert!(!codes(&unpowered_cleared).iter().any(|code| code == "erc/power-in-unpowered"));
    assert!(!codes(&unpowered_cleared).iter().any(|code| code == "erc/input-input-conflict"));

    let input_revision = set("U1", "1", Some(PinType::Input))["revision"].as_u64().expect("revision");
    let inputs = inspect_with_options(
        handle,
        InspectOptions { checks: Some(vec!["electrical".to_string()]), ..InspectOptions::default() },
    )
    .expect("inspect");
    assert!(codes(&inputs).iter().any(|code| code == "erc/input-input-conflict"));

    let no_op = set("U1", "1", Some(PinType::Input));
    assert_eq!(no_op["revision"].as_u64().expect("revision"), input_revision);
    let cleared = set("U1", "1", None);
    assert_eq!(cleared["pinType"], Value::Null);
    let after_clear = inspect_with_options(
        handle,
        InspectOptions { checks: Some(vec!["electrical".to_string()]), ..InspectOptions::default() },
    )
    .expect("inspect");
    assert!(!codes(&after_clear).iter().any(|code| code == "erc/input-input-conflict"));

    let error = set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "U1".to_string(), pin: "9".to_string(), pin_type: Some(PinType::Output) },
    )
    .expect_err("missing pin");
    assert_eq!(error.code, "object/not-found");
    close(handle).expect("close");
}

#[test]
fn drivers_check_reports_undriven_inputs_and_open_power_pins() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    let add = |reference: &str, x: f64| {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "gate".to_string(),
                x,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component")
    };
    add("U1", 0.0);
    add("U2", 20.0);
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "SIGNAL".to_string(), nodes: vec!["U1.1".to_string(), "U2.1".to_string()] },
    )
    .expect("connect signal");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["U1.2".to_string(), "U2.2".to_string()] },
    )
    .expect("connect rail");
    let set = |reference: &str, pin: &str, pin_type: Option<PinType>| {
        set_pin_type(
            handle,
            SetPinTypeRequest { sheet: None, reference: reference.to_string(), pin: pin.to_string(), pin_type },
        )
        .expect("set pin type")
    };
    let codes = || {
        inspect_with_options(handle, InspectOptions { checks: Some(vec!["drivers".to_string()]), ..InspectOptions::default() })
            .expect("inspect")["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|issue| issue["code"].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };

    set("U1", "1", Some(PinType::Input));
    set("U1", "2", Some(PinType::PowerIn));
    let undriven = codes();
    assert!(undriven.iter().any(|code| code == "erc/undriven-input"), "{undriven:?}");
    assert!(!undriven.iter().any(|code| code == "erc/unconnected-power-pin"), "the power pin reaches VCC");

    set("U2", "1", Some(PinType::Output));
    assert!(!codes().iter().any(|code| code == "erc/undriven-input"), "an output drives SIGNAL");

    add("U3", 40.0);
    set("U3", "1", Some(PinType::PowerIn));
    assert!(codes().iter().any(|code| code == "erc/unconnected-power-pin"));

    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 40.0,
            y: 0.0,
            node: Some(Node { reference: "U3".to_string(), pin: "1".to_string() }),
        },
    )
    .expect("add no-connect");
    assert!(!codes().iter().any(|code| code == "erc/unconnected-power-pin"), "the flag marks the pin open");

    set("U2", "1", None);
    let passive_only = codes();
    assert!(passive_only.iter().any(|code| code == "erc/undriven-input"), "an untyped driver does not count");
    close(handle).expect("close");
}

#[test]
fn drawing_properties_are_edited_and_written_to_the_title_block() {
    let dir = TempDir::new("properties");
    let document = dir.join("title.oeda");
    let created = create(CreateRequest {
        title: Some("Blinker".to_string()),
        rev: Some("A".to_string()),
        company: Some("Acme".to_string()),
        date: Some("2026-11-22".to_string()),
        template: None,
    })
    .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    assert_eq!(created["design"]["rev"], "A");
    assert_eq!(created["design"]["company"], "Acme");
    assert_eq!(created["design"]["date"], "2026-11-22");

    let untouched = update_design(handle, UpdateDesignRequest::default()).expect("no-op");
    assert_eq!(untouched["revision"], 1);

    let updated = update_design(
        handle,
        UpdateDesignRequest { title: Some("Blinker II".to_string()), rev: Some("B".to_string()), company: None, date: None },
    )
    .expect("update");
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["title"], "Blinker II");
    assert_eq!(updated["rev"], "B");
    assert_eq!(updated["company"], "Acme");

    save(handle, &document, false).expect("save");
    let opened = open(&document).expect("open");
    let reopened = opened["handle"].as_u64().expect("handle");
    assert_eq!(opened["design"]["rev"], "B");
    assert_eq!(opened["design"]["company"], "Acme");
    assert_eq!(opened["design"]["date"], "2026-11-22");

    let kicad_path = dir.join("title.kicad_sch");
    export(reopened, "kicad_sch", &kicad_path).expect("export");
    let text = fs::read_to_string(&kicad_path).expect("read");
    assert!(text.contains("(title_block"));
    assert!(text.contains("(title \"Blinker II\")"));
    assert!(text.contains("(rev \"B\")"));
    assert!(text.contains("(company \"Acme\")"));
    assert!(text.contains("(date \"2026-11-22\")"));

    let (imported, losses) = crate::kicad::import(&text).expect("import");
    assert_eq!(imported.title.as_deref(), Some("Blinker II"));
    assert_eq!(imported.rev, "B");
    assert_eq!(imported.company, "Acme");
    assert_eq!(imported.date, "2026-11-22");
    assert!(losses.is_empty());

    let cleared =
        update_design(handle, UpdateDesignRequest { title: Some(String::new()), rev: None, company: None, date: None })
            .expect("clear title");
    assert_eq!(cleared["title"], Value::Null);
    assert_eq!(cleared["revision"], 3);

    close(handle).expect("close");
    close(reopened).expect("close");
}

#[test]
fn kicad_round_trip_carries_pin_electrical_types() {
    let dir = TempDir::new("pin-types");
    let document = dir.join("typed.oeda");
    let kicad_path = dir.join("typed.kicad_sch");

    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "U1".to_string(),
            value: "driver".to_string(),
            x: 10.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "U1".to_string(), pin: "1".to_string(), pin_type: Some(PinType::Output) },
    )
    .expect("type pin 1");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "U1".to_string(), pin: "2".to_string(), pin_type: Some(PinType::PowerIn) },
    )
    .expect("type pin 2");
    save(handle, &document, false).expect("save");
    export(handle, "kicad_sch", &kicad_path).expect("export");
    let text = fs::read_to_string(&kicad_path).expect("read");
    assert!(text.contains("(pin output line"));
    assert!(text.contains("(pin power_in line"));

    let (imported, losses) = crate::kicad::import(&text).expect("import");
    assert!(losses.is_empty(), "unexpected losses: {losses:?}");
    let component = imported.sheets[0].components.iter().find(|component| component.reference == "U1").expect("U1");
    assert_eq!(component.pin_types.get("1"), Some(&PinType::Output));
    assert_eq!(component.pin_types.get("2"), Some(&PinType::PowerIn));
    close(handle).expect("close");
}

#[test]
fn kicad_sch_export_is_deterministic_and_round_trips() {
    let dir = TempDir::new("kicad");
    let first = dir.join("blinker.kicad_sch");
    let second = dir.join("blinker-again.kicad_sch");

    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    set_component_footprint(
        handle,
        SetFootprintRequest {
            sheet: None,
            reference: "R1".to_string(),
            footprint: "Resistor_SMD:R_0805_2012Metric".to_string(),
        },
    )
    .expect("assign footprint");

    let report = export(handle, "kicad_sch", &first).expect("export kicad_sch");
    assert_eq!(report["format"], "kicad_sch");
    assert_eq!(report["editable"], false);
    assert_eq!(report["artifact"]["symbols"], 2);
    assert_eq!(report["artifact"]["wires"], 1);
    assert_eq!(report["artifact"]["labels"], 2);
    assert_eq!(report["bytes"], fs::metadata(&first).expect("metadata").len() as usize);
    export(handle, "kicad_sch", &second).expect("export again");

    let text = fs::read_to_string(&first).expect("read kicad_sch");
    assert_eq!(text, fs::read_to_string(&second).expect("read second kicad_sch"), "export must be byte-stable");
    assert!(text.starts_with("(kicad_sch (version 20211123) (generator \"open-eda\")\n"));
    assert!(text.trim_end().ends_with(')'));
    assert!(text.contains("(paper \"A4\")"));
    assert!(text.contains("(symbol (lib_id \"open-eda:2pin\") (at 40 30 0)"));
    assert!(text.contains("(property \"Reference\" \"R1\""));
    assert!(text.contains("(property \"Value\" \"10k\""));
    assert!(text.contains("(property \"Footprint\" \"Resistor_SMD:R_0805_2012Metric\""));
    assert!(text.contains("(property \"Footprint\" \"\""));
    assert!(text.contains("(pin \"1\" (uuid \"00000000-0000-0000-0000-"));
    assert!(text.contains("(wire (pts (xy 40 30) (xy 70 30)) (stroke (width 0) (type default)) (uuid \""));
    assert!(text.contains("(label \"VCC\" (at 40 30 0) (effects (font (size 1.27 1.27)) (justify left bottom))"));
    assert!(!text.contains(".0000"), "coordinates keep no redundant trailing zeros");

    let imported = import_kicad_sch(&text).expect("import kicad_sch");
    assert_eq!(imported["losses"], json!([]));
    let design = &imported["design"];
    assert_eq!(design["revision"], 1);
    assert_eq!(design["units"], "mm");
    assert_eq!(design["next_component"], 3);
    assert_eq!(design["next_net"], 3);
    assert_eq!(design["next_wire"], 2);

    let sheet = &design["sheets"][0];
    assert_eq!(sheet["id"], "sheet-1");
    let components = sheet["components"].as_array().expect("components");
    assert_eq!(components.len(), 2);
    assert_eq!(components[0]["ref"], "R1");
    assert_eq!(components[0]["value"], "10k");
    assert_eq!(components[0]["x"], 40.0);
    assert_eq!(components[0]["y"], 30.0);
    assert_eq!(components[0]["rotation"], 0.0);
    assert_eq!(components[0]["pins"], json!(["1", "2"]));
    assert_eq!(components[0]["footprint"], "Resistor_SMD:R_0805_2012Metric");
    assert_eq!(components[1]["ref"], "C1");
    assert_eq!(components[1]["footprint"], Value::Null);

    let wires = sheet["wires"].as_array().expect("wires");
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0]["from"], json!([40.0, 30.0]));
    assert_eq!(wires[0]["to"], json!([70.0, 30.0]));

    let nets = sheet["nets"].as_array().expect("nets");
    let names: Vec<&str> = nets.iter().filter_map(|net| net["name"].as_str()).collect();
    assert_eq!(names, vec!["VCC", "GND"]);
    assert_eq!(nets[0]["nodes"][0]["ref"], "R1");

    close(handle).expect("close");
}

#[test]
fn kicad_sch_round_trips_custom_component_properties() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "MPN".to_string(),
            value: "ABC-123".to_string(),
        },
    )
    .expect("set MPN");
    set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "Datasheet".to_string(),
            value: "https://example.test/r1.pdf".to_string(),
        },
    )
    .expect("set datasheet");

    let dir = TempDir::new("kicad-properties");
    let path = dir.join("blinker.kicad_sch");
    export(handle, "kicad_sch", &path).expect("export kicad_sch");
    let text = fs::read_to_string(&path).expect("read kicad_sch");
    assert!(text.contains("(property \"MPN\" \"ABC-123\""), "MPN is written: {text}");
    assert!(text.contains("(property \"Datasheet\" \"https://example.test/r1.pdf\""), "Datasheet is written");

    let imported = import_kicad_sch(&text).expect("import kicad_sch");
    assert_eq!(imported["losses"], json!([]));
    let properties = &imported["design"]["sheets"][0]["components"][0]["properties"];
    assert_eq!(properties["MPN"], json!("ABC-123"));
    assert_eq!(properties["Datasheet"], json!("https://example.test/r1.pdf"));
    close(handle).expect("close");
}

#[test]
fn kicad_sym_export_writes_importable_symbol_declarations() {
    let dir = TempDir::new("kicad-sym-export");
    let path = dir.join("blinker.kicad_sym");

    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    for reference in ["R1", "C1"] {
        set_pin_type(
            handle,
            SetPinTypeRequest {
                sheet: None,
                reference: reference.to_string(),
                pin: "1".to_string(),
                pin_type: Some(PinType::Output),
            },
        )
        .expect("type pin 1");
    }

    let report = export(handle, "kicad_sym", &path).expect("export kicad_sym");
    assert_eq!(report["format"], "kicad_sym");
    assert_eq!(report["artifact"], json!({ "kind": "kicad_sym", "symbols": 1, "pins": 2, "components": 2 }));
    assert_eq!(report["bytes"], fs::metadata(&path).expect("metadata").len() as usize);

    let text = fs::read_to_string(&path).expect("read kicad_sym");
    assert!(text.starts_with("(kicad_symbol_lib (version 20231120) (generator \"open-eda\")\n"));
    assert!(text.trim_end().ends_with(')'));
    assert!(text.contains("(symbol \"open-eda_2pin_"));
    assert!(text.contains("(pin output line"));
    assert!(text.contains("(number \"2\""));
    assert!(!text.contains("location:"), "the writer must not invent a body");

    let imported = import_kicad_symbol(&text).expect("import symbol library");
    assert_eq!(imported["losses"], json!([]));
    assert_eq!(imported["totals"], json!({ "symbols": 1, "pins": 2 }));
    let symbol = &imported["symbols"][0];
    assert_eq!(symbol["unitCount"], 1);
    assert_eq!(symbol["pins"][0]["number"], "1");
    assert_eq!(symbol["pins"][0]["type"], "output");
    assert_eq!(symbol["pins"][1]["type"], "passive");
    close(handle).expect("close");
}

#[test]
fn kicad_import_reports_unsupported_constructs_instead_of_dropping_them() {
    let text = "\
(kicad_sch (version 20211123) (generator \"open-eda\") (uuid \"u\") (paper \"A4\")
  (lib_symbols)
  (junction (at 1 2) (diameter 0) (uuid \"j\"))
  (bus (width 4) (at 0 0))
  (text \"note\" (at 3 4 0) (effects (font (size 1.27 1.27))) (uuid \"t\"))
  (wire (pts (xy 0 0) (xy 5 0)) (stroke (width 0) (type default)) (uuid \"w\"))
)
";
    let imported = import_kicad_sch(text).expect("import");
    let losses = imported["losses"].as_array().expect("losses");
    assert_eq!(losses.len(), 1);
    for entry in losses {
        assert_eq!(entry["code"], "kicad/unsupported-element");
        assert!(!entry["detail"].as_str().expect("detail").is_empty());
        assert!(entry["detail"].as_str().expect("detail").contains("bus"));
    }
    let sheet = &imported["design"]["sheets"][0];
    assert_eq!(sheet["junctions"], json!([{ "x": 1.0, "y": 2.0 }]));
    assert_eq!(sheet["texts"], json!([{ "text": "note", "x": 3.0, "y": 4.0 }]));
    assert_eq!(sheet["wires"].as_array().expect("wires").len(), 1);
    assert_eq!(imported["design"]["next_wire"], 2);
}

#[test]
fn kicad_import_reads_global_and_hierarchical_labels_and_no_connect_flags() {
    let text = "\
(kicad_sch (version 20211123) (generator \"open-eda\") (paper \"A4\")
  (lib_symbols)
  (symbol (lib_id \"Device:R\") (at 10 20 0)
    (property \"Reference\" \"R1\" (at 10 18 0) (effects (font (size 1.27 1.27))))
    (property \"Value\" \"10k\" (at 10 22 0) (effects (font (size 1.27 1.27))))
    (pin \"1\" (uuid \"p1\"))
    (pin \"2\" (uuid \"p2\")))
  (global_label \"VCC\" (shape input) (at 10 20 0) (effects (font (size 1.27 1.27))))
  (hierarchical_label \"SIG\" (shape input) (at 30 40 0) (effects (font (size 1.27 1.27))))
  (no_connect (at 50 60) (uuid \"n\"))
)
";
    let imported = import_kicad_sch(text).expect("import");
    let losses = imported["losses"].as_array().expect("losses");
    assert_eq!(losses.len(), 2);
    for entry in losses {
        assert_eq!(entry["code"], "kicad/label-scope");
        let detail = entry["detail"].as_str().expect("detail");
        assert!(detail.contains("VCC") || detail.contains("SIG"));
        assert!(detail.contains("sheet-local"));
    }

    let sheet = &imported["design"]["sheets"][0];
    assert_eq!(sheet["no_connects"], json!([{ "x": 50.0, "y": 60.0 }]));
    assert_eq!(
        sheet["labels"],
        json!([
            { "id": "label-1", "text": "VCC", "x": 10.0, "y": 20.0, "scope": "global", "rotation": 0.0, "justify": "left" },
            { "id": "label-2", "text": "SIG", "x": 30.0, "y": 40.0, "scope": "hierarchical", "rotation": 0.0, "justify": "left" },
        ])
    );
    let nets = sheet["nets"].as_array().expect("nets");
    let names: Vec<&str> = nets.iter().filter_map(|net| net["name"].as_str()).collect();
    assert_eq!(names, vec!["VCC", "SIG"]);
    assert_eq!(nets[0]["nodes"], json!([{ "ref": "R1", "pin": "1" }]));
    assert_eq!(nets[1]["nodes"], json!([]));

    let design: Design = serde_json::from_value(imported["design"].clone()).expect("design");
    let rendered = kicad::render(&design);
    assert!(rendered.contains("(no_connect (at 50 60)"));
    assert!(rendered.contains("(global_label \"VCC\" (shape passive)"));
    assert!(rendered.contains("(hierarchical_label \"SIG\" (shape passive)"));
    assert!(!rendered.contains("  (label \"VCC\""));

    let again = import_kicad_sch(&rendered).expect("reimport");
    assert_eq!(again["losses"].as_array().expect("losses").len(), 2);
    assert_eq!(again["design"]["sheets"][0]["no_connects"], json!([{ "x": 50.0, "y": 60.0 }]));
    assert_eq!(again["design"]["sheets"][0]["labels"][0]["scope"], "global");
    assert_eq!(again["design"]["sheets"][0]["labels"][1]["scope"], "hierarchical");
}

#[test]
fn kicad_import_reads_pin_names_from_the_library_and_keeps_rotation() {
    let text = "\
(kicad_sch (version 20211123) (generator \"open-eda\") (paper \"A4\")
  (lib_symbols
    (symbol \"Device:R\" (pin_numbers hide)
      (symbol \"Device:R_1_1\"
        (pin passive line (at 0 0 0) (length 2.54)
          (name \"~\" (effects (font (size 1.27 1.27))))
          (number \"1\" (effects (font (size 1.27 1.27)))))
        (pin passive line (at 0 0 0) (length 2.54)
          (name \"~\" (effects (font (size 1.27 1.27))))
          (number \"2\" (effects (font (size 1.27 1.27))))))))
  (symbol (lib_id \"Device:R\") (at 12.34567 20 90) (unit 1) (uuid \"s\")
    (property \"Reference\" \"R7\" (at 0 0 0) (effects (font (size 1.27 1.27))))
    (property \"Value\" \"1k\" (at 0 0 0) (effects (font (size 1.27 1.27)))))
)
";
    let imported = import_kicad_sch(text).expect("import");
    assert_eq!(imported["losses"], json!([]));
    let component = &imported["design"]["sheets"][0]["components"][0];
    assert_eq!(component["ref"], "R7");
    assert_eq!(component["value"], "1k");
    assert_eq!(component["x"], 12.34567);
    assert_eq!(component["rotation"], 90.0);
    assert_eq!(component["pins"], json!(["1", "2"]));
    assert_eq!(imported["design"]["next_component"], 2);
}

#[test]
fn kicad_import_rejects_documents_outside_the_supported_grammar() {
    assert_eq!(import_kicad_sch("").unwrap_err().code, "format/invalid");
    assert_eq!(import_kicad_sch("   \n\t ").unwrap_err().code, "format/invalid");
    assert_eq!(import_kicad_sch("(not-a-schematic)").unwrap_err().code, "format/invalid");
    assert_eq!(import_kicad_sch("(kicad_sch (version 20211123)").unwrap_err().code, "format/invalid");
    assert_eq!(import_kicad_sch("(kicad_sch))").unwrap_err().code, "format/invalid");
    assert_eq!(import_kicad_sch("not an s-expression").unwrap_err().code, "format/invalid");
}

#[test]
fn kicad_symbol_library_import_reports_units_and_pins() {
    let text = "\
(kicad_symbol_lib (version 20211014) (generator kicad_symbol_editor)
  (symbol \"74HC00\"
    (property \"Reference\" \"U\" (at 0 0 0))
    (symbol \"74HC00_0_1\"
      (pin power_in line (at 0 0 0) (length 2.54)
        (name \"VCC\" (effects (font (size 1.27 1.27))))
        (number \"14\" (effects (font (size 1.27 1.27))))))
    (symbol \"74HC00_1_1\"
      (pin input line (at 0 0 0) (length 2.54)
        (name \"A\" (effects (font (size 1.27 1.27))))
        (number \"1\" (effects (font (size 1.27 1.27))))))
    (symbol \"74HC00_2_1\"
      (pin input line (at 0 0 0) (length 2.54)
        (name \"B\" (effects (font (size 1.27 1.27))))
        (number \"2\" (effects (font (size 1.27 1.27)))))))
  (image (at 0 0))
)
";
    let report = import_kicad_symbol(text).expect("import kicad_sym");
    assert_eq!(report["kind"], "kicad_sym");
    assert_eq!(report["totals"], json!({ "symbols": 1, "pins": 3 }));
    let symbol = &report["symbols"][0];
    assert_eq!(symbol["name"], "74HC00");
    assert_eq!(symbol["unitCount"], 2);
    let pins = symbol["pins"].as_array().expect("pins");
    assert_eq!(pins.len(), 3);
    assert_eq!(pins[0], json!({ "number": "14", "name": "VCC", "type": "power_in", "unit": 1 }));
    assert_eq!(pins[1], json!({ "number": "1", "name": "A", "type": "input", "unit": 1 }));
    assert_eq!(pins[2], json!({ "number": "2", "name": "B", "type": "input", "unit": 2 }));
    let losses = report["losses"].as_array().expect("losses");
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0]["code"], "kicad_sym/unsupported-element");
    assert!(losses[0]["detail"].as_str().expect("detail").contains("image"));
}

#[test]
fn kicad_symbol_library_rejects_other_documents() {
    assert_eq!(import_kicad_symbol("").unwrap_err().code, "format/invalid");
    assert_eq!(import_kicad_symbol("(kicad_sch (version 1))").unwrap_err().code, "format/invalid");
    let report = import_kicad_symbol("(kicad_symbol_lib (version 20211014))").expect("empty library");
    assert_eq!(report["totals"], json!({ "symbols": 0, "pins": 0 }));
    assert_eq!(report["losses"], json!([]));
}

#[test]
fn kicad_symbol_library_check_reports_duplicates_and_nameless_pins() {
    let text = "\
(kicad_symbol_lib (version 20211014) (generator kicad_symbol_editor)
  (symbol \"AMP\"
    (symbol \"AMP_1_1\"
      (pin input line (at 0 0 0) (length 2.54)
        (name \"+\" (effects (font (size 1.27 1.27))))
        (number \"1\" (effects (font (size 1.27 1.27)))))
      (pin input line (at 0 0 0) (length 2.54)
        (name \"-\" (effects (font (size 1.27 1.27))))
        (number \"1\" (effects (font (size 1.27 1.27)))))
      (pin passive line (at 0 0 0) (length 2.54)
        (name \"\" (effects (font (size 1.27 1.27))))
        (number \"3\" (effects (font (size 1.27 1.27)))))))
  (symbol \"AMP\"
    (symbol \"AMP_1_1\"
      (pin input line (at 0 0 0) (length 2.54)
        (name \"A\" (effects (font (size 1.27 1.27))))
        (number \"1\" (effects (font (size 1.27 1.27)))))))
  (symbol \"EMPTY\")
)
";
    let report = check_kicad_symbol(text).expect("check kicad_sym");
    assert_eq!(report["format"], "open-eda-library-check");
    assert_eq!(report["version"], 1);
    assert_eq!(report["symbols"], 3);
    assert_eq!(report["pins"], 4);
    assert_eq!(report["passed"], false);
    let findings = report["findings"].as_array().expect("findings");
    let codes = findings.iter().map(|finding| finding["code"].as_str().unwrap_or("")).collect::<Vec<_>>();
    assert_eq!(
        codes,
        vec![
            "library/duplicate-pin-number",
            "library/pin-without-name",
            "library/duplicate-symbol",
            "library/symbol-without-pins",
        ]
    );
    assert_eq!(findings[0]["severity"], "error");
    assert_eq!(findings[0]["symbol"], "AMP");
    assert_eq!(findings[0]["pin"], "1");
    assert_eq!(findings[0]["unit"], 1);
    assert_eq!(findings[1]["severity"], "warning");
    assert_eq!(findings[1]["pin"], "3");
    assert_eq!(findings[2]["pin"], Value::Null);
    assert_eq!(findings[3]["symbol"], "EMPTY");

    let clean = "\
(kicad_symbol_lib (version 20211014) (generator kicad_symbol_editor)
  (symbol \"R\"
    (symbol \"R_1_1\"
      (pin passive line (at 0 0 0) (length 2.54)
        (name \"~\" (effects (font (size 1.27 1.27))))
        (number \"1\" (effects (font (size 1.27 1.27)))))
      (pin passive line (at 0 0 0) (length 2.54)
        (name \"~\" (effects (font (size 1.27 1.27))))
        (number \"2\" (effects (font (size 1.27 1.27)))))))
)
";
    let passed = check_kicad_symbol(clean).expect("check clean library");
    assert_eq!(passed["passed"], true);
    assert_eq!(passed["findings"], json!([]));
    assert_eq!(check_kicad_symbol("(kicad_sch (version 1))").unwrap_err().code, "format/invalid");
}

fn wire(handle: u64, net: Option<&str>, from: [f64; 2], to: [f64; 2]) {
    add_wire(handle, AddWireRequest { sheet: None, net: net.map(str::to_string), from, to }).expect("add wire");
}

#[test]
fn derive_nets_reuses_labels_and_allocates_collision_free_names() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    wire(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);
    wire(handle, None, [10.0, 5.0], [20.0, 5.0]);
    wire(handle, None, [20.0, 5.0], [30.0, 5.0]);

    let derived = derive_nets(handle, None).expect("derive");
    assert_eq!(derived["groups"], 2);
    assert_eq!(derived["assigned"], 1);
    assert_eq!(derived["created"], 2);

    let listing = list(handle).expect("list");
    let nets: Vec<&str> = listing["nets"].as_array().expect("nets").iter().filter_map(|net| net["name"].as_str()).collect();
    assert_eq!(nets, vec!["VCC", "N$1"]);
    let wires = listing["wires"].as_array().expect("wires");
    assert_eq!(wires[0]["net"], "VCC");
    assert_eq!(wires[1]["net"], "N$1");
    assert_eq!(wires[2]["net"], "N$1");

    let revision = derived["revision"].as_u64().expect("revision");
    let again = derive_nets(handle, None).expect("derive again");
    assert_eq!(again["revision"], revision + 1);
    assert_eq!(again["assigned"], 0);
    assert_eq!(again["created"], 0);
    assert_eq!(again["groups"], 2);
    close(handle).expect("close");
}

#[test]
fn connectivity_check_reports_conflicts_unassigned_groups_and_floating_junctions() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    wire(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);
    wire(handle, Some("GND"), [10.0, 0.0], [20.0, 0.0]);
    wire(handle, None, [0.0, 50.0], [10.0, 50.0]);
    add_junction(handle, AddJunctionRequest { sheet: None, x: 100.0, y: 100.0 }).expect("junction");

    let report = inspect(handle, Some(vec!["connectivity".to_string()])).expect("inspect");
    let codes: Vec<&str> =
        report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert!(codes.contains(&"wire-net-conflict"));
    assert!(codes.contains(&"wire-net-unassigned"));
    assert!(codes.contains(&"junction-floating"));
    let conflict = report["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .find(|issue| issue["code"] == "wire-net-conflict")
        .expect("conflict");
    assert_eq!(conflict["severity"], "error");
    assert!(conflict["message"].as_str().expect("message").contains("VCC"));
    assert!(conflict["message"].as_str().expect("message").contains("GND"));
    close(handle).expect("close");
}

#[test]
fn connectivity_report_lists_pins_and_counts_open_ends() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, pins) in [("R1", vec!["1", "2"]), ("R2", vec!["1"])] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: pins.into_iter().map(str::to_string).collect(),
            },
        )
        .expect("add component");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "R2.1".to_string()] },
    )
    .expect("connect VCC");
    wire(handle, None, [0.0, 0.0], [10.0, 0.0]);
    add_junction(handle, AddJunctionRequest { sheet: None, x: 100.0, y: 100.0 }).expect("junction");

    let report = connectivity_report(handle).expect("connectivity report");
    assert_eq!(report["format"], "open-eda-connectivity");
    assert_eq!(report["version"], 1);
    assert_eq!(report["revision"], summary(handle).expect("summary")["revision"]);
    assert_eq!(report["totals"]["nets"], 1);
    assert_eq!(report["totals"]["nodes"], 2);
    assert_eq!(report["totals"]["singleNodeNets"], 0);
    assert_eq!(report["totals"]["danglingNets"], 0);
    assert_eq!(report["totals"]["unconnectedPins"], 1);
    assert_eq!(report["totals"]["wireGroups"], 1);
    assert_eq!(report["totals"]["unassignedWireGroups"], 1);
    assert_eq!(report["totals"]["conflictingWireGroups"], 0);
    assert_eq!(report["totals"]["floatingJunctions"], 1);

    let sheet = &report["sheets"][0];
    assert_eq!(sheet["id"], "sheet-1");
    assert_eq!(sheet["nets"][0]["name"], "VCC");
    assert_eq!(sheet["nets"][0]["nodes"], 2);
    assert_eq!(sheet["nets"][0]["pins"], json!(["R1.1", "R2.1"]));
    assert_eq!(sheet["nets"][0]["singleNode"], false);
    assert_eq!(sheet["nets"][0]["dangling"], false);
    assert_eq!(sheet["unconnectedPins"], json!([{ "ref": "R1", "pin": "2" }]));
    assert_eq!(sheet["wires"], json!({ "segments": 1, "groups": 1, "unassigned": 1, "conflicts": 0 }));
    assert_eq!(sheet["junctions"], json!({ "total": 1, "floating": 1 }));
    close(handle).expect("close");
}

#[test]
fn connectivity_report_flags_single_node_and_dangling_nets_without_mutating() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 4,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "U1".to_string(),
                value: "buffer".to_string(),
                footprint: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: vec![
                Net {
                    id: "net-1".to_string(),
                    name: "SIG".to_string(),
                    nodes: vec![Node { reference: "U1".to_string(), pin: "1".to_string() }],
                },
                Net { id: "net-2".to_string(), name: "SPARE".to_string(), nodes: Vec::new() },
            ],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 3,
        next_wire: 1,
    };

    let report = connectivity_report_of(&design);
    assert_eq!(report["totals"]["nets"], 2);
    assert_eq!(report["totals"]["singleNodeNets"], 1);
    assert_eq!(report["totals"]["danglingNets"], 1);
    assert_eq!(report["totals"]["unconnectedPins"], 1);
    let nets = report["sheets"][0]["nets"].as_array().expect("nets");
    assert_eq!(nets[1]["name"], "SPARE");
    assert_eq!(nets[1]["pins"], json!([]));
    assert_eq!(nets[1]["dangling"], true);
    assert_eq!(design.revision, 4, "the report never mutates the design");
}

#[test]
fn component_report_lists_pins_nets_properties_and_no_connect() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "U1".to_string(),
            value: "buffer".to_string(),
            x: 12.5,
            y: -3.0,
            rotation: 90.0,
            pins: vec!["1".to_string(), "2".to_string(), "3".to_string()],
        },
    )
    .expect("add U1");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["U1.1".to_string()] })
        .expect("connect VCC");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIG".to_string(), nodes: vec!["U1.2".to_string()] })
        .expect("connect SIG");
    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 0.0,
            y: 0.0,
            node: Some(Node { reference: "U1".to_string(), pin: "3".to_string() }),
        },
    )
    .expect("mark pin 3 open");
    for (key, value) in [("MPN", "BUF42"), ("Datasheet", "DS-1")] {
        set_component_property(
            handle,
            SetComponentPropertyRequest {
                sheet: None,
                reference: "U1".to_string(),
                key: key.to_string(),
                value: value.to_string(),
            },
        )
        .expect("set property");
    }

    let report = component_report(handle, "U1").expect("component report");
    assert_eq!(report["format"], "open-eda-component");
    assert_eq!(report["version"], 1);
    assert_eq!(report["sheet"], json!({ "id": "sheet-1", "name": "Sheet 1" }));
    assert_eq!(
        report["component"],
        json!({
            "id": "component-1",
            "reference": "U1",
            "value": "buffer",
            "footprint": "",
            "x": 12.5,
            "y": -3.0,
            "rotation": 90.0,
            "properties": [
                { "key": "Datasheet", "value": "DS-1" },
                { "key": "MPN", "value": "BUF42" },
            ],
        })
    );
    assert_eq!(
        report["pins"],
        json!([
            { "name": "1", "nets": ["VCC"], "noConnect": false },
            { "name": "2", "nets": ["SIG"], "noConnect": false },
            { "name": "3", "nets": [], "noConnect": true },
        ])
    );

    let revision = summary(handle).expect("summary")["revision"].clone();
    let again = component_report(handle, "U1").expect("component report");
    assert_eq!(again, report, "the report is deterministic for one document state");
    assert_eq!(summary(handle).expect("summary")["revision"], revision, "the report never mutates the design");
    close(handle).expect("close");
}

#[test]
fn component_report_reports_unknown_references_as_object_not_found() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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

    let revision = summary(handle).expect("summary")["revision"].clone();
    let error = component_report(handle, "R9").expect_err("unknown reference");
    assert_eq!(error.code, "object/not-found");
    assert!(error.message.contains("R9"));
    assert_eq!(summary(handle).expect("summary")["revision"], revision, "a failed lookup adds no revision");
    close(handle).expect("close");
}

#[test]
fn net_show_reports_nodes_wires_labels_and_design_matches() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    add_sheet(handle, Some("Power")).expect("add sheet");
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("sheet-2".to_string()),
            reference: "R2".to_string(),
            value: "10k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R2");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("VCC on sheet 1");
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("sheet-2".to_string()), net: "VCC".to_string(), nodes: vec!["R2.1".to_string()] },
    )
    .expect("VCC on sheet 2");
    wire(handle, Some("GND"), [0.0, 0.0], [1.0, 0.0]);
    wire(handle, Some("VCC"), [2.0, 0.0], [3.0, 0.0]);
    add_wire(
        handle,
        AddWireRequest { sheet: Some("sheet-2".to_string()), net: Some("VCC".to_string()), from: [4.0, 0.0], to: [5.0, 0.0] },
    )
    .expect("VCC wire on sheet 2");
    for (text, x, y, scope) in [("GND", 0.0, 0.0, LabelScope::Local), ("VCC", 1.0, 2.0, LabelScope::Global)] {
        add_label(
            handle,
            AddLabelRequest { sheet: None, text: text.to_string(), x, y, scope: Some(scope), rotation: None, justify: None },
        )
        .expect("label");
    }
    add_label(
        handle,
        AddLabelRequest {
            sheet: Some("sheet-2".to_string()),
            text: "VCC".to_string(),
            x: 3.0,
            y: 4.0,
            scope: Some(LabelScope::Hierarchical),
            rotation: None,
            justify: None,
        },
    )
    .expect("label on sheet 2");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: vec!["VCC".to_string()] }).expect("add bus");
    add_net_class(
        handle,
        NetClassRequest {
            name: "Power".to_string(),
            width: 0.25,
            clearance: 0.1,
            via: 0.2,
            nets: vec!["VCC".to_string()],
            buses: Vec::new(),
        },
    )
    .expect("add net class");
    let revision = summary(handle).expect("summary")["revision"].clone();

    let report = net_show(handle, "  VCC  ", None).expect("net report");
    assert_eq!(report["format"], "open-eda-net");
    assert_eq!(report["version"], 1);
    assert_eq!(report["name"], "VCC", "the requested name is trimmed");
    assert_eq!(report["revision"], revision);
    assert_eq!(report["design"]["id"], created["design"]["id"]);
    assert_eq!(
        report["sheets"],
        json!([
            {
                "sheet": { "id": "sheet-1", "name": "Sheet 1" },
                "net": { "id": "net-1", "name": "VCC", "nodes": [{ "ref": "R1", "pin": "1" }] },
                "wires": [{ "index": 1, "from": [2.0, 0.0], "to": [3.0, 0.0] }],
                "labels": [{ "text": "VCC", "scope": "global", "x": 1.0, "y": 2.0 }],
            },
            {
                "sheet": { "id": "sheet-2", "name": "Power" },
                "net": { "id": "net-2", "name": "VCC", "nodes": [{ "ref": "R2", "pin": "1" }] },
                "wires": [{ "index": 0, "from": [4.0, 0.0], "to": [5.0, 0.0] }],
                "labels": [{ "text": "VCC", "scope": "hierarchical", "x": 3.0, "y": 4.0 }],
            },
        ])
    );
    assert_eq!(report["nodeCount"], 2, "the node count aggregates every matching sheet");
    assert_eq!(report["bus"], json!({ "id": "bus-1", "name": "DATA", "width": 2, "members": ["DATA0", "DATA1"] }));
    assert_eq!(
        report["netClass"],
        json!({ "id": "net-class-1", "name": "Power", "width": 0.25, "clearance": 0.1, "via": 0.2 })
    );

    let again = net_show(handle, "VCC", None).expect("net report again");
    assert_eq!(again, report, "the report is deterministic for one document state");
    assert_eq!(summary(handle).expect("summary")["revision"], revision, "the report never mutates the design");
    close(handle).expect("close");
}

#[test]
fn net_show_scopes_to_one_sheet_and_reports_unknown_names() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_sheet(handle, Some("Power")).expect("add sheet");
    for (sheet, reference, pin, net) in [(None, "R1", "1", "VCC"), (Some("sheet-2".to_string()), "R2", "1", "VCC")] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: sheet.clone(),
                reference: reference.to_string(),
                value: "10k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
        connect_net(
            handle,
            ConnectNetRequest { sheet: sheet.clone(), net: net.to_string(), nodes: vec![format!("{reference}.{pin}")] },
        )
        .expect("connect");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("sheet-2".to_string()), net: "GND".to_string(), nodes: vec!["R2.2".to_string()] },
    )
    .expect("GND only on sheet 2");

    let all = net_show(handle, "VCC", None).expect("design-wide");
    let ids: Vec<&str> =
        all["sheets"].as_array().expect("sheets").iter().filter_map(|entry| entry["sheet"]["id"].as_str()).collect();
    assert_eq!(ids, ["sheet-1", "sheet-2"], "matches keep document order");

    let by_name = net_show(handle, "VCC", Some("Power")).expect("sheet by name");
    assert_eq!(by_name["sheets"].as_array().expect("sheets").len(), 1);
    assert_eq!(by_name["sheets"][0]["sheet"]["id"], "sheet-2");
    assert_eq!(by_name["bus"], Value::Null, "no bus matches this net");
    assert_eq!(by_name["netClass"], Value::Null, "no net class matches this net");

    let by_id = net_show(handle, "VCC", Some("sheet-1")).expect("sheet by id");
    assert_eq!(by_id["sheets"].as_array().expect("sheets").len(), 1);
    assert_eq!(by_id["sheets"][0]["sheet"]["id"], "sheet-1");

    let revision = summary(handle).expect("summary")["revision"].clone();
    assert_eq!(net_show(handle, "VCC", Some("Missing")).unwrap_err().code, "sheet/not-found");
    let missing = net_show(handle, "NOPE", None).expect_err("unknown net");
    assert_eq!(missing.code, "net/not-found");
    assert!(missing.message.contains("NOPE"));
    assert_eq!(net_show(handle, "GND", Some("sheet-1")).expect_err("net on another sheet").code, "net/not-found");
    assert_eq!(summary(handle).expect("summary")["revision"], revision, "failed lookups add no revision");
    close(handle).expect("close");
}

#[test]
fn net_show_rejects_an_empty_name() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    let revision = summary(handle).expect("summary")["revision"].clone();

    assert_eq!(net_show(handle, "   ", None).unwrap_err().code, "session/argument-invalid");
    assert_eq!(net_show(handle, "", Some("sheet-1")).unwrap_err().code, "session/argument-invalid");
    assert_eq!(summary(handle).expect("summary")["revision"], revision, "a rejected name adds no revision");
    close(handle).expect("close");
}

#[test]
fn power_check_flags_underconnected_and_duplicated_rails() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    .expect("R1");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("VCC");

    add_sheet(handle, Some("Power")).expect("sheet");
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("sheet-2".to_string()),
            reference: "R2".to_string(),
            value: "1k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("R2");
    connect_net(
        handle,
        ConnectNetRequest {
            sheet: Some("sheet-2".to_string()),
            net: "VCC".to_string(),
            nodes: vec!["R2.1".to_string(), "R2.2".to_string()],
        },
    )
    .expect("VCC on sheet 2");

    let report = inspect(handle, Some(vec!["power".to_string()])).expect("inspect");
    let codes: Vec<&str> =
        report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert!(codes.contains(&"power-net-underconnected"));
    assert!(codes.contains(&"power-net-duplicate"));
    assert_eq!(codes.iter().filter(|code| **code == "power-net-duplicate").count(), 1);
    close(handle).expect("close");
}

#[test]
fn net_csv_export_writes_one_row_per_node_and_expands_buses() {
    let dir = TempDir::new("net-csv");
    let path = dir.join("blinker.csv");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    rename_sheet(handle, "sheet-1", "Main, A").expect("rename sheet");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add bus");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "DATA".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect DATA");

    let report = export(handle, "net_csv", &path).expect("export net_csv");
    assert_eq!(report["format"], "net_csv");
    assert_eq!(report["artifact"]["kind"], "net_csv");
    assert_eq!(report["artifact"]["nets"], 3);
    assert_eq!(report["artifact"]["rows"], 6);
    assert_eq!(report["bytes"], fs::metadata(&path).expect("metadata").len() as usize);
    assert_eq!(
        fs::read_to_string(&path).expect("read net csv"),
        "Sheet,Net,Ref,Pin\n\
             \"Main, A\",VCC,R1,1\n\
             \"Main, A\",VCC,C1,1\n\
             \"Main, A\",GND,R1,2\n\
             \"Main, A\",GND,C1,2\n\
             \"Main, A\",DATA0,R1,1\n\
             \"Main, A\",DATA1,R1,1\n"
    );
    close(handle).expect("close");
}

#[test]
fn hierarchical_netlist_prefixes_nets_with_the_sheet_path() {
    let dir = TempDir::new("net-hier");
    let first = dir.join("hier.net");
    let second = dir.join("hier-again.net");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    rename_sheet(handle, "sheet-1", "Main").expect("rename sheet");
    add_sheet(handle, Some("Power Supply")).expect("add sheet");
    add_component(
        handle,
        AddComponentRequest {
            sheet: Some("Power Supply".to_string()),
            reference: "R2".to_string(),
            value: "1k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R2");
    connect_net(
        handle,
        ConnectNetRequest { sheet: Some("Power Supply".to_string()), net: "VCC".to_string(), nodes: vec!["R2.1".to_string()] },
    )
    .expect("connect Power VCC");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add bus");

    let report = export(handle, "net_hier", &first).expect("export net_hier");
    assert_eq!(report["format"], "net_hier");
    assert_eq!(report["artifact"]["kind"], "net_hier");
    assert_eq!(report["artifact"]["sheets"], 2);
    assert_eq!(report["artifact"]["buses"], 1);

    let text = fs::read_to_string(&first).expect("read hierarchical netlist");
    assert!(text.starts_with("# OpenEDA hierarchical netlist v1\n"));
    assert!(text.contains("BUS DATA 2\n"));
    assert!(text.contains("SHEET Main\n"));
    assert!(text.contains("NET Main/VCC\n"));
    assert!(text.contains("NODE R1.1\n"));
    assert!(text.contains("SHEET Power_Supply\n"));
    assert!(text.contains("NET Power_Supply/VCC\n"));
    assert!(text.contains("NODE R2.1\n"));

    export(handle, "net_hier", &second).expect("export again");
    assert_eq!(text, fs::read_to_string(&second).expect("read second"), "export must be byte-stable");
    close(handle).expect("close");
}

#[test]
fn report_export_renders_object_and_net_tables() {
    let dir = TempDir::new("report");
    let path = dir.join("report.md");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    .expect("R1");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "C1".to_string(),
            value: "100n".to_string(),
            x: 10.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("C1");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("VCC");
    wire(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);

    let exported = export(handle, "report", &path).expect("export");
    assert_eq!(exported["artifact"]["kind"], "report");
    assert!(exported["artifact"]["sections"].as_u64().expect("sections") >= 3);
    let text = fs::read_to_string(&path).expect("read");
    assert!(text.contains("| Net | Nodes | Pins |"));
    assert!(text.contains("| VCC | 2 | R1.1, C1.1 |"));
    assert!(text.contains("## Objects"));
    assert!(text.contains("## Inspection"));
    close(handle).expect("close");
}

#[test]
fn json_export_projects_stable_document_sections() {
    let dir = TempDir::new("json-projection");
    let first = dir.join("blinker.json");
    let second = dir.join("blinker-again.json");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    blinker(handle);
    add_junction(handle, AddJunctionRequest { sheet: None, x: 55.0, y: 30.0 }).expect("junction");
    add_text(handle, AddTextRequest { sheet: None, text: "Revision A".to_string(), x: 20.0, y: 12.0 }).expect("text");

    let report = export(handle, "json", &first).expect("export json");
    assert_eq!(report["format"], "json");
    assert_eq!(report["editable"], false);
    assert_eq!(report["artifact"]["kind"], "json");
    assert_eq!(report["artifact"]["sheets"], 1);
    assert_eq!(report["artifact"]["components"], 2);
    assert_eq!(report["artifact"]["nets"], 2);
    assert_eq!(report["artifact"]["wires"], 1);
    assert_eq!(report["artifact"]["junctions"], 1);
    assert_eq!(report["artifact"]["noConnects"], 0);
    assert_eq!(report["artifact"]["texts"], 1);
    assert_eq!(report["bytes"], fs::metadata(&first).expect("metadata").len() as usize);

    let text = fs::read_to_string(&first).expect("read json");
    assert!(text.starts_with("{\n"));
    assert!(text.ends_with('\n'));
    assert!(!text.contains("next_component"), "internal counters stay out of the projection");
    let projection: Value = serde_json::from_str(&text).expect("projection is valid JSON");
    assert_eq!(projection["format"], "open-eda-projection");
    assert_eq!(projection["version"], 1);
    assert_eq!(projection["design"], "design-1");
    assert_eq!(projection["title"], "Blinker");
    assert_eq!(projection["revision"], report["revision"]);
    assert_eq!(projection["units"], "mm");

    let sheets = projection["sheets"].as_array().expect("sheets");
    assert_eq!(sheets.len(), 1);
    let sheet = &sheets[0];
    assert_eq!(sheet["id"], "sheet-1");
    assert_eq!(sheet["name"], "Sheet 1");
    assert_eq!(sheet["components"][0]["ref"], "R1");
    assert_eq!(sheet["components"][0]["value"], "10k");
    assert_eq!(sheet["components"][0]["footprint"], "");
    assert_eq!(sheet["components"][0]["x"], 40.0);
    assert_eq!(sheet["components"][0]["pins"], json!(["1", "2"]));
    assert_eq!(sheet["nets"][0]["name"], "VCC");
    assert_eq!(sheet["nets"][0]["nodes"][1], json!({ "ref": "C1", "pin": "1" }));
    assert_eq!(sheet["wires"][0]["net"], "VCC");
    assert_eq!(sheet["wires"][0]["from"], json!([40.0, 30.0]));
    assert_eq!(sheet["wires"][0]["to"], json!([70.0, 30.0]));
    assert_eq!(sheet["junctions"][0], json!({ "x": 55.0, "y": 30.0 }));
    assert_eq!(sheet["texts"][0], json!({ "text": "Revision A", "x": 20.0, "y": 12.0 }));

    export(handle, "json", &second).expect("export json again");
    assert_eq!(text, fs::read_to_string(&second).expect("read second json"), "export must be byte-stable");
    close(handle).expect("close");
}

#[test]
fn junction_and_text_index_edits_are_validated() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_junction(handle, AddJunctionRequest { sheet: None, x: 1.0, y: 2.0 }).expect("junction");
    add_text(handle, AddTextRequest { sheet: None, text: "note".to_string(), x: 3.0, y: 4.0 }).expect("text");

    let moved = move_junction(handle, 0, None, 5.0, 6.0).expect("move junction");
    assert_eq!(moved["junction"]["x"], 5.0);
    assert_eq!(moved["junction"]["y"], 6.0);
    let moved = move_text(handle, 0, None, 7.0, 8.0).expect("move text");
    assert_eq!(moved["text"]["x"], 7.0);
    assert_eq!(moved["text"]["y"], 8.0);

    let deleted = delete_text(handle, 0, None).expect("delete text");
    assert_eq!(deleted["remaining"], 0);
    delete_junction(handle, 0, None).expect("delete junction");
    assert_eq!(move_junction(handle, 0, None, 0.0, 0.0).unwrap_err().code, "junction/not-found");
    assert_eq!(delete_text(handle, 0, None).unwrap_err().code, "object/not-found");
    assert_eq!(move_text(handle, 0, None, f64::NAN, 0.0).unwrap_err().code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn disconnect_and_delete_net_keep_wires_and_report_counts() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    .expect("R1");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "C1".to_string(),
            value: "100n".to_string(),
            x: 10.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("C1");
    connect_net(
        handle,
        ConnectNetRequest {
            sheet: None,
            net: "VCC".to_string(),
            nodes: vec!["R1.1".to_string(), "R1.2".to_string(), "C1.1".to_string()],
        },
    )
    .expect("VCC");
    wire(handle, Some("VCC"), [0.0, 0.0], [10.0, 0.0]);

    let disconnected =
        disconnect_net(handle, DisconnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.2".to_string()] })
            .expect("disconnect");
    assert_eq!(disconnected["remaining"], 2);
    assert_eq!(disconnected["removed"], 1);

    let deleted = delete_net(handle, DeleteNetRequest { sheet: None, net: "VCC".to_string() }).expect("delete");
    assert_eq!(deleted["deleted"], "VCC");
    assert_eq!(deleted["detached"], 1);
    let listing = list(handle).expect("list");
    assert_eq!(listing["nets"].as_array().expect("nets").len(), 0);
    assert_eq!(listing["wires"][0]["net"], Value::Null);
    close(handle).expect("close");
}

#[test]
fn merge_nets_absorbs_nodes_wires_and_class_assignments() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for reference in ["R1", "C1"] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("VCC");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VIN".to_string(), nodes: vec!["C1.1".to_string(), "R1.1".to_string()] },
    )
    .expect("VIN");
    wire(handle, Some("VIN"), [0.0, 0.0], [10.0, 0.0]);
    add_net_class(
        handle,
        NetClassRequest {
            name: "Power".to_string(),
            width: 0.5,
            clearance: 0.0,
            via: 0.0,
            nets: vec!["VIN".to_string()],
            buses: Vec::new(),
        },
    )
    .expect("add net class");

    let merged = merge_nets(handle, MergeNetsRequest { sheet: None, target: "VCC".to_string(), source: "VIN".to_string() })
        .expect("merge");
    assert_eq!(merged["net"]["id"], "net-1");
    assert_eq!(merged["net"]["name"], "VCC");
    assert_eq!(merged["source"], "VIN");
    assert_eq!(merged["movedNodes"], 1);
    assert_eq!(merged["repointedWires"], 1);

    let nets = list_nets(handle, None).expect("list nets");
    assert_eq!(nets["nets"].as_array().expect("nets").len(), 1);
    let nodes: Vec<&str> =
        nets["nets"][0]["nodes"].as_array().expect("nodes").iter().filter_map(|node| node["ref"].as_str()).collect();
    assert_eq!(nodes, ["R1", "C1"]);
    let listed = list_wires(handle, None).expect("list wires");
    assert_eq!(listed["wires"][0]["net"], "VCC");
    let classes = list_net_classes(handle).expect("list classes");
    assert_eq!(classes["netClasses"][0]["nets"], json!(["VCC"]));

    let itself = merge_nets(handle, MergeNetsRequest { sheet: None, target: "VCC".to_string(), source: "VCC".to_string() })
        .expect_err("same net");
    assert_eq!(itself.code, "session/argument-invalid");
    let missing = merge_nets(handle, MergeNetsRequest { sheet: None, target: "VCC".to_string(), source: "GND".to_string() })
        .expect_err("unknown source");
    assert_eq!(missing.code, "object/not-found");
    close(handle).expect("close");
}

#[test]
fn inspection_flags_duplicate_pins_and_dangling_nets() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 2,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "U1".to_string(),
                value: "opamp".to_string(),
                footprint: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "1".to_string(), "2".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: vec![Net { id: "net-1".to_string(), name: "SPARE".to_string(), nodes: Vec::new() }],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 2,
        next_wire: 1,
    };

    let issues = inspect_design(&design, &["erc".to_string()]);
    assert!(issues.iter().any(|issue| issue["code"] == "erc/duplicate-pin"));
    let dangling = issues.iter().find(|issue| issue["code"] == "erc/dangling-net").expect("dangling net finding");
    assert_eq!(dangling["severity"], "warning");
    assert!(dangling["message"].as_str().expect("message").contains("SPARE"));
    assert!(!issues.iter().any(|issue| issue["code"] == "erc/single-node-net"));
}

#[test]
fn inspection_rejects_names_that_break_the_node_and_net_grammar() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 2,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![
                Component {
                    id: "component-1".to_string(),
                    reference: "R 1".to_string(),
                    value: "1k".to_string(),
                    footprint: String::new(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string(), "2".to_string()],
                    pin_types: BTreeMap::new(),
                    properties: BTreeMap::new(),
                },
                Component {
                    id: "component-2".to_string(),
                    reference: "U1".to_string(),
                    value: "mcu".to_string(),
                    footprint: String::new(),
                    x: 10.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1.2".to_string()],
                    pin_types: BTreeMap::new(),
                    properties: BTreeMap::new(),
                },
            ],
            nets: vec![Net {
                id: "net-1".to_string(),
                name: "V CC".to_string(),
                nodes: vec![
                    Node { reference: "R 1".to_string(), pin: "1".to_string() },
                    Node { reference: "R 1".to_string(), pin: "2".to_string() },
                ],
            }],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 3,
        next_net: 2,
        next_wire: 1,
    };

    let issues = inspect_design(&design, &["erc".to_string()]);
    for code in ["erc/reference-illegal", "erc/pin-name-illegal", "erc/net-name-illegal"] {
        let finding = issues.iter().find(|issue| issue["code"] == code).unwrap_or_else(|| panic!("missing {code}"));
        assert_eq!(finding["severity"], "error", "{code} severity");
    }
    assert_eq!(issues.iter().filter(|issue| issue["code"] == "erc/pin-name-illegal").count(), 1);
}

#[test]
fn nets_check_reports_bus_member_collisions() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 2,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "U1".to_string(),
                value: "buffer".to_string(),
                footprint: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: vec![Net {
                id: "net-1".to_string(),
                name: "DATA1".to_string(),
                nodes: vec![
                    Node { reference: "U1".to_string(), pin: "1".to_string() },
                    Node { reference: "U1".to_string(), pin: "2".to_string() },
                ],
            }],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4, aliases: Vec::new() }],
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 2,
        next_wire: 1,
    };

    let issues = inspect_design(&design, &["nets".to_string()]);
    let collision =
        issues.iter().find(|issue| issue["code"] == "net-bus-member-collision").expect("bus member collision finding");
    assert_eq!(collision["severity"], "error");
    assert_eq!(collision["sheetId"], "sheet-1");
    assert_eq!(collision["objectId"], "net-1");
    assert!(collision["message"].as_str().expect("message").contains("DATA"));
}

#[test]
fn nets_check_reports_two_nets_resolving_to_one_bus_through_an_alias() {
    let design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 2,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "U1".to_string(),
                value: "buffer".to_string(),
                footprint: String::new(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: vec![
                Net {
                    id: "net-1".to_string(),
                    name: "DATA".to_string(),
                    nodes: vec![Node { reference: "U1".to_string(), pin: "1".to_string() }],
                },
                Net {
                    id: "net-2".to_string(),
                    name: "D".to_string(),
                    nodes: vec![Node { reference: "U1".to_string(), pin: "2".to_string() }],
                },
            ],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4, aliases: vec!["D".to_string()] }],
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 3,
        next_wire: 1,
    };

    let issues = inspect_design(&design, &["nets".to_string()]);
    let collision =
        issues.iter().find(|issue| issue["code"] == "net-bus-alias-collision").expect("bus alias collision finding");
    assert_eq!(collision["severity"], "error");
    assert_eq!(collision["objectId"], "net-2");
    assert!(collision["message"].as_str().expect("message").contains("net-1"));
}

#[test]
fn buses_are_declared_listed_expanded_and_removed() {
    let dir = TempDir::new("bus");
    let path = dir.join("bus.net");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    .expect("R1");

    let added =
        add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 4, aliases: vec!["D".to_string()] }).expect("add bus");
    assert_eq!(added["bus"], json!({ "id": "bus-1", "name": "DATA", "width": 4, "aliases": ["D"] }));
    assert_eq!(added["revision"], 3);
    assert_eq!(
        add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).unwrap_err().code,
        "object/duplicate"
    );
    assert_eq!(
        add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 2, aliases: vec!["D".to_string()] }).unwrap_err().code,
        "object/duplicate",
        "an alias cannot resolve to two buses"
    );
    assert_eq!(
        add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 2, aliases: vec!["DATA1".to_string()] })
            .unwrap_err()
            .code,
        "object/duplicate",
        "an alias cannot shadow a bus member name"
    );
    assert_eq!(
        add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 0, aliases: Vec::new() }).unwrap_err().code,
        "session/argument-invalid"
    );
    assert_eq!(
        add_bus(handle, AddBusRequest { name: "  ".to_string(), width: 1, aliases: Vec::new() }).unwrap_err().code,
        "session/argument-invalid"
    );
    assert_eq!(
        add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 1, aliases: vec!["A B".to_string()] })
            .unwrap_err()
            .code,
        "session/argument-invalid",
        "an alias must be a legal net name"
    );

    export(handle, "net", &path).expect("export declared bus");
    let text = fs::read_to_string(&path).expect("read netlist");
    assert!(text.contains("BUS DATA 4\n"));
    for index in 0..4 {
        assert!(text.contains(&format!("NET DATA{index}\n\n")), "member {index} is listed without nodes");
    }

    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "D".to_string(), nodes: vec!["R1.1".to_string(), "R1.2".to_string()] },
    )
    .expect("connect alias D");
    export(handle, "net", &path).expect("export expanded bus");
    let text = fs::read_to_string(&path).expect("read expanded netlist");
    for index in 0..4 {
        assert!(
            text.contains(&format!("NET DATA{index}\nNODE R1.1\nNODE R1.2\n\n")),
            "member {index} repeats the parent nodes"
        );
    }
    assert!(!text.contains("NET D0\n"), "members use the canonical bus name, never the alias");

    let listed = list_buses(handle).expect("list buses");
    assert_eq!(listed["buses"].as_array().expect("buses").len(), 1);
    assert_eq!(listed["buses"][0]["name"], "DATA");
    assert_eq!(listed["buses"][0]["aliases"], json!(["D"]));

    let removed = remove_bus(handle, "D").expect("remove by alias");
    assert_eq!(removed["deleted"], "DATA");
    assert_eq!(removed["remaining"], 0);
    assert_eq!(list_buses(handle).expect("list again")["buses"], json!([]));
    assert_eq!(remove_bus(handle, "DATA").unwrap_err().code, "object/not-found");
    close(handle).expect("close");
}

#[test]
fn net_classes_keep_attributes_and_follow_net_and_bus_edits() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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
    connect_net(handle, ConnectNetRequest { sheet: None, net: "GND".to_string(), nodes: vec!["R1.2".to_string()] })
        .expect("connect GND");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 4, aliases: vec!["D".to_string()] }).expect("add bus");

    let added = add_net_class(
        handle,
        NetClassRequest {
            name: "HighSpeed".to_string(),
            width: 0.2,
            clearance: 0.15,
            via: 0.4,
            nets: vec!["VCC".to_string(), "GND".to_string()],
            buses: vec!["D".to_string()],
        },
    )
    .expect("add net class");
    assert_eq!(added["netClass"]["id"], "net-class-1");
    assert_eq!(added["netClass"]["buses"], json!(["DATA"]), "an alias is stored as its canonical bus name");
    let revision = added["revision"].as_u64().expect("revision");
    let unchanged = add_net_class(
        handle,
        NetClassRequest {
            name: "HighSpeed".to_string(),
            width: 0.2,
            clearance: 0.15,
            via: 0.4,
            nets: vec!["VCC".to_string(), "GND".to_string()],
            buses: vec!["D".to_string()],
        },
    )
    .expect("redeclare equal class");
    assert_eq!(unchanged["revision"], revision, "an unchanged class adds no revision");

    rename_net(handle, RenameNetRequest { sheet: None, net: "VCC".to_string(), name: "VCC_3V3".to_string() })
        .expect("rename net");
    let listed = list_net_classes(handle).expect("list net classes");
    assert_eq!(listed["netClasses"][0]["nets"], json!(["GND", "VCC_3V3"]));
    delete_net(handle, DeleteNetRequest { sheet: None, net: "GND".to_string() }).expect("delete net");
    let listed = list_net_classes(handle).expect("list after net delete");
    assert_eq!(listed["netClasses"][0]["nets"], json!(["VCC_3V3"]), "a deleted net leaves its class");
    remove_bus(handle, "DATA").expect("remove bus");
    let listed = list_net_classes(handle).expect("list after bus delete");
    assert!(listed["netClasses"][0].get("buses").is_none(), "a removed bus leaves its class");

    let removed = remove_net_class(handle, "HighSpeed").expect("remove class");
    assert_eq!(removed["deleted"], "HighSpeed");
    assert_eq!(removed["remaining"], 0);
    close(handle).expect("close");
}

#[test]
fn net_class_declaration_rejects_bad_attributes_and_unknown_targets() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add R1");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect VCC");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add bus");

    let base = |name: &str, width: f64, clearance: f64, via: f64, nets: Vec<String>, buses: Vec<String>| NetClassRequest {
        name: name.to_string(),
        width,
        clearance,
        via,
        nets,
        buses,
    };
    assert_eq!(
        add_net_class(handle, base("", 0.2, 0.0, 0.0, Vec::new(), Vec::new())).unwrap_err().code,
        "session/argument-invalid",
        "a class needs a name"
    );
    assert_eq!(
        add_net_class(handle, base("Bad Name", 0.2, 0.0, 0.0, Vec::new(), Vec::new())).unwrap_err().code,
        "session/argument-invalid",
        "a class name cannot contain whitespace"
    );
    assert_eq!(
        add_net_class(handle, base("Signal", 0.0, 0.0, 0.0, Vec::new(), Vec::new())).unwrap_err().code,
        "session/argument-invalid",
        "a width must be above zero"
    );
    assert_eq!(
        add_net_class(handle, base("Signal", 0.2, -1.0, 0.0, Vec::new(), Vec::new())).unwrap_err().code,
        "session/argument-invalid",
        "clearance cannot be negative"
    );
    assert_eq!(
        add_net_class(handle, base("Signal", 0.2, 0.0, -1.0, Vec::new(), Vec::new())).unwrap_err().code,
        "session/argument-invalid",
        "a via diameter cannot be negative"
    );
    assert_eq!(
        add_net_class(handle, base("Signal", 0.2, 0.0, 0.0, vec!["NOPE".to_string()], Vec::new())).unwrap_err().code,
        "object/not-found",
        "a class cannot assign a net the document does not declare"
    );
    assert_eq!(
        add_net_class(handle, base("Signal", 0.2, 0.0, 0.0, Vec::new(), vec!["NOPE".to_string()])).unwrap_err().code,
        "object/not-found",
        "a class cannot assign a bus the document does not declare"
    );
    let added = add_net_class(handle, base("Signal", 0.2, 0.0, 0.0, vec!["VCC".to_string()], vec!["DATA".to_string()]))
        .expect("valid class");
    assert_eq!(added["netClass"]["nets"], json!(["VCC"]));
    close(handle).expect("close");
}

#[test]
fn reference_prefix_rules_report_mismatches_and_keep_waivers_working() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for reference in ["R1", "C1", "U1"] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add component");
    }
    for (class, prefix) in [("resistor", "R"), ("capacitor", "C"), ("ic", "U")] {
        set_reference_prefix(handle, ReferencePrefixRequest { class: class.to_string(), prefix: prefix.to_string() })
            .expect("declare prefix");
    }
    for (reference, class) in [("R1", "resistor"), ("C1", "capacitor"), ("U1", "resistor")] {
        set_component_class(
            handle,
            ComponentClassRequest { sheet: None, reference: reference.to_string(), class: class.to_string() },
        )
        .expect("assign class");
    }

    let report = inspect(handle, Some(vec!["references".to_string()])).expect("inspect references");
    let codes: Vec<(&str, &str)> = report["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .map(|issue| (issue["code"].as_str().unwrap_or(""), issue["objectId"].as_str().unwrap_or("")))
        .collect();
    assert_eq!(codes, vec![("reference/prefix-mismatch", "component-3")], "only U1 breaks its class prefix");
    assert_eq!(report["passed"], false);

    let waived = inspect_with_options(
        handle,
        InspectOptions {
            checks: Some(vec!["references".to_string()]),
            severity: BTreeMap::new(),
            waivers: vec![Waiver {
                code: "reference/prefix-mismatch".to_string(),
                sheet_id: None,
                object_id: Some("component-3".to_string()),
            }],
        },
    )
    .expect("inspect with waiver");
    assert_eq!(waived["passed"], true);
    assert_eq!(waived["issues"], json!([]));
    assert_eq!(waived["waived"].as_array().expect("waived").len(), 1);

    remove_reference_prefix(handle, "capacitor").expect("remove rule");
    let report = inspect(handle, Some(vec!["references".to_string()])).expect("inspect after rule removal");
    assert!(
        report["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .any(|issue| issue["code"] == "reference/class-unknown" && issue["objectId"] == "component-2"),
        "a removed rule leaves the assignment visible instead of silently accepted"
    );
    close(handle).expect("close");
}

#[test]
fn reference_prefix_rules_validate_declarations_and_assignments() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            pins: vec!["1".to_string()],
        },
    )
    .expect("add R1");

    let rule = |class: &str, prefix: &str| ReferencePrefixRequest { class: class.to_string(), prefix: prefix.to_string() };
    assert_eq!(
        set_reference_prefix(handle, rule("", "R")).unwrap_err().code,
        "session/argument-invalid",
        "a rule needs a class"
    );
    assert_eq!(
        set_reference_prefix(handle, rule("resistor", "r")).unwrap_err().code,
        "session/argument-invalid",
        "a prefix is uppercase"
    );
    assert_eq!(
        set_reference_prefix(handle, rule("resistor", "R1")).unwrap_err().code,
        "session/argument-invalid",
        "digits are not part of a prefix"
    );
    assert_eq!(set_reference_prefix(handle, rule("resistor", "TOOLONGPREFIX")).unwrap_err().code, "session/argument-invalid");
    assert_eq!(
        set_component_class(
            handle,
            ComponentClassRequest { sheet: None, reference: "R1".to_string(), class: "resistor".to_string() }
        )
        .unwrap_err()
        .code,
        "object/not-found",
        "a class needs a declared rule before it can be assigned"
    );
    let declared = set_reference_prefix(handle, rule("resistor", "R")).expect("declare");
    let revision = declared["revision"].as_u64().expect("revision");
    let unchanged = set_reference_prefix(handle, rule("resistor", "R")).expect("redeclare equal rule");
    assert_eq!(unchanged["revision"], revision, "an unchanged rule adds no revision");
    assert_eq!(
        set_component_class(
            handle,
            ComponentClassRequest { sheet: None, reference: "R9".to_string(), class: "resistor".to_string() }
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );

    let assigned = set_component_class(
        handle,
        ComponentClassRequest { sheet: None, reference: "R1".to_string(), class: "resistor".to_string() },
    )
    .expect("assign");
    assert_eq!(assigned["componentId"], "component-1");
    let listed = list_component_classes(handle).expect("list assignments");
    assert_eq!(listed["componentClasses"][0]["reference"], "R1");
    assert_eq!(listed["componentClasses"][0]["class"], "resistor");
    let cleared =
        set_component_class(handle, ComponentClassRequest { sheet: None, reference: "R1".to_string(), class: String::new() })
            .expect("clear assignment");
    assert_eq!(cleared["class"], Value::Null);
    assert_eq!(list_component_classes(handle).expect("list after clear")["componentClasses"], json!([]));
    close(handle).expect("close");
}

const KICAD_NETLIST: &str = r#"(export (version "E")
  (design (source "demo.kicad_sch"))
  (components
    (comp (ref "R1") (value "10k") (footprint "Resistor_SMD:R_0805"))
    (comp (ref "C1") (value "100n") (footprint "Capacitor_SMD:C_0603"))
  )
  (nets
    (net (code "1") (name "VCC")
      (node (ref "R1") (pin "1"))
      (node (ref "C1") (pin "1")))
    (net (code "2") (name "GND")
      (node (ref "R1") (pin "2"))
      (node (ref "C1") (pin "2")))
  )
)"#;

fn declared_netlist_design(handle: u64) {
    for (reference, value, footprint) in [("R1", "10k", "Resistor_SMD:R_0805"), ("C1", "100n", "Capacitor_SMD:C_0603")] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: value.to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
        set_component_footprint(
            handle,
            SetFootprintRequest { sheet: None, reference: reference.to_string(), footprint: footprint.to_string() },
        )
        .expect("set footprint");
    }
    for (net, nodes) in
        [("VCC", vec!["R1.1".to_string(), "C1.1".to_string()]), ("GND", vec!["R1.2".to_string(), "C1.2".to_string()])]
    {
        connect_net(handle, ConnectNetRequest { sheet: None, net: net.to_string(), nodes }).expect("connect net");
    }
}

#[test]
fn kicad_netlist_import_builds_a_design_that_compares_equal() {
    let imported = import_kicad_net(KICAD_NETLIST).expect("import netlist");
    assert_eq!(imported["kind"], "net");
    assert_eq!(imported["totals"], json!({ "components": 2, "nets": 2, "pins": 4 }));
    assert_eq!(imported["losses"], json!([]));
    assert_eq!(imported["design"]["sheets"][0]["components"][0]["ref"], "R1");
    assert_eq!(imported["design"]["sheets"][0]["components"][0]["pins"], json!(["1", "2"]));

    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    declared_netlist_design(handle);
    let report = compare_kicad_net(handle, KICAD_NETLIST).expect("compare");
    assert_eq!(report["format"], "open-eda-netlist-diff");
    assert_eq!(report["equal"], true, "{report}");
    assert_eq!(report["components"]["added"], json!([]));
    assert_eq!(report["components"]["modified"], json!([]));
    assert_eq!(report["nets"]["modified"], json!([]));
    close(handle).expect("close");
}

#[test]
fn kicad_netlist_import_reports_losses_and_netlist_conflicts() {
    let lossy = import_kicad_net(
        r#"(export (version "E")
  (components
    (comp (ref "R1") (value "10k"))
    (comp (ref "R1") (value "1k"))
    (comp (value "no reference"))
  )
  (nets
    (net (code "1") (name "VCC") (node (ref "R1") (pin "1")) (node (ref "U9") (pin "1")))
  )
  (unknown_section (value "x"))
)"#,
    )
    .expect("import lossy netlist");
    let codes: Vec<&str> =
        lossy["losses"].as_array().expect("losses").iter().map(|loss| loss["code"].as_str().unwrap_or("")).collect();
    assert!(codes.contains(&"net/duplicate-reference"));
    assert!(codes.contains(&"net/missing-reference"));
    assert!(codes.contains(&"net/missing-component"));
    assert!(codes.contains(&"net/unsupported-element"));
    assert_eq!(lossy["totals"]["components"], 1);
    assert_eq!(lossy["totals"]["nets"], 1);

    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    declared_netlist_design(handle);
    update_component(
        handle,
        UpdateComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: Some("12k".to_string()),
            footprint: None,
            x: None,
            y: None,
            rotation: None,
        },
    )
    .expect("change value");
    disconnect_net(handle, DisconnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["C1.1".to_string()] })
        .expect("drop a node");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "SIG".to_string(), nodes: vec!["R1.2".to_string()] })
        .expect("add a local net");

    let report = compare_kicad_net(handle, KICAD_NETLIST).expect("compare");
    assert_eq!(report["equal"], false);
    assert_eq!(report["components"]["modified"][0]["reference"], "R1");
    assert_eq!(report["components"]["modified"][0]["changes"]["value"]["to"], "10k");
    assert_eq!(report["nets"]["modified"][0]["name"], "VCC");
    assert_eq!(report["nets"]["modified"][0]["addedNodes"], json!(["C1.1"]));
    assert_eq!(report["nets"]["removed"][0]["name"], "SIG");
    assert_eq!(import_kicad_net("(kicad_sch (version 20230121))").unwrap_err().code, "format/invalid");
    close(handle).expect("close");
}

#[test]
fn annotate_renumbers_by_placement_and_updates_net_nodes() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x) in [("R2", 10.0), ("R1", 20.0), ("C1", 40.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R2.1".to_string(), "R1.1".to_string()] },
    )
    .expect("connect");
    let before = summary(handle).expect("summary")["revision"].as_u64().expect("revision");

    let report = annotate(handle).expect("annotate");
    assert_eq!(report["annotated"], 2);
    let changes = report["changes"].as_array().expect("changes");
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0]["from"], "R2");
    assert_eq!(changes[0]["to"], "R1");
    assert_eq!(changes[1]["from"], "R1");
    assert_eq!(changes[1]["to"], "R2");
    assert_eq!(report["revision"], before + 1);

    let listed = list(handle).expect("list");
    let references = listed["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|component| component["ref"].as_str().expect("ref"))
        .collect::<Vec<_>>();
    assert_eq!(references, vec!["R1", "R2", "C1"]);
    let nodes = listed["nets"][0]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|node| format!("{}.{}", node["ref"].as_str().expect("ref"), node["pin"].as_str().expect("pin")))
        .collect::<Vec<_>>();
    assert_eq!(nodes, vec!["R1.1", "R2.1"]);

    let again = annotate(handle).expect("annotate again");
    assert_eq!(again["annotated"], 0);
    assert_eq!(again["revision"], report["revision"]);
    close(handle).expect("close");
}

#[test]
fn annotate_orders_four_components_top_to_bottom_and_left_to_right() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x, y) in [("R9", 30.0, 20.0), ("R3", 10.0, 10.0), ("R7", 20.0, 10.0), ("R1", 10.0, 20.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }

    let report = annotate(handle).expect("annotate");
    assert_eq!(report["count"], 4);
    assert_eq!(report["changed"], 4);
    let changes = report["changes"].as_array().expect("changes");
    let moves = changes
        .iter()
        .map(|change| (change["from"].as_str().expect("from"), change["to"].as_str().expect("to")))
        .collect::<Vec<_>>();
    assert_eq!(moves, vec![("R3", "R1"), ("R7", "R2"), ("R1", "R3"), ("R9", "R4")]);
    assert_eq!(report["skipped"], json!([]));

    let listed = list(handle).expect("list");
    let references = listed["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|component| component["ref"].as_str().expect("ref"))
        .collect::<Vec<_>>();
    assert_eq!(references, vec!["R4", "R1", "R2", "R3"]);
    close(handle).expect("close");
}

#[test]
fn annotate_counts_each_prefix_independently() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x, y) in
        [("C2", 10.0, 10.0), ("R5", 20.0, 10.0), ("C1", 30.0, 10.0), ("U7", 10.0, 20.0), ("R2", 20.0, 20.0)]
    {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }

    let report = annotate(handle).expect("annotate");
    assert_eq!(report["count"], 4);
    let listed = list(handle).expect("list");
    let references = listed["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|component| component["ref"].as_str().expect("ref"))
        .collect::<Vec<_>>();
    assert_eq!(references, vec!["C1", "R1", "C2", "U1", "R2"]);
    close(handle).expect("close");
}

#[test]
fn annotate_reports_no_change_for_an_ordered_design() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x, y) in [("R1", 10.0, 10.0), ("R2", 20.0, 10.0), ("C1", 10.0, 20.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }
    let before = summary(handle).expect("summary")["revision"].as_u64().expect("revision");

    let report = annotate(handle).expect("annotate");
    assert_eq!(report["count"], 0);
    assert_eq!(report["changed"], 0);
    assert_eq!(report["annotated"], 0);
    assert_eq!(report["changes"], json!([]));
    assert_eq!(report["skipped"], json!([]));
    assert_eq!(report["revision"], before);
    close(handle).expect("close");
}

#[test]
fn annotate_skips_references_without_a_prefix() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x) in [("123", 10.0), ("R2", 20.0), ("R1", 30.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }

    let report = annotate(handle).expect("annotate");
    assert_eq!(report["count"], 2);
    let skipped = report["skipped"].as_array().expect("skipped");
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["reference"], "123");
    assert_eq!(skipped[0]["sheetId"], "sheet-1");
    assert_eq!(skipped[0]["componentId"], "component-1");
    let listed = list(handle).expect("list");
    let references = listed["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|component| component["ref"].as_str().expect("ref"))
        .collect::<Vec<_>>();
    assert_eq!(references, vec!["123", "R1", "R2"]);
    close(handle).expect("close");
}

#[test]
fn variants_filter_and_override_bom_rows_and_persist() {
    let dir = TempDir::new("variants");
    let document = dir.join("variants.oeda");
    let default_bom = dir.join("default.csv");
    let variant_bom = dir.join("lite.csv");
    let variant_json = dir.join("lite.json");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, value) in [("R1", "10k"), ("R2", "10k"), ("C1", "100n")] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: value.to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add");
    }
    let added = add_variant(
        handle,
        VariantRequest {
            name: "lite".to_string(),
            dnp: vec!["R2".to_string(), "R2".to_string()],
            values: BTreeMap::from([("R1".to_string(), "22k".to_string())]),
        },
    )
    .expect("add variant");
    assert_eq!(added["variant"]["dnp"], json!(["R2"]));
    assert_eq!(added["variant"]["values"]["R1"], "22k");
    assert_eq!(list_variants(handle).expect("list")["variants"].as_array().expect("variants").len(), 1);

    export(handle, "bom", &default_bom).expect("export default bom");
    let default_text = fs::read_to_string(&default_bom).expect("read default bom");
    assert!(default_text.contains("10k,,\"R1,R2\",2"));

    let report = export_with_variant(handle, "bom", &variant_bom, Some("lite")).expect("export variant");
    assert_eq!(report["artifact"]["variant"], "lite");
    assert_eq!(report["artifact"]["dnp"], 1);
    assert_eq!(report["artifact"]["rows"], 2);
    assert_eq!(
        fs::read_to_string(&variant_bom).expect("read variant bom"),
        "Comment,Footprint,Designator,Quantity\n100n,,C1,1\n22k,,R1,1\n"
    );
    let json_report = export_with_variant(handle, "bom_json", &variant_json, Some("lite")).expect("export variant json");
    assert_eq!(json_report["artifact"]["variant"], "lite");
    assert!(!fs::read_to_string(&variant_json).expect("read json").contains("R2"));

    assert_eq!(export_with_variant(handle, "bom", &default_bom, Some("missing")).unwrap_err().code, "object/not-found");
    assert_eq!(export_with_variant(handle, "net", &default_bom, Some("lite")).unwrap_err().code, "session/argument-invalid");
    assert_eq!(
        add_variant(handle, VariantRequest { name: "lite".to_string(), dnp: vec!["R9".to_string()], values: BTreeMap::new() })
            .unwrap_err()
            .code,
        "object/not-found"
    );
    assert_eq!(
        add_variant(handle, VariantRequest { name: "  ".to_string(), dnp: Vec::new(), values: BTreeMap::new() })
            .unwrap_err()
            .code,
        "session/argument-invalid"
    );

    save(handle, &document, false).expect("save");
    close(handle).expect("close");
    let opened = open(&document).expect("open");
    let reopened = opened["handle"].as_u64().expect("handle");
    let listed = list_variants(reopened).expect("list after reopen");
    assert_eq!(listed["variants"][0]["name"], "lite");
    assert_eq!(listed["variants"][0]["dnp"], json!(["R2"]));
    let removed = remove_variant(reopened, "lite").expect("remove");
    assert_eq!(removed["deleted"], "lite");
    assert_eq!(remove_variant(reopened, "lite").unwrap_err().code, "object/not-found");
    close(reopened).expect("close");
}

#[test]
fn annotation_rewrites_variant_references() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, x) in [("R2", 10.0), ("R1", 20.0)] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "10k".to_string(),
                x,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string()],
            },
        )
        .expect("add");
    }
    add_variant(
        handle,
        VariantRequest {
            name: "lite".to_string(),
            dnp: vec!["R2".to_string()],
            values: BTreeMap::from([("R1".to_string(), "22k".to_string())]),
        },
    )
    .expect("add variant");
    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 10.0,
            y: 0.0,
            node: Some(Node { reference: "R2".to_string(), pin: "1".to_string() }),
        },
    )
    .expect("add no-connect");
    annotate(handle).expect("annotate");
    let variant = &list_variants(handle).expect("list")["variants"][0];
    assert_eq!(variant["dnp"], json!(["R1"]));
    assert_eq!(variant["values"]["R2"], "22k");
    let flags = list_no_connects(handle, None).expect("list no-connects");
    assert_eq!(flags["noConnects"][0]["node"]["ref"], "R1");
    assert_eq!(flags["noConnects"][0]["node"]["pin"], "1");
    close(handle).expect("close");
}

#[test]
fn power_check_reports_driver_conflicts() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for reference in ["U1", "U2"] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: "regulator".to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add");
    }
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["U1.1".to_string(), "U2.1".to_string()] },
    )
    .expect("connect");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VBAT".to_string(), nodes: vec!["U1.2".to_string(), "U2.2".to_string()] },
    )
    .expect("connect");
    for (reference, pin, pin_type) in [
        ("U1", "1", PinType::PowerOut),
        ("U2", "1", PinType::PowerOut),
        ("U1", "2", PinType::PowerOut),
        ("U2", "2", PinType::Output),
    ] {
        set_pin_type(
            handle,
            SetPinTypeRequest { sheet: None, reference: reference.to_string(), pin: pin.to_string(), pin_type: Some(pin_type) },
        )
        .expect("type pin");
    }

    let report = inspect(handle, Some(vec!["power".to_string()])).expect("inspect power");
    let codes =
        report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect::<Vec<_>>();
    assert!(codes.contains(&"power/driver-conflict"));
    assert!(codes.contains(&"power/output-driver-conflict"));
    let conflict = report["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .find(|issue| issue["code"] == "power/driver-conflict")
        .expect("driver conflict");
    assert_eq!(conflict["severity"], "error");
    assert!(conflict["message"].as_str().expect("message").contains("U1.1, U2.1"));

    set_pin_type(handle, SetPinTypeRequest { sheet: None, reference: "U2".to_string(), pin: "1".to_string(), pin_type: None })
        .expect("clear");
    let single = inspect(handle, Some(vec!["power".to_string()])).expect("inspect power");
    let codes =
        single["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect::<Vec<_>>();
    assert!(!codes.contains(&"power/driver-conflict"));
    assert!(codes.contains(&"power/output-driver-conflict"));
    close(handle).expect("close");
}

#[test]
fn component_properties_are_edited_listed_and_persisted() {
    let dir = TempDir::new("component-properties");
    let document = dir.join("props.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
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

    let added = set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "MPN".to_string(),
            value: "ABC-123".to_string(),
        },
    )
    .expect("set MPN");
    assert_eq!(added["component"]["properties"]["MPN"], "ABC-123");
    assert_eq!(added["key"], "MPN");
    let revision = added["revision"].as_u64().expect("revision");
    let unchanged = set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "MPN".to_string(),
            value: "ABC-123".to_string(),
        },
    )
    .expect("repeat MPN");
    assert_eq!(unchanged["revision"], revision, "an unchanged property adds no revision");

    set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "Description".to_string(),
            value: "10k resistor".to_string(),
        },
    )
    .expect("set Description");
    let listed = list_component_properties(handle, "R1", None).expect("list properties");
    assert_eq!(
        listed["properties"],
        json!([
            { "key": "Description", "value": "10k resistor" },
            { "key": "MPN", "value": "ABC-123" },
        ])
    );
    assert_eq!(listed["componentId"], "component-1");
    assert_eq!(listed["sheetId"], "sheet-1");

    assert_eq!(
        set_component_property(
            handle,
            SetComponentPropertyRequest {
                sheet: None,
                reference: "R1".to_string(),
                key: "Bad Key".to_string(),
                value: "x".to_string(),
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );
    assert_eq!(
        set_component_property(
            handle,
            SetComponentPropertyRequest {
                sheet: None,
                reference: "R1".to_string(),
                key: String::new(),
                value: "x".to_string(),
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );
    assert_eq!(
        set_component_property(
            handle,
            SetComponentPropertyRequest {
                sheet: None,
                reference: "R9".to_string(),
                key: "MPN".to_string(),
                value: "x".to_string(),
            },
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );

    save(handle, &document, true).expect("save");
    let opened = open(&document).expect("open");
    let reopened = opened["handle"].as_u64().expect("handle");
    let persisted = list_component_properties(reopened, "R1", None).expect("list reopened");
    assert_eq!(persisted["properties"][1]["value"], "ABC-123");
    remove_component_property(
        reopened,
        RemoveComponentPropertyRequest { sheet: None, reference: "R1".to_string(), key: "Description".to_string() },
    )
    .expect("remove Description");
    assert_eq!(
        remove_component_property(
            reopened,
            RemoveComponentPropertyRequest { sheet: None, reference: "R1".to_string(), key: "Description".to_string() },
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );
    let listed = list_component_properties(reopened, "R1", None).expect("list after remove");
    assert_eq!(listed["properties"], json!([{ "key": "MPN", "value": "ABC-123" }]));
    close(reopened).expect("close reopened");
    close(handle).expect("close");
}

#[test]
fn hierarchical_sheets_and_ports_are_edited_listed_and_persisted() {
    let dir = TempDir::new("hierarchy");
    let document = dir.join("hierarchy.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    let added = add_hierarchy(
        handle,
        AddHierarchyRequest {
            name: "Power".to_string(),
            file: "power.kicad_sch".to_string(),
            x: 10.0,
            y: 20.0,
            width: 60.0,
            height: 40.0,
        },
    )
    .expect("add hierarchy");
    assert_eq!(added["hierarchy"]["id"], "hierarchy-1");
    assert_eq!(added["hierarchy"]["file"], "power.kicad_sch");
    assert_eq!(added["hierarchy"].get("ports"), None);
    assert_eq!(
        add_hierarchy(
            handle,
            AddHierarchyRequest {
                name: "Power".to_string(),
                file: "other.kicad_sch".to_string(),
                x: 0.0,
                y: 0.0,
                width: 60.0,
                height: 40.0,
            },
        )
        .unwrap_err()
        .code,
        "object/duplicate"
    );
    assert_eq!(
        add_hierarchy(
            handle,
            AddHierarchyRequest {
                name: "Bad".to_string(),
                file: "bad.kicad_sch".to_string(),
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 40.0,
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );

    let port = add_hierarchy_port(
        handle,
        AddHierarchyPortRequest {
            sheet: "Power".to_string(),
            name: "VIN".to_string(),
            direction: PortDirection::Input,
            x: 10.0,
            y: 24.0,
        },
    )
    .expect("add port");
    assert_eq!(port["port"]["name"], "VIN");
    assert_eq!(port["port"]["direction"], "input");
    assert_eq!(port["hierarchy"]["ports"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        add_hierarchy_port(
            handle,
            AddHierarchyPortRequest {
                sheet: "Power".to_string(),
                name: "VIN".to_string(),
                direction: PortDirection::Output,
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap_err()
        .code,
        "object/duplicate"
    );
    assert_eq!(
        add_hierarchy_port(
            handle,
            AddHierarchyPortRequest {
                sheet: "Missing".to_string(),
                name: "GND".to_string(),
                direction: PortDirection::Passive,
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );

    let listed = list_hierarchy(handle).expect("list hierarchy");
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["hierarchy"][0]["name"], "Power");
    assert_eq!(listed["hierarchy"][0]["ports"][0]["name"], "VIN");

    save(handle, &document, true).expect("save");
    let opened = open(&document).expect("open");
    let reopened = opened["handle"].as_u64().expect("handle");
    let persisted = list_hierarchy(reopened).expect("list reopened");
    assert_eq!(persisted["hierarchy"][0]["ports"][0]["direction"], "input");
    let removed_port = remove_hierarchy_port(
        reopened,
        RemoveHierarchyPortRequest { sheet: "hierarchy-1".to_string(), name: "VIN".to_string() },
    )
    .expect("remove port");
    assert_eq!(removed_port["removed"], "VIN");
    assert_eq!(
        remove_hierarchy_port(reopened, RemoveHierarchyPortRequest { sheet: "Power".to_string(), name: "VIN".to_string() },)
            .unwrap_err()
            .code,
        "object/not-found"
    );
    let removed = remove_hierarchy(reopened, "Power").expect("remove hierarchy");
    assert_eq!(removed["removed"], "hierarchy-1");
    assert_eq!(list_hierarchy(reopened).expect("list after remove")["count"], 0);
    assert_eq!(remove_hierarchy(reopened, "Power").unwrap_err().code, "object/not-found");
    close(reopened).expect("close reopened");
    close(handle).expect("close");
}

#[test]
fn copy_component_duplicates_placement_and_keeps_metadata() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 40.0,
            y: 30.0,
            rotation: 90.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add component");
    set_component_footprint(
        handle,
        SetFootprintRequest {
            sheet: None,
            reference: "R1".to_string(),
            footprint: "Resistor_SMD:R_0805_2012Metric".to_string(),
        },
    )
    .expect("set footprint");
    set_pin_type(
        handle,
        SetPinTypeRequest { sheet: None, reference: "R1".to_string(), pin: "1".to_string(), pin_type: Some(PinType::Passive) },
    )
    .expect("set pin type");
    set_component_property(
        handle,
        SetComponentPropertyRequest {
            sheet: None,
            reference: "R1".to_string(),
            key: "MPN".to_string(),
            value: "ABC-123".to_string(),
        },
    )
    .expect("set property");
    set_reference_prefix(handle, ReferencePrefixRequest { class: "resistor".to_string(), prefix: "R".to_string() })
        .expect("declare prefix");
    set_component_class(
        handle,
        ComponentClassRequest { sheet: None, reference: "R1".to_string(), class: "resistor".to_string() },
    )
    .expect("assign class");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect net");
    let before = list_components(handle, None).expect("list components")["revision"].as_u64().expect("revision");

    let copied = copy_component(
        handle,
        CopyComponentRequest { sheet: None, reference: "R1".to_string(), new_reference: "R2".to_string(), dx: 5.0, dy: -5.0 },
    )
    .expect("copy component");
    assert_eq!(copied["component"]["id"], "component-2");
    assert_eq!(copied["component"]["ref"], "R2");
    assert_eq!(copied["component"]["value"], "10k");
    assert_eq!(copied["component"]["footprint"], "Resistor_SMD:R_0805_2012Metric");
    assert_eq!(copied["component"]["x"], 45.0);
    assert_eq!(copied["component"]["y"], 25.0);
    assert_eq!(copied["component"]["rotation"], 90.0);
    assert_eq!(copied["component"]["pins"], json!(["1", "2"]));
    assert_eq!(copied["component"]["pinTypes"]["1"], "passive");
    assert_eq!(copied["component"]["properties"]["MPN"], "ABC-123");
    assert_eq!(copied["revision"], before + 1);
    assert_eq!(copied["canUndo"], true);

    let classes = list_component_classes(handle).expect("list classes");
    let assignments = classes["componentClasses"].as_array().expect("assignments");
    assert_eq!(assignments.len(), 2);
    assert!(assignments.iter().any(|entry| entry["reference"] == "R2" && entry["class"] == "resistor"));

    let nets = list_nets(handle, None).expect("list nets");
    assert_eq!(nets["nets"].as_array().map(Vec::len), Some(1));
    assert_eq!(nets["nets"][0]["nodes"], json!([{ "ref": "R1", "pin": "1" }]));

    assert_eq!(
        copy_component(
            handle,
            CopyComponentRequest {
                sheet: None,
                reference: "R1".to_string(),
                new_reference: "R2".to_string(),
                dx: 0.0,
                dy: 0.0,
            },
        )
        .unwrap_err()
        .code,
        "object/duplicate"
    );
    assert_eq!(
        copy_component(
            handle,
            CopyComponentRequest {
                sheet: None,
                reference: "R9".to_string(),
                new_reference: "R3".to_string(),
                dx: 0.0,
                dy: 0.0,
            },
        )
        .unwrap_err()
        .code,
        "object/not-found"
    );
    assert_eq!(
        copy_component(
            handle,
            CopyComponentRequest {
                sheet: None,
                reference: "R1".to_string(),
                new_reference: "  ".to_string(),
                dx: 0.0,
                dy: 0.0,
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );
    assert_eq!(
        copy_component(
            handle,
            CopyComponentRequest {
                sheet: None,
                reference: "R1".to_string(),
                new_reference: "R3".to_string(),
                dx: f64::NAN,
                dy: 0.0,
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );

    undo(handle).expect("undo copy");
    let restored = list_components(handle, None).expect("list after undo");
    assert_eq!(restored["components"].as_array().map(Vec::len), Some(1));
    close(handle).expect("close");
}

#[test]
fn texts_are_listed_and_updated_in_place() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_text(handle, AddTextRequest { sheet: None, text: "NOTE".to_string(), x: 10.0, y: 20.0 }).expect("add text");
    add_text(handle, AddTextRequest { sheet: None, text: "SECOND".to_string(), x: 30.0, y: 40.0 }).expect("add second text");

    let listed = list_texts(handle, None).expect("list texts");
    assert_eq!(listed["sheetId"], "sheet-1");
    assert_eq!(
        listed["texts"],
        json!([
            { "index": 0, "text": "NOTE", "x": 10.0, "y": 20.0 },
            { "index": 1, "text": "SECOND", "x": 30.0, "y": 40.0 },
        ])
    );
    let before = listed["revision"].as_u64().expect("revision");

    let updated = update_text(
        handle,
        UpdateTextRequest { sheet: None, index: 0, text: Some("UPDATED".to_string()), x: Some(12.0), y: None },
    )
    .expect("update text");
    assert_eq!(updated["index"], 0);
    assert_eq!(updated["text"]["text"], "UPDATED");
    assert_eq!(updated["text"]["x"], 12.0);
    assert_eq!(updated["text"]["y"], 20.0);
    assert_eq!(updated["revision"], before + 1);

    let after = list_texts(handle, None).expect("list after update");
    assert_eq!(after["texts"][0]["text"], "UPDATED");
    assert_eq!(after["texts"][0]["x"], 12.0);

    move_text(handle, 1, None, 35.0, 45.0).expect("move text");
    assert_eq!(list_texts(handle, None).expect("list after move")["texts"][1]["x"], 35.0);

    assert_eq!(
        update_text(handle, UpdateTextRequest { sheet: None, index: 0, text: Some("   ".to_string()), x: None, y: None },)
            .unwrap_err()
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_text(handle, UpdateTextRequest { sheet: None, index: 0, text: None, x: None, y: None }).unwrap_err().code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_text(handle, UpdateTextRequest { sheet: None, index: 0, text: None, x: Some(f64::NAN), y: None })
            .unwrap_err()
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_text(handle, UpdateTextRequest { sheet: None, index: 9, text: None, x: Some(1.0), y: None }).unwrap_err().code,
        "object/not-found"
    );
    assert_eq!(
        update_text(
            handle,
            UpdateTextRequest { sheet: Some("missing".to_string()), index: 0, text: None, x: Some(1.0), y: None },
        )
        .unwrap_err()
        .code,
        "sheet/not-found"
    );

    undo(handle).expect("undo move");
    assert_eq!(list_texts(handle, None).expect("list after undo")["texts"][1]["x"], 30.0);
    close(handle).expect("close");
}

#[test]
fn labels_are_added_listed_updated_and_deleted() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");

    let local = add_label(
        handle,
        AddLabelRequest { sheet: None, text: "VCC".to_string(), x: 10.0, y: 20.0, scope: None, rotation: None, justify: None },
    )
    .expect("add local label");
    assert_eq!(local["label"]["id"], "label-1");
    assert_eq!(local["label"]["scope"], "local");
    assert_eq!(local["label"]["rotation"], 0.0);
    assert_eq!(local["label"]["justify"], "left");

    let global = add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: "GND".to_string(),
            x: 30.0,
            y: 40.0,
            scope: Some(LabelScope::Global),
            rotation: Some(90.0),
            justify: Some("right".to_string()),
        },
    )
    .expect("add global label");
    assert_eq!(global["label"]["id"], "label-2");
    assert_eq!(global["label"]["scope"], "global");
    assert_eq!(global["label"]["rotation"], 90.0);
    assert_eq!(global["label"]["justify"], "right");

    let listed = list_labels(handle, None).expect("list labels");
    assert_eq!(listed["sheetId"], "sheet-1");
    assert_eq!(listed["labels"].as_array().expect("labels").len(), 2);
    assert_eq!(listed["labels"][0]["text"], "VCC");
    assert_eq!(listed["labels"][1]["scope"], "global");

    let revised = update_label(
        handle,
        UpdateLabelRequest {
            sheet: None,
            id: "label-3".to_string(),
            text: Some("NOPE".to_string()),
            x: None,
            y: None,
            scope: None,
            rotation: None,
            justify: None,
        },
    )
    .expect_err("missing label");
    assert_eq!(revised.code, "object/not-found");

    let updated = update_label(
        handle,
        UpdateLabelRequest {
            sheet: None,
            id: "label-1".to_string(),
            text: Some("VDD".to_string()),
            x: Some(11.0),
            y: None,
            scope: Some(LabelScope::Hierarchical),
            rotation: Some(180.0),
            justify: Some("center".to_string()),
        },
    )
    .expect("update label");
    assert_eq!(updated["label"]["text"], "VDD");
    assert_eq!(updated["label"]["x"], 11.0);
    assert_eq!(updated["label"]["y"], 20.0);
    assert_eq!(updated["label"]["scope"], "hierarchical");
    assert_eq!(updated["label"]["rotation"], 180.0);
    assert_eq!(updated["label"]["justify"], "center");

    assert_eq!(
        add_label(
            handle,
            AddLabelRequest { sheet: None, text: "  ".to_string(), x: 0.0, y: 0.0, scope: None, rotation: None, justify: None },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );
    assert_eq!(
        add_label(
            handle,
            AddLabelRequest {
                sheet: None,
                text: "X".to_string(),
                x: 0.0,
                y: 0.0,
                scope: None,
                rotation: None,
                justify: Some("top".to_string()),
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_label(
            handle,
            UpdateLabelRequest {
                sheet: None,
                id: "label-1".to_string(),
                text: None,
                x: None,
                y: None,
                scope: None,
                rotation: None,
                justify: None,
            },
        )
        .unwrap_err()
        .code,
        "session/argument-invalid"
    );

    let removed = delete_label(handle, "label-2", None).expect("delete label");
    assert_eq!(removed["label"]["id"], "label-2");
    assert_eq!(removed["remaining"], 1);
    assert_eq!(list_labels(handle, None).expect("list after delete")["labels"].as_array().unwrap().len(), 1);

    undo(handle).expect("undo delete");
    assert_eq!(list_labels(handle, None).expect("list after undo")["labels"].as_array().unwrap().len(), 2);
    close(handle).expect("close");
}

#[test]
fn stats_reports_per_sheet_counts_and_design_totals() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_junction(handle, AddJunctionRequest { sheet: None, x: 1.0, y: 2.0 }).expect("junction");
    add_no_connect(handle, AddNoConnectRequest { sheet: None, x: 3.0, y: 4.0, node: None }).expect("no connect");
    add_text(handle, AddTextRequest { sheet: None, text: "NOTE".to_string(), x: 5.0, y: 6.0 }).expect("text");
    add_label(
        handle,
        AddLabelRequest { sheet: None, text: "NET".to_string(), x: 7.0, y: 8.0, scope: None, rotation: None, justify: None },
    )
    .expect("label");
    add_sheet(handle, Some("Power")).expect("add sheet");
    add_junction(handle, AddJunctionRequest { sheet: Some("Power".to_string()), x: 9.0, y: 10.0 })
        .expect("junction on second sheet");

    let report = stats(handle).expect("stats");
    assert_eq!(report["format"], "open-eda-stats");
    assert_eq!(report["version"], 1);
    assert_eq!(report["sheets"].as_array().expect("sheets").len(), 2);
    assert_eq!(report["sheets"][0]["id"], "sheet-1");
    assert_eq!(report["sheets"][0]["junctions"], 1);
    assert_eq!(report["sheets"][0]["noConnects"], 1);
    assert_eq!(report["sheets"][0]["texts"], 1);
    assert_eq!(report["sheets"][0]["labels"], 1);
    assert_eq!(report["sheets"][1]["name"], "Power");
    assert_eq!(report["sheets"][1]["junctions"], 1);
    assert_eq!(report["totals"]["sheets"], 2);
    assert_eq!(report["totals"]["junctions"], 2);
    assert_eq!(report["totals"]["noConnects"], 1);
    assert_eq!(report["totals"]["texts"], 1);
    assert_eq!(report["totals"]["labels"], 1);
    assert_eq!(report["totals"]["buses"], 0);
    assert_eq!(report["totals"]["bomRows"], 0);

    assert_eq!(list_junctions(handle, None).expect("junctions")["junctions"][0]["index"], 0);
    assert_eq!(list_no_connects(handle, None).expect("no connects")["noConnects"][0]["x"], 3.0);
    let sheets = list_sheets(handle).expect("sheets");
    assert_eq!(sheets["sheets"][1]["name"], "Power");
    assert_eq!(sheets["sheets"][0]["id"], "sheet-1");
    close(handle).expect("close");
}

fn hierarchy_document(dir: &TempDir, name: &str, build: impl FnOnce(u64)) -> std::path::PathBuf {
    let path = dir.join(name);
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    build(handle);
    save(handle, &path, true).expect("save");
    close(handle).expect("close");
    path
}

fn add_hierarchy_symbol(handle: u64, name: &str, file: &str) {
    add_hierarchy(
        handle,
        AddHierarchyRequest { name: name.to_string(), file: file.to_string(), x: 0.0, y: 0.0, width: 60.0, height: 40.0 },
    )
    .expect("add hierarchy");
}

fn add_hierarchical_label(handle: u64, label: &str) {
    add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: label.to_string(),
            x: 0.0,
            y: 0.0,
            scope: Some(LabelScope::Hierarchical),
            rotation: None,
            justify: None,
        },
    )
    .expect("add hierarchical label");
}

fn hierarchy_codes(report: &Value) -> Vec<&str> {
    report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect()
}

#[test]
fn hierarchy_check_passes_when_ports_and_labels_match_without_writing() {
    let dir = TempDir::new("hierarchy-pass");
    let child = hierarchy_document(&dir, "child.oeda", |handle| add_hierarchical_label(handle, "IN"));
    let parent = hierarchy_document(&dir, "parent.oeda", |handle| {
        add_hierarchy_symbol(handle, "Child", "child.oeda");
        add_hierarchy_port(
            handle,
            AddHierarchyPortRequest {
                sheet: "Child".to_string(),
                name: "IN".to_string(),
                direction: PortDirection::Input,
                x: 0.0,
                y: 0.0,
            },
        )
        .expect("add port");
    });
    let parent_text = fs::read_to_string(&parent).expect("read parent");
    let child_text = fs::read_to_string(&child).expect("read child");

    let opened = open(&parent).expect("open");
    let handle = opened["handle"].as_u64().expect("handle");
    let report = check_hierarchy(handle).expect("check");
    assert_eq!(report["format"], "open-eda-hierarchy");
    assert_eq!(report["version"], 1);
    assert_eq!(report["sheets"], 1);
    assert_eq!(report["issues"], json!([]));
    assert_eq!(report["passed"], true);
    assert_eq!(fs::read_to_string(&parent).expect("re-read parent"), parent_text);
    assert_eq!(fs::read_to_string(&child).expect("re-read child"), child_text);
    close(handle).expect("close");
}

#[test]
fn hierarchy_check_reports_port_and_label_mismatches_in_order() {
    let dir = TempDir::new("hierarchy-mismatch");
    hierarchy_document(&dir, "child.oeda", |handle| {
        add_hierarchical_label(handle, "IN");
        add_hierarchical_label(handle, "EXTRA");
    });
    let parent = hierarchy_document(&dir, "parent.oeda", |handle| {
        add_hierarchy_symbol(handle, "Child", "child.oeda");
        for name in ["IN", "MISSING"] {
            add_hierarchy_port(
                handle,
                AddHierarchyPortRequest {
                    sheet: "Child".to_string(),
                    name: name.to_string(),
                    direction: PortDirection::Input,
                    x: 0.0,
                    y: 0.0,
                },
            )
            .expect("add port");
        }
    });

    let opened = open(&parent).expect("open");
    let handle = opened["handle"].as_u64().expect("handle");
    locked_sessions().get_mut(&handle).expect("session").design.hierarchy[0].ports.push(SheetPort {
        name: "IN".to_string(),
        direction: PortDirection::Input,
        x: 0.0,
        y: 0.0,
    });
    let report = check_hierarchy(handle).expect("check");
    assert_eq!(report["sheets"], 1);
    assert_eq!(
        hierarchy_codes(&report),
        vec!["hierarchy/duplicate-port", "hierarchy/port-without-label", "hierarchy/label-without-port"]
    );
    assert_eq!(report["issues"][0]["severity"], "error");
    assert_eq!(report["issues"][1]["severity"], "error");
    assert_eq!(report["issues"][2]["severity"], "warning");
    assert!(report["issues"][0]["sheet"].as_str().expect("sheet").ends_with("child.oeda"));
    assert_eq!(report["passed"], false);
    close(handle).expect("close");
}

#[test]
fn hierarchy_check_classifies_missing_unsupported_and_invalid_children() {
    let dir = TempDir::new("hierarchy-children");
    fs::write(dir.join("broken.oeda"), "not json").expect("write broken");
    fs::write(dir.join("notes.txt"), "text").expect("write text");
    let parent = hierarchy_document(&dir, "parent.oeda", |handle| {
        add_hierarchy_symbol(handle, "Missing", "missing.oeda");
        add_hierarchy_symbol(handle, "Notes", "notes.txt");
        add_hierarchy_symbol(handle, "Broken", "broken.oeda");
    });

    let opened = open(&parent).expect("open");
    let handle = opened["handle"].as_u64().expect("handle");
    let report = check_hierarchy(handle).expect("check");
    assert_eq!(report["sheets"], 0);
    assert_eq!(
        hierarchy_codes(&report),
        vec!["hierarchy/file-missing", "hierarchy/child-format-unsupported", "hierarchy/child-invalid"]
    );
    assert_eq!(report["issues"][1]["severity"], "warning");
    assert_eq!(report["passed"], false);
    close(handle).expect("close");
}

#[test]
fn hierarchy_check_stops_at_cycles_and_the_depth_limit() {
    let dir = TempDir::new("hierarchy-cycle");
    let first = hierarchy_document(&dir, "a.oeda", |handle| add_hierarchy_symbol(handle, "B", "b.oeda"));
    hierarchy_document(&dir, "b.oeda", |handle| add_hierarchy_symbol(handle, "A", "a.oeda"));
    let opened = open(&first).expect("open");
    let handle = opened["handle"].as_u64().expect("handle");
    let report = check_hierarchy(handle).expect("check");
    assert_eq!(report["sheets"], 1);
    assert_eq!(hierarchy_codes(&report), vec!["hierarchy/cycle"]);
    assert_eq!(report["passed"], false);
    close(handle).expect("close");

    let dir = TempDir::new("hierarchy-depth");
    for depth in (0..=33).rev() {
        let next = (depth < 33).then(|| format!("depth-{}.oeda", depth + 1));
        hierarchy_document(&dir, &format!("depth-{depth}.oeda"), |handle| {
            if let Some(file) = next {
                add_hierarchy_symbol(handle, "Next", &file);
            }
        });
    }
    let opened = open(&dir.join("depth-0.oeda")).expect("open root");
    let handle = opened["handle"].as_u64().expect("handle");
    let report = check_hierarchy(handle).expect("check");
    assert_eq!(report["sheets"], 32);
    assert_eq!(hierarchy_codes(&report), vec!["hierarchy/depth-exceeded"]);
    assert_eq!(report["passed"], false);
    close(handle).expect("close");
}

#[test]
fn hierarchy_check_needs_a_document_path_and_stays_out_of_inspect() {
    let dir = TempDir::new("hierarchy-path");
    let target = dir.join("saved.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    assert_eq!(check_hierarchy(handle).expect_err("no path").code, "hierarchy/document-path-missing");
    assert_eq!(
        inspect(handle, Some(vec!["hierarchy".to_string()])).expect_err("inspect check").code,
        "capability/not-implemented"
    );
    save(handle, &target, false).expect("save");
    let report = check_hierarchy(handle).expect("check after save");
    assert_eq!(report["sheets"], 0);
    assert_eq!(report["passed"], true);
    close(handle).expect("close");
}

#[test]
fn two_pin_parts_render_standard_symbols_without_moving_connection_points() {
    let dir = TempDir::new("symbols");
    let svg_path = dir.join("symbols.svg");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    let parts = [
        ("U1", "MCU", 60.0, 40.0, vec!["VCC".to_string(), "GND".to_string(), "IO1".to_string(), "IO2".to_string()]),
        ("R1", "10k", 20.0, 20.0, vec!["1".to_string(), "2".to_string()]),
        ("C1", "+100n", 20.0, 40.0, vec!["1".to_string(), "2".to_string()]),
        ("L1", "10u", 20.0, 60.0, vec!["1".to_string(), "2".to_string()]),
        ("D1", "LED", 20.0, 80.0, vec!["A".to_string(), "K".to_string()]),
        ("Q1", "2N2222", 20.0, 100.0, vec!["B".to_string(), "C".to_string(), "E".to_string()]),
    ];
    for (reference, value, x, y, pins) in parts {
        add_component(
            handle,
            AddComponentRequest { sheet: None, reference: reference.into(), value: value.into(), x, y, rotation: 0.0, pins },
        )
        .expect(reference);
    }
    export(handle, "svg", &svg_path).expect("export svg");
    let svg = fs::read_to_string(&svg_path).expect("read svg");

    // The resistor zigzag sits between the unchanged stub roots, whose pin-end
    // connection points and stub lines must stay exactly where they were.
    assert!(
        svg.contains("<polyline points=\"14.92,20.00 16.19,17.46 18.73,22.54 21.27,17.46 23.81,22.54 25.08,20.00\""),
        "{svg}"
    );
    assert!(svg.contains("<polyline points=\"7.30,20.00 14.92,20.00\""), "{svg}");
    assert!(svg.contains("<polyline points=\"25.08,20.00 32.70,20.00\""), "{svg}");
    assert!(svg.contains("<line x1=\"7.30\" y1=\"20.00\" x2=\"4.76\" y2=\"20.00\""), "{svg}");
    assert!(svg.contains("<circle cx=\"4.76\" cy=\"20.00\" r=\"0.45\""), "{svg}");
    assert!(svg.contains("<circle cx=\"35.24\" cy=\"20.00\" r=\"0.45\""), "{svg}");
    // The generic box is gone from the two-pin part, and its labels moved out.
    assert!(!svg.contains("<rect x=\"7.30\" y=\"12.38\""), "{svg}");
    assert!(svg.contains("<text x=\"20.00\" y=\"15.50\" font-family=\"monospace\" font-size=\"3.2\""), "{svg}");
    assert!(svg.contains("<text x=\"20.00\" y=\"24.50\" font-family=\"monospace\" font-size=\"2.6\""), "{svg}");

    // The polarised capacitor paints two parallel plates and a plus sign.
    assert!(svg.contains("<polyline points=\"18.73,37.46 18.73,42.54\""), "{svg}");
    assert!(svg.contains("<polyline points=\"21.27,37.46 21.27,42.54\""), "{svg}");
    assert!(svg.contains("<polyline points=\"14.92,36.19 17.46,36.19\""), "{svg}");
    assert!(svg.contains("<circle cx=\"4.76\" cy=\"40.00\" r=\"0.45\""), "{svg}");
    assert!(svg.contains("<circle cx=\"35.24\" cy=\"40.00\" r=\"0.45\""), "{svg}");

    // The inductor paints four coil bumps between the same connection points.
    assert!(svg.contains("<polyline points=\"14.92,60.00 14.96,59.67"), "{svg}");
    assert!(svg.contains("25.08,60.00 32.70,60.00\""), "{svg}");

    // The LED paints a filled triangle, a cathode bar and two outward arrows.
    assert!(svg.contains("<polygon points=\"17.46,77.46 17.46,82.54 22.54,80.00\" fill=\"#1f6feb\""), "{svg}");
    assert!(svg.contains("<polyline points=\"22.54,77.46 22.54,82.54\""), "{svg}");
    assert!(svg.contains("<polyline points=\"21.27,77.46 23.81,74.92\""), "{svg}");
    assert!(svg.contains("<polyline points=\"22.54,74.92 23.81,74.92\""), "{svg}");
    assert!(svg.contains("<circle cx=\"4.76\" cy=\"80.00\" r=\"0.45\""), "{svg}");
    assert!(svg.contains("<circle cx=\"35.24\" cy=\"80.00\" r=\"0.45\""), "{svg}");

    // The transistor keeps its box and pin artwork and gains the ring.
    assert!(svg.contains("<rect x=\"7.30\" y=\"92.38\" width=\"25.40\" height=\"15.24\""), "{svg}");
    assert!(svg.contains("<polyline points=\"35.88,100.00"), "{svg}");

    // The four-pin IC is untouched, box, stubs and inside-box labels alike.
    assert!(svg.contains("<rect x=\"47.30\" y=\"32.38\" width=\"25.40\" height=\"15.24\""), "{svg}");
    assert!(svg.contains("<text x=\"48.20\" y=\"39.43\" font-family=\"monospace\" font-size=\"2.0\""), "{svg}");
    assert!(svg.contains("<text x=\"60.00\" y=\"39.40\" font-family=\"monospace\" font-size=\"3.2\""), "{svg}");
    close(handle).expect("close");
}

#[test]
fn pdf_replaces_the_resistor_box_with_the_zigzag_and_keeps_stub_endpoints() {
    let dir = TempDir::new("symbol-pdf");
    let path = dir.join("blinker.pdf");
    let created =
        create(CreateRequest { title: Some("Blinker".to_string()), rev: None, company: None, date: None, template: None })
            .expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 20.0,
            y: 20.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add R1");

    export(handle, "pdf", &path).expect("export pdf");
    let text = fs::read_to_string(&path).expect("read pdf");
    // The zigzag body between the unchanged stub roots, shifted up so the
    // drawing is centred in the printable area instead of sitting on the
    // bottom margin.
    assert!(text.contains("292.67 317.50 m 324.75 381.67 l"), "{text}");
    // The pin stub and its connection points keep their horizontal mapping.
    assert!(text.contains("0.5 w 100.17 317.50 m 36.00 317.50 l S"), "{text}");
    // The generic box path is gone.
    assert!(!text.contains("100.17 421.00 m"), "{text}");
    assert!(text.contains("(10k) Tj") && text.contains("(R1) Tj"), "{text}");
    close(handle).expect("close");
}

#[test]
fn empty_design_exports_every_supported_format_without_panic() {
    let dir = TempDir::new("empty-export");
    let created =
        create(CreateRequest { title: Some("Empty".to_string()), rev: None, company: None, date: None, template: None })
            .expect("an empty design is created");
    let handle = created["handle"].as_u64().expect("the handle is a number");
    for format in SUPPORTED_EXPORTS {
        let target = dir.join(&format!("empty.{format}"));
        let report = export(handle, format, &target).unwrap_or_else(|error| panic!("`{format}` failed: {}", error.message));
        let bytes = report["bytes"].as_u64().expect("the report counts bytes");
        assert!(bytes > 0, "`{format}` wrote an empty file for an empty design");
    }
    close(handle).expect("close");
}

#[test]
fn kicad_net_export_projects_components_and_nets_and_round_trips() {
    let dir = TempDir::new("kicad-net");
    let path = dir.join("blinker.kicad_net");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, value) in [("R1", "10k"), ("C1", "100n")] {
        add_component(
            handle,
            AddComponentRequest {
                sheet: None,
                reference: reference.to_string(),
                value: value.to_string(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
            },
        )
        .expect("add component");
    }
    set_component_footprint(
        handle,
        SetFootprintRequest { sheet: None, reference: "R1".to_string(), footprint: "Resistor_SMD:R_0805".to_string() },
    )
    .expect("set footprint");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect VCC");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "GND".to_string(), nodes: vec!["R1.2".to_string()] })
        .expect("connect GND");

    let report = export(handle, "kicad_net", &path).expect("export kicad_net");
    assert_eq!(report["format"], "kicad_net");
    assert_eq!(report["artifact"]["components"], 2);
    assert_eq!(report["artifact"]["nets"], 2);
    assert_eq!(report["artifact"]["nodes"], 3);

    let text = fs::read_to_string(&path).expect("read netlist");
    assert!(text.starts_with("(export (version \"D\")\n"), "{text}");
    assert!(
        text.contains("(comp (ref \"R1\")\n      (value \"10k\")\n      (footprint \"Resistor_SMD:R_0805\")\n      (lib_id \"open-eda:2pin\")\n    )"),
        "{text}"
    );
    assert!(text.contains("(comp (ref \"C1\")\n      (value \"100n\")\n      (footprint \"\")"), "{text}");
    assert!(
        text.contains("(net (code \"1\") (name \"VCC\")\n      (node (ref \"R1\") (pin \"1\"))\n      (node (ref \"C1\") (pin \"1\"))\n    )"),
        "{text}"
    );
    assert!(text.contains("(net (code \"2\") (name \"GND\")\n      (node (ref \"R1\") (pin \"2\"))\n    )"), "{text}");

    let again = dir.join("again.kicad_net");
    export(handle, "kicad_net", &again).expect("re-export");
    assert_eq!(fs::read_to_string(&again).expect("read again"), text, "the projection is byte-stable");

    let imported = import_kicad_net(&text).expect("import netlist");
    assert_eq!(imported["losses"], json!([]), "{:?}", imported["losses"]);
    assert_eq!(imported["totals"], json!({ "components": 2, "nets": 2, "pins": 3 }));
    let diff = compare_kicad_net(handle, &text).expect("compare netlist");
    assert_eq!(diff["equal"], true, "{diff}");
    close(handle).expect("close");
}

#[test]
fn png_export_writes_a_raster_preview_and_accepts_the_sheet_selection() {
    let dir = TempDir::new("png-export");
    let path = dir.join("preview.png");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: "R1".to_string(),
            value: "10k".to_string(),
            x: 40.0,
            y: 30.0,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add component");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [10.0, 10.0], to: [90.0, 10.0] })
        .expect("add wire");

    let report = export(handle, "png", &path).expect("export png");
    assert_eq!(report["format"], "png");
    assert_eq!(report["artifact"]["kind"], "png");
    assert_eq!(report["artifact"]["width"], 1024);
    assert_eq!(report["artifact"]["height"], 768);
    assert_eq!(report["artifact"]["components"], 1);
    assert_eq!(report["artifact"]["wires"], 1);
    let bytes = fs::read(&path).expect("read png");
    assert_eq!(bytes.len(), report["bytes"].as_u64().expect("bytes") as usize);
    assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "the file is a PNG");

    let again = dir.join("again.png");
    export(handle, "png", &again).expect("re-export");
    assert_eq!(fs::read(&again).expect("read again"), bytes, "the raster is byte-stable");

    let selected = dir.join("sheet.png");
    let sheet_report = export_with_options(handle, "png", &selected, None, Some("Sheet 1")).expect("export sheet");
    assert_eq!(sheet_report["artifact"]["components"], 1);
    let unknown = export_with_options(handle, "png", &selected, None, Some("Nope")).expect_err("unknown sheet");
    assert_eq!(unknown.code, "sheet/not-found");
    let variant = export_with_variant(handle, "png", &selected, Some("missing")).expect_err("variant rejected");
    assert_eq!(variant.code, "session/argument-invalid");
    close(handle).expect("close");
}
