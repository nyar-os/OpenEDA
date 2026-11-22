//! Connection extraction for one schematic sheet.
//!
//! Wires are grouped with a union-find: two wires connect when one endpoint of
//! either lies on the other segment within [`TOLERANCE`], and a junction joins
//! every wire it lies on. Each group then collects its distinct, non-empty
//! `wire.net` labels, so a group with two labels is a conflict and a group with
//! none is unassigned.
//!
//! The module only inspects and reports; mutations live in the crate root so
//! they share the handle store, revision counter and undo history.

use boost_collection::UnionFind;
use geometry_planar::{Point, point_segment_distance};

use crate::{Sheet, Wire};

/// Distance, in design units, below which two points count as coincident.
pub const TOLERANCE: f64 = 1e-4;

/// Stable code of a wire group that carries more than one net label.
pub const WIRE_NET_CONFLICT: &str = "wire-net-conflict";
/// Stable code of a wire group that carries no net label.
pub const WIRE_NET_UNASSIGNED: &str = "wire-net-unassigned";
/// Stable code of a junction that does not lie on any wire.
pub const JUNCTION_FLOATING: &str = "junction-floating";

/// One connected group of wire indices plus the labels it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct WireGroup {
    /// Indices into [`Sheet::wires`], ascending.
    pub wires: Vec<usize>,
    /// Distinct non-empty net labels carried by the group, sorted.
    pub nets: Vec<String>,
}

impl WireGroup {
    /// Whether the group names more than one net.
    pub fn is_conflict(&self) -> bool {
        self.nets.len() > 1
    }

    /// Whether the group names no net at all.
    pub fn is_unassigned(&self) -> bool {
        self.nets.is_empty()
    }
}

/// One connectivity finding, before the revision context is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Stable issue code, one of the constants in this module.
    pub code: &'static str,
    /// Severity understood by the inspector: `error` or `warning`.
    pub severity: &'static str,
    /// Object identity the finding points at.
    pub object_id: String,
    /// Human-readable message, deterministic for one sheet state.
    pub message: String,
}

/// Groups the wires of one sheet into electrically connected sets.
///
/// The result is ordered by union-find root, which is itself deterministic for
/// one wire order, and the wire indices inside a group are ascending.
pub fn groups(sheet: &Sheet) -> Vec<WireGroup> {
    let wires = &sheet.wires;
    let mut set = UnionFind::new(wires.len());

    for (first, candidate) in wires.iter().enumerate() {
        for (second, other) in wires.iter().enumerate().skip(first + 1) {
            if connected(candidate, other) {
                set.union(first, second);
            }
        }
    }

    for junction in &sheet.junctions {
        let point = [junction.x, junction.y];
        let mut touched: Option<usize> = None;
        for (index, wire) in wires.iter().enumerate() {
            if on_wire(point, wire) {
                match touched {
                    Some(previous) => {
                        set.union(previous, index);
                    }
                    None => touched = Some(index),
                }
            }
        }
    }

    let mut buckets: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for index in 0..wires.len() {
        let root = set.find(index);
        buckets.entry(root).or_default().push(index);
    }

    buckets
        .into_values()
        .map(|indices| {
            let mut nets = std::collections::BTreeSet::new();
            for &index in &indices {
                if let Some(net) = &wires[index].net
                    && !net.is_empty()
                {
                    nets.insert(net.clone());
                }
            }
            WireGroup { wires: indices, nets: nets.into_iter().collect() }
        })
        .collect()
}

/// Reports every connectivity finding of one sheet.
pub fn findings(sheet: &Sheet) -> Vec<Finding> {
    let mut findings = Vec::new();
    for group in groups(sheet) {
        if group.is_conflict() {
            let anchor = wire_id(sheet, group.wires[0]);
            let labels = group.nets.iter().map(|net| format!("`{net}`")).collect::<Vec<_>>().join(" and ");
            findings.push(Finding {
                code: WIRE_NET_CONFLICT,
                severity: "error",
                object_id: anchor.clone(),
                message: format!("wire group starting at `{anchor}` carries conflicting nets {labels}"),
            });
        }
        else if group.is_unassigned() {
            let anchor = wire_id(sheet, group.wires[0]);
            findings.push(Finding {
                code: WIRE_NET_UNASSIGNED,
                severity: "warning",
                object_id: anchor.clone(),
                message: format!("wire group starting at `{anchor}` is not attached to a net"),
            });
        }
    }
    for (index, junction) in sheet.junctions.iter().enumerate() {
        let point = [junction.x, junction.y];
        if !sheet.wires.iter().any(|wire| on_wire(point, wire)) {
            findings.push(Finding {
                code: JUNCTION_FLOATING,
                severity: "warning",
                object_id: format!("junction-{index}"),
                message: format!("junction at ({}, {}) does not lie on any wire", junction.x, junction.y),
            });
        }
    }
    findings
}

fn wire_id(sheet: &Sheet, index: usize) -> String {
    sheet.wires.get(index).map(|wire| wire.id.clone()).unwrap_or_else(|| format!("wire-{index}"))
}

/// Whether two points coincide within [`TOLERANCE`].
pub(crate) fn point_eq(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= TOLERANCE && (a[1] - b[1]).abs() <= TOLERANCE
}

/// Distance from `point` to the segment `start`–`end`.
fn distance_to_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let delta_x = end[0] - start[0];
    let delta_y = end[1] - start[1];
    if delta_x * delta_x + delta_y * delta_y <= f64::EPSILON {
        let dx = point[0] - start[0];
        let dy = point[1] - start[1];
        return (dx * dx + dy * dy).sqrt();
    }
    point_segment_distance(Point::new(point[0], point[1]), Point::new(start[0], start[1]), Point::new(end[0], end[1]))
}

/// Whether `point` lies on the segment `start`–`end`.
fn on_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> bool {
    distance_to_segment(point, start, end) <= TOLERANCE
}

/// Whether `point` lies on `wire`.
pub(crate) fn on_wire(point: [f64; 2], wire: &Wire) -> bool {
    on_segment(point, wire.from, wire.to)
}

/// Whether a wire endpoint is attached to a junction, another wire, or a
/// component.
///
/// The model stores component placement but not per-pin geometry, so a
/// component counts through its placement point: an endpoint that coincides
/// with `component.(x, y)` is treated as attached to that component. Another
/// wire counts when the endpoint lies on its segment, which covers both shared
/// endpoints and T-junctions. The wire itself is excluded.
pub fn endpoint_attached(sheet: &Sheet, wire: &Wire, point: [f64; 2]) -> bool {
    if sheet.junctions.iter().any(|junction| point_eq(point, [junction.x, junction.y])) {
        return true;
    }
    if sheet.components.iter().any(|component| point_eq(point, [component.x, component.y])) {
        return true;
    }
    sheet.wires.iter().any(|other| other.id != wire.id && on_wire(point, other))
}

/// Whether two wires join: coincident endpoints, or one endpoint on the other
/// segment (a T-junction). Interior crossings are not connections.
pub(crate) fn connected(first: &Wire, second: &Wire) -> bool {
    point_eq(first.from, second.from)
        || point_eq(first.from, second.to)
        || point_eq(first.to, second.from)
        || point_eq(first.to, second.to)
        || on_segment(first.from, second.from, second.to)
        || on_segment(first.to, second.from, second.to)
        || on_segment(second.from, first.from, first.to)
        || on_segment(second.to, first.from, first.to)
}

#[cfg(test)]
#[path = "../tests/unit/connectivity.rs"]
mod tests;
