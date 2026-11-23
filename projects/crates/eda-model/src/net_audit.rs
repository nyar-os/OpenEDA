//! Per-net schematic health report.
//!
//! The report answers one question about every declared net: what it connects,
//! how much drawing it owns and whether that is enough to be meaningful. It is a
//! read-only projection of the model arrays, so it never opens a child sheet,
//! never writes a document and never bumps the revision.

use serde_json::{Value, json};

use crate::{Design, ServiceError, net_class_for_net, segment_length, with_session};

/// Tag written into the read-only net audit report.
const NET_AUDIT_FORMAT: &str = "open-eda-net-audit";

/// Version written into the read-only net audit report.
const NET_AUDIT_VERSION: u64 = 1;

/// Code reported for a declared net that has exactly one node.
const SINGLE_PIN: &str = "net/single-pin";

/// Code reported for a declared net that has no nodes.
const EMPTY: &str = "net/empty";

/// Code reported for a declared net with several nodes but no wire or label.
const NO_WIRE: &str = "net/no-wire";

/// Rounds one length to the four decimal places the other reports print.
fn rounded(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// Reports the per-net health of one open design.
///
/// Every declared net becomes one `nets` entry in document order (sheet order,
/// then declaration order). An entry keeps the declaration's node, wire and label
/// counts, its longest drawing length and the net class that claims the name.
/// Three warning codes cover a net that is too small to matter: `net/empty`,
/// `net/single-pin` and `net/no-wire`. The report is deterministic for one
/// revision and never mutates the design.
pub fn net_audit(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| Ok(audit_of(&session.design)))
}

fn audit_of(design: &Design) -> Value {
    let mut nets = Vec::new();
    let mut issues = Vec::new();
    let mut pin_count = 0usize;
    let mut wire_count = 0usize;
    let mut total_length = 0.0;

    for sheet in &design.sheets {
        for net in &sheet.nets {
            let pins = net
                .nodes
                .iter()
                .filter(|node| {
                    design.sheets.iter().any(|other| {
                        other
                            .components
                            .iter()
                            .any(|component| component.reference == node.reference && component.pins.contains(&node.pin))
                    })
                })
                .count();
            let wires: Vec<_> = sheet.wires.iter().filter(|wire| wire.net.as_deref() == Some(net.name.as_str())).collect();
            let labels = sheet.labels.iter().filter(|label| label.text == net.name).count();
            let length = wires.iter().map(|wire| segment_length(wire.from, wire.to)).sum::<f64>();
            let nodes = net.nodes.len();
            let connected = nodes >= 2 || !wires.is_empty() || labels > 0;

            pin_count += nodes;
            wire_count += wires.len();
            total_length += length;

            let issue = if nodes == 0 {
                Some((EMPTY, "declared net has no nodes"))
            }
            else if nodes == 1 {
                Some((SINGLE_PIN, "declared net has exactly one node"))
            }
            else if wires.is_empty() && labels == 0 {
                Some((NO_WIRE, "declared net has nodes but no wire or label"))
            }
            else {
                None
            };
            if let Some((code, message)) = issue {
                issues.push(json!({
                    "code": code,
                    "severity": "warning",
                    "net": net.name,
                    "sheet": sheet.id,
                    "message": message,
                }));
            }

            nets.push(json!({
                "name": net.name,
                "class": net_class_for_net(design, &net.name).map(|class| class.name.clone()),
                "nodes": nodes,
                "pins": pins,
                "wires": wires.len(),
                "labels": labels,
                "junctions": 0,
                "length": rounded(length),
                "connected": connected,
            }));
        }
    }

    let status = if issues.is_empty() { "pass" } else { "warn" };
    json!({
        "report": NET_AUDIT_FORMAT,
        "version": NET_AUDIT_VERSION,
        "revision": design.revision,
        "netCount": nets.len(),
        "pinCount": pin_count,
        "wireCount": wire_count,
        "totalLength": rounded(total_length),
        "status": status,
        "nets": nets,
        "issues": issues,
    })
}
