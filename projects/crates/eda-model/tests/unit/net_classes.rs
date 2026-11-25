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

fn bus(id: &str, name: &str, width: u64, aliases: &[&str]) -> Bus {
    Bus {
        id: id.to_string(),
        name: name.to_string(),
        width,
        aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
    }
}

fn class(id: &str, name: &str, nets: &[&str], buses: &[&str]) -> NetClass {
    NetClass {
        id: id.to_string(),
        name: name.to_string(),
        width: 0.25,
        clearance: 0.1,
        via: 0.2,
        nets: nets.iter().map(|net| (*net).to_string()).collect(),
        buses: buses.iter().map(|bus| (*bus).to_string()).collect(),
    }
}

fn make_design(sheets: Vec<Sheet>, buses: Vec<Bus>, classes: Vec<NetClass>) -> Design {
    Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 7,
        units: "mm".to_string(),
        sheets,
        buses,
        variants: Vec::new(),
        rules: DesignRules { net_classes: classes, ..DesignRules::default() },
        hierarchy: Vec::new(),
        next_component: 1,
        next_net: 1,
        next_wire: 1,
    }
}

fn net_class_issues(design: &Design) -> Vec<Value> {
    inspect_design(design, &["net_classes".to_string()])
}

fn find<'a>(issues: &'a [Value], code: &str) -> &'a Value {
    issues.iter().find(|issue| issue["code"] == code).unwrap_or_else(|| panic!("missing `{code}` finding: {issues:?}"))
}

#[test]
fn a_class_that_assigns_declared_nets_and_buses_has_no_findings() {
    let design = make_design(
        vec![sheet("sheet-1", vec![net("net-1", "VCC"), net("net-2", "DATA0")])],
        vec![bus("bus-1", "DATA", 2, &[])],
        vec![class("class-1", "Power", &["VCC"], &["DATA"])],
    );

    assert!(net_class_issues(&design).is_empty());
}

#[test]
fn assignments_to_undeclared_nets_and_buses_are_warnings_on_the_class() {
    let design = make_design(
        vec![sheet("sheet-1", vec![net("net-1", "GND")])],
        vec![bus("bus-1", "DATA", 2, &["D"])],
        vec![class("class-1", "Power", &["VCC", "VBAT"], &["MISSING"])],
    );

    let issues = net_class_issues(&design);
    let unknown_net = find(&issues, "net_class/unknown-net");
    assert_eq!(unknown_net["severity"], "warning");
    assert_eq!(unknown_net["objectId"], "class-1");
    assert_eq!(unknown_net["sheetId"], "");
    let message = unknown_net["message"].as_str().expect("message");
    assert!(message.contains("`VCC`") && message.contains("`VBAT`"), "{message}");

    let unknown_bus = find(&issues, "net_class/unknown-bus");
    assert_eq!(unknown_bus["severity"], "warning");
    assert_eq!(unknown_bus["objectId"], "class-1");
    assert!(unknown_bus["message"].as_str().expect("message").contains("`MISSING`"));

    // An alias resolves to the declared bus, so only the missing name reports.
    let by_alias = make_design(
        vec![sheet("sheet-1", vec![net("net-1", "GND")])],
        vec![bus("bus-1", "DATA", 2, &["D"])],
        vec![class("class-1", "Power", &[], &["D"])],
    );
    assert!(net_class_issues(&by_alias).is_empty());
}

#[test]
fn a_net_claimed_by_two_classes_is_one_error_on_that_net() {
    let design = make_design(
        vec![sheet("sheet-1", vec![net("net-1", "VCC")])],
        Vec::new(),
        vec![class("class-1", "Power", &["VCC"], &[]), class("class-2", "Supply", &["VCC"], &[])],
    );

    let issues = net_class_issues(&design);
    assert_eq!(issues.len(), 1, "{issues:?}");
    let shared = find(&issues, "net_class/shared-net");
    assert_eq!(shared["severity"], "error");
    assert_eq!(shared["sheetId"], "sheet-1");
    assert_eq!(shared["objectId"], "net-1");
    let message = shared["message"].as_str().expect("message");
    assert!(message.contains("`Power`") && message.contains("`Supply`"), "{message}");
}

#[test]
fn a_direct_assignment_that_overlaps_a_bus_assignment_is_an_error() {
    let design = make_design(
        vec![sheet("sheet-1", vec![net("net-1", "VCC")])],
        vec![bus("bus-1", "VCC", 1, &[])],
        vec![class("class-1", "Power", &["VCC"], &[]), class("class-2", "Supply", &[], &["VCC"])],
    );

    let issues = net_class_issues(&design);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(find(&issues, "net_class/shared-net")["objectId"], "net-1");
}

#[test]
fn a_bus_shared_by_two_classes_reports_once_and_does_not_repeat_its_members() {
    let design = make_design(
        vec![sheet("sheet-1", vec![net("net-1", "DATA0"), net("net-2", "DATA1")])],
        vec![bus("bus-1", "DATA", 2, &[])],
        vec![class("class-1", "Fast", &[], &["DATA"]), class("class-2", "Slow", &[], &["DATA"])],
    );

    let issues = net_class_issues(&design);
    assert_eq!(issues.len(), 1, "{issues:?}");
    let shared = find(&issues, "net_class/shared-bus");
    assert_eq!(shared["severity"], "error");
    assert_eq!(shared["objectId"], "bus-1");
    assert_eq!(shared["sheetId"], "");
}

#[test]
fn an_empty_class_is_a_warning_and_runs_are_deterministic() {
    let design = make_design(Vec::new(), Vec::new(), vec![class("class-1", "Unused", &[], &[])]);

    let first = net_class_issues(&design);
    let second = net_class_issues(&design);
    assert_eq!(first, second);
    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(first[0]["code"], "net_class/empty");
    assert_eq!(first[0]["severity"], "warning");
    assert_eq!(first[0]["revision"], 7);
    assert_eq!(design.revision, 7);
}

#[test]
fn the_check_only_runs_when_it_is_selected() {
    let design = make_design(Vec::new(), Vec::new(), vec![class("class-1", "Unused", &[], &[])]);

    assert!(inspect_design(&design, &["bus".to_string()]).is_empty());
}
