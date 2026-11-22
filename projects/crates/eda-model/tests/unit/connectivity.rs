
use super::*;
use crate::{Junction, Sheet};

fn wire(id: &str, net: Option<&str>, from: [f64; 2], to: [f64; 2]) -> Wire {
    Wire { id: id.to_string(), net: net.map(str::to_string), from, to }
}

fn sheet(wires: Vec<Wire>, junctions: Vec<Junction>) -> Sheet {
    Sheet {
        id: "sheet-1".to_string(),
        name: "Sheet 1".to_string(),
        components: Vec::new(),
        nets: Vec::new(),
        wires,
        junctions,
        no_connects: Vec::new(),
        texts: Vec::new(),
        labels: Vec::new(),
    }
}

#[test]
fn chain_and_tee_junctions_union_into_one_group() {
    let sheet = sheet(
        vec![
            wire("wire-1", None, [0.0, 0.0], [10.0, 0.0]),
            wire("wire-2", None, [10.0, 0.0], [20.0, 0.0]),
            wire("wire-3", None, [15.0, 0.0], [15.0, 10.0]),
            wire("wire-4", None, [20.0, 0.0], [30.0, 0.0]),
        ],
        vec![Junction { x: 15.0, y: 0.0 }],
    );
    let groups = groups(&sheet);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].wires, vec![0, 1, 2, 3]);
}

#[test]
fn crossing_interiors_do_not_connect() {
    let sheet =
        sheet(vec![wire("wire-1", None, [0.0, 0.0], [10.0, 10.0]), wire("wire-2", None, [0.0, 10.0], [10.0, 0.0])], Vec::new());
    assert_eq!(groups(&sheet).len(), 2);
}

#[test]
fn conflicts_unassigned_groups_and_floating_junctions_are_reported() {
    let sheet = sheet(
        vec![
            wire("wire-1", Some("VCC"), [0.0, 0.0], [10.0, 0.0]),
            wire("wire-2", Some("GND"), [10.0, 0.0], [20.0, 0.0]),
            wire("wire-3", None, [0.0, 50.0], [10.0, 50.0]),
        ],
        vec![Junction { x: 100.0, y: 100.0 }],
    );
    let findings = findings(&sheet);
    let codes: Vec<&str> = findings.iter().map(|finding| finding.code).collect();
    assert!(codes.contains(&WIRE_NET_CONFLICT));
    assert!(codes.contains(&WIRE_NET_UNASSIGNED));
    assert!(codes.contains(&JUNCTION_FLOATING));
    let conflict = findings.iter().find(|finding| finding.code == WIRE_NET_CONFLICT).expect("conflict");
    assert!(conflict.message.contains("VCC"));
    assert!(conflict.message.contains("GND"));
    assert_eq!(conflict.object_id, "wire-1");
}
