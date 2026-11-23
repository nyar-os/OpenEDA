//! Dangling wire endpoint inspection for one schematic sheet.
//!
//! A wire endpoint is dangling when nothing anchors it: no other wire endpoint
//! or interior, no placed component, no junction, no net-label anchor, no
//! no-connect marker and, on the root sheet, no sheet-port anchor. The rule
//! reuses the connectivity module's [`point_eq`] and [`on_wire`] helpers, so it
//! agrees with the `connectivity` check and `erc/floating-wire` instead of
//! inventing a second epsilon.
//!
//! Two model boundaries follow the rest of the crate. The model stores component
//! placement but not pin geometry, so a component anchors through its placement
//! point exactly as `connectivity::endpoint_attached` already does. Bus entries
//! are not modelled at all, so an endpoint can never be attached to one.
//! Hierarchical sheet ports belong to the root sheet declaration, matching the
//! pin trace, and are checked only there.
//!
//! A zero-length wire, whose two endpoints coincide within the connectivity
//! tolerance, is reported once at endpoint 0 (the `from` point) instead of twice
//! at the same coordinate.
//!
//! The module only inspects and reports. It never opens a child document: the
//! check sees the sheets already present in the design.

use crate::{
    Design, Sheet, Wire,
    connectivity::{on_wire, point_eq},
};

/// Stable code reported for one wire endpoint that touches nothing.
pub const DANGLING_ENDPOINT: &str = "wire/dangling-endpoint";

/// One dangling endpoint finding of one sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// 0-based index of the wire in [`Sheet::wires`].
    pub wire_index: usize,
    /// Identity of the wire the endpoint belongs to.
    pub wire_id: String,
    /// Endpoint index: 0 is `from`, 1 is `to`.
    pub endpoint: usize,
    /// Endpoint X coordinate.
    pub x: f64,
    /// Endpoint Y coordinate.
    pub y: f64,
    /// Human-readable message, deterministic for one sheet state.
    pub message: String,
}

/// Whether `point` is anchored by something other than `wire`.
///
/// Junctions, no-connect markers and label anchors attach by coordinate. A
/// component attaches through its placement point, because the model stores no
/// pin geometry. On the root sheet a hierarchy port anchor also attaches. Any
/// other wire attaches when the point lies on its segment, which covers a shared
/// endpoint and a T-junction; the wire itself is excluded.
pub fn endpoint_attached(design: &Design, sheet: &Sheet, root: bool, wire: &Wire, point: [f64; 2]) -> bool {
    if sheet.junctions.iter().any(|junction| point_eq(point, [junction.x, junction.y])) {
        return true;
    }
    if sheet.no_connects.iter().any(|flag| point_eq(point, [flag.x, flag.y])) {
        return true;
    }
    if sheet.labels.iter().any(|label| point_eq(point, [label.x, label.y])) {
        return true;
    }
    if root && design.hierarchy.iter().any(|symbol| symbol.ports.iter().any(|port| point_eq(point, [port.x, port.y]))) {
        return true;
    }
    if sheet.components.iter().any(|component| point_eq(point, [component.x, component.y])) {
        return true;
    }
    sheet.wires.iter().any(|other| other.id != wire.id && on_wire(point, other))
}

/// Reports every dangling endpoint of the sheet at `sheet_index`.
///
/// Findings follow the sheet's wire declaration order and, inside one wire,
/// endpoint order (`from` before `to`). A zero-length wire yields at most one
/// finding at its `from` point. A wire whose endpoints are both attached yields
/// none.
pub fn findings(design: &Design, sheet_index: usize) -> Vec<Finding> {
    let Some(sheet) = design.sheets.get(sheet_index)
    else {
        return Vec::new();
    };
    let root = sheet_index == 0;
    let mut findings = Vec::new();
    for (wire_index, wire) in sheet.wires.iter().enumerate() {
        let zero_length = point_eq(wire.from, wire.to);
        let endpoints: &[usize] = if zero_length { &[0] } else { &[0, 1] };
        for &endpoint in endpoints {
            let point = if endpoint == 0 { wire.from } else { wire.to };
            if endpoint_attached(design, sheet, root, wire, point) {
                continue;
            }
            let message = if zero_length {
                format!(
                    "wire `{}` is zero-length at ({}, {}) and touches nothing: no other wire, component, junction, label, no-connect flag or sheet port",
                    wire.id, point[0], point[1]
                )
            }
            else {
                let end = if endpoint == 0 { "from" } else { "to" };
                format!(
                    "wire `{}` endpoint `{end}` at ({}, {}) touches nothing: no other wire, component, junction, label, no-connect flag or sheet port",
                    wire.id, point[0], point[1]
                )
            };
            findings.push(Finding { wire_index, wire_id: wire.id.clone(), endpoint, x: point[0], y: point[1], message });
        }
    }
    findings
}
