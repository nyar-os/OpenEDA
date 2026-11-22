use super::*;

fn net(id: &str, name: &str) -> Net {
    Net { id: id.to_string(), name: name.to_string(), nodes: Vec::new() }
}

fn sheet(id: &str, nets: Vec<Net>) -> Sheet {
    Sheet {
        id: id.to_string(),
        name: "Sheet 1".to_string(),
        components: Vec::new(),
        nets,
        wires: Vec::new(),
        junctions: Vec::new(),
        no_connects: Vec::new(),
        texts: Vec::new(),
        labels: Vec::new(),
    }
}

fn design(sheets: Vec<Sheet>, buses: Vec<Bus>) -> Design {
    Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets,
        buses,
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 1,
        next_net: 1,
        next_wire: 1,
    }
}

fn bus(id: &str, name: &str, width: u64, aliases: &[&str]) -> Bus {
    Bus {
        id: id.to_string(),
        name: name.to_string(),
        width,
        aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
    }
}

fn bus_issues(design: &Design) -> Vec<Value> {
    inspect_design(design, &["bus".to_string()])
}

fn find<'a>(issues: &'a [Value], code: &str) -> &'a Value {
    issues.iter().find(|issue| issue["code"] == code).unwrap_or_else(|| panic!("missing `{code}` finding: {issues:?}"))
}

#[test]
fn a_bus_backed_by_its_net_or_alias_has_no_findings() {
    let by_name = design(vec![sheet("sheet-1", vec![net("net-1", "DATA")])], vec![bus("bus-1", "DATA", 4, &[])]);
    assert!(bus_issues(&by_name).is_empty());

    let by_alias = design(vec![sheet("sheet-1", vec![net("net-1", "D")])], vec![bus("bus-1", "DATA", 4, &["D"])]);
    assert!(bus_issues(&by_alias).is_empty());
}

#[test]
fn two_buses_sharing_a_name_are_errors() {
    let design = design(vec![sheet("sheet-1", Vec::new())], vec![bus("bus-1", "DATA", 2, &[]), bus("bus-2", "DATA", 2, &[])]);

    let issues = bus_issues(&design);
    let duplicates: Vec<&Value> = issues.iter().filter(|issue| issue["code"] == "bus/duplicate-name").collect();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0]["severity"], "error");
    assert_eq!(duplicates[0]["objectId"], "bus-2");
    assert!(duplicates[0]["message"].as_str().expect("message").contains("DATA"));
}

#[test]
fn a_bus_member_no_net_provides_is_an_error() {
    let design = design(vec![sheet("sheet-1", Vec::new())], vec![bus("bus-1", "DATA", 3, &[])]);

    let issues = bus_issues(&design);
    assert_eq!(issues.len(), 1);
    let missing = find(&issues, "bus/member-missing");
    assert_eq!(missing["severity"], "error");
    assert_eq!(missing["sheetId"], "");
    assert_eq!(missing["objectId"], "bus-1");
    let message = missing["message"].as_str().expect("message");
    assert!(message.contains("DATA0, DATA1, DATA2"), "{message}");
}

#[test]
fn a_net_claimed_by_two_buses_is_an_error() {
    let design = design(
        vec![sheet(
            "sheet-1",
            vec![net("net-1", "DATA0"), net("net-2", "DATA1"), net("net-3", "DATA00"), net("net-4", "DATA01")],
        )],
        vec![bus("bus-1", "DATA", 2, &[]), bus("bus-2", "DATA0", 2, &[])],
    );

    let issues = bus_issues(&design);
    assert_eq!(issues.len(), 1);
    let shared = find(&issues, "bus/shared-net");
    assert_eq!(shared["severity"], "error");
    assert_eq!(shared["sheetId"], "sheet-1");
    assert_eq!(shared["objectId"], "net-1");
    let message = shared["message"].as_str().expect("message");
    assert!(message.contains("DATA0") && message.contains("`DATA`") && message.contains("`DATA0`"), "{message}");
}

#[test]
fn a_bus_with_zero_members_is_a_warning() {
    let design = design(vec![sheet("sheet-1", Vec::new())], vec![bus("bus-1", "DATA", 0, &[])]);

    let issues = bus_issues(&design);
    assert_eq!(issues.len(), 1);
    let empty = find(&issues, "bus/empty");
    assert_eq!(empty["severity"], "warning");
    assert_eq!(empty["objectId"], "bus-1");
    assert!(empty["message"].as_str().expect("message").contains("zero member nets"));
}

#[test]
fn the_bus_check_runs_through_the_public_surface() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add bus");

    let report = inspect(handle, Some(vec!["bus".to_string()])).expect("inspect bus");
    assert_eq!(report["passed"], false);
    let codes: Vec<&str> =
        report["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect();
    assert_eq!(codes, vec!["bus/member-missing"]);

    connect_net(handle, ConnectNetRequest { sheet: None, net: "DATA".to_string(), nodes: Vec::new() }).expect("connect DATA");
    let cleared = inspect(handle, Some(vec!["bus".to_string()])).expect("inspect bus again");
    assert_eq!(cleared["issues"], json!([]));
    assert_eq!(cleared["passed"], true);

    close(handle).expect("close");
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("open-eda-bus-{label}-{}-{sequence}", std::process::id()));
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

