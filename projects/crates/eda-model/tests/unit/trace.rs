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

fn place(handle: u64, sheet: Option<&str>, reference: &str, x: f64, y: f64) {
    add_component(
        handle,
        AddComponentRequest {
            sheet: sheet.map(str::to_string),
            reference: reference.to_string(),
            value: "10k".to_string(),
            x,
            y,
            rotation: 0.0,
            pins: vec!["1".to_string(), "2".to_string()],
        },
    )
    .expect("add component");
}

fn connect(handle: u64, sheet: Option<&str>, net: &str, nodes: &[&str]) {
    connect_net(
        handle,
        ConnectNetRequest {
            sheet: sheet.map(str::to_string),
            net: net.to_string(),
            nodes: nodes.iter().map(|node| (*node).to_string()).collect(),
        },
    )
    .expect("connect net");
}

fn wire(handle: u64, sheet: Option<&str>, net: &str, from: [f64; 2], to: [f64; 2]) {
    add_wire(handle, AddWireRequest { sheet: sheet.map(str::to_string), net: Some(net.to_string()), from, to })
        .expect("add wire");
}

fn label(handle: u64, sheet: Option<&str>, text: &str, scope: LabelScope, x: f64, y: f64) {
    add_label(
        handle,
        AddLabelRequest {
            sheet: sheet.map(str::to_string),
            text: text.to_string(),
            x,
            y,
            scope: Some(scope),
            rotation: None,
            justify: None,
        },
    )
    .expect("add label");
}

fn hop_kinds(report: &Value) -> Vec<&str> {
    report["hops"].as_array().expect("hops").iter().map(|hop| hop["type"].as_str().expect("type")).collect()
}

#[test]
fn trace_connects_two_pins_on_one_net_through_a_wire() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 40.0, 30.0);
    place(handle, None, "R2", 70.0, 30.0);
    connect(handle, None, "VCC", &["R1.1", "R2.1"]);
    wire(handle, None, "VCC", [40.0, 30.0], [70.0, 30.0]);

    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace");
    assert_eq!(report["format"], "open-eda-trace");
    assert_eq!(report["version"], 1);
    assert_eq!(report["from"], "R1.1");
    assert_eq!(report["to"], "R2.1");
    assert_eq!(report["connected"], true);
    assert_eq!(report["via"], "wire");
    assert_eq!(hop_kinds(&report), vec!["pin", "wire", "pin"]);
    assert_eq!(report["sheets"], json!(["Sheet 1"]));
    let hops = report["hops"].as_array().expect("hops");
    assert_eq!(hops[0]["name"], "R1.1");
    assert_eq!(hops[0]["sheet"], "Sheet 1");
    assert_eq!(hops[1]["index"], 0);
    assert!(hops[1]["detail"].as_str().expect("detail").contains("wire-1"));
    close(handle).expect("close");
}

#[test]
fn trace_walks_a_junction_that_joins_crossing_wires() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, None, "R2", 90.0, 10.0);
    connect(handle, None, "A", &["R1.1"]);
    connect(handle, None, "B", &["R2.1"]);
    wire(handle, None, "A", [0.0, 50.0], [100.0, 50.0]);
    wire(handle, None, "B", [50.0, 0.0], [50.0, 100.0]);

    let before = trace_pins(handle, "R1.1", "R2.1").expect("trace before");
    assert_eq!(before["connected"], false);
    assert_eq!(before["via"], "none");
    assert_eq!(before["hops"], json!([]));
    assert_eq!(before["sheets"], json!([]));

    add_junction(handle, AddJunctionRequest { sheet: None, x: 50.0, y: 50.0 }).expect("add junction");
    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace after");
    assert_eq!(report["connected"], true);
    assert_eq!(report["via"], "wire");
    assert_eq!(hop_kinds(&report), vec!["pin", "wire", "junction", "wire", "pin"]);
    let hops = report["hops"].as_array().expect("hops");
    assert_eq!(hops[2]["index"], 0);
    assert!(hops[2]["detail"].as_str().expect("detail").contains("junction"));
    close(handle).expect("close");
}

#[test]
fn trace_falls_back_to_matching_labels() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, None, "R2", 90.0, 10.0);
    connect(handle, None, "A", &["R1.1"]);
    connect(handle, None, "B", &["R2.1"]);
    wire(handle, None, "A", [0.0, 0.0], [20.0, 0.0]);
    wire(handle, None, "B", [80.0, 0.0], [100.0, 0.0]);
    label(handle, None, "SIG", LabelScope::Local, 0.0, 0.0);
    label(handle, None, "SIG", LabelScope::Local, 100.0, 0.0);

    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace");
    assert_eq!(report["connected"], true);
    assert_eq!(report["via"], "label");
    assert_eq!(hop_kinds(&report), vec!["pin", "wire", "label", "label", "wire", "pin"]);
    let hops = report["hops"].as_array().expect("hops");
    assert_eq!(hops[2]["name"], "SIG");
    assert_eq!(hops[2]["index"], 0);
    close(handle).expect("close");
}

#[test]
fn trace_crosses_sheets_through_global_labels_only() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_sheet(handle, Some("Power")).expect("add sheet");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, Some("Power"), "R2", 10.0, 10.0);
    connect(handle, None, "A", &["R1.1"]);
    connect(handle, Some("Power"), "B", &["R2.1"]);
    wire(handle, None, "A", [0.0, 0.0], [20.0, 0.0]);
    wire(handle, Some("Power"), "B", [0.0, 0.0], [20.0, 0.0]);

    label(handle, None, "VCC", LabelScope::Local, 0.0, 0.0);
    label(handle, Some("Power"), "VCC", LabelScope::Local, 0.0, 0.0);
    let local = trace_pins(handle, "R1.1", "R2.1").expect("trace local");
    assert_eq!(local["connected"], false, "a local label stays on its sheet");

    label(handle, None, "VCC", LabelScope::Global, 0.0, 0.0);
    label(handle, Some("Power"), "VCC", LabelScope::Global, 0.0, 0.0);
    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace global");
    assert_eq!(report["connected"], true);
    assert_eq!(report["via"], "label");
    assert_eq!(report["sheets"], json!(["Sheet 1", "Power"]));
    let kinds = hop_kinds(&report);
    assert_eq!(kinds.first(), Some(&"pin"));
    assert_eq!(kinds.last(), Some(&"pin"));
    assert!(kinds.contains(&"label"));
    close(handle).expect("close");
}

