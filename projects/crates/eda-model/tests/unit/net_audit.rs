use super::*;

/// Seeds one design with a healthy net, one net per warning code and a node that
/// names no placed pin.
fn seeded_design() -> u64 {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    for (reference, value) in [("R1", "10k"), ("C1", "100n"), ("U1", "MCU")] {
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
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.1".to_string(), "C1.1".to_string()] },
    )
    .expect("connect VCC");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [3.0, 4.0] })
        .expect("add wire");
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [3.0, 4.0], to: [3.0, 6.0] })
        .expect("add wire");
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
    add_net_class(
        handle,
        NetClassRequest {
            name: "Power".to_string(),
            width: 0.2,
            clearance: 0.1,
            via: 0.3,
            nets: vec!["VCC".to_string()],
            buses: Vec::new(),
        },
    )
    .expect("add net class");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "GND".to_string(), nodes: vec!["R1.2".to_string()] })
        .expect("connect GND");
    connect_net(
        handle,
        ConnectNetRequest { sheet: None, net: "LINK".to_string(), nodes: vec!["C1.2".to_string(), "U1.1".to_string()] },
    )
    .expect("connect LINK");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "EMPTY".to_string(), nodes: Vec::new() }).expect("declare EMPTY");
    // `connect_net` only accepts placed pins, so the dangling node is pushed
    // directly to prove the report counts it as a node but not as a pin.
    mutate(handle, |design| {
        design.sheets[0].nets.push(Net {
            id: "net-ghost".to_string(),
            name: "GHOST".to_string(),
            nodes: vec![Node { reference: "X9".to_string(), pin: "1".to_string() }],
        });
        Ok(Map::new())
    })
    .expect("push ghost net");
    handle
}

fn revision(handle: u64) -> u64 {
    list(handle).expect("list")["revision"].as_u64().expect("revision")
}

#[test]
fn audit_reports_counts_length_class_and_every_issue_code() {
    let handle = seeded_design();
    let before = revision(handle);
    let report = net_audit(handle).expect("audit");

    assert_eq!(report["report"], "open-eda-net-audit");
    assert_eq!(report["status"], "warn");
    assert_eq!(report["netCount"], 5);
    assert_eq!(report["pinCount"], 6, "one count per declared node, including the dangling one");
    assert_eq!(report["wireCount"], 2);
    assert_eq!(report["totalLength"], 7.0, "3-4-5 plus a 2 unit segment");

    let names: Vec<&str> =
        report["nets"].as_array().expect("nets").iter().map(|net| net["name"].as_str().expect("name")).collect();
    assert_eq!(names, vec!["VCC", "GND", "LINK", "EMPTY", "GHOST"], "declaration order");

    let vcc = &report["nets"][0];
    assert_eq!(vcc["class"], "Power");
    assert_eq!(vcc["nodes"], 2);
    assert_eq!(vcc["pins"], 2);
    assert_eq!(vcc["wires"], 2);
    assert_eq!(vcc["labels"], 1);
    assert_eq!(vcc["junctions"], 0);
    assert_eq!(vcc["length"], 7.0);
    assert_eq!(vcc["connected"], true);

    let gnd = &report["nets"][1];
    assert_eq!(gnd["class"], Value::Null);
    assert_eq!(gnd["connected"], false, "one node and no drawing is not connected");

    let link = &report["nets"][2];
    assert_eq!(link["nodes"], 2);
    assert_eq!(link["connected"], true, "two nodes are enough even without a wire");

    let empty = &report["nets"][3];
    assert_eq!(empty["nodes"], 0);
    assert_eq!(empty["connected"], false);

    let ghost = &report["nets"][4];
    assert_eq!(ghost["nodes"], 1);
    assert_eq!(ghost["pins"], 0, "a node that names no placed pin is not a pin");

    let codes: Vec<&str> =
        report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().expect("code")).collect();
    assert_eq!(codes, vec!["net/single-pin", "net/no-wire", "net/empty", "net/single-pin"]);
    for issue in report["issues"].as_array().expect("issues") {
        assert_eq!(issue["severity"], "warning");
        assert_eq!(issue["sheet"], "sheet-1");
    }
    assert_eq!(report["issues"][0]["net"], "GND");
    assert_eq!(report["issues"][1]["net"], "LINK");
    assert_eq!(report["issues"][2]["net"], "EMPTY");
    assert_eq!(report["issues"][3]["net"], "GHOST");

    assert_eq!(revision(handle), before, "the audit never bumps the revision");
    close(handle).expect("close");
}

#[test]
fn a_healthy_design_passes_and_repeated_audits_are_identical() {
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
    add_wire(handle, AddWireRequest { sheet: None, net: Some("VCC".to_string()), from: [0.0, 0.0], to: [10.0, 0.0] })
        .expect("add wire");

    let first = net_audit(handle).expect("audit");
    let second = net_audit(handle).expect("audit");
    assert_eq!(first, second, "the same document always produces the same report");
    assert_eq!(first["status"], "pass");
    assert_eq!(first["issues"], json!([]));
    close(handle).expect("close");
}

#[test]
fn an_unknown_or_closed_handle_is_design_not_found() {
    assert_eq!(net_audit(9_999_999).unwrap_err().code, "design/not-found");

    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    close(handle).expect("close");
    assert_eq!(net_audit(handle).unwrap_err().code, "design/not-found");
}