fn add_resistor(handle: u64) {
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
}

fn update(
    handle: u64,
    id: &str,
    name: Option<&str>,
    width: Option<u64>,
    aliases: Option<Vec<&str>>,
) -> Result<Value, ServiceError> {
    update_bus(
        handle,
        id,
        UpdateBusRequest {
            name: name.map(str::to_string),
            width,
            aliases: aliases.map(|entries| entries.into_iter().map(str::to_string).collect()),
        },
    )
}

#[test]
fn updating_a_bus_renames_only_the_declaration() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_resistor(handle);
    connect_net(handle, ConnectNetRequest { sheet: None, net: "DATA".to_string(), nodes: vec!["R1.1".to_string()] })
        .expect("connect DATA");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: vec!["D".to_string()] }).expect("add bus");
    add_net_class(
        handle,
        NetClassRequest {
            name: "BusClass".to_string(),
            width: 0.2,
            clearance: 0.0,
            via: 0.0,
            nets: vec!["DATA".to_string()],
            buses: vec!["DATA".to_string()],
        },
    )
    .expect("add net class");
    let revision = list_buses(handle).expect("list")["revision"].as_u64().expect("revision");

    let response = update(handle, "bus-1", Some("ADDR"), None, None).expect("rename");
    assert_eq!(response["changed"], true);
    assert_eq!(response["bus"]["id"], "bus-1");
    assert_eq!(response["bus"]["name"], "ADDR");
    assert_eq!(response["bus"]["width"], 2);
    assert_eq!(response["bus"]["aliases"], json!(["D"]));
    assert_eq!(response["revision"].as_u64().expect("revision"), revision + 1);

    let report = net_show(handle, "DATA", None).expect("the net is still declared");
    assert_eq!(report["name"], "DATA");
    assert_eq!(report["sheets"][0]["net"]["nodes"], json!([{ "ref": "R1", "pin": "1" }]));
    assert_eq!(report["bus"], Value::Null, "a renamed bus no longer resolves the old net name");

    let classes = list_net_classes(handle).expect("list net classes");
    assert_eq!(classes["netClasses"][0]["nets"], json!(["DATA"]), "nets are not rewritten");
    assert_eq!(classes["netClasses"][0]["buses"], json!(["DATA"]), "net-class assignments are not rewritten");

    close(handle).expect("close");
}

#[test]
fn changing_the_width_moves_only_the_derived_members() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    connect_net(handle, ConnectNetRequest { sheet: None, net: "DATA".to_string(), nodes: Vec::new() }).expect("connect DATA");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add bus");
    let before = net_show(handle, "DATA", None).expect("net show")["bus"]["members"].clone();
    assert_eq!(before, json!(["DATA0", "DATA1"]));

    let response = update(handle, "DATA", None, Some(4), None).expect("widen");
    assert_eq!(response["changed"], true);
    assert_eq!(response["bus"]["width"], 4);
    let after = net_show(handle, "DATA", None).expect("net show")["bus"]["members"].clone();
    assert_eq!(after, json!(["DATA0", "DATA1", "DATA2", "DATA3"]));

    let unchanged = update(handle, "DATA", None, Some(4), None).expect("same width");
    assert_eq!(unchanged["changed"], false);
    assert_eq!(unchanged["revision"], response["revision"], "a no-op adds no revision");

    close(handle).expect("close");
}

#[test]
fn aliases_are_replaced_as_a_whole_and_can_be_cleared() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: vec!["D".to_string(), "DIN".to_string()] })
        .expect("add bus");

    let replaced = update(handle, "DATA", None, None, Some(vec!["A", "B"])).expect("replace aliases");
    assert_eq!(replaced["changed"], true);
    assert_eq!(replaced["bus"]["aliases"], json!(["A", "B"]));

    let cleared = update(handle, "DATA", None, None, Some(Vec::new())).expect("clear aliases");
    assert_eq!(cleared["changed"], true);
    assert_eq!(cleared["bus"].get("aliases"), None, "an empty alias list is not serialized");
    let listed = list_buses(handle).expect("list");
    assert_eq!(listed["buses"][0].get("aliases"), None);

    let again = update(handle, "DATA", None, None, Some(Vec::new())).expect("clear again");
    assert_eq!(again["changed"], false);
    assert_eq!(again["revision"], cleared["revision"]);

    close(handle).expect("close");
}