#[test]
fn trace_crosses_hierarchical_labels_through_a_sheet_port() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_sheet(handle, Some("Power")).expect("add sheet");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, Some("Power"), "R2", 10.0, 10.0);
    connect(handle, None, "A", &["R1.1"]);
    connect(handle, Some("Power"), "B", &["R2.1"]);
    wire(handle, None, "A", [0.0, 0.0], [20.0, 0.0]);
    wire(handle, Some("Power"), "B", [0.0, 0.0], [20.0, 0.0]);
    label(handle, None, "IN", LabelScope::Hierarchical, 0.0, 0.0);
    label(handle, Some("Power"), "IN", LabelScope::Hierarchical, 0.0, 0.0);

    let before = trace_pins(handle, "R1.1", "R2.1").expect("trace before");
    assert_eq!(before["connected"], false, "a hierarchical label alone does not cross sheets");

    add_hierarchy(
        handle,
        AddHierarchyRequest {
            name: "Power block".to_string(),
            file: "power.oeda".to_string(),
            x: 0.0,
            y: 0.0,
            width: 60.0,
            height: 40.0,
        },
    )
    .expect("add hierarchy");
    add_hierarchy_port(
        handle,
        AddHierarchyPortRequest {
            sheet: "Power block".to_string(),
            name: "IN".to_string(),
            direction: PortDirection::Input,
            x: 0.0,
            y: 0.0,
        },
    )
    .expect("add port");

    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace after");
    assert_eq!(report["connected"], true);
    assert_eq!(report["via"], "label");
    assert_eq!(hop_kinds(&report), vec!["pin", "wire", "label", "sheet-port", "label", "wire", "pin"]);
    assert_eq!(report["sheets"], json!(["Sheet 1", "Power"]));
    let hops = report["hops"].as_array().expect("hops");
    assert_eq!(hops[3]["name"], "IN");
    assert_eq!(hops[3]["index"], 0);
    close(handle).expect("close");
}

#[test]
fn trace_reports_disconnected_pins_without_an_error() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, None, "R2", 90.0, 10.0);
    connect(handle, None, "A", &["R1.1"]);
    connect(handle, None, "B", &["R2.1"]);

    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace");
    assert_eq!(report["connected"], false);
    assert_eq!(report["via"], "none");
    assert_eq!(report["hops"], json!([]));
    assert_eq!(report["sheets"], json!([]));
    close(handle).expect("close");
}

#[test]
fn trace_accepts_a_pin_to_itself_as_a_direct_zero_hop_path() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);

    let report = trace_pins(handle, "R1.1", "R1.1").expect("trace");
    assert_eq!(report["connected"], true);
    assert_eq!(report["via"], "direct");
    assert_eq!(report["hops"], json!([]));
    assert_eq!(report["sheets"], json!([]));
    close(handle).expect("close");
}

#[test]
fn trace_rejects_unknown_references_and_pins() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);

    let missing_component = trace_pins(handle, "R9.1", "R1.1").expect_err("missing component");
    assert_eq!(missing_component.code, "trace/component-missing");
    let missing_pin = trace_pins(handle, "R1.9", "R1.1").expect_err("missing pin");
    assert_eq!(missing_pin.code, "trace/pin-missing");
    let malformed = trace_pins(handle, "R1", "R1.1").expect_err("malformed");
    assert_eq!(malformed.code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn trace_takes_the_shortest_path_and_breaks_ties_by_index() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, None, "R2", 90.0, 10.0);
    connect(handle, None, "VCC", &["R1.1", "R2.1"]);
    wire(handle, None, "VCC", [0.0, 0.0], [20.0, 0.0]);
    wire(handle, None, "VCC", [80.0, 0.0], [100.0, 0.0]);

    let report = trace_pins(handle, "R1.1", "R2.1").expect("trace");
    assert_eq!(report["connected"], true);
    assert_eq!(hop_kinds(&report), vec!["pin", "wire", "pin"]);
    let hops = report["hops"].as_array().expect("hops");
    assert_eq!(hops[1]["index"], 0, "the lower wire index wins the tie");
    let repeated = trace_pins(handle, "R1.1", "R2.1").expect("trace again");
    assert_eq!(report, repeated);
    close(handle).expect("close");
}

#[test]
fn trace_reads_without_writing_the_document() {
    let directory = TempDir::new("trace-read-only");
    let document = directory.join("trace.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    place(handle, None, "R1", 10.0, 10.0);
    place(handle, None, "R2", 90.0, 10.0);
    connect(handle, None, "VCC", &["R1.1", "R2.1"]);
    wire(handle, None, "VCC", [0.0, 0.0], [20.0, 0.0]);
    save(handle, &document, false).expect("save");
    let before = fs::read_to_string(&document).expect("read before");
    let revision = summary(handle).expect("summary")["revision"].clone();

    trace_pins(handle, "R1.1", "R2.1").expect("trace");

    let after = fs::read_to_string(&document).expect("read after");
    assert_eq!(before, after);
    assert_eq!(summary(handle).expect("summary")["revision"], revision);
    close(handle).expect("close");
}
