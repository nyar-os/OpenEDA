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