#[test]
fn a_partial_update_keeps_every_untouched_field() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 4, aliases: vec!["D".to_string()] }).expect("add bus");

    let widened = update(handle, "DATA", None, Some(6), None).expect("widen");
    assert_eq!(widened["bus"]["name"], "DATA");
    assert_eq!(widened["bus"]["aliases"], json!(["D"]));

    let renamed = update(handle, "DATA", Some("ADDR"), None, None).expect("rename");
    assert_eq!(renamed["bus"]["width"], 6);
    assert_eq!(renamed["bus"]["aliases"], json!(["D"]));

    let aliased = update(handle, "ADDR", None, None, Some(vec!["A"])).expect("realias");
    assert_eq!(aliased["bus"]["name"], "ADDR");
    assert_eq!(aliased["bus"]["width"], 6);
    assert_eq!(aliased["bus"]["aliases"], json!(["A"]));

    close(handle).expect("close");
}

#[test]
fn a_rename_to_another_bus_name_is_refused() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add DATA");
    add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 2, aliases: Vec::new() }).expect("add ADDR");

    assert_eq!(update(handle, "DATA", Some("ADDR"), None, None).unwrap_err().code, "object/duplicate");
    assert_eq!(update(handle, "DATA", Some("ADDR0"), None, None).unwrap_err().code, "object/duplicate");
    assert_eq!(list_buses(handle).expect("list")["buses"][0]["name"], "DATA");

    close(handle).expect("close");
}

#[test]
fn alias_updates_reject_name_member_and_alias_collisions() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: vec!["D".to_string()] }).expect("add DATA");
    add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 2, aliases: vec!["A".to_string()] }).expect("add ADDR");

    assert_eq!(update(handle, "DATA", None, None, Some(vec!["ADDR"])).unwrap_err().code, "object/duplicate");
    assert_eq!(update(handle, "DATA", None, None, Some(vec!["ADDR0"])).unwrap_err().code, "object/duplicate");
    assert_eq!(update(handle, "DATA", None, None, Some(vec!["A"])).unwrap_err().code, "object/duplicate");
    assert_eq!(update(handle, "DATA", None, None, Some(vec!["D", "D"])).unwrap_err().code, "session/argument-invalid");
    assert_eq!(update(handle, "DATA", None, None, Some(vec!["DATA"])).unwrap_err().code, "session/argument-invalid");
    assert_eq!(update(handle, "DATA", None, None, Some(vec!["DATA0"])).unwrap_err().code, "object/duplicate");

    let listed = list_buses(handle).expect("list");
    assert_eq!(listed["buses"][0]["aliases"], json!(["D"]), "a refused update leaves the declaration untouched");

    close(handle).expect("close");
}

#[test]
fn invalid_widths_and_empty_requests_are_refused() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).expect("add bus");
    let revision = list_buses(handle).expect("list")["revision"].as_u64().expect("revision");

    assert_eq!(update(handle, "DATA", None, Some(0), None).unwrap_err().code, "session/argument-invalid");
    assert_eq!(update(handle, "DATA", None, None, None).unwrap_err().code, "session/argument-invalid");
    assert_eq!(update(handle, "DATA", Some("  "), None, None).unwrap_err().code, "session/argument-invalid");
    assert_eq!(update(handle, "DATA", None, None, Some(vec![" "])).unwrap_err().code, "session/argument-invalid");
    assert_eq!(list_buses(handle).expect("list")["revision"].as_u64().expect("revision"), revision);

    close(handle).expect("close");
}

#[test]
fn an_unknown_bus_id_is_not_found() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");

    assert_eq!(update(handle, "bus-9", Some("ADDR"), None, None).unwrap_err().code, "object/not-found");
    assert_eq!(update(handle, "DATA", None, Some(2), None).unwrap_err().code, "object/not-found");

    close(handle).expect("close");
}

#[test]
fn bus_updates_are_deterministic_and_survive_save_and_reopen() {
    let temp = TempDir::new("update");
    let path = temp.join("design.oeda");
    let created = create(CreateRequest::default()).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: vec!["D".to_string()] }).expect("add bus");

    let first = update(handle, "DATA", Some("ADDR"), Some(3), Some(vec!["A"])).expect("update");
    let second = update(handle, "ADDR", Some("ADDR"), Some(3), Some(vec!["A"])).expect("repeat update");
    let third = update(handle, "ADDR", Some("ADDR"), Some(3), Some(vec!["A"])).expect("third update");
    assert_eq!(first["bus"], second["bus"]);
    assert_eq!(second["changed"], false);
    assert_eq!(first["revision"], second["revision"], "an unchanged update adds no revision");
    assert_eq!(
        serde_json::to_string(&second).expect("second json"),
        serde_json::to_string(&third).expect("third json"),
        "repeated unchanged updates produce the same report"
    );

    save(handle, &path, false).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let reopened_handle = reopened["handle"].as_u64().expect("reopened handle");
    let listed = list_buses(reopened_handle).expect("list reopened");
    assert_eq!(listed["buses"][0]["name"], "ADDR");
    assert_eq!(listed["buses"][0]["width"], 3);
    assert_eq!(listed["buses"][0]["aliases"], json!(["A"]));
    close(reopened_handle).expect("close reopened");
}
