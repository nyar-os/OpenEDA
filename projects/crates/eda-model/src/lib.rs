//! Editable schematic design model for OpenEDA.
//!
//! This crate owns the native `.oeda` document, the process-wide handle store,
//! the schematic editing operations, the ERC inspection rules and the netlist
//! and SVG projections. Every entry point (Node-API, command line, studio) reads
//! the same model and the same capability matrix, so no surface can claim a
//! capability the model does not implement.
//!
//! The MVP covers schematic capture only: sheets, components, nets, design-level
//! buses, wires, junctions and text annotations, with per-handle undo/redo,
//! netlist, SPICE, SVG, BOM, PDF and JSON projections, plus KiCad schematic
//! exchange for that subset.
//! Board layout, clearance analysis, board exchange and Gerber output are
//! declared but not implemented, and the capability matrix says so.

#![deny(missing_docs)]

mod connectivity;
mod diff;
mod kicad;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt, fs,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use boost_check::{Capability, CapabilityMatrix, CheckReport, Issue, Severity, Support};
use boost_session::History;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Tag written into every native document.
pub const DOCUMENT_FORMAT: &str = "open-eda-design";
/// Version written into every native document.
pub const DOCUMENT_VERSION: u64 = 1;

/// Design formats the product must classify, even while most are unsupported.
pub const DESIGN_FORMATS: &[&str] =
    &["kicad_sch", "kicad_pcb", "sch", "brd", "net", "edif", "spice", "cir", "gbr", "drl", "step", "svg", "pdf", "png", "bom"];

/// Rule checks declared by the studio inspector.
const INSPECT_CHECKS: &[&str] =
    &["erc", "electrical", "drc", "nets", "connectivity", "footprints", "clearance", "power", "references"];

/// Rule checks this build actually runs.
const SUPPORTED_CHECKS: &[&str] = &["erc", "electrical", "nets", "connectivity", "footprints", "power", "references"];

/// Render targets declared for preview surfaces.
const RENDER_FORMATS: &[&str] = &["svg", "png", "pdf"];

/// Export targets declared by the command line.
const EXPORT_TARGETS: &[&str] = &[
    "kicad_sch",
    "kicad_sym",
    "kicad_pcb",
    "net",
    "net_csv",
    "net_hier",
    "spice",
    "spice_subckt",
    "gbr",
    "pdf",
    "svg",
    "bom",
    "bom_json",
    "bom_html",
    "report",
    "json",
];

/// Export targets this build actually writes.
const SUPPORTED_EXPORTS: &[&str] = &[
    "kicad_sch",
    "kicad_sym",
    "net",
    "net_csv",
    "net_hier",
    "spice",
    "spice_subckt",
    "svg",
    "pdf",
    "bom",
    "bom_json",
    "bom_html",
    "report",
    "json",
];

/// Tag written into the deterministic JSON document projection.
const JSON_PROJECTION_FORMAT: &str = "open-eda-projection";

/// Version written into the deterministic JSON document projection.
const JSON_PROJECTION_VERSION: u64 = 1;

/// Tag written into the connectivity report.
const CONNECTIVITY_REPORT_FORMAT: &str = "open-eda-connectivity";

/// Version written into the connectivity report.
const CONNECTIVITY_REPORT_VERSION: u64 = 1;

/// Tag written into the deterministic JSON BOM projection.
const BOM_PROJECTION_FORMAT: &str = "open-eda-bom";

/// Version written into the deterministic JSON BOM projection.
const BOM_PROJECTION_VERSION: u64 = 1;

/// Loss code reported when a SPICE element pin has no net and gets an NC node.
const SPICE_OPEN_PIN: &str = "spice/open-pin";

/// Loss code reported when an element has fewer pins than its SPICE mapping needs.
const SPICE_PIN_COUNT: &str = "spice/pin-count";

/// Import sources this build actually reads.
const SUPPORTED_IMPORTS: &[&str] = &["kicad_sch", "net"];

/// Loss codes the KiCad schematic importer reports.
const KICAD_LOSS_CODES: &[&str] =
    &[kicad::UNSUPPORTED_ELEMENT, kicad::DUPLICATE_REFERENCE, kicad::MISSING_REFERENCE, kicad::LABEL_SCOPE];

/// Loss codes the KiCad netlist importer reports.
const NET_LOSS_CODES: &[&str] = &[
    kicad::NET_UNSUPPORTED_ELEMENT,
    kicad::NET_DUPLICATE_REFERENCE,
    kicad::NET_MISSING_REFERENCE,
    kicad::NET_MISSING_COMPONENT,
];

/// Render targets this build actually renders for preview. Only `svg` is a
/// preview projection; `pdf` is written by the export writer, not the preview.
const SUPPORTED_RENDERERS: &[&str] = &["svg"];

/// Snapshots kept per open handle; the oldest is dropped first.
const HISTORY_LIMIT: usize = 32;

/// Component box half width, in design units, used by the SVG projection.
const COMPONENT_HALF_WIDTH: f64 = 12.7;
/// Component box half height, in design units, used by the SVG projection.
const COMPONENT_HALF_HEIGHT: f64 = 7.62;

/// Stable failure of one design operation.
#[derive(Debug)]
pub struct ServiceError {
    /// Stable, comparable code.
    pub code: String,
    /// Human-readable message; never carries binary payloads.
    pub message: String,
}

impl ServiceError {
    /// Builds a failure from a stable code and a message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::new("format/invalid", message)
}

fn not_implemented(operation: &str) -> ServiceError {
    ServiceError::new("capability/not-implemented", format!("`{operation}` is not implemented in this build"))
}

fn not_found(message: impl Into<String>) -> ServiceError {
    ServiceError::new("object/not-found", message)
}

fn duplicate(message: impl Into<String>) -> ServiceError {
    ServiceError::new("object/duplicate", message)
}

/// Arguments of [`create`].
#[derive(Debug, Clone, Default)]
pub struct CreateRequest {
    /// Design title.
    pub title: Option<String>,
    /// Drawing revision label, for example `A`.
    pub rev: Option<String>,
    /// Company or organisation of the title block.
    pub company: Option<String>,
    /// Drawing date text.
    pub date: Option<String>,
    /// Template the design starts from; only `blank` (or no template) is implemented.
    pub template: Option<String>,
}

/// One end of a net: a component reference and one of its pins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    /// Reference designator of the component, for example `R1`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Pin name of that component.
    pub pin: String,
}

/// One electrical net on a sheet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Net {
    /// Stable object identity, for example `net-1`.
    pub id: String,
    /// Net name, for example `VCC`.
    pub name: String,
    /// Connected component pins.
    pub nodes: Vec<Node>,
}

/// One declared bus: a named group of `width` member nets.
///
/// A bus belongs to the design, not to one sheet. The netlist projection uses
/// the fixed expansion rule `DATA` with width 4 -> `DATA0`, `DATA1`, `DATA2`,
/// `DATA3`: the member name is the bus name followed by the 0-based index with
/// no separator. A net whose name equals a bus name expands into those members,
/// each repeating the parent net's nodes. A declared bus without such a net
/// still lists its member names, with no nodes, so the declaration is never
/// silent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bus {
    /// Stable object identity, for example `bus-1`.
    pub id: String,
    /// Bus name, for example `DATA`.
    pub name: String,
    /// Number of member nets; at least 1.
    pub width: u64,
    /// Alternate names that resolve to this bus.
    ///
    /// A net named with an alias expands exactly like a net named with the bus
    /// name, so `D` with alias `DATA` still yields the members `DATA0`... The
    /// canonical member names always use the bus name, never the alias.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// One BOM variant: the parts that are not populated and the value overrides.
///
/// Variants belong to the design, not to a sheet, because a reference designator
/// is unique across the whole document. The base document is the default
/// population; a variant only subtracts designators and replaces value text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Variant {
    /// Variant name, unique inside the design.
    pub name: String,
    /// Reference designators that are not populated in this variant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dnp: Vec<String>,
    /// Alternate value text keyed by reference designator.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
}

/// One net class: routing attributes shared by the nets and buses assigned to it.
///
/// A class belongs to the design, not to one sheet. Width, clearance and via
/// diameter are stored in the design unit (`mm` or `mil`) and are never
/// converted; a net name or bus name may belong to at most one class. The model
/// only declares and assigns these attributes, it does not route or check
/// clearance with them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetClass {
    /// Stable object identity, for example `net-class-1`.
    pub id: String,
    /// Class name, unique inside the design.
    pub name: String,
    /// Trace width; must be greater than zero.
    #[serde(default)]
    pub width: f64,
    /// Clearance to neighbouring copper; zero means no declared requirement.
    #[serde(default)]
    pub clearance: f64,
    /// Via diameter; zero means no via is declared for the class.
    #[serde(default)]
    pub via: f64,
    /// Net names assigned to this class.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    /// Declared bus names assigned to this class.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<String>,
}

/// One reference designator prefix rule: the prefix a component class must use.
///
/// The rule is declarative. A component is assigned to a class through
/// [`set_component_class`], and the `references` inspection check compares its
/// reference designator against the prefix of that class.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReferencePrefixRule {
    /// Component class name, for example `resistor`.
    pub class: String,
    /// Expected uppercase letter prefix, for example `R`.
    pub prefix: String,
}

/// Design-wide rules carried by the document.
///
/// The bundle is absent from older documents, where every member defaults to
/// empty. Rules never change the schematic itself: net classes declare routing
/// attributes and reference-prefix rules declare the expected designator prefix
/// per component class.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DesignRules {
    /// Declared net classes; absent when the design declares none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net_classes: Vec<NetClass>,
    /// Declared reference-prefix rules keyed by component class.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_prefixes: Vec<ReferencePrefixRule>,
    /// Component classes keyed by stable component id; absent when none is assigned.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub component_classes: BTreeMap<String, String>,
}

impl DesignRules {
    /// Whether the document declares no rule at all.
    fn is_empty(&self) -> bool {
        self.net_classes.is_empty() && self.reference_prefixes.is_empty() && self.component_classes.is_empty()
    }
}

/// Electrical role of one pin.
///
/// The `electrical` inspection check reads these roles; an untyped pin is
/// treated as unknown and never produces a finding. The names match the KiCad
/// pin electrical types, so the same vocabulary is used on every surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinType {
    /// Logic or analogue input.
    Input,
    /// Logic or analogue output.
    Output,
    /// Input and output on the same pin.
    Bidirectional,
    /// Tri-state output.
    TriState,
    /// Pin with no electrical role, such as a resistor terminal.
    Passive,
    /// Power input consumed by the component.
    PowerIn,
    /// Power output supplied by the component.
    PowerOut,
    /// Open-collector output.
    OpenCollector,
    /// Open-emitter output.
    OpenEmitter,
    /// Role not specified.
    Unspecified,
}

/// One schematic component placed on a sheet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Component {
    /// Stable object identity, for example `component-1`.
    pub id: String,
    /// Reference designator, unique inside the design.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Value text, for example `10k`.
    pub value: String,
    /// Optional footprint assignment, for example `Resistor_SMD:R_0805_2012Metric`.
    ///
    /// An empty string means no footprint has been assigned. The BOM groups by
    /// value and footprint, and the KiCad writer emits the property verbatim.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub footprint: String,
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
    /// Rotation in degrees.
    pub rotation: f64,
    /// Pin names this component exposes.
    pub pins: Vec<String>,
    /// Electrical role of the pins that have one, keyed by pin name.
    ///
    /// An empty map means no pin has been typed yet. The `electrical` check
    /// only reads pins listed here and ignores the rest.
    #[serde(default, rename = "pinTypes", skip_serializing_if = "BTreeMap::is_empty")]
    pub pin_types: BTreeMap<String, PinType>,
}

/// One schematic wire segment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Wire {
    /// Stable object identity, for example `wire-1`.
    pub id: String,
    /// Declared net the wire belongs to, when any.
    pub net: Option<String>,
    /// Segment start point.
    pub from: [f64; 2],
    /// Segment end point.
    pub to: [f64; 2],
}

/// One schematic junction (connection dot).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Junction {
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
}

/// One no-connect flag placed on a schematic sheet.
///
/// The flag records that the point is deliberately left unconnected. The model
/// keeps its placement so the flag survives a KiCad round trip. When `node` is
/// present the flag also marks one component pin, so ERC and the connectivity
/// report treat that pin as intentionally open instead of reporting a warning.
/// KiCad stores only the placement, so the pin attachment is a design-model
/// annotation rather than a KiCad file field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoConnect {
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
    /// Pin this flag deliberately leaves open; absent on coordinate-only flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<Node>,
}

/// One free text annotation drawn on a schematic sheet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextLabel {
    /// Text content.
    pub text: String,
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
}

/// Electrical direction of one hierarchical sheet port.
///
/// The names match the KiCad sheet-pin vocabulary, so the same text is written
/// and read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    /// Signal enters the child sheet.
    Input,
    /// Signal leaves the child sheet.
    Output,
    /// Signal flows both ways.
    Bidirectional,
    /// Tri-state connection.
    TriState,
    /// Port with no electrical direction.
    Passive,
}

/// One port declared on a hierarchical sheet symbol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SheetPort {
    /// Port name, unique inside the sheet symbol.
    pub name: String,
    /// Electrical direction of the port.
    pub direction: PortDirection,
    /// Port placement X coordinate in design units.
    pub x: f64,
    /// Port placement Y coordinate in design units.
    pub y: f64,
}

/// One hierarchical sheet symbol placed in a parent schematic.
///
/// A symbol names the child sheet file and its ports. The model does not read
/// that child file, so the symbol is a declaration: it survives import and
/// export, participates in validation and is reported by the JSON projection,
/// but its contents are not merged into the flat `sheets` list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HierarchicalSheet {
    /// Stable object identity, for example `hierarchy-1`.
    pub id: String,
    /// Sheet name shown on the parent symbol.
    pub name: String,
    /// Child schematic file name, for example `power.kicad_sch`.
    pub file: String,
    /// Symbol top-left X coordinate in design units.
    pub x: f64,
    /// Symbol top-left Y coordinate in design units.
    pub y: f64,
    /// Symbol width in design units.
    pub width: f64,
    /// Symbol height in design units.
    pub height: f64,
    /// Ports declared on this symbol; absent in older documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<SheetPort>,
}

/// One schematic sheet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sheet {
    /// Stable sheet identity, for example `sheet-1`.
    pub id: String,
    /// Sheet name shown in the studio.
    pub name: String,
    /// Components placed on this sheet.
    pub components: Vec<Component>,
    /// Nets declared on this sheet.
    pub nets: Vec<Net>,
    /// Wire segments drawn on this sheet.
    pub wires: Vec<Wire>,
    /// Junctions drawn on this sheet; absent in older documents.
    #[serde(default)]
    pub junctions: Vec<Junction>,
    /// No-connect flags drawn on this sheet; absent in older documents.
    #[serde(default)]
    pub no_connects: Vec<NoConnect>,
    /// Free text annotations drawn on this sheet; absent in older documents.
    #[serde(default)]
    pub texts: Vec<TextLabel>,
}

/// Native design document model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Design {
    /// Stable design identity.
    pub id: String,
    /// Optional design title.
    pub title: Option<String>,
    /// Drawing revision label, for example `A`; the edit counter is `revision`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rev: String,
    /// Company or organisation drawn in the title block.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub company: String,
    /// Drawing date text, for example `2026-11-22`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    /// Committed revision; every successful edit increments it.
    pub revision: u64,
    /// Design units, `mm` or `mil`.
    pub units: String,
    /// Schematic sheets; a created design always has one.
    pub sheets: Vec<Sheet>,
    /// Declared buses; absent in older documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<Bus>,
    /// BOM variants; absent in older documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    /// Hierarchical sheet symbols; absent in older documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hierarchy: Vec<HierarchicalSheet>,
    /// Design-wide rules such as net classes; absent in older documents.
    #[serde(default, skip_serializing_if = "DesignRules::is_empty")]
    pub rules: DesignRules,
    /// Next component number used to build deterministic object identities.
    pub next_component: u64,
    /// Next net number used to build deterministic object identities.
    pub next_net: u64,
    /// Next wire number used to build deterministic object identities.
    pub next_wire: u64,
}

/// Request of [`update_design`].
///
/// A present field replaces the current value and an empty string clears it; an
/// absent field keeps its value.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDesignRequest {
    /// New title; an empty string clears it.
    #[serde(default)]
    pub title: Option<String>,
    /// New drawing revision label; an empty string clears it.
    #[serde(default)]
    pub rev: Option<String>,
    /// New company text; an empty string clears it.
    #[serde(default)]
    pub company: Option<String>,
    /// New drawing date text; an empty string clears it.
    #[serde(default)]
    pub date: Option<String>,
}

/// Request of [`add_component`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator.
    pub reference: String,
    /// Value text.
    pub value: String,
    /// Placement X coordinate.
    pub x: f64,
    /// Placement Y coordinate.
    pub y: f64,
    /// Rotation in degrees; defaults to zero.
    #[serde(default)]
    pub rotation: f64,
    /// Pin names, for example `["1", "2"]`.
    pub pins: Vec<String>,
}

/// Request of [`connect_net`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectNetRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Net name.
    pub net: String,
    /// Nodes in `REF.PIN` form.
    pub nodes: Vec<String>,
}

/// Request of [`add_wire`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWireRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Declared net name, when the wire belongs to one.
    #[serde(default)]
    pub net: Option<String>,
    /// Segment start point.
    pub from: [f64; 2],
    /// Segment end point.
    pub to: [f64; 2],
}

/// Request of [`update_wire`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWireRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Stable wire identity, for example `wire-1`.
    pub id: String,
    /// New declared net name. An empty string clears the assignment.
    #[serde(default)]
    pub net: Option<String>,
    /// Replacement start point.
    #[serde(default)]
    pub from: Option<[f64; 2]>,
    /// Replacement end point.
    #[serde(default)]
    pub to: Option<[f64; 2]>,
}

/// Request of [`add_junction`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddJunctionRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Placement X coordinate.
    pub x: f64,
    /// Placement Y coordinate.
    pub y: f64,
}

/// Request of [`add_text`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTextRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Text content.
    pub text: String,
    /// Placement X coordinate.
    pub x: f64,
    /// Placement Y coordinate.
    pub y: f64,
}

/// Request of [`add_no_connect`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddNoConnectRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Placement X coordinate.
    pub x: f64,
    /// Placement Y coordinate.
    pub y: f64,
    /// Pin the flag deliberately leaves open; optional.
    #[serde(default)]
    pub node: Option<Node>,
}

/// Request of [`move_component`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// New placement X coordinate.
    pub x: f64,
    /// New placement Y coordinate.
    pub y: f64,
    /// New rotation in degrees; the current rotation is kept when omitted.
    #[serde(default)]
    pub rotation: Option<f64>,
}

/// Request of [`update_component`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// New value text.
    pub value: String,
}

/// Request of [`set_component_footprint`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFootprintRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Footprint text, for example `Resistor_SMD:R_0805_2012Metric`; an empty
    /// string clears the assignment.
    #[serde(default)]
    pub footprint: String,
}

/// Request of [`set_pin_type`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPinTypeRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Pin name as declared by the component.
    pub pin: String,
    /// Electrical role to assign; a missing or null value clears the role.
    #[serde(default)]
    pub pin_type: Option<PinType>,
}

/// Request of [`rename_net`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameNetRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Current net id or name.
    pub net: String,
    /// New net name.
    pub name: String,
}

/// Request of [`disconnect_net`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectNetRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Net id or name.
    pub net: String,
    /// Nodes to remove, in `REF.PIN` form; every node is removed when empty.
    #[serde(default)]
    pub nodes: Vec<String>,
}

/// Request of [`add_bus`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddBusRequest {
    /// Bus name, for example `DATA`.
    pub name: String,
    /// Number of member nets; must be at least 1.
    pub width: u64,
    /// Alternate names that resolve to this bus; empty when none are declared.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Request of [`add_variant`].
///
/// A variant with an existing name is replaced, so one request always describes
/// the whole variant: its DNP list and its value overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantRequest {
    /// Variant name.
    pub name: String,
    /// Reference designators that are not populated.
    #[serde(default)]
    pub dnp: Vec<String>,
    /// Alternate value text keyed by reference designator.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

/// Request of [`add_net_class`].
///
/// A class with an existing name is replaced as a whole, so one request always
/// describes its attributes and both assignment lists.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetClassRequest {
    /// Class name, for example `HighSpeed`.
    pub name: String,
    /// Trace width; must be greater than zero.
    pub width: f64,
    /// Clearance to neighbouring copper; zero means no declared requirement.
    #[serde(default)]
    pub clearance: f64,
    /// Via diameter; zero means no via is declared for the class.
    #[serde(default)]
    pub via: f64,
    /// Net names assigned to the class; every name must already be declared.
    #[serde(default)]
    pub nets: Vec<String>,
    /// Declared bus names assigned to the class.
    #[serde(default)]
    pub buses: Vec<String>,
}

/// Request of [`set_reference_prefix`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferencePrefixRequest {
    /// Component class name, for example `resistor`.
    pub class: String,
    /// Expected uppercase letter prefix, for example `R`.
    pub prefix: String,
}

/// Request of [`set_component_class`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentClassRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Class to assign; an empty string clears the assignment.
    #[serde(default)]
    pub class: String,
}

/// Request of [`delete_net`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteNetRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Net id or name.
    pub net: String,
}

/// Options of [`inspect_with_options`].
///
/// `severity` maps a finding code to `error`, `warning`, `info` or `off`, so a
/// project can promote or silence one rule without changing the others.
/// `waivers` move matching findings from `issues` to `waived`; a waiver with no
/// `sheetId` or `objectId` matches the code everywhere.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectOptions {
    /// Checks to run; every supported check runs when omitted.
    #[serde(default)]
    pub checks: Option<Vec<String>>,
    /// Per-code severity overrides.
    #[serde(default)]
    pub severity: BTreeMap<String, String>,
    /// Findings accepted for this run.
    #[serde(default)]
    pub waivers: Vec<Waiver>,
}

/// One accepted finding of [`inspect_with_options`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiver {
    /// Finding code to waive, for example `erc/unconnected-pin`.
    pub code: String,
    /// Restrict the waiver to one sheet id.
    #[serde(default)]
    pub sheet_id: Option<String>,
    /// Restrict the waiver to one object id.
    #[serde(default)]
    pub object_id: Option<String>,
}

/// What this build can actually do.
///
/// The matrix is the single source of truth for the CLI, the SDK and the
/// studio, so a method name is never treated as a capability. Netlist, SPICE,
/// SVG, BOM (CSV, JSON and HTML), PDF, report, JSON document, KiCad schematic
/// and KiCad symbol library
/// projections are supported,
/// KiCad schematics are also imported, and every other exchange format and rule
/// check is reported as unsupported. Design-level buses are edited through
/// `add_bus`, `remove_bus` and `list_buses`; a bus carries a width and optional
/// aliases that resolve to the same expansion. ERC reports the stable codes
/// `erc/duplicate-net-name`, `erc/duplicate-pin`, `erc/single-node-net`,
/// `erc/dangling-net`, `erc/unconnected-pin`, `erc/floating-wire`,
/// `erc/reference-illegal`, `erc/pin-name-illegal`, `erc/net-name-illegal` and
/// `erc/unconnected-hierarchical-port`
/// next to the legacy codes. A no-connect flag that names a component pin
/// removes `erc/unconnected-pin` and `unreferenced-pin` for that pin, because
/// the open pin is deliberate. The `nets` check also reports `net-bus-member-collision` when a
/// declared net would be written twice as a bus member, and
/// `net-bus-alias-collision` when two nets on one sheet resolve to the same bus
/// through an alias. The JSON keys of this
/// report are unchanged.
pub fn capabilities() -> Value {
    let matrix = capability_matrix();
    let level = |id: String| if matrix.supports(&id) { "supported" } else { "unsupported" };

    let mut formats = Map::new();
    for &format in DESIGN_FORMATS {
        let import = level(format!("format.{format}.import"));
        let edit = level(format!("format.{format}.edit"));
        let export = level(format!("format.{format}.export"));
        let loss_codes = match format {
            "kicad_sch" => json!(KICAD_LOSS_CODES),
            "net" => json!(NET_LOSS_CODES),
            _ => json!([]),
        };
        formats
            .insert(format.to_string(), json!({ "import": import, "edit": edit, "export": export, "lossCodes": loss_codes }));
    }

    let mut checks = Map::new();
    for &check in INSPECT_CHECKS {
        checks.insert(check.to_string(), Value::String(level(format!("check.{check}")).to_string()));
    }

    let mut renderers = Map::new();
    for &renderer in RENDER_FORMATS {
        renderers.insert(renderer.to_string(), Value::String(level(format!("renderer.{renderer}")).to_string()));
    }

    let mut export_targets = Map::new();
    for &target in EXPORT_TARGETS {
        export_targets.insert(target.to_string(), Value::String(level(format!("export.{target}")).to_string()));
    }

    json!({
        "engine": { "package": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
        "project": { "import": "partial", "edit": "partial", "export": "partial" },
        "formats": Value::Object(formats),
        "checks": Value::Object(checks),
        "renderers": Value::Object(renderers),
        "exportTargets": Value::Object(export_targets),
    })
}

/// The capability rows the JSON matrix projects, kept in one shared structure.
fn capability_matrix() -> CapabilityMatrix {
    let mut matrix = CapabilityMatrix::new();
    for &format in DESIGN_FORMATS {
        let import = support_of(SUPPORTED_IMPORTS.contains(&format));
        let export = support_of(SUPPORTED_EXPORTS.contains(&format));
        matrix.insert(Capability::new(format!("format.{format}.import"), "format", import));
        matrix.insert(Capability::new(format!("format.{format}.edit"), "format", Support::Unsupported));
        matrix.insert(Capability::new(format!("format.{format}.export"), "format", export));
    }
    for &check in INSPECT_CHECKS {
        matrix.insert(Capability::new(format!("check.{check}"), "check", support_of(SUPPORTED_CHECKS.contains(&check))));
    }
    for &renderer in RENDER_FORMATS {
        matrix.insert(Capability::new(
            format!("renderer.{renderer}"),
            "renderer",
            support_of(SUPPORTED_RENDERERS.contains(&renderer)),
        ));
    }
    for &target in EXPORT_TARGETS {
        matrix.insert(Capability::new(format!("export.{target}"), "export", support_of(SUPPORTED_EXPORTS.contains(&target))));
    }
    matrix
}

fn support_of(supported: bool) -> Support {
    if supported { Support::Supported } else { Support::Unsupported }
}

/// One open design plus its bounded undo/redo history.
///
/// Snapshots are whole documents; the newest edit is at the back of `undo`.
/// A new mutation clears `redo`, and neither stack is written to `.oeda`.
struct Session {
    design: Design,
    history: History<Design>,
}

impl Session {
    fn new(design: Design) -> Self {
        Self { design, history: History::new(HISTORY_LIMIT) }
    }
}

struct Store {
    sessions: Mutex<std::collections::HashMap<u64, Session>>,
    next_handle: AtomicU64,
}

fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| Store { sessions: Mutex::new(std::collections::HashMap::new()), next_handle: AtomicU64::new(1) })
}

fn locked_sessions() -> std::sync::MutexGuard<'static, std::collections::HashMap<u64, Session>> {
    store().sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn insert_design(design: Design) -> u64 {
    let handle = store().next_handle.fetch_add(1, Ordering::SeqCst);
    locked_sessions().insert(handle, Session::new(design));
    handle
}

fn with_session<T>(handle: u64, action: impl FnOnce(&Session) -> Result<T, ServiceError>) -> Result<T, ServiceError> {
    let sessions = locked_sessions();
    let session = sessions
        .get(&handle)
        .ok_or_else(|| ServiceError::new("design/not-found", format!("no design is open under handle {handle}")))?;
    action(session)
}

/// Runs one mutation and records the previous document when the revision moved.
///
/// The closure returns the payload without the shared fields; this function
/// adds `revision`, `design`, `canUndo` and `canRedo` after the fact so every
/// edit reports history consistently. A failed mutation leaves no snapshot.
fn mutate(
    handle: u64,
    action: impl FnOnce(&mut Design) -> Result<Map<String, Value>, ServiceError>,
) -> Result<Value, ServiceError> {
    let mut sessions = locked_sessions();
    let session = sessions
        .get_mut(&handle)
        .ok_or_else(|| ServiceError::new("design/not-found", format!("no design is open under handle {handle}")))?;
    let snapshot = session.design.clone();
    let before = session.design.revision;
    let mut payload = action(&mut session.design)?;
    if session.design.revision != before {
        session.history.record(snapshot);
    }
    payload.insert("revision".to_string(), json!(session.design.revision));
    payload.insert("design".to_string(), summary_of(session));
    payload.insert("canUndo".to_string(), json!(session.history.can_undo()));
    payload.insert("canRedo".to_string(), json!(session.history.can_redo()));
    Ok(Value::Object(payload))
}

/// Creates a design from a request and returns its handle and summary.
pub fn create(request: CreateRequest) -> Result<Value, ServiceError> {
    if let Some(template) = request.template.as_deref()
        && template != "blank"
    {
        return Err(not_implemented(&format!("template `{template}`")));
    }
    let design = Design {
        id: "design-1".to_string(),
        title: request.title,
        rev: request.rev.unwrap_or_default(),
        company: request.company.unwrap_or_default(),
        date: request.date.unwrap_or_default(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Sheet 1".to_string(),
            components: Vec::new(),
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 1,
        next_net: 1,
        next_wire: 1,
    };
    let session = Session::new(design);
    let summary = summary_of(&session);
    let handle = insert_design(session.design);
    Ok(json!({ "handle": handle, "design": summary }))
}

/// Opens a native design document and returns its handle and summary.
pub fn open(path: &Path) -> Result<Value, ServiceError> {
    let design = read_design(path)?;
    let session = Session::new(design);
    let summary = summary_of(&session);
    let handle = insert_design(session.design);
    Ok(json!({ "handle": handle, "design": summary }))
}

/// Reads and validates one native design document without opening a handle.
fn read_design(path: &Path) -> Result<Design, ServiceError> {
    let text = fs::read_to_string(path).map_err(|error| invalid(format!("cannot read `{}`: {error}", path.display())))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| invalid(format!("`{}` is not valid JSON: {error}", path.display())))?;

    let tag = value.get("format").and_then(Value::as_str).unwrap_or("");
    if tag != DOCUMENT_FORMAT {
        return Err(invalid(format!("`{}` is not an OpenEDA design (format tag `{tag}`)", path.display())));
    }
    let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version != DOCUMENT_VERSION {
        return Err(ServiceError::new(
            "format/unsupported-version",
            format!("document version {version} is not supported; this build reads version {DOCUMENT_VERSION}"),
        ));
    }
    let document = value.get("document").cloned().ok_or_else(|| invalid("the document section is missing"))?;
    let design: Design = serde_json::from_value(document)
        .map_err(|error| invalid(format!("the document does not match the design schema: {error}")))?;
    validate_design(&design)?;
    Ok(design)
}

/// Compares two native design documents and returns a read-only diff report.
///
/// Both documents are read and validated but no handle is opened, so the report
/// never changes either file. Missing or malformed inputs fail with the same
/// codes as [`open`].
pub fn diff_documents(left: &Path, right: &Path) -> Result<Value, ServiceError> {
    let left_design = read_design(left)?;
    let right_design = read_design(right)?;
    Ok(diff::compare(&left.display().to_string(), &left_design, &right.display().to_string(), &right_design))
}

/// Writes one open design to a native document path.
///
/// Saving never adds an undo step: it observes the committed revision instead.
pub fn save(handle: u64, path: &Path, overwrite: bool) -> Result<Value, ServiceError> {
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    validate_design(&design)?;
    let document = DocumentFile { format: DOCUMENT_FORMAT, version: DOCUMENT_VERSION, document: &design };
    let text =
        serde_json::to_string_pretty(&document).map_err(|error| invalid(format!("cannot serialize the design: {error}")))?;
    let bytes = write_atomic(path, &text, overwrite)?;
    Ok(json!({ "path": path.display().to_string(), "revision": design.revision, "bytes": bytes }))
}

#[derive(Serialize)]
struct DocumentFile<'a> {
    format: &'static str,
    version: u64,
    document: &'a Design,
}

/// Updates the drawing properties of one open design.
///
/// Each field that is present replaces the current value and an empty string
/// clears it; a field that is absent keeps its value. The revision advances
/// only when at least one property actually changed, so a no-op update adds no
/// undo step.
pub fn update_design(handle: u64, request: UpdateDesignRequest) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let before = (design.title.clone(), design.rev.clone(), design.company.clone(), design.date.clone());
        if let Some(title) = request.title {
            let title = title.trim();
            design.title = if title.is_empty() { None } else { Some(title.to_string()) };
        }
        if let Some(rev) = request.rev {
            design.rev = rev.trim().to_string();
        }
        if let Some(company) = request.company {
            design.company = company.trim().to_string();
        }
        if let Some(date) = request.date {
            design.date = date.trim().to_string();
        }
        if before != (design.title.clone(), design.rev.clone(), design.company.clone(), design.date.clone()) {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("title".to_string(), json!(design.title));
        payload.insert("rev".to_string(), json!(design.rev));
        payload.insert("company".to_string(), json!(design.company));
        payload.insert("date".to_string(), json!(design.date));
        Ok(payload)
    })
}

/// Runs the requested rule checks on one open design.
pub fn inspect(handle: u64, checks: Option<Vec<String>>) -> Result<Value, ServiceError> {
    inspect_with_options(handle, InspectOptions { checks, ..InspectOptions::default() })
}

/// Runs the requested rule checks with severity overrides and waivers.
///
/// A `severity` entry of `off` drops the finding, the other levels replace it,
/// and a matching waiver moves the finding from `issues` to `waived`. `passed`
/// only considers active errors, so an accepted or demoted finding never fails
/// the report.
pub fn inspect_with_options(handle: u64, options: InspectOptions) -> Result<Value, ServiceError> {
    for (code, level) in &options.severity {
        if code.trim().is_empty() {
            return Err(ServiceError::new("session/argument-invalid", "a severity override needs a finding code"));
        }
        if !matches!(level.as_str(), "error" | "warning" | "info" | "off") {
            return Err(ServiceError::new(
                "session/argument-invalid",
                format!("severity level `{level}` for `{code}` must be error, warning, info or off"),
            ));
        }
    }
    for waiver in &options.waivers {
        if waiver.code.trim().is_empty() {
            return Err(ServiceError::new("session/argument-invalid", "a waiver needs a finding code"));
        }
    }
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let selected = options.checks.unwrap_or_else(|| SUPPORTED_CHECKS.iter().map(|name| (*name).to_string()).collect());
    for check in &selected {
        if !INSPECT_CHECKS.contains(&check.as_str()) || !SUPPORTED_CHECKS.contains(&check.as_str()) {
            return Err(not_implemented(&format!("inspect check `{check}`")));
        }
    }
    let found = inspect_design(&design, &selected);
    let mut active = Vec::new();
    let mut waived = Vec::new();
    for mut issue in found {
        let code = issue["code"].as_str().unwrap_or("").to_string();
        if let Some(level) = options.severity.get(&code) {
            if level == "off" {
                continue;
            }
            issue["severity"] = json!(level);
        }
        if options.waivers.iter().any(|waiver| waiver_matches(waiver, &issue)) {
            issue["waived"] = json!(true);
            waived.push(issue);
        }
        else {
            active.push(issue);
        }
    }
    let passed = !active.iter().any(|issue| issue["severity"] == "error");
    let counts = json!({ "active": active.len(), "waived": waived.len() });
    Ok(json!({
        "revision": design.revision,
        "checkerVersion": format!("open-eda-erc/{}", env!("CARGO_PKG_VERSION")),
        "passed": passed,
        "issues": active,
        "waived": waived,
        "counts": counts,
    }))
}

/// Whether one waiver accepts one finding.
fn waiver_matches(waiver: &Waiver, issue: &Value) -> bool {
    if issue["code"].as_str() != Some(waiver.code.as_str()) {
        return false;
    }
    if let Some(sheet_id) = &waiver.sheet_id
        && issue["sheetId"].as_str() != Some(sheet_id.as_str())
    {
        return false;
    }
    if let Some(object_id) = &waiver.object_id
        && issue["objectId"].as_str() != Some(object_id.as_str())
    {
        return false;
    }
    true
}

/// Reads the summary of one open design.
pub fn summary(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| Ok(summary_of(session)))
}

/// Lists components, nets and wires of one open design.
pub fn list(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let mut components = Vec::new();
        let mut nets = Vec::new();
        let mut wires = Vec::new();
        for sheet in &design.sheets {
            components.extend(sheet.components.iter().cloned());
            nets.extend(sheet.nets.iter().cloned());
            wires.extend(sheet.wires.iter().cloned());
        }
        Ok(json!({
            "revision": design.revision,
            "design": summary_of(session),
            "components": components,
            "nets": nets,
            "wires": wires,
        }))
    })
}

/// Lists components of one open design.
pub fn list_components(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let sheet = &design.sheets[index];
        Ok(json!({
            "revision": design.revision,
            "design": summary_of(session),
            "sheetId": sheet.id,
            "components": sheet.components,
        }))
    })
}

/// Lists nets of one open design.
pub fn list_nets(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let sheet = &design.sheets[index];
        Ok(json!({
            "revision": design.revision,
            "design": summary_of(session),
            "sheetId": sheet.id,
            "nets": sheet.nets,
        }))
    })
}

/// Lists declared buses of one open design.
pub fn list_buses(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        Ok(json!({
            "revision": design.revision,
            "design": summary_of(session),
            "buses": design.buses,
        }))
    })
}

/// Declares one design-level bus.
///
/// Names must be non-empty and unique across the design; a duplicate name is
/// reported as `object/duplicate`. `width` must be at least 1; zero is reported
/// as `session/argument-invalid`. Aliases are trimmed, must be legal net names,
/// and must not collide with a bus name, another alias or a member name of any
/// bus, because one net name would then resolve to two different things.
pub fn add_bus(handle: u64, request: AddBusRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a bus name must not be empty"));
    }
    if request.width == 0 {
        return Err(ServiceError::new("session/argument-invalid", "a bus width must be at least 1"));
    }
    let mut aliases = Vec::new();
    for raw in request.aliases {
        let alias = raw.trim().to_string();
        if alias.is_empty() {
            return Err(ServiceError::new("session/argument-invalid", "a bus alias must not be empty"));
        }
        if breaks_net_grammar(&alias) {
            return Err(ServiceError::new(
                "session/argument-invalid",
                format!("bus alias `{alias}` contains whitespace or a control character"),
            ));
        }
        if aliases.contains(&alias) {
            return Err(ServiceError::new(
                "session/argument-invalid",
                format!("bus alias `{alias}` is declared more than once"),
            ));
        }
        aliases.push(alias);
    }
    mutate(handle, |design| {
        let declared = |candidate: &str| {
            design.buses.iter().any(|bus| {
                bus.name == candidate
                    || bus.aliases.iter().any(|alias| alias == candidate)
                    || (0..bus.width).any(|index| format!("{}{index}", bus.name) == candidate)
            })
        };
        if declared(&name) {
            return Err(duplicate(format!("bus name `{name}` already exists")));
        }
        for alias in &aliases {
            if alias == &name {
                return Err(ServiceError::new("session/argument-invalid", "a bus alias must differ from its bus name"));
            }
            if declared(alias) {
                return Err(duplicate(format!("bus alias `{alias}` already resolves to a declared bus")));
            }
            if (0..request.width).any(|index| format!("{name}{index}") == *alias) {
                return Err(duplicate(format!("bus alias `{alias}` is already a member name of bus `{name}`")));
            }
        }
        let id = free_bus_id(design);
        let bus = Bus { id, name: name.clone(), width: request.width, aliases };
        design.buses.push(bus.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("bus".to_string(), json!(bus));
        Ok(payload)
    })
}

/// Removes one bus selected by id or name.
///
/// A missing bus is reported as `object/not-found`; the payload reports the
/// `deleted` bus name and the `remaining` bus count.
pub fn remove_bus(handle: u64, bus: &str) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let position = design
            .buses
            .iter()
            .position(|entry| entry.id == bus || entry.name == bus || entry.aliases.iter().any(|alias| alias == bus))
            .ok_or_else(|| not_found(format!("no bus `{bus}` is declared")))?;
        let removed = design.buses.remove(position);
        for net_class in &mut design.rules.net_classes {
            net_class.buses.retain(|entry| entry != &removed.name);
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.name));
        payload.insert("remaining".to_string(), json!(design.buses.len()));
        Ok(payload)
    })
}

/// Declares or replaces one BOM variant.
///
/// Every designator in `dnp` and in `values` must be placed in the design; a
/// variant never invents a part. A variant whose content is unchanged adds no
/// revision.
pub fn add_variant(handle: u64, request: VariantRequest) -> Result<Value, ServiceError> {
    if request.name.trim().is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a variant name must not be empty"));
    }
    mutate(handle, |design| {
        let placed: HashSet<&str> = design
            .sheets
            .iter()
            .flat_map(|sheet| sheet.components.iter())
            .map(|component| component.reference.as_str())
            .collect();
        let mut dnp = Vec::new();
        let mut seen = HashSet::new();
        for reference in &request.dnp {
            if !placed.contains(reference.as_str()) {
                return Err(not_found(format!("variant `{}` marks missing component `{reference}` as DNP", request.name)));
            }
            if seen.insert(reference.clone()) {
                dnp.push(reference.clone());
            }
        }
        for reference in request.values.keys() {
            if !placed.contains(reference.as_str()) {
                return Err(not_found(format!("variant `{}` overrides missing component `{reference}`", request.name)));
            }
        }
        let variant = Variant { name: request.name.clone(), dnp, values: request.values };
        let existing = design.variants.iter().position(|entry| entry.name == variant.name);
        let changed = match existing {
            Some(position) if design.variants[position] == variant => false,
            Some(position) => {
                design.variants[position] = variant.clone();
                true
            }
            None => {
                design.variants.push(variant.clone());
                true
            }
        };
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("variant".to_string(), json!(variant));
        Ok(payload)
    })
}

/// Removes one BOM variant by name.
pub fn remove_variant(handle: u64, name: &str) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let position = design
            .variants
            .iter()
            .position(|entry| entry.name == name)
            .ok_or_else(|| not_found(format!("no variant `{name}` is declared")))?;
        let removed = design.variants.remove(position);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.name));
        payload.insert("remaining".to_string(), json!(design.variants.len()));
        Ok(payload)
    })
}

/// Lists the BOM variants declared by one design.
pub fn list_variants(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| Ok(json!({ "revision": session.design.revision, "variants": session.design.variants })))
}

/// Declares or replaces one net class.
///
/// The width must be a finite number greater than zero, and clearance and via
/// diameter must be finite and not negative. Every assigned net must already be
/// declared on some sheet and every assigned bus must be declared; an alias
/// resolves to its bus name, so the class always stores the canonical name. A
/// class with an existing name is replaced, an unchanged class adds no revision,
/// and a name already used by another class in the same document cannot occur
/// because names are the identity.
pub fn add_net_class(handle: u64, request: NetClassRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a net class name must not be empty"));
    }
    if breaks_net_grammar(&name) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("net class name `{name}` contains whitespace or a control character"),
        ));
    }
    if !request.width.is_finite() || request.width <= 0.0 {
        return Err(ServiceError::new("session/argument-invalid", "a net class width must be a finite number above zero"));
    }
    if !request.clearance.is_finite() || request.clearance < 0.0 {
        return Err(ServiceError::new(
            "session/argument-invalid",
            "net class clearance must be a finite number that is not negative",
        ));
    }
    if !request.via.is_finite() || request.via < 0.0 {
        return Err(ServiceError::new(
            "session/argument-invalid",
            "a net class via diameter must be a finite number that is not negative",
        ));
    }
    mutate(handle, |design| {
        let mut nets: Vec<String> = Vec::new();
        for net in &request.nets {
            let net = net.trim();
            if net.is_empty() {
                return Err(ServiceError::new("session/argument-invalid", "a net class assignment must not be empty"));
            }
            let declared = design.sheets.iter().any(|sheet| sheet.nets.iter().any(|entry| entry.name == net));
            if !declared {
                return Err(not_found(format!("net class `{name}` assigns undeclared net `{net}`")));
            }
            if !nets.iter().any(|entry| entry == net) {
                nets.push(net.to_string());
            }
        }
        let mut buses: Vec<String> = Vec::new();
        for bus in &request.buses {
            let bus = bus.trim();
            if bus.is_empty() {
                return Err(ServiceError::new("session/argument-invalid", "a bus class assignment must not be empty"));
            }
            let canonical = design
                .buses
                .iter()
                .find(|entry| entry.name == bus || entry.aliases.iter().any(|alias| alias == bus))
                .map(|entry| entry.name.clone())
                .ok_or_else(|| not_found(format!("net class `{name}` assigns undeclared bus `{bus}`")))?;
            if !buses.iter().any(|entry| entry == &canonical) {
                buses.push(canonical);
            }
        }
        let id = match design.rules.net_classes.iter().find(|entry| entry.name == name) {
            Some(existing) => existing.id.clone(),
            None => free_net_class_id(design),
        };
        let net_class = NetClass {
            id,
            name: name.clone(),
            width: request.width,
            clearance: request.clearance,
            via: request.via,
            nets,
            buses,
        };
        let existing = design.rules.net_classes.iter().position(|entry| entry.name == name);
        let changed = match existing {
            Some(position) if design.rules.net_classes[position] == net_class => false,
            Some(position) => {
                design.rules.net_classes[position] = net_class.clone();
                true
            }
            None => {
                design.rules.net_classes.push(net_class.clone());
                true
            }
        };
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("netClass".to_string(), json!(net_class));
        Ok(payload)
    })
}

/// Removes one net class selected by id or name.
pub fn remove_net_class(handle: u64, net_class: &str) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let position = design
            .rules
            .net_classes
            .iter()
            .position(|entry| entry.id == net_class || entry.name == net_class)
            .ok_or_else(|| not_found(format!("no net class `{net_class}` is declared")))?;
        let removed = design.rules.net_classes.remove(position);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.name));
        payload.insert("remaining".to_string(), json!(design.rules.net_classes.len()));
        Ok(payload)
    })
}

/// Lists the net classes declared by one design.
pub fn list_net_classes(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        Ok(json!({
            "revision": session.design.revision,
            "netClasses": session.design.rules.net_classes,
        }))
    })
}

/// Declares or replaces one reference designator prefix rule.
///
/// A prefix is one to eight uppercase ASCII letters, so `R`, `C`, `U`, `J` and
/// `TP` are all valid. Redeclaring the same class replaces its prefix, and an
/// unchanged rule adds no revision.
pub fn set_reference_prefix(handle: u64, request: ReferencePrefixRequest) -> Result<Value, ServiceError> {
    let class = request.class.trim().to_string();
    if class.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a reference rule needs a component class"));
    }
    if breaks_net_grammar(&class) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("component class `{class}` contains whitespace or a control character"),
        ));
    }
    if !valid_reference_prefix(&request.prefix) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("prefix `{}` must be one to eight uppercase ASCII letters, for example R or TP", request.prefix),
        ));
    }
    mutate(handle, |design| {
        let rule = ReferencePrefixRule { class: class.clone(), prefix: request.prefix.clone() };
        let existing = design.rules.reference_prefixes.iter().position(|entry| entry.class == class);
        let changed = match existing {
            Some(position) if design.rules.reference_prefixes[position] == rule => false,
            Some(position) => {
                design.rules.reference_prefixes[position] = rule.clone();
                true
            }
            None => {
                design.rules.reference_prefixes.push(rule.clone());
                true
            }
        };
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("referencePrefix".to_string(), json!(rule));
        Ok(payload)
    })
}

/// Removes one reference prefix rule by component class.
///
/// Components already assigned to that class keep their assignment; the
/// `references` check then reports them as `reference/class-unknown` until the
/// rule is declared again or the assignment is cleared.
pub fn remove_reference_prefix(handle: u64, class: &str) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let position = design
            .rules
            .reference_prefixes
            .iter()
            .position(|entry| entry.class == class)
            .ok_or_else(|| not_found(format!("no reference rule for class `{class}` is declared")))?;
        let removed = design.rules.reference_prefixes.remove(position);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.class));
        payload.insert("remaining".to_string(), json!(design.rules.reference_prefixes.len()));
        Ok(payload)
    })
}

/// Lists the reference prefix rules declared by one design.
pub fn list_reference_prefixes(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        Ok(json!({
            "revision": session.design.revision,
            "referencePrefixes": session.design.rules.reference_prefixes,
        }))
    })
}

/// Assigns one placed component to a component class, or clears it.
///
/// The class must already have a reference prefix rule, so an assignment can
/// never name an unknown class at declaration time. The assignment is stored by
/// the component's stable id, so annotation and reference renames do not detach
/// it. An empty `class` clears the assignment, and an unchanged assignment adds
/// no revision.
pub fn set_component_class(handle: u64, request: ComponentClassRequest) -> Result<Value, ServiceError> {
    let class = request.class.trim().to_string();
    if !class.is_empty() && breaks_net_grammar(&class) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("component class `{class}` contains whitespace or a control character"),
        ));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let component =
            design.sheets[index].components.iter().find(|component| component.reference == request.reference).ok_or_else(
                || not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)),
            )?;
        let id = component.id.clone();
        if !class.is_empty() && !design.rules.reference_prefixes.iter().any(|rule| rule.class == class) {
            return Err(not_found(format!("no reference rule for class `{class}` is declared")));
        }
        let previous = design.rules.component_classes.get(&id).cloned();
        let changed = if class.is_empty() {
            design.rules.component_classes.remove(&id).is_some()
        }
        else if previous.as_deref() == Some(class.as_str()) {
            false
        }
        else {
            design.rules.component_classes.insert(id.clone(), class.clone());
            true
        };
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("reference".to_string(), json!(request.reference));
        payload.insert("componentId".to_string(), json!(id));
        payload.insert("class".to_string(), if class.is_empty() { Value::Null } else { json!(class) });
        Ok(payload)
    })
}

/// Lists the component class assignments with their resolved references.
pub fn list_component_classes(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let mut assignments = Vec::new();
        for sheet in &design.sheets {
            for component in &sheet.components {
                if let Some(class) = design.rules.component_classes.get(&component.id) {
                    assignments.push(json!({
                        "componentId": component.id,
                        "reference": component.reference,
                        "sheetId": sheet.id,
                        "class": class,
                    }));
                }
            }
        }
        Ok(json!({
            "revision": design.revision,
            "componentClasses": assignments,
        }))
    })
}

/// Adds one component to a sheet and returns the created object.
pub fn add_component(handle: u64, request: AddComponentRequest) -> Result<Value, ServiceError> {
    if request.reference.trim().is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a component reference must not be empty"));
    }
    if !request.x.is_finite() || !request.y.is_finite() || !request.rotation.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "component coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        if design.sheets.iter().any(|sheet| sheet.components.iter().any(|component| component.reference == request.reference)) {
            return Err(duplicate(format!("reference `{}` already exists in this design", request.reference)));
        }
        let sheet = &mut design.sheets[index];
        let mut used: HashSet<String> = sheet.components.iter().map(|component| component.id.clone()).collect();
        let id = next_id(&mut design.next_component, "component", &mut used);
        let component = Component {
            id,
            reference: request.reference,
            value: request.value,
            footprint: String::new(),
            x: request.x,
            y: request.y,
            rotation: request.rotation,
            pins: request.pins,
            pin_types: BTreeMap::new(),
        };
        sheet.components.push(component.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        Ok(payload)
    })
}

/// Deletes one component and every net node that referenced it.
///
/// Deleting a component removes its nodes rather than rejecting the edit. A net
/// left without nodes is dropped, and wires that pointed at that net are
/// detached so the document stays valid.
pub fn delete_component(handle: u64, reference: &str, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, sheet)?;
        let removed_id = design.sheets[index]
            .components
            .iter()
            .find(|component| component.reference == reference)
            .map(|component| component.id.clone())
            .ok_or_else(|| not_found(format!("no component with reference `{reference}` is placed on this sheet")))?;
        design.sheets[index].components.retain(|component| component.reference != reference);
        design.rules.component_classes.remove(&removed_id);
        let target = &mut design.sheets[index];
        for net in &mut target.nets {
            net.nodes.retain(|node| node.reference != reference);
        }
        let mut dropped: HashSet<String> = HashSet::new();
        target.nets.retain(|net| {
            if net.nodes.is_empty() {
                dropped.insert(net.name.clone());
                false
            }
            else {
                true
            }
        });
        for wire in &mut target.wires {
            if let Some(name) = &wire.net
                && dropped.contains(name)
            {
                wire.net = None;
            }
        }
        let dropped: Vec<String> = dropped.into_iter().collect();
        prune_net_class_nets(design, &dropped);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(reference));
        payload.insert("removedNets".to_string(), json!(dropped));
        Ok(payload)
    })
}

/// Moves one placed component, keeping its reference, value and pins.
pub fn move_component(handle: u64, request: MoveComponentRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "component coordinates must be finite numbers"));
    }
    if let Some(rotation) = request.rotation
        && !rotation.is_finite()
    {
        return Err(ServiceError::new("session/argument-invalid", "component rotation must be a finite number"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        component.x = request.x;
        component.y = request.y;
        if let Some(rotation) = request.rotation {
            component.rotation = rotation;
        }
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        Ok(payload)
    })
}

/// Updates the value text of one placed component.
pub fn update_component(handle: u64, request: UpdateComponentRequest) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        component.value = request.value;
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        Ok(payload)
    })
}

/// Assigns or clears the footprint text of one placed component.
///
/// An empty `footprint` clears the assignment. The operation never invents a
/// footprint: it stores exactly the text the caller passed, trimmed, so the BOM
/// and KiCad writer emit it verbatim.
pub fn set_component_footprint(handle: u64, request: SetFootprintRequest) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        component.footprint = request.footprint.trim().to_string();
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        Ok(payload)
    })
}

/// Assigns or clears the electrical role of one component pin.
///
/// The pin must be declared by the component. A missing or null `pin_type`
/// clears the role. Assigning the role the pin already has is a no-op and adds
/// no revision. The role feeds the `electrical` inspection check.
pub fn set_pin_type(handle: u64, request: SetPinTypeRequest) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let (reference, current, changed) = {
            let component =
                sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                    not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
                })?;
            if !component.pins.contains(&request.pin) {
                return Err(not_found(format!("component `{}` does not declare pin `{}`", request.reference, request.pin)));
            }
            let changed = match request.pin_type {
                Some(pin_type) => component.pin_types.insert(request.pin.clone(), pin_type) != Some(pin_type),
                None => component.pin_types.remove(&request.pin).is_some(),
            };
            (component.reference.clone(), component.pin_types.get(&request.pin).copied(), changed)
        };
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("reference".to_string(), json!(reference));
        payload.insert("pin".to_string(), json!(request.pin));
        payload.insert("pinType".to_string(), json!(current));
        Ok(payload)
    })
}

/// Renumbers every component reference designator deterministically.
///
/// Components are visited sheet by sheet, then top to bottom and left to right,
/// and numbered per reference prefix starting at 1. Net nodes follow their
/// component, so connectivity is preserved. A design that is already annotated
/// keeps its revision.
pub fn annotate(handle: u64) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let mut targets: Vec<(usize, usize)> = Vec::new();
        for (sheet_index, sheet) in design.sheets.iter().enumerate() {
            for component_index in 0..sheet.components.len() {
                targets.push((sheet_index, component_index));
            }
        }
        targets.sort_by(|&(left_sheet, left_index), &(right_sheet, right_index)| {
            let left = &design.sheets[left_sheet].components[left_index];
            let right = &design.sheets[right_sheet].components[right_index];
            left_sheet
                .cmp(&right_sheet)
                .then(left.y.total_cmp(&right.y))
                .then(left.x.total_cmp(&right.x))
                .then(left.reference.cmp(&right.reference))
        });

        let mut counters: BTreeMap<String, u64> = BTreeMap::new();
        let mut renames: HashMap<String, String> = HashMap::new();
        let mut changes = Vec::new();
        for (sheet_index, component_index) in targets {
            let component = &design.sheets[sheet_index].components[component_index];
            let prefix = designator_prefix(&component.reference);
            let counter = counters.entry(prefix.clone()).or_insert(0);
            *counter += 1;
            let next = format!("{prefix}{counter}");
            if component.reference != next {
                renames.insert(component.reference.clone(), next.clone());
                changes.push(json!({
                    "sheetId": design.sheets[sheet_index].id,
                    "componentId": component.id,
                    "from": component.reference,
                    "to": next,
                }));
            }
        }

        for sheet in &mut design.sheets {
            for component in &mut sheet.components {
                if let Some(next) = renames.get(&component.reference) {
                    component.reference = next.clone();
                }
            }
            for net in &mut sheet.nets {
                for node in &mut net.nodes {
                    if let Some(next) = renames.get(&node.reference) {
                        node.reference = next.clone();
                    }
                }
            }
        }
        for variant in &mut design.variants {
            for reference in &mut variant.dnp {
                if let Some(next) = renames.get(reference) {
                    *reference = next.clone();
                }
            }
            let values = variant
                .values
                .iter()
                .map(|(reference, value)| (renames.get(reference).cloned().unwrap_or_else(|| reference.clone()), value.clone()))
                .collect();
            variant.values = values;
        }
        if !changes.is_empty() {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("annotated".to_string(), json!(renames.len()));
        payload.insert("changes".to_string(), json!(changes));
        Ok(payload)
    })
}

/// Letters at the start of a reference designator; a designator without one
/// belongs to the generic `U` group.
fn designator_prefix(reference: &str) -> String {
    let prefix: String = reference.chars().take_while(|character| character.is_ascii_alphabetic()).collect();
    if prefix.is_empty() { "U".to_string() } else { prefix }
}

/// Connects nodes to a named net, creating the net when it does not exist.
pub fn connect_net(handle: u64, request: ConnectNetRequest) -> Result<Value, ServiceError> {
    if request.net.trim().is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a net name must not be empty"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let mut parsed = Vec::new();
        for node in &request.nodes {
            let (reference, pin) = node
                .rsplit_once('.')
                .filter(|(reference, pin)| !reference.is_empty() && !pin.is_empty())
                .ok_or_else(|| ServiceError::new("session/argument-invalid", format!("`{node}` is not a `REF.PIN` node")))?;
            let component = sheet
                .components
                .iter()
                .find(|component| component.reference == reference)
                .ok_or_else(|| not_found(format!("no component with reference `{reference}` is placed on this sheet")))?;
            if !component.pins.iter().any(|candidate| candidate == pin) {
                return Err(not_found(format!("component `{reference}` does not declare pin `{pin}`")));
            }
            parsed.push(Node { reference: reference.to_string(), pin: pin.to_string() });
        }
        let index = sheet.nets.iter().position(|net| net.name == request.net);
        let net_index = match index {
            Some(index) => index,
            None => {
                let mut used: HashSet<String> = sheet.nets.iter().map(|net| net.id.clone()).collect();
                let id = next_id(&mut design.next_net, "net", &mut used);
                sheet.nets.push(Net { id, name: request.net.clone(), nodes: Vec::new() });
                sheet.nets.len() - 1
            }
        };
        let net = &mut sheet.nets[net_index];
        for node in parsed {
            if !net.nodes.contains(&node) {
                net.nodes.push(node);
            }
        }
        let net = net.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("net".to_string(), json!(net));
        Ok(payload)
    })
}

/// Renames a net and every wire that referenced its old name.
pub fn rename_net(handle: u64, request: RenameNetRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a net name must not be empty"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let target = sheet
            .nets
            .iter()
            .position(|net| net.id == request.net || net.name == request.net)
            .ok_or_else(|| not_found(format!("no net `{}` is declared on this sheet", request.net)))?;
        if sheet.nets.iter().any(|net| net.id != sheet.nets[target].id && net.name == name) {
            return Err(duplicate(format!("net name `{name}` already exists on this sheet")));
        }
        let previous = sheet.nets[target].name.clone();
        sheet.nets[target].name = name.clone();
        for wire in &mut sheet.wires {
            if wire.net.as_deref() == Some(previous.as_str()) {
                wire.net = Some(name.clone());
            }
        }
        for net_class in &mut design.rules.net_classes {
            if net_class.nets.iter().any(|entry| entry == &previous) {
                net_class.nets.retain(|entry| entry != &previous && entry != &name);
                net_class.nets.push(name.clone());
            }
        }
        let net = sheet.nets[target].clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("net".to_string(), json!(net));
        payload.insert("previousName".to_string(), json!(previous));
        Ok(payload)
    })
}

/// Derives net names for the connected wire groups of one sheet.
///
/// A group that already names one net keeps that label, and the net is created
/// when the label is not declared yet. A group without a label receives a fresh
/// `N$<n>` that avoids every declared name and every label already in use.
/// Conflicting groups are left untouched for an explicit edit. The payload
/// reports `assigned` groups that changed, `created` nets that were added and
/// the total number of connected `groups` found. The edit always bumps the
/// revision and records undo.
pub fn derive_nets(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let discovered = connectivity::groups(&design.sheets[sheet_position]);
        let total = discovered.len();
        let sheet = &mut design.sheets[sheet_position];
        let mut used: HashSet<String> = sheet.nets.iter().map(|net| net.name.clone()).collect();
        used.extend(sheet.wires.iter().filter_map(|wire| wire.net.clone()).filter(|net| !net.is_empty()));
        let mut counter = 1u64;
        let mut assigned = 0usize;
        let mut created = 0usize;
        for group in &discovered {
            if group.is_conflict() {
                continue;
            }
            let name = match group.nets.first() {
                Some(name) => name.clone(),
                None => loop {
                    let candidate = format!("N${counter}");
                    counter += 1;
                    if used.insert(candidate.clone()) {
                        break candidate;
                    }
                },
            };
            if !sheet.nets.iter().any(|net| net.name == name) {
                let mut ids: HashSet<String> = sheet.nets.iter().map(|net| net.id.clone()).collect();
                let id = next_id(&mut design.next_net, "net", &mut ids);
                sheet.nets.push(Net { id, name: name.clone(), nodes: Vec::new() });
                created += 1;
            }
            let mut changed = false;
            for &wire in &group.wires {
                if sheet.wires[wire].net.as_deref() != Some(name.as_str()) {
                    sheet.wires[wire].net = Some(name.clone());
                    changed = true;
                }
            }
            if changed {
                assigned += 1;
            }
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("assigned".to_string(), json!(assigned));
        payload.insert("created".to_string(), json!(created));
        payload.insert("groups".to_string(), json!(total));
        Ok(payload)
    })
}

/// Removes `REF.PIN` nodes from one net without deleting the net or its wires.
///
/// An empty `nodes` list removes every node. Nodes that are not on the net are
/// ignored. The payload returns the net, the `remaining` node count and how
/// many nodes were `removed`. The revision only moves when a node was removed.
pub fn disconnect_net(handle: u64, request: DisconnectNetRequest) -> Result<Value, ServiceError> {
    let mut wanted = Vec::with_capacity(request.nodes.len());
    for node in &request.nodes {
        let Some((reference, pin)) = node.split_once('.')
        else {
            return Err(ServiceError::new("session/argument-invalid", format!("`{node}` is not a `REF.PIN` node")));
        };
        if reference.is_empty() || pin.is_empty() {
            return Err(ServiceError::new("session/argument-invalid", format!("`{node}` is not a `REF.PIN` node")));
        }
        wanted.push((reference.to_string(), pin.to_string()));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[sheet_position];
        let position = sheet
            .nets
            .iter()
            .position(|net| net.id == request.net || net.name == request.net)
            .ok_or_else(|| not_found(format!("no net `{}` is declared on this sheet", request.net)))?;
        let before = sheet.nets[position].nodes.len();
        if wanted.is_empty() {
            sheet.nets[position].nodes.clear();
        }
        else {
            sheet.nets[position]
                .nodes
                .retain(|node| !wanted.iter().any(|(reference, pin)| node.reference == *reference && node.pin == *pin));
        }
        let removed = before - sheet.nets[position].nodes.len();
        let net = sheet.nets[position].clone();
        if removed > 0 {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("net".to_string(), json!(net));
        payload.insert("remaining".to_string(), json!(net.nodes.len()));
        payload.insert("removed".to_string(), json!(removed));
        Ok(payload)
    })
}

/// Deletes one net and detaches every wire that named it.
///
/// Wires are kept, their `net` becomes `null`, and the payload reports the
/// `deleted` net name together with the `detached` wire count.
pub fn delete_net(handle: u64, request: DeleteNetRequest) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[sheet_position];
        let position = sheet
            .nets
            .iter()
            .position(|net| net.id == request.net || net.name == request.net)
            .ok_or_else(|| not_found(format!("no net `{}` is declared on this sheet", request.net)))?;
        let removed = sheet.nets.remove(position);
        let mut detached = 0usize;
        for wire in &mut sheet.wires {
            if wire.net.as_deref() == Some(removed.name.as_str()) {
                wire.net = None;
                detached += 1;
            }
        }
        prune_net_class_nets(design, std::slice::from_ref(&removed.name));
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.name));
        payload.insert("detached".to_string(), json!(detached));
        Ok(payload)
    })
}

/// Adds one no-connect flag to a sheet.
///
/// A flag without a `node` is a plain placement mark. A flag with a `node`
/// additionally marks that component pin as deliberately open; the pin must
/// already exist, and one pin can only carry one flag.
pub fn add_no_connect(handle: u64, request: AddNoConnectRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "no-connect coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &design.sheets[index];
        if let Some(node) = &request.node {
            let component = sheet
                .components
                .iter()
                .find(|component| component.reference == node.reference)
                .ok_or_else(|| not_found(format!("no component `{}` is placed on this sheet", node.reference)))?;
            if !component.pins.iter().any(|pin| pin == &node.pin) {
                return Err(not_found(format!("component `{}` declares no pin `{}`", node.reference, node.pin)));
            }
            if sheet.no_connects.iter().any(|flag| flag.node.as_ref() == Some(node)) {
                return Err(duplicate(format!("pin `{}.{}` already carries a no-connect flag", node.reference, node.pin)));
            }
        }
        let flag = NoConnect { x: request.x, y: request.y, node: request.node };
        design.sheets[index].no_connects.push(flag.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("noConnect".to_string(), json!(flag));
        Ok(payload)
    })
}

/// Deletes the no-connect flag at `index` (0-based) on a sheet.
pub fn delete_no_connect(handle: u64, index: usize, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[position];
        if index >= target.no_connects.len() {
            return Err(not_found(format!("no no-connect flag exists at index {index} on this sheet")));
        }
        target.no_connects.remove(index);
        let remaining = target.no_connects.len();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("index".to_string(), json!(index));
        payload.insert("remaining".to_string(), json!(remaining));
        Ok(payload)
    })
}

/// Deletes the junction at `index` (0-based) on a sheet.
///
/// The index is checked against the sheet's junction list; an out-of-range
/// value is reported as `object/not-found`.
pub fn delete_junction(handle: u64, index: usize, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        if index >= target.junctions.len() {
            return Err(not_found(format!("no junction at index {index} on this sheet")));
        }
        let removed = target.junctions.remove(index);
        let remaining = target.junctions.len();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("index".to_string(), json!(index));
        payload.insert("junction".to_string(), json!(removed));
        payload.insert("remaining".to_string(), json!(remaining));
        Ok(payload)
    })
}

/// Moves the junction at `index` (0-based) to `(x, y)`.
///
/// Coordinates must be finite and the index must be inside the sheet's
/// junction list.
pub fn move_junction(handle: u64, index: usize, sheet: Option<&str>, x: f64, y: f64) -> Result<Value, ServiceError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "junction coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        let Some(junction) = target.junctions.get_mut(index)
        else {
            return Err(not_found(format!("no junction at index {index} on this sheet")));
        };
        junction.x = x;
        junction.y = y;
        let moved = junction.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("index".to_string(), json!(index));
        payload.insert("junction".to_string(), json!(moved));
        Ok(payload)
    })
}

/// Deletes the text annotation at `index` (0-based) on a sheet.
///
/// The index is checked against the sheet's text list; an out-of-range value is
/// reported as `object/not-found`.
pub fn delete_text(handle: u64, index: usize, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        if index >= target.texts.len() {
            return Err(not_found(format!("no text at index {index} on this sheet")));
        }
        let removed = target.texts.remove(index);
        let remaining = target.texts.len();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("index".to_string(), json!(index));
        payload.insert("text".to_string(), json!(removed));
        payload.insert("remaining".to_string(), json!(remaining));
        Ok(payload)
    })
}

/// Moves the text annotation at `index` (0-based) to `(x, y)`.
///
/// Coordinates must be finite and the index must be inside the sheet's text
/// list.
pub fn move_text(handle: u64, index: usize, sheet: Option<&str>, x: f64, y: f64) -> Result<Value, ServiceError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "text coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        let Some(text) = target.texts.get_mut(index)
        else {
            return Err(not_found(format!("no text at index {index} on this sheet")));
        };
        text.x = x;
        text.y = y;
        let moved = text.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("index".to_string(), json!(index));
        payload.insert("text".to_string(), json!(moved));
        Ok(payload)
    })
}

/// Adds one wire segment to a sheet.
pub fn add_wire(handle: u64, request: AddWireRequest) -> Result<Value, ServiceError> {
    if !request.from.iter().chain(request.to.iter()).all(|value| value.is_finite()) {
        return Err(ServiceError::new("session/argument-invalid", "wire coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let mut used: HashSet<String> = sheet.wires.iter().map(|wire| wire.id.clone()).collect();
        let id = next_id(&mut design.next_wire, "wire", &mut used);
        let wire = Wire { id, net: request.net, from: request.from, to: request.to };
        sheet.wires.push(wire.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("wire".to_string(), json!(wire));
        Ok(payload)
    })
}

/// Updates the endpoints or net assignment of one wire segment.
///
/// An omitted field keeps its current value, so a request only carries what it
/// changes. An empty `net` clears the assignment; a non-empty name is stored
/// verbatim, matching [`add_wire`], which does not require the net to exist.
/// A request that changes nothing is rejected instead of adding a revision.
pub fn update_wire(handle: u64, request: UpdateWireRequest) -> Result<Value, ServiceError> {
    for point in request.from.iter().chain(request.to.iter()) {
        if !point.iter().all(|value| value.is_finite()) {
            return Err(ServiceError::new("session/argument-invalid", "wire coordinates must be finite numbers"));
        }
    }
    if request.net.is_none() && request.from.is_none() && request.to.is_none() {
        return Err(ServiceError::new("session/argument-invalid", "a wire update needs `net`, `from` or `to`"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let wire = sheet
            .wires
            .iter_mut()
            .find(|candidate| candidate.id == request.id)
            .ok_or_else(|| not_found(format!("no wire `{}` is drawn on this sheet", request.id)))?;
        if let Some(net) = request.net {
            let trimmed = net.trim();
            wire.net = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
        }
        if let Some(from) = request.from {
            wire.from = from;
        }
        if let Some(to) = request.to {
            wire.to = to;
        }
        let updated = wire.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("wire".to_string(), json!(updated));
        Ok(payload)
    })
}

/// Lists the wires drawn on one sheet.
pub fn list_wires(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let target = &design.sheets[index];
        Ok(json!({
            "revision": design.revision,
            "design": summary_of(session),
            "sheetId": target.id,
            "wires": target.wires,
        }))
    })
}

/// Deletes one wire segment.
pub fn delete_wire(handle: u64, wire: &str, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, sheet)?;
        let target = &mut design.sheets[index];
        let before = target.wires.len();
        target.wires.retain(|candidate| candidate.id != wire);
        if target.wires.len() == before {
            return Err(not_found(format!("no wire `{wire}` is drawn on this sheet")));
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(wire));
        Ok(payload)
    })
}

/// Adds one junction (connection dot) to a sheet.
pub fn add_junction(handle: u64, request: AddJunctionRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "junction coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let junction = Junction { x: request.x, y: request.y };
        design.sheets[index].junctions.push(junction.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("junction".to_string(), json!(junction));
        Ok(payload)
    })
}

/// Adds one free text annotation to a sheet.
pub fn add_text(handle: u64, request: AddTextRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "text coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let text = TextLabel { text: request.text, x: request.x, y: request.y };
        design.sheets[index].texts.push(text.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("text".to_string(), json!(text));
        Ok(payload)
    })
}

/// Adds an empty sheet and returns it.
pub fn add_sheet(handle: u64, name: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let name = match name {
            Some(name) if name.trim().is_empty() => {
                return Err(ServiceError::new("session/argument-invalid", "a sheet name must not be empty"));
            }
            Some(name) => name.trim().to_string(),
            None => {
                let mut counter = design.sheets.len() + 1;
                loop {
                    let candidate = format!("Sheet {counter}");
                    if !design.sheets.iter().any(|sheet| sheet.name == candidate) {
                        break candidate;
                    }
                    counter += 1;
                }
            }
        };
        if design.sheets.iter().any(|sheet| sheet.name == name) {
            return Err(duplicate(format!("sheet name `{name}` already exists")));
        }
        let id = free_sheet_id(design);
        let sheet = Sheet {
            id,
            name,
            components: Vec::new(),
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
        };
        design.sheets.push(sheet.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("sheet".to_string(), json!(sheet));
        Ok(payload)
    })
}

/// Renames one sheet; names must stay unique inside the design.
pub fn rename_sheet(handle: u64, sheet: &str, name: &str) -> Result<Value, ServiceError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a sheet name must not be empty"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, Some(sheet))?;
        if design.sheets.iter().enumerate().any(|(position, candidate)| position != index && candidate.name == name) {
            return Err(duplicate(format!("sheet name `{name}` already exists")));
        }
        let target = &mut design.sheets[index];
        target.name = name.clone();
        let sheet = target.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("sheet".to_string(), json!(sheet));
        Ok(payload)
    })
}

/// Deletes one empty sheet.
///
/// The last sheet cannot be deleted, and a sheet that still holds components,
/// nets or wires is refused with `sheet/not-empty` instead of dropping objects.
pub fn delete_sheet(handle: u64, sheet: &str) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, Some(sheet))?;
        if design.sheets.len() == 1 {
            return Err(ServiceError::new("sheet/last-sheet", "a design must keep at least one sheet"));
        }
        let target = &design.sheets[index];
        let objects = target.components.len() + target.nets.len() + target.wires.len();
        if objects > 0 {
            return Err(ServiceError::new(
                "sheet/not-empty",
                format!("sheet `{}` still holds {objects} object(s); delete them before the sheet", target.id),
            ));
        }
        let removed = design.sheets.remove(index);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("sheet".to_string(), json!({ "id": removed.id, "name": removed.name }));
        Ok(payload)
    })
}

/// Restores the document snapshot taken before the newest edit.
pub fn undo(handle: u64) -> Result<Value, ServiceError> {
    let mut sessions = locked_sessions();
    let session = sessions
        .get_mut(&handle)
        .ok_or_else(|| ServiceError::new("design/not-found", format!("no design is open under handle {handle}")))?;
    let revision = session.design.revision;
    let current = session.design.clone();
    let snapshot = session
        .history
        .undo(current)
        .ok_or_else(|| ServiceError::new("history/empty", "there is no edit to undo on this handle"))?;
    session.design = snapshot;
    session.design.revision = revision + 1;
    Ok(json!({
        "revision": session.design.revision,
        "design": summary_of(session),
        "canUndo": session.history.can_undo(),
        "canRedo": session.history.can_redo(),
    }))
}

/// Reapplies the snapshot that the newest undo displaced.
pub fn redo(handle: u64) -> Result<Value, ServiceError> {
    let mut sessions = locked_sessions();
    let session = sessions
        .get_mut(&handle)
        .ok_or_else(|| ServiceError::new("design/not-found", format!("no design is open under handle {handle}")))?;
    let revision = session.design.revision;
    let current = session.design.clone();
    let snapshot = session
        .history
        .redo(current)
        .ok_or_else(|| ServiceError::new("history/empty", "there is no undone edit to redo on this handle"))?;
    session.design = snapshot;
    session.design.revision = revision + 1;
    Ok(json!({
        "revision": session.design.revision,
        "design": summary_of(session),
        "canUndo": session.history.can_undo(),
        "canRedo": session.history.can_redo(),
    }))
}

/// Closes one open design and releases its handle and history.
pub fn close(handle: u64) -> Result<(), ServiceError> {
    let mut sessions = locked_sessions();
    match sessions.remove(&handle) {
        Some(_) => Ok(()),
        None => Err(ServiceError::new("design/not-found", format!("no design is open under handle {handle}"))),
    }
}

/// Projects one open design to an exchange target.
pub fn export(handle: u64, format: &str, path: &Path) -> Result<Value, ServiceError> {
    export_with_variant(handle, format, path, None)
}

/// Projects one open design to an exchange target, optionally through a BOM
/// variant.
///
/// A variant only changes `bom`, `bom_json` and `bom_html`: its DNP designators
/// are left out
/// and its value overrides replace the component value. Any other format rejects
/// the variant instead of silently ignoring it.
pub fn export_with_variant(handle: u64, format: &str, path: &Path, variant: Option<&str>) -> Result<Value, ServiceError> {
    if !SUPPORTED_EXPORTS.contains(&format) {
        return Err(not_implemented(&format!("export `{format}`")));
    }
    if variant.is_some() && format != "bom" && format != "bom_json" && format != "bom_html" {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("a BOM variant applies to `bom`, `bom_json` and `bom_html`, not `{format}`"),
        ));
    }
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let selected = match variant {
        Some(name) => Some(
            design
                .variants
                .iter()
                .find(|entry| entry.name == name)
                .ok_or_else(|| not_found(format!("no variant `{name}` is declared")))?,
        ),
        None => None,
    };
    let mut losses: Vec<Value> = Vec::new();
    let (contents, artifact) = match format {
        "net" => {
            let artifact = json!({
                "kind": "net",
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
                "nodes": design
                    .sheets
                    .iter()
                    .flat_map(|sheet| &sheet.nets)
                    .map(|net| net.nodes.len())
                    .sum::<usize>(),
            });
            (render_netlist(&design), artifact)
        }
        "net_csv" => {
            let (contents, rows) = render_net_csv(&design);
            let artifact = json!({
                "kind": "net_csv",
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
                "rows": rows,
            });
            (contents, artifact)
        }
        "net_hier" => {
            let artifact = json!({
                "kind": "net_hier",
                "sheets": design.sheets.len(),
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
                "nodes": design
                    .sheets
                    .iter()
                    .flat_map(|sheet| &sheet.nets)
                    .map(|net| net.nodes.len())
                    .sum::<usize>(),
                "buses": design.buses.len(),
            });
            (render_hierarchical_netlist(&design), artifact)
        }
        "spice" => {
            let (contents, spice_losses) = render_spice(&design);
            losses = spice_losses;
            let artifact = json!({
                "kind": "spice",
                "elements": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
            });
            (contents, artifact)
        }
        "spice_subckt" => {
            let (contents, spice_losses) = render_spice_subcircuits(&design);
            losses = spice_losses;
            let artifact = json!({
                "kind": "spice_subckt",
                "subcircuits": design.sheets.len(),
                "elements": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
            });
            (contents, artifact)
        }
        "svg" => {
            let artifact = json!({
                "kind": "svg",
                "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": design.sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
            });
            (render_svg(&design), artifact)
        }
        "bom" => {
            let rows = bom_rows(&design, selected);
            let mut artifact = json!({
                "kind": "bom",
                "rows": rows.len(),
                "components": rows.iter().map(|row| row.designators.len()).sum::<usize>(),
            });
            if let Some(variant) = selected {
                artifact["variant"] = json!(variant.name);
                artifact["dnp"] = json!(variant.dnp.len());
            }
            (render_bom(&rows), artifact)
        }
        "bom_json" => {
            let rows = bom_rows(&design, selected);
            let mut artifact = json!({
                "kind": "bom_json",
                "rows": rows.len(),
                "components": rows.iter().map(|row| row.designators.len()).sum::<usize>(),
            });
            if let Some(variant) = selected {
                artifact["variant"] = json!(variant.name);
                artifact["dnp"] = json!(variant.dnp.len());
            }
            (render_bom_json(&design, &rows)?, artifact)
        }
        "bom_html" => {
            let rows = bom_rows(&design, selected);
            let mut artifact = json!({
                "kind": "bom_html",
                "rows": rows.len(),
                "components": rows.iter().map(|row| row.designators.len()).sum::<usize>(),
            });
            if let Some(variant) = selected {
                artifact["variant"] = json!(variant.name);
                artifact["dnp"] = json!(variant.dnp.len());
            }
            (render_bom_html(&design, &rows, selected), artifact)
        }
        "pdf" => {
            let artifact = json!({
                "kind": "pdf",
                "pages": 1,
                "mediaBox": "a4-landscape",
                "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": design.sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
            });
            (render_pdf(&design), artifact)
        }
        "report" => {
            let (contents, sections, issue_count) = render_report(&design);
            let artifact = json!({
                "kind": "report",
                "sections": sections,
                "issues": issue_count,
            });
            (contents, artifact)
        }
        "json" => {
            let artifact = json!({
                "kind": "json",
                "sheets": design.sheets.len(),
                "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
                "wires": design.sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
                "junctions": design.sheets.iter().map(|sheet| sheet.junctions.len()).sum::<usize>(),
                "noConnects": design.sheets.iter().map(|sheet| sheet.no_connects.len()).sum::<usize>(),
                "texts": design.sheets.iter().map(|sheet| sheet.texts.len()).sum::<usize>(),
                "netClasses": design.rules.net_classes.len(),
                "referencePrefixes": design.rules.reference_prefixes.len(),
            });
            (render_json_projection(&design)?, artifact)
        }
        "kicad_sch" => {
            let artifact = json!({
                "kind": "kicad_sch",
                "symbols": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": design.sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
                "labels": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
                "hierarchy": design.hierarchy.len(),
                "ports": design.hierarchy.iter().map(|sheet| sheet.ports.len()).sum::<usize>(),
            });
            (kicad::render(&design), artifact)
        }
        "kicad_sym" => {
            let mut library_ids = BTreeSet::new();
            let mut pins = 0usize;
            for sheet in &design.sheets {
                for component in &sheet.components {
                    if library_ids.insert(kicad::lib_id_for(component)) {
                        pins += component.pins.len();
                    }
                }
            }
            let artifact = json!({
                "kind": "kicad_sym",
                "symbols": library_ids.len(),
                "pins": pins,
                "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
            });
            (kicad::render_symbol_library(&design), artifact)
        }
        _ => return Err(not_implemented(&format!("export `{format}`"))),
    };
    let bytes = write_atomic(path, &contents, true)?;
    let mut report = json!({
        "revision": design.revision,
        "format": format,
        "path": path.display().to_string(),
        "bytes": bytes,
        "editable": false,
        "artifact": artifact,
    });
    if format == "spice" || format == "spice_subckt" {
        report["losses"] = json!(losses);
    }
    Ok(report)
}

/// Imports one KiCad schematic into a validated design document.
///
/// The reader covers the subset this build models: placed symbols with their
/// reference, value, placement and pins, wires and net labels. Every construct
/// outside that subset is reported in `losses` with a stable code instead of
/// being dropped silently. The returned document is not written to disk; pass
/// its `design` section to the document writer when the caller wants a `.oeda`
/// file.
pub fn import_kicad_sch(text: &str) -> Result<Value, ServiceError> {
    let (design, losses) = kicad::import(text)?;
    validate_design(&design)?;
    Ok(json!({ "design": design, "losses": losses }))
}

/// Imports one KiCad netlist into a validated design document.
///
/// A netlist carries components, their value and footprint, and the nets with
/// their `REF.PIN` nodes; it carries no placement, wires, labels or bus
/// declarations. Components are laid out on a deterministic grid and every
/// node contributes its pin, so an unconnected pin cannot be recovered. Each
/// construct outside that subset is reported in `losses` with a stable code.
/// The returned document is not written to disk.
pub fn import_kicad_net(text: &str) -> Result<Value, ServiceError> {
    let (design, losses) = kicad::import_net(text)?;
    validate_design(&design)?;
    let totals = json!({
        "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
        "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
        "pins": design
            .sheets
            .iter()
            .flat_map(|sheet| sheet.components.iter())
            .map(|component| component.pins.len())
            .sum::<usize>(),
    });
    Ok(json!({ "kind": "net", "design": design, "totals": totals, "losses": losses }))
}

/// Compares one open design with a KiCad netlist.
///
/// The design is the reference side: `added` entries exist only in the netlist,
/// `removed` entries exist only in the design, and `modified` entries list the
/// components whose value or footprint differs plus the nets whose `REF.PIN`
/// node sets differ. Placement, wires and pin order are not compared because a
/// netlist does not carry them. Netlist losses are returned next to the report.
pub fn compare_kicad_net(handle: u64, text: &str) -> Result<Value, ServiceError> {
    let (imported, losses) = kicad::import_net(text)?;
    validate_design(&imported)?;
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let mut report = kicad::compare_net(&design, &imported);
    report["losses"] = json!(losses);
    Ok(report)
}

/// Imports one KiCad `.kicad_sym` symbol library and reports its symbols.
///
/// Only the declaration is read: each symbol reports its name, unit count and
/// pins with their number, name, electrical role and unit. Nothing is written to
/// disk and no design handle is created. A construct outside that subset is
/// reported as a `kicad_sym/unsupported-element` loss entry.
pub fn import_kicad_symbol(text: &str) -> Result<Value, ServiceError> {
    kicad::import_symbol_library(text)
}

/// Renders one sheet of an open design for a read-only preview.
///
/// Only `svg` is a preview projection; `png` and `pdf` stay export targets, so
/// requesting them reports `capability/not-implemented` instead of inventing a
/// file. The returned document is the same SVG projection the export writer
/// uses, narrowed to exactly one sheet, and is deterministic for one document
/// state. Nothing is written to disk.
pub fn render(handle: u64, format: &str, sheet: Option<&str>) -> Result<Value, ServiceError> {
    if format != "svg" {
        return Err(ServiceError::new(
            "capability/not-implemented",
            format!("preview render `{format}` is not implemented in this build; use `export` to write {format}"),
        ));
    }
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let index = match sheet {
        None => 0,
        Some(needle) => design
            .sheets
            .iter()
            .position(|sheet| sheet.id == needle || sheet.name == needle)
            .ok_or_else(|| not_found(format!("sheet `{needle}` is not part of this design")))?,
    };
    let target = &design.sheets[index];
    let projection = svg_projection(&[target]);
    Ok(json!({
        "revision": design.revision,
        "format": "svg",
        "svg": projection.document,
        "width": projection.width,
        "height": projection.height,
        "sheet": target.id,
    }))
}

fn summary_of(session: &Session) -> Value {
    let design = &session.design;
    let mut totals = [0usize; 4];
    let sheets = design
        .sheets
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let counts = [sheet.components.len(), sheet.nets.len(), sheet.wires.len()];
            totals[0] += 1;
            totals[1] += counts[0];
            totals[2] += counts[1];
            totals[3] += counts[2];
            json!({
                "id": sheet.id,
                "index": index,
                "name": sheet.name,
                "title": sheet.name,
                "objects": counts.iter().sum::<usize>(),
                "components": counts[0],
                "nets": counts[1],
                "wires": counts[2],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "id": design.id,
        "revision": design.revision,
        "title": design.title,
        "rev": design.rev,
        "company": design.company,
        "date": design.date,
        "units": design.units,
        "sheets": sheets,
        "counts": {
            "sheets": totals[0],
            "components": totals[1],
            "nets": totals[2],
            "wires": totals[3],
        },
        "canUndo": session.history.can_undo(),
        "canRedo": session.history.can_redo(),
        "history": { "undo": session.history.undo_len(), "redo": session.history.redo_len(), "limit": HISTORY_LIMIT },
    })
}

fn sheet_index(design: &Design, sheet: Option<&str>) -> Result<usize, ServiceError> {
    match sheet {
        None => Ok(0),
        Some(needle) => design
            .sheets
            .iter()
            .position(|sheet| sheet.id == needle || sheet.name == needle)
            .ok_or_else(|| invalid(format!("sheet `{needle}` is not part of this design"))),
    }
}

fn next_id(counter: &mut u64, prefix: &str, used: &mut HashSet<String>) -> String {
    loop {
        let candidate = format!("{prefix}-{counter}");
        *counter += 1;
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
}

/// Picks a free `sheet-N` identity without adding a document counter field.
fn free_sheet_id(design: &Design) -> String {
    let used: HashSet<&str> = design.sheets.iter().map(|sheet| sheet.id.as_str()).collect();
    let mut counter = design.sheets.len() + 1;
    loop {
        let candidate = format!("sheet-{counter}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
        counter += 1;
    }
}

/// Picks a free `bus-N` identity without adding a document counter field.
fn free_bus_id(design: &Design) -> String {
    let used: HashSet<&str> = design.buses.iter().map(|bus| bus.id.as_str()).collect();
    let mut counter = design.buses.len() + 1;
    loop {
        let candidate = format!("bus-{counter}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
        counter += 1;
    }
}

/// Removes dropped net names from every net class assignment.
///
/// A class never keeps a net the document no longer declares, so deleting a net
/// or a component that emptied it also detaches it from its class.
fn prune_net_class_nets(design: &mut Design, dropped: &[String]) {
    if dropped.is_empty() {
        return;
    }
    for net_class in &mut design.rules.net_classes {
        net_class.nets.retain(|net| !dropped.iter().any(|name| name == net));
    }
}

/// Picks a free `net-class-N` identity without adding a document counter field.
fn free_net_class_id(design: &Design) -> String {
    let used: HashSet<&str> = design.rules.net_classes.iter().map(|entry| entry.id.as_str()).collect();
    let mut counter = design.rules.net_classes.len() + 1;
    loop {
        let candidate = format!("net-class-{counter}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
        counter += 1;
    }
}

fn validate_design(design: &Design) -> Result<(), ServiceError> {
    if design.id.trim().is_empty() {
        return Err(invalid("the design id must not be empty"));
    }
    if design.units != "mm" && design.units != "mil" {
        return Err(invalid(format!("units `{}` must be `mm` or `mil`", design.units)));
    }
    if design.sheets.is_empty() {
        return Err(invalid("a design must contain at least one sheet"));
    }
    let mut sheet_ids = HashSet::new();
    let mut references = HashSet::new();
    let mut bus_ids = HashSet::new();
    let mut bus_names = HashSet::new();
    for bus in &design.buses {
        if !bus_ids.insert(bus.id.clone()) {
            return Err(invalid(format!("bus id `{}` appears more than once", bus.id)));
        }
        if bus.name.trim().is_empty() {
            return Err(invalid(format!("bus `{}` has an empty name", bus.id)));
        }
        if !bus_names.insert(bus.name.clone()) {
            return Err(invalid(format!("bus name `{}` appears more than once", bus.name)));
        }
        if bus.width == 0 {
            return Err(invalid(format!("bus `{}` has width 0; the minimum is 1", bus.name)));
        }
        for index in 0..bus.width {
            if !bus_names.insert(format!("{}{index}", bus.name)) {
                return Err(invalid(format!("a member name of bus `{}` collides with a declared bus or alias", bus.name)));
            }
        }
        for alias in &bus.aliases {
            if alias.trim().is_empty() {
                return Err(invalid(format!("bus `{}` has an empty alias", bus.name)));
            }
            if breaks_net_grammar(alias) {
                return Err(invalid(format!("bus alias `{alias}` contains whitespace or a control character")));
            }
            if !bus_names.insert(alias.clone()) {
                return Err(invalid(format!("bus alias `{alias}` collides with a declared bus name, member name or alias")));
            }
        }
    }
    let mut variant_names = HashSet::new();
    for variant in &design.variants {
        if variant.name.trim().is_empty() {
            return Err(invalid("a variant has an empty name"));
        }
        if !variant_names.insert(variant.name.clone()) {
            return Err(invalid(format!("variant name `{}` appears more than once", variant.name)));
        }
    }
    let mut net_class_ids = HashSet::new();
    let mut net_class_names = HashSet::new();
    for net_class in &design.rules.net_classes {
        if !net_class_ids.insert(net_class.id.clone()) {
            return Err(invalid(format!("net class id `{}` appears more than once", net_class.id)));
        }
        if net_class.name.trim().is_empty() {
            return Err(invalid(format!("net class `{}` has an empty name", net_class.id)));
        }
        if breaks_net_grammar(&net_class.name) {
            return Err(invalid(format!("net class name `{}` contains whitespace or a control character", net_class.name)));
        }
        if !net_class_names.insert(net_class.name.clone()) {
            return Err(invalid(format!("net class name `{}` appears more than once", net_class.name)));
        }
        if !net_class.width.is_finite() || net_class.width <= 0.0 {
            return Err(invalid(format!("net class `{}` has a width that is not a positive number", net_class.name)));
        }
        if !net_class.clearance.is_finite() || net_class.clearance < 0.0 {
            return Err(invalid(format!("net class `{}` has a negative clearance", net_class.name)));
        }
        if !net_class.via.is_finite() || net_class.via < 0.0 {
            return Err(invalid(format!("net class `{}` has a negative via diameter", net_class.name)));
        }
        let mut assigned_nets = HashSet::new();
        for net in &net_class.nets {
            if net.trim().is_empty() {
                return Err(invalid(format!("net class `{}` assigns an empty net name", net_class.name)));
            }
            if !assigned_nets.insert(net.as_str()) {
                return Err(invalid(format!("net class `{}` assigns net `{net}` more than once", net_class.name)));
            }
            if !design.sheets.iter().any(|sheet| sheet.nets.iter().any(|entry| entry.name == *net)) {
                return Err(invalid(format!("net class `{}` assigns undeclared net `{net}`", net_class.name)));
            }
        }
        let mut assigned_buses = HashSet::new();
        for bus in &net_class.buses {
            if bus.trim().is_empty() {
                return Err(invalid(format!("net class `{}` assigns an empty bus name", net_class.name)));
            }
            if !assigned_buses.insert(bus.as_str()) {
                return Err(invalid(format!("net class `{}` assigns bus `{bus}` more than once", net_class.name)));
            }
            if !design.buses.iter().any(|entry| entry.name == *bus) {
                return Err(invalid(format!("net class `{}` assigns undeclared bus `{bus}`", net_class.name)));
            }
        }
    }
    let mut rule_classes = HashSet::new();
    for rule in &design.rules.reference_prefixes {
        if rule.class.trim().is_empty() {
            return Err(invalid("a reference rule has an empty component class"));
        }
        if breaks_net_grammar(&rule.class) {
            return Err(invalid(format!("component class `{}` contains whitespace or a control character", rule.class)));
        }
        if !rule_classes.insert(rule.class.clone()) {
            return Err(invalid(format!("reference rule class `{}` appears more than once", rule.class)));
        }
        if !valid_reference_prefix(&rule.prefix) {
            return Err(invalid(format!(
                "reference rule `{}` has prefix `{}`, which is not one to eight uppercase letters",
                rule.class, rule.prefix
            )));
        }
    }
    for (component_id, class) in &design.rules.component_classes {
        if class.trim().is_empty() {
            return Err(invalid(format!("component `{component_id}` has an empty class assignment")));
        }
        if !design.sheets.iter().any(|sheet| sheet.components.iter().any(|component| &component.id == component_id)) {
            return Err(invalid(format!("component class assignment references missing component `{component_id}`")));
        }
    }
    let mut hierarchy_ids = HashSet::new();
    let mut hierarchy_names = HashSet::new();
    for hierarchy in &design.hierarchy {
        if !hierarchy_ids.insert(hierarchy.id.clone()) {
            return Err(invalid(format!("hierarchical sheet id `{}` appears more than once", hierarchy.id)));
        }
        if hierarchy.name.trim().is_empty() {
            return Err(invalid(format!("hierarchical sheet `{}` has an empty name", hierarchy.id)));
        }
        if !hierarchy_names.insert(hierarchy.name.clone()) {
            return Err(invalid(format!("hierarchical sheet name `{}` appears more than once", hierarchy.name)));
        }
        if hierarchy.file.trim().is_empty() {
            return Err(invalid(format!("hierarchical sheet `{}` has an empty file name", hierarchy.name)));
        }
        if !hierarchy.x.is_finite() || !hierarchy.y.is_finite() || !hierarchy.width.is_finite() || !hierarchy.height.is_finite()
        {
            return Err(invalid(format!("hierarchical sheet `{}` has a non-finite coordinate", hierarchy.name)));
        }
        if hierarchy.width <= 0.0 || hierarchy.height <= 0.0 {
            return Err(invalid(format!("hierarchical sheet `{}` has a non-positive size", hierarchy.name)));
        }
        let mut port_names = HashSet::new();
        for port in &hierarchy.ports {
            if port.name.trim().is_empty() {
                return Err(invalid(format!("hierarchical sheet `{}` has a port with an empty name", hierarchy.name)));
            }
            if !port_names.insert(port.name.clone()) {
                return Err(invalid(format!(
                    "hierarchical sheet `{}` declares port `{}` more than once",
                    hierarchy.name, port.name
                )));
            }
            if !port.x.is_finite() || !port.y.is_finite() {
                return Err(invalid(format!(
                    "port `{}` of hierarchical sheet `{}` has a non-finite coordinate",
                    port.name, hierarchy.name
                )));
            }
        }
    }
    for sheet in &design.sheets {
        if !sheet_ids.insert(sheet.id.clone()) {
            return Err(invalid(format!("sheet id `{}` appears more than once", sheet.id)));
        }
        if sheet.name.trim().is_empty() {
            return Err(invalid(format!("sheet `{}` has an empty name", sheet.id)));
        }
        let mut component_ids = HashSet::new();
        let mut net_ids = HashSet::new();
        let mut net_names = HashSet::new();
        let mut wire_ids = HashSet::new();
        for component in &sheet.components {
            if !component_ids.insert(component.id.clone()) {
                return Err(invalid(format!("component id `{}` appears more than once", component.id)));
            }
            if component.reference.trim().is_empty() {
                return Err(invalid(format!("component `{}` has an empty reference", component.id)));
            }
            if !references.insert(component.reference.clone()) {
                return Err(invalid(format!("reference `{}` appears more than once", component.reference)));
            }
            if !component.x.is_finite() || !component.y.is_finite() || !component.rotation.is_finite() {
                return Err(invalid(format!("component `{}` has a non-finite coordinate", component.reference)));
            }
            for pin in component.pin_types.keys() {
                if !component.pins.iter().any(|declared| declared == pin) {
                    return Err(invalid(format!("component `{}` types missing pin `{pin}`", component.reference)));
                }
            }
        }
        for net in &sheet.nets {
            if !net_ids.insert(net.id.clone()) {
                return Err(invalid(format!("net id `{}` appears more than once", net.id)));
            }
            if net.name.trim().is_empty() {
                return Err(invalid(format!("net `{}` has an empty name", net.id)));
            }
            if !net_names.insert(net.name.clone()) {
                return Err(invalid(format!("net name `{}` appears more than once on sheet `{}`", net.name, sheet.id)));
            }
            for node in &net.nodes {
                let component =
                    sheet.components.iter().find(|component| component.reference == node.reference).ok_or_else(|| {
                        invalid(format!("net `{}` references missing component `{}`", net.name, node.reference))
                    })?;
                if !component.pins.iter().any(|pin| pin == &node.pin) {
                    return Err(invalid(format!(
                        "net `{}` references missing pin `{}` on component `{}`",
                        net.name, node.pin, node.reference
                    )));
                }
            }
        }
        for wire in &sheet.wires {
            if !wire_ids.insert(wire.id.clone()) {
                return Err(invalid(format!("wire id `{}` appears more than once", wire.id)));
            }
            if !wire.from.iter().chain(wire.to.iter()).all(|value| value.is_finite()) {
                return Err(invalid(format!("wire `{}` has a non-finite coordinate", wire.id)));
            }
            if let Some(net) = &wire.net
                && !net_names.contains(net)
            {
                return Err(invalid(format!("wire `{}` references undeclared net `{net}`", wire.id)));
            }
        }
        for junction in &sheet.junctions {
            if !junction.x.is_finite() || !junction.y.is_finite() {
                return Err(invalid(format!("sheet `{}` has a junction with a non-finite coordinate", sheet.id)));
            }
        }
        let mut no_connect_pins = HashSet::new();
        for no_connect in &sheet.no_connects {
            if !no_connect.x.is_finite() || !no_connect.y.is_finite() {
                return Err(invalid(format!("sheet `{}` has a no-connect with a non-finite coordinate", sheet.id)));
            }
            if let Some(node) = &no_connect.node {
                let component = sheet
                    .components
                    .iter()
                    .find(|component| component.reference == node.reference)
                    .ok_or_else(|| invalid(format!("a no-connect references missing component `{}`", node.reference)))?;
                if !component.pins.iter().any(|pin| pin == &node.pin) {
                    return Err(invalid(format!(
                        "a no-connect references missing pin `{}` on component `{}`",
                        node.pin, node.reference
                    )));
                }
                if !no_connect_pins.insert((node.reference.clone(), node.pin.clone())) {
                    return Err(invalid(format!(
                        "pin `{}.{}` carries more than one no-connect flag",
                        node.reference, node.pin
                    )));
                }
            }
        }
        for text in &sheet.texts {
            if !text.x.is_finite() || !text.y.is_finite() {
                return Err(invalid(format!("sheet `{}` has a text with a non-finite coordinate", sheet.id)));
            }
        }
    }
    for variant in &design.variants {
        for reference in variant.dnp.iter().chain(variant.values.keys()) {
            if !references.contains(reference) {
                return Err(invalid(format!("variant `{}` references missing component `{reference}`", variant.name)));
            }
        }
    }
    Ok(())
}

/// Whether a configured prefix is one to eight uppercase ASCII letters.
fn valid_reference_prefix(prefix: &str) -> bool {
    !prefix.is_empty() && prefix.len() <= 8 && prefix.chars().all(|character| character.is_ascii_uppercase())
}

/// Splits a reference designator into its leading letters and the remaining text.
fn reference_parts(reference: &str) -> (&str, &str) {
    let split = reference.find(|character: char| !character.is_ascii_alphabetic()).unwrap_or(reference.len());
    reference.split_at(split)
}

/// Whether a reference or pin name breaks the `REF.PIN` node grammar.
///
/// The netlist and SPICE writers join a reference and a pin with a dot, so
/// whitespace or a dot inside either token makes the node ambiguous.
fn breaks_node_grammar(text: &str) -> bool {
    text.chars().any(|character| character.is_whitespace() || character.is_control() || character == '.')
}

/// Whether a net name breaks the `NET <name>` line grammar.
///
/// A net name is written verbatim on one line, so whitespace or a control
/// character would split the record.
fn breaks_net_grammar(text: &str) -> bool {
    text.chars().any(|character| character.is_whitespace() || character.is_control())
}

fn inspect_design(design: &Design, checks: &[String]) -> Vec<Value> {
    let mut issues = CheckReport::new();
    if checks.iter().any(|check| check == "erc") {
        let mut seen = HashSet::new();
        for sheet in &design.sheets {
            for component in &sheet.components {
                if component.reference.trim().is_empty() {
                    issues.push(issue(
                        "component-empty-reference",
                        "error",
                        design.revision,
                        &sheet.id,
                        &component.id,
                        true,
                        format!("component `{}` has an empty reference", component.id),
                    ));
                }
                if !seen.insert(component.reference.clone()) {
                    issues.push(issue(
                        "duplicate-reference",
                        "error",
                        design.revision,
                        &sheet.id,
                        &component.id,
                        true,
                        format!("reference `{}` is declared more than once", component.reference),
                    ));
                }
                let mut pins = HashSet::new();
                for pin in &component.pins {
                    if !pins.insert(pin.as_str()) {
                        issues.push(issue(
                            "erc/duplicate-pin",
                            "error",
                            design.revision,
                            &sheet.id,
                            &component.id,
                            true,
                            format!("component `{}` declares pin `{pin}` more than once", component.reference),
                        ));
                    }
                    if breaks_node_grammar(pin) {
                        issues.push(issue(
                            "erc/pin-name-illegal",
                            "error",
                            design.revision,
                            &sheet.id,
                            &component.id,
                            true,
                            format!(
                                "component `{}` declares pin `{pin}`, which contains whitespace or a dot; `REF.PIN` nodes cannot carry it",
                                component.reference
                            ),
                        ));
                    }
                }
                if breaks_node_grammar(&component.reference) {
                    issues.push(issue(
                        "erc/reference-illegal",
                        "error",
                        design.revision,
                        &sheet.id,
                        &component.id,
                        true,
                        format!(
                            "reference `{}` contains whitespace or a dot, so `REF.PIN` netlist nodes would be ambiguous",
                            component.reference
                        ),
                    ));
                }
            }
            let mut net_names = HashSet::new();
            for net in &sheet.nets {
                if !net_names.insert(net.name.as_str()) {
                    issues.push(issue(
                        "duplicate-net-name",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net name `{}` is declared more than once on this sheet", net.name),
                    ));
                    issues.push(issue(
                        "erc/duplicate-net-name",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net name `{}` is declared more than once on this sheet", net.name),
                    ));
                }
            }
            for net in &sheet.nets {
                if breaks_net_grammar(&net.name) {
                    issues.push(issue(
                        "erc/net-name-illegal",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net name `{}` contains whitespace or a control character, so the `NET` line would not round-trip",
                            net.name
                        ),
                    ));
                }
                if net.nodes.is_empty() {
                    issues.push(issue(
                        "erc/dangling-net",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net `{}` has no nodes; nothing is connected to it", net.name),
                    ));
                }
                else if net.nodes.len() == 1 {
                    issues.push(issue(
                        "erc/single-node-net",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net `{}` connects exactly one node", net.name),
                    ));
                }
            }
            let mut connected: HashSet<(String, String)> = HashSet::new();
            for net in &sheet.nets {
                for node in &net.nodes {
                    connected.insert((node.reference.clone(), node.pin.clone()));
                    let component = sheet.components.iter().find(|component| component.reference == node.reference);
                    match component {
                        None => issues.push(issue(
                            "net-node-missing-component",
                            "error",
                            design.revision,
                            &sheet.id,
                            &net.id,
                            true,
                            format!("net `{}` references missing component `{}`", net.name, node.reference),
                        )),
                        Some(component) if !component.pins.iter().any(|pin| pin == &node.pin) => issues.push(issue(
                            "net-node-missing-pin",
                            "error",
                            design.revision,
                            &sheet.id,
                            &net.id,
                            true,
                            format!(
                                "net `{}` references missing pin `{}` on component `{}`",
                                net.name, node.pin, node.reference
                            ),
                        )),
                        Some(_) => {}
                    }
                }
            }
            let mut no_connect_pins: HashSet<(String, String)> = HashSet::new();
            for flag in &sheet.no_connects {
                if let Some(node) = &flag.node {
                    no_connect_pins.insert((node.reference.clone(), node.pin.clone()));
                }
            }
            for component in &sheet.components {
                for pin in &component.pins {
                    if !connected.contains(&(component.reference.clone(), pin.clone()))
                        && !no_connect_pins.contains(&(component.reference.clone(), pin.clone()))
                    {
                        let message = format!("pin `{}` of component `{}` is not connected to a net", pin, component.reference);
                        issues.push(issue(
                            "unreferenced-pin",
                            "warning",
                            design.revision,
                            &sheet.id,
                            &component.id,
                            true,
                            message.clone(),
                        ));
                        issues.push(issue(
                            "erc/unconnected-pin",
                            "warning",
                            design.revision,
                            &sheet.id,
                            &component.id,
                            true,
                            message,
                        ));
                    }
                }
            }
            for wire in &sheet.wires {
                let mut floating = Vec::new();
                for point in [wire.from, wire.to] {
                    if !crate::connectivity::endpoint_attached(sheet, wire, point) {
                        floating.push(format!("({}, {})", point[0], point[1]));
                    }
                }
                if !floating.is_empty() {
                    issues.push(issue(
                        "erc/floating-wire",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &wire.id,
                        true,
                        format!("wire `{}` has {} unattached endpoint(s) at {}", wire.id, floating.len(), floating.join(", ")),
                    ));
                }
            }
        }
        // A hierarchical port is a declaration on the parent symbol; the model
        // does not store the wires drawn on that pin. The only connectivity
        // signal left is a net that shares the port name anywhere in the
        // document, so a port no net names is reported instead of assumed
        // connected.
        let net_names: HashSet<&str> =
            design.sheets.iter().flat_map(|sheet| sheet.nets.iter()).map(|net| net.name.as_str()).collect();
        for sheet in &design.hierarchy {
            for port in &sheet.ports {
                if !net_names.contains(port.name.as_str()) {
                    issues.push(issue(
                        "erc/unconnected-hierarchical-port",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &format!("{}:port:{}", sheet.id, port.name),
                        true,
                        format!(
                            "hierarchical port `{}` on sheet `{}` matches no net, so nothing is connected to it",
                            port.name, sheet.name
                        ),
                    ));
                }
            }
        }
    }
    if checks.iter().any(|check| check == "electrical") {
        for sheet in &design.sheets {
            for net in &sheet.nets {
                let mut inputs = Vec::new();
                let mut outputs = Vec::new();
                let mut power_inputs = Vec::new();
                let mut power_outputs = Vec::new();
                for node in &net.nodes {
                    let Some(component) = sheet.components.iter().find(|component| component.reference == node.reference)
                    else {
                        continue;
                    };
                    let Some(pin_type) = component.pin_types.get(&node.pin).copied()
                    else {
                        continue;
                    };
                    let label = format!("{}.{}", node.reference, node.pin);
                    match pin_type {
                        PinType::Input => inputs.push(label),
                        PinType::Output => outputs.push(label),
                        PinType::PowerIn => power_inputs.push(label),
                        PinType::PowerOut => power_outputs.push(label),
                        PinType::Bidirectional
                        | PinType::TriState
                        | PinType::Passive
                        | PinType::OpenCollector
                        | PinType::OpenEmitter
                        | PinType::Unspecified => {}
                    }
                }
                if inputs.len() > 1 {
                    issues.push(issue(
                        "erc/input-input-conflict",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` connects {} input pins; two inputs cannot drive the same net",
                            net.name,
                            inputs.len()
                        ),
                    ));
                }
                if outputs.len() > 1 {
                    issues.push(issue(
                        "erc/output-output-conflict",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` connects {} output pins; they would drive against each other",
                            net.name,
                            outputs.len()
                        ),
                    ));
                }
                if !power_inputs.is_empty() && power_outputs.is_empty() {
                    issues.push(issue(
                        "erc/power-in-unpowered",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` has {} power-input pin(s) but no power-output pin to drive it",
                            net.name,
                            power_inputs.len()
                        ),
                    ));
                }
            }
        }
    }
    if checks.iter().any(|check| check == "nets") {
        for sheet in &design.sheets {
            if sheet.components.is_empty() && sheet.nets.is_empty() && sheet.wires.is_empty() {
                issues.push(issue(
                    "sheet-empty",
                    "warning",
                    design.revision,
                    &sheet.id,
                    &sheet.id,
                    true,
                    format!("sheet `{}` has no components, nets or wires", sheet.name),
                ));
            }
            for net in &sheet.nets {
                if net.nodes.len() < 2 {
                    issues.push(issue(
                        "net-fewer-than-two-nodes",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net `{}` connects {} node(s); a net needs at least two", net.name, net.nodes.len()),
                    ));
                }
            }
            let declared: HashSet<&str> = sheet.nets.iter().map(|net| net.name.as_str()).collect();
            for wire in &sheet.wires {
                match &wire.net {
                    None => issues.push(issue(
                        "wire-floating",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &wire.id,
                        true,
                        format!("wire `{}` is not attached to a net", wire.id),
                    )),
                    Some(net) if !declared.contains(net.as_str()) => issues.push(issue(
                        "wire-unknown-net",
                        "error",
                        design.revision,
                        &sheet.id,
                        &wire.id,
                        true,
                        format!("wire `{}` references undeclared net `{net}`", wire.id),
                    )),
                    Some(_) => {}
                }
            }
        }
        // A bus member is a net name the netlist writes on its own. A declared
        // net with the same name would emit `NET <member>` twice, so the
        // collision is reported instead of passing the netlist through.
        let mut members: HashMap<String, &str> = HashMap::new();
        for bus in &design.buses {
            for index in 0..bus.width {
                members.insert(format!("{}{}", bus.name, index), bus.name.as_str());
            }
        }
        for sheet in &design.sheets {
            let mut resolved: HashMap<&str, &str> = HashMap::new();
            for net in &sheet.nets {
                if let Some(bus) = members.get(&net.name) {
                    issues.push(issue(
                        "net-bus-member-collision",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net `{}` collides with a member of bus `{bus}`", net.name),
                    ));
                }
                if let Some(bus) = bus_for_net(design, &net.name)
                    && let Some(first) = resolved.insert(bus.name.as_str(), net.id.as_str())
                {
                    issues.push(issue(
                        "net-bus-alias-collision",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` resolves to bus `{}` through an alias, but net `{first}` already does the same",
                            net.name, bus.name
                        ),
                    ));
                }
            }
        }
    }
    if checks.iter().any(|check| check == "footprints") {
        for sheet in &design.sheets {
            for component in &sheet.components {
                if component.value.trim().is_empty() {
                    issues.push(issue(
                        "component-empty-value",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &component.id,
                        true,
                        format!("component `{}` has no value, so it cannot be matched to a footprint", component.reference),
                    ));
                }
            }
        }
    }
    if checks.iter().any(|check| check == "connectivity") {
        for sheet in &design.sheets {
            for finding in connectivity::findings(sheet) {
                issues.push(issue(
                    finding.code,
                    finding.severity,
                    design.revision,
                    &sheet.id,
                    &finding.object_id,
                    true,
                    finding.message,
                ));
            }
        }
    }
    if checks.iter().any(|check| check == "power") {
        let mut rails: BTreeMap<String, Vec<(&str, &str)>> = BTreeMap::new();
        for sheet in &design.sheets {
            for net in &sheet.nets {
                if !is_power_rail(&net.name) {
                    continue;
                }
                if net.nodes.len() < 2 {
                    issues.push(issue(
                        "power-net-underconnected",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("power rail `{}` connects {} node(s); a rail needs at least two", net.name, net.nodes.len()),
                    ));
                }
                rails.entry(net.name.to_ascii_uppercase()).or_default().push((&sheet.id, &net.id));
            }
        }
        // A power output is a supply source, not a logic driver: two of them on
        // one net are two supplies tied together, and a supply fighting a logic
        // output is a short. Both are reported here, while the `electrical`
        // check stays limited to logic input/output roles.
        for sheet in &design.sheets {
            for net in &sheet.nets {
                let mut power_outputs = Vec::new();
                let mut outputs = Vec::new();
                for node in &net.nodes {
                    let Some(component) = sheet.components.iter().find(|component| component.reference == node.reference)
                    else {
                        continue;
                    };
                    let label = format!("{}.{}", node.reference, node.pin);
                    match component.pin_types.get(&node.pin) {
                        Some(PinType::PowerOut) => power_outputs.push(label),
                        Some(PinType::Output) => outputs.push(label),
                        _ => {}
                    }
                }
                if power_outputs.len() > 1 {
                    issues.push(issue(
                        "power/driver-conflict",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` is driven by {} power outputs ({}); only one supply may drive a net",
                            net.name,
                            power_outputs.len(),
                            power_outputs.join(", ")
                        ),
                    ));
                }
                if power_outputs.len() == 1 && !outputs.is_empty() {
                    issues.push(issue(
                        "power/output-driver-conflict",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` is driven by power output `{}` and logic output {}; the two would drive against each other",
                            net.name,
                            power_outputs[0],
                            outputs.join(", ")
                        ),
                    ));
                }
            }
        }
        for (rail, occurrences) in rails {
            let sheets: BTreeSet<&str> = occurrences.iter().map(|(sheet_id, _)| *sheet_id).collect();
            if sheets.len() > 1 {
                let (sheet_id, object_id) = occurrences[0];
                let names = sheets.into_iter().collect::<Vec<_>>().join(", ");
                issues.push(issue(
                    "power-net-duplicate",
                    "warning",
                    design.revision,
                    sheet_id,
                    object_id,
                    true,
                    format!("rail `{rail}` appears on multiple sheets ({names}); rails are global by name"),
                ));
            }
        }
    }
    if checks.iter().any(|check| check == "references") {
        let mut rules: HashMap<&str, &str> = HashMap::new();
        for rule in &design.rules.reference_prefixes {
            rules.insert(rule.class.as_str(), rule.prefix.as_str());
        }
        for sheet in &design.sheets {
            for component in &sheet.components {
                let Some(class) = design.rules.component_classes.get(&component.id)
                else {
                    continue;
                };
                let Some(prefix) = rules.get(class.as_str())
                else {
                    issues.push(issue(
                        "reference/class-unknown",
                        "warning",
                        design.revision,
                        &sheet.id,
                        &component.id,
                        true,
                        format!(
                            "component `{}` is assigned to class `{class}`, but no reference prefix rule declares that class",
                            component.reference
                        ),
                    ));
                    continue;
                };
                let (letters, digits) = reference_parts(&component.reference);
                if letters != *prefix || digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
                    issues.push(issue(
                        "reference/prefix-mismatch",
                        "error",
                        design.revision,
                        &sheet.id,
                        &component.id,
                        true,
                        format!(
                            "reference `{}` does not use prefix `{prefix}` of class `{class}` followed by a number",
                            component.reference
                        ),
                    ));
                }
            }
        }
    }
    issues.normalize();
    issues.issues.iter().map(issue_json).collect()
}

/// Whether a net name is a power rail, matched case-insensitively by prefix.
fn is_power_rail(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.starts_with('+') || trimmed.starts_with('-') {
        return true;
    }
    let upper = trimmed.to_ascii_uppercase();
    ["VCC", "VDD", "VSS", "GND", "VBAT", "VIN", "PWR"].iter().any(|prefix| upper.starts_with(prefix))
}

fn issue(
    code: &str,
    severity: &str,
    revision: u64,
    sheet_id: &str,
    object_id: &str,
    recoverable: bool,
    message: String,
) -> Issue {
    Issue::new(code, severity_of(severity), message)
        .with_detail("revision", revision.to_string())
        .with_detail("sheetId", sheet_id)
        .with_detail("objectId", object_id)
        .with_detail("recoverable", recoverable.to_string())
}

fn severity_of(severity: &str) -> Severity {
    match severity {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => Severity::Info,
    }
}

/// Projects one shared issue back to the report JSON the surfaces already read.
fn issue_json(issue: &Issue) -> Value {
    json!({
        "code": issue.code,
        "severity": issue.severity.as_str(),
        "revision": issue.details.get("revision").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
        "sheetId": issue.details.get("sheetId"),
        "objectId": issue.details.get("objectId"),
        "recoverable": issue.details.get("recoverable").is_some_and(|value| value == "true"),
        "message": issue.message,
    })
}

/// Lists the symbol library a design actually uses.
///
/// Components are grouped by the library id the KiCad writer derives from their
/// pins. Each entry reports the pin count, the references that place the symbol,
/// the distinct assigned footprints and how many placed components still have no
/// footprint. The report is read-only and deterministic for one document state.
pub fn symbols(handle: u64) -> Result<Value, ServiceError> {
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    Ok(symbol_library(&design))
}

fn symbol_library(design: &Design) -> Value {
    struct Usage {
        pins: usize,
        references: Vec<String>,
        footprints: BTreeSet<String>,
        missing_footprint: usize,
    }

    let mut groups: BTreeMap<String, Usage> = BTreeMap::new();
    for sheet in &design.sheets {
        for component in &sheet.components {
            let usage = groups.entry(kicad::lib_id_for(component)).or_insert_with(|| Usage {
                pins: component.pins.len(),
                references: Vec::new(),
                footprints: BTreeSet::new(),
                missing_footprint: 0,
            });
            usage.references.push(component.reference.clone());
            if component.footprint.is_empty() {
                usage.missing_footprint += 1;
            }
            else {
                usage.footprints.insert(component.footprint.clone());
            }
        }
    }

    let mut components = 0usize;
    let mut missing_footprint = 0usize;
    let symbol_count = groups.len();
    let entries = groups
        .into_iter()
        .map(|(lib_id, mut usage)| {
            usage.references.sort();
            components += usage.references.len();
            missing_footprint += usage.missing_footprint;
            json!({
                "libId": lib_id,
                "pins": usage.pins,
                "components": usage.references.len(),
                "references": usage.references,
                "footprints": usage.footprints.into_iter().collect::<Vec<_>>(),
                "missingFootprint": usage.missing_footprint,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "revision": design.revision,
        "symbols": entries,
        "totals": {
            "symbols": symbol_count,
            "components": components,
            "missingFootprint": missing_footprint,
        },
    })
}

/// Reports what each net actually connects.
///
/// The report is the read-only companion of the `connectivity` check: instead of
/// findings it lists every net with its node count and `REF.PIN` pin list, the
/// pins no net reaches, and the wire-group and junction totals per sheet. It
/// never mutates the document and is deterministic for one revision.
pub fn connectivity_report(handle: u64) -> Result<Value, ServiceError> {
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    Ok(connectivity_report_of(&design))
}

fn connectivity_report_of(design: &Design) -> Value {
    let mut net_total = 0usize;
    let mut node_total = 0usize;
    let mut single_node_total = 0usize;
    let mut dangling_total = 0usize;
    let mut unconnected_total = 0usize;
    let mut group_total = 0usize;
    let mut unassigned_total = 0usize;
    let mut conflict_total = 0usize;
    let mut floating_junction_total = 0usize;

    let sheets = design
        .sheets
        .iter()
        .map(|sheet| {
            let nets = sheet
                .nets
                .iter()
                .map(|net| {
                    let pins = net.nodes.iter().map(|node| format!("{}.{}", node.reference, node.pin)).collect::<Vec<_>>();
                    let single_node = net.nodes.len() == 1;
                    let dangling = net.nodes.is_empty();
                    net_total += 1;
                    node_total += pins.len();
                    single_node_total += usize::from(single_node);
                    dangling_total += usize::from(dangling);
                    json!({
                        "id": net.id,
                        "name": net.name,
                        "nodes": pins.len(),
                        "pins": pins,
                        "singleNode": single_node,
                        "dangling": dangling,
                    })
                })
                .collect::<Vec<_>>();

            let mut connected: HashSet<(String, String)> = HashSet::new();
            for net in &sheet.nets {
                for node in &net.nodes {
                    connected.insert((node.reference.clone(), node.pin.clone()));
                }
            }
            let mut marked: HashSet<(String, String)> = HashSet::new();
            for flag in &sheet.no_connects {
                if let Some(node) = &flag.node {
                    marked.insert((node.reference.clone(), node.pin.clone()));
                }
            }
            let unconnected = sheet
                .components
                .iter()
                .flat_map(|component| {
                    component
                        .pins
                        .iter()
                        .filter(|pin| {
                            let key = (component.reference.clone(), (*pin).clone());
                            !connected.contains(&key) && !marked.contains(&key)
                        })
                        .map(|pin| json!({ "ref": component.reference, "pin": pin }))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            unconnected_total += unconnected.len();

            let groups = connectivity::groups(sheet);
            let unassigned = groups.iter().filter(|group| group.is_unassigned()).count();
            let conflicts = groups.iter().filter(|group| group.is_conflict()).count();
            let floating_junctions =
                connectivity::findings(sheet).iter().filter(|finding| finding.code == connectivity::JUNCTION_FLOATING).count();
            group_total += groups.len();
            unassigned_total += unassigned;
            conflict_total += conflicts;
            floating_junction_total += floating_junctions;

            json!({
                "id": sheet.id,
                "name": sheet.name,
                "nets": nets,
                "wires": {
                    "segments": sheet.wires.len(),
                    "groups": groups.len(),
                    "unassigned": unassigned,
                    "conflicts": conflicts,
                },
                "junctions": { "total": sheet.junctions.len(), "floating": floating_junctions },
                "noConnects": sheet.no_connects.len(),
                "unconnectedPins": unconnected,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "format": CONNECTIVITY_REPORT_FORMAT,
        "version": CONNECTIVITY_REPORT_VERSION,
        "revision": design.revision,
        "sheets": sheets,
        "totals": {
            "nets": net_total,
            "nodes": node_total,
            "singleNodeNets": single_node_total,
            "danglingNets": dangling_total,
            "unconnectedPins": unconnected_total,
            "wireGroups": group_total,
            "unassignedWireGroups": unassigned_total,
            "conflictingWireGroups": conflict_total,
            "floatingJunctions": floating_junction_total,
        },
    })
}

/// Resolves a net name to the bus whose name or alias it matches.
///
/// Aliases are alternate names for the same declaration: a net called `D` with
/// alias `D` on bus `DATA` expands into `DATA0`..`DATAn`, never `D0`..`Dn`.
fn bus_for_net<'a>(design: &'a Design, name: &str) -> Option<&'a Bus> {
    design.buses.iter().find(|bus| bus.name == name || bus.aliases.iter().any(|alias| alias == name))
}

/// Projects one design to the deterministic text netlist.
///
/// Every declared bus is announced as `BUS <name> <width>`. A net whose name
/// equals a bus name or one of its aliases expands into the bus member nets
/// using the fixed rule `DATA` with width 4 -> `DATA0`..`DATA3` (name plus
/// 0-based index, no separator); each member repeats the parent net's nodes. A
/// declared bus with no such net still lists its member names, with no nodes,
/// after the sheets.
fn render_netlist(design: &Design) -> String {
    let mut text = String::from("# OpenEDA netlist v1\n");
    text.push_str(&format!("# design {} revision {}\n", design.id, design.revision));
    for bus in &design.buses {
        text.push_str(&format!("BUS {} {}\n", bus.name, bus.width));
    }
    if !design.buses.is_empty() {
        text.push('\n');
    }
    let mut expanded: HashSet<&str> = HashSet::new();
    for sheet in &design.sheets {
        for net in &sheet.nets {
            if let Some(bus) = bus_for_net(design, &net.name) {
                expanded.insert(bus.name.as_str());
                for index in 0..bus.width {
                    text.push_str(&format!("NET {}{}\n", bus.name, index));
                    for node in &net.nodes {
                        text.push_str(&format!("NODE {}.{}\n", node.reference, node.pin));
                    }
                    text.push('\n');
                }
            }
            else {
                text.push_str(&format!("NET {}\n", net.name));
                for node in &net.nodes {
                    text.push_str(&format!("NODE {}.{}\n", node.reference, node.pin));
                }
                text.push('\n');
            }
        }
    }
    for bus in &design.buses {
        if expanded.contains(bus.name.as_str()) {
            continue;
        }
        for index in 0..bus.width {
            text.push_str(&format!("NET {}{}\n\n", bus.name, index));
        }
    }
    text
}

/// Projects one design to a CSV pin-level netlist plus its data-row count.
///
/// The header is `Sheet,Net,Ref,Pin` and every node becomes one row. A net whose
/// name equals a bus name or alias expands into the same member nets the text
/// netlist writes, so both projections agree. Rows follow sheet, net and node
/// order and are byte-stable for one revision.
fn render_net_csv(design: &Design) -> (String, usize) {
    let mut text = String::from("Sheet,Net,Ref,Pin\n");
    let mut rows = 0usize;
    for sheet in &design.sheets {
        for net in &sheet.nets {
            let members: Vec<String> = match bus_for_net(design, &net.name) {
                Some(bus) => (0..bus.width).map(|index| format!("{}{index}", bus.name)).collect(),
                None => vec![net.name.clone()],
            };
            for member in members {
                for node in &net.nodes {
                    text.push_str(&format!(
                        "{},{},{},{}\n",
                        csv_field(&sheet.name),
                        csv_field(&member),
                        csv_field(&node.reference),
                        csv_field(&node.pin)
                    ));
                    rows += 1;
                }
            }
        }
    }
    (text, rows)
}

/// Projects one design to a hierarchical netlist whose net names carry the
/// sheet path.
///
/// The document is flat, so every sheet contributes one path segment: its
/// sanitized name, falling back to the sheet id when the name has no usable
/// characters. Two sheets that sanitize to the same segment get a numeric
/// suffix, so `NET <path>/<name>` stays unique. Declared buses are announced
/// with `BUS <name> <width>` and expanded exactly like the flat netlist.
fn render_hierarchical_netlist(design: &Design) -> String {
    let mut text = String::from("# OpenEDA hierarchical netlist v1\n");
    text.push_str(&format!("# design {} revision {}\n", design.id, design.revision));
    for bus in &design.buses {
        text.push_str(&format!("BUS {} {}\n", bus.name, bus.width));
    }
    if !design.buses.is_empty() {
        text.push('\n');
    }
    let mut paths: HashSet<String> = HashSet::new();
    for sheet in &design.sheets {
        let base = hierarchical_path_segment(&sheet.name, &sheet.id);
        let mut path = base.clone();
        let mut suffix = 2;
        while paths.contains(&path) {
            path = format!("{base}_{suffix}");
            suffix += 1;
        }
        paths.insert(path.clone());
        text.push_str(&format!("SHEET {path}\n"));
        for net in &sheet.nets {
            let members: Vec<String> = match bus_for_net(design, &net.name) {
                Some(bus) => (0..bus.width).map(|index| format!("{}{index}", bus.name)).collect(),
                None => vec![net.name.clone()],
            };
            for member in members {
                text.push_str(&format!("NET {path}/{member}\n"));
                for node in &net.nodes {
                    text.push_str(&format!("NODE {}.{}\n", node.reference, node.pin));
                }
                text.push('\n');
            }
        }
    }
    text
}

/// Turns a sheet name into one path segment; an unusable name falls back to the
/// sheet id so the prefix is never empty.
fn hierarchical_path_segment(name: &str, id: &str) -> String {
    let segment: String = name
        .chars()
        .map(
            |character| {
                if character.is_ascii_alphanumeric() || character == '_' || character == '-' { character } else { '_' }
            },
        )
        .collect();
    let trimmed = segment.trim_matches('_');
    if trimmed.is_empty() {
        let fallback = id.trim_matches(|character: char| character == '-' || character == '_');
        if fallback.is_empty() { "sheet".to_string() } else { fallback.to_string() }
    }
    else {
        trimmed.to_string()
    }
}

/// Projects one design to a deterministic SPICE netlist plus its losses.
///
/// Element lines follow the reference prefix, pins keep their declared order,
/// and every pin without a net gets a unique `NC_<ref>_<pin>` node that is also
/// reported as a loss. Net names are legalized once for the whole design so the
/// output is byte-stable and two different nets never collapse onto one node.
fn render_spice(design: &Design) -> (String, Vec<Value>) {
    let mut losses = Vec::new();
    let mut net_names: HashMap<String, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for sheet in &design.sheets {
        for net in &sheet.nets {
            if net_names.contains_key(&net.name) {
                continue;
            }
            let base = legal_node_name(&net.name);
            let mut candidate = base.clone();
            let mut suffix = 2;
            while used.contains(&candidate) {
                candidate = format!("{base}_{suffix}");
                suffix += 1;
            }
            used.insert(candidate.clone());
            net_names.insert(net.name.clone(), candidate);
        }
    }

    let mut text = String::from("* OpenEDA SPICE netlist v1\n");
    text.push_str(&format!("* design {} revision {}\n", design.id, design.revision));
    for sheet in &design.sheets {
        for component in &sheet.components {
            let nodes = component
                .pins
                .iter()
                .map(|pin| spice_pin_node(sheet, &net_names, &mut used, component, pin, &mut losses))
                .collect::<Vec<_>>();
            if let Some(line) = spice_element(component, &nodes, &mut losses) {
                text.push_str(&line);
                text.push('\n');
            }
        }
    }
    text.push_str(".END\n");
    (text, losses)
}

/// Projects one design to a deterministic SPICE subcircuit library plus its
/// losses.
///
/// Every sheet becomes one `.SUBCKT <name> <ports>` block: the ports are the
/// sheet's nets in declaration order, legalized once so a net never collides
/// with another port. Component elements follow the same mapping as the flat
/// SPICE writer, including the `NC_<ref>_<pin>` nodes and losses for open pins.
/// Subcircuit names are derived from the sheet name and made unique, so two
/// sheets can share a name without producing a duplicate definition.
fn render_spice_subcircuits(design: &Design) -> (String, Vec<Value>) {
    let mut losses = Vec::new();
    let mut text = String::from("* OpenEDA SPICE subcircuits v1\n");
    text.push_str(&format!("* design {} revision {}\n", design.id, design.revision));

    let mut subcircuit_names: HashSet<String> = HashSet::new();
    for sheet in &design.sheets {
        let base = legal_node_name(&sheet.name);
        let mut name = base.clone();
        let mut suffix = 2;
        while subcircuit_names.contains(&name) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        subcircuit_names.insert(name.clone());

        let mut used: HashSet<String> = HashSet::new();
        let mut net_names: HashMap<String, String> = HashMap::new();
        for net in &sheet.nets {
            if net_names.contains_key(&net.name) {
                continue;
            }
            let base = legal_node_name(&net.name);
            let mut port = base.clone();
            let mut suffix = 2;
            while used.contains(&port) {
                port = format!("{base}_{suffix}");
                suffix += 1;
            }
            used.insert(port.clone());
            net_names.insert(net.name.clone(), port);
        }
        let mut seen: HashSet<String> = HashSet::new();
        let ports = sheet
            .nets
            .iter()
            .filter_map(|net| net_names.get(&net.name).cloned())
            .filter(|port| seen.insert(port.clone()))
            .collect::<Vec<_>>();

        if ports.is_empty() {
            text.push_str(&format!(".SUBCKT {name}\n"));
        }
        else {
            text.push_str(&format!(".SUBCKT {name} {}\n", ports.join(" ")));
        }
        for component in &sheet.components {
            let nodes = component
                .pins
                .iter()
                .map(|pin| spice_pin_node(sheet, &net_names, &mut used, component, pin, &mut losses))
                .collect::<Vec<_>>();
            if let Some(line) = spice_element(component, &nodes, &mut losses) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        text.push_str(&format!(".ENDS {name}\n"));
    }
    text.push_str(".END\n");
    (text, losses)
}

/// Resolves one component pin to its legal node name, inventing a stable NC node
/// and a loss entry when the pin is not connected to a net.
fn spice_pin_node(
    sheet: &Sheet,
    net_names: &HashMap<String, String>,
    used: &mut HashSet<String>,
    component: &Component,
    pin: &str,
    losses: &mut Vec<Value>,
) -> String {
    for net in &sheet.nets {
        if net.nodes.iter().any(|node| node.reference == component.reference && node.pin == pin)
            && let Some(name) = net_names.get(&net.name)
        {
            return name.clone();
        }
    }
    let base = format!("NC_{}_{}", sanitize_token(&component.reference), sanitize_token(pin));
    let mut candidate = base.clone();
    let mut suffix = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    used.insert(candidate.clone());
    losses.push(json!({
        "code": SPICE_OPEN_PIN,
        "detail": format!(
            "pin `{pin}` of component `{}` is not connected to a net; wrote node `{candidate}`",
            component.reference
        ),
    }));
    candidate
}

/// Formats one SPICE element line from the reference prefix and the resolved
/// node names. A component that declares no pins is reported instead of being
/// written as a malformed line.
fn spice_element(component: &Component, nodes: &[String], losses: &mut Vec<Value>) -> Option<String> {
    if nodes.is_empty() {
        losses.push(json!({
            "code": SPICE_PIN_COUNT,
            "detail": format!("component `{}` declares no pins and was not written", component.reference),
        }));
        return None;
    }
    let reference = legal_node_name(&component.reference);
    let value = spice_text(&component.value);
    let model = if value.is_empty() { reference.clone() } else { value.clone() };
    let prefix = component
        .reference
        .chars()
        .find(|character| character.is_ascii_alphabetic())
        .map(|character| character.to_ascii_uppercase())
        .unwrap_or('X');
    let named = |letter: char| {
        if reference.to_ascii_uppercase().starts_with(letter) { reference.clone() } else { format!("{letter}{reference}") }
    };
    let subcircuit = if reference.to_ascii_uppercase().starts_with('X') { reference.clone() } else { format!("X{reference}") };
    let generic = || {
        let mut line = format!("{subcircuit} {}", nodes.join(" "));
        line.push(' ');
        line.push_str(if value.is_empty() { &reference } else { &value });
        line
    };

    let expected = match prefix {
        'R' | 'C' | 'L' | 'V' | 'I' | 'D' => Some(2),
        'Q' | 'J' => Some(3),
        _ => None,
    };
    if let Some(expected) = expected
        && nodes.len() != expected
    {
        losses.push(json!({
            "code": SPICE_PIN_COUNT,
            "detail": format!(
                "component `{}` declares {} pin(s) but its `{prefix}` mapping needs {expected}; wrote a generic X element",
                component.reference,
                nodes.len()
            ),
        }));
        return Some(generic());
    }

    let line = match prefix {
        'R' | 'C' | 'L' => format!("{} {} {} {}", named(prefix), nodes[0], nodes[1], value_or_zero(&value)),
        'V' | 'I' => format!("{} {} {} {}", named(prefix), nodes[0], nodes[1], value_or_zero(&value)),
        'D' => format!("{} {} {} {}", named(prefix), nodes[0], nodes[1], model),
        'Q' => format!("{} {} {} {} {}", named(prefix), nodes[0], nodes[1], nodes[2], model),
        'J' => format!("{} {} {} {} {}", named(prefix), nodes[0], nodes[1], nodes[2], model),
        _ => generic(),
    };
    Some(line)
}

/// Keeps a value on one line; an empty value still produces a legal token.
fn value_or_zero(value: &str) -> &str {
    if value.is_empty() { "0" } else { value }
}

/// Collapses line breaks so one element or annotation stays on one line.
fn spice_text(text: &str) -> String {
    text.chars()
        .map(|character| if matches!(character, '\n' | '\r' | '\t') { ' ' } else { character })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Legalizes a net name for SPICE: letters, digits and underscores survive,
/// every other character becomes `_`, an empty name becomes `N`, and a leading
/// digit is prefixed with `N`.
fn legal_node_name(name: &str) -> String {
    let mut legal = sanitize_token(name);
    if legal.is_empty() {
        legal.push('N');
    }
    if legal.as_bytes()[0].is_ascii_digit() {
        legal.insert(0, 'N');
    }
    legal
}

/// Replaces characters that SPICE node and element names cannot carry.
fn sanitize_token(name: &str) -> String {
    name.chars().map(|character| if character.is_ascii_alphanumeric() || character == '_' { character } else { '_' }).collect()
}

/// One SVG projection plus the viewport size it declared.
struct SvgProjection {
    document: String,
    width: f64,
    height: f64,
}

fn render_svg(design: &Design) -> String {
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    svg_projection(&sheets).document
}

/// Projects the given sheets onto one SVG document.
///
/// The export writer passes every sheet; the preview passes exactly one, so the
/// viewport and the painted geometry always describe the same sheet set.
fn svg_projection(sheets: &[&Sheet]) -> SvgProjection {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for sheet in sheets {
        for component in &sheet.components {
            any = true;
            min_x = min_x.min(component.x - COMPONENT_HALF_WIDTH);
            max_x = max_x.max(component.x + COMPONENT_HALF_WIDTH);
            min_y = min_y.min(component.y - COMPONENT_HALF_HEIGHT);
            max_y = max_y.max(component.y + COMPONENT_HALF_HEIGHT);
        }
        for wire in &sheet.wires {
            any = true;
            for point in [wire.from, wire.to] {
                min_x = min_x.min(point[0]);
                max_x = max_x.max(point[0]);
                min_y = min_y.min(point[1]);
                max_y = max_y.max(point[1]);
            }
        }
        for junction in &sheet.junctions {
            any = true;
            min_x = min_x.min(junction.x);
            max_x = max_x.max(junction.x);
            min_y = min_y.min(junction.y);
            max_y = max_y.max(junction.y);
        }
        for text in &sheet.texts {
            any = true;
            min_x = min_x.min(text.x);
            max_x = max_x.max(text.x);
            min_y = min_y.min(text.y);
            max_y = max_y.max(text.y);
        }
    }
    if !any {
        min_x = 0.0;
        min_y = 0.0;
        max_x = 100.0;
        max_y = 60.0;
    }
    let padding = 10.0;
    let view_x = min_x - padding;
    let view_y = min_y - padding;
    let view_width = (max_x - min_x + 2.0 * padding).max(1.0);
    let view_height = (max_y - min_y + 2.0 * padding).max(1.0);
    let width = view_width.round();
    let height = view_height.round();

    let mut svg = String::new();
    svg.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{:.2} {:.2} {:.2} {:.2}\" width=\"{:.0}\" height=\"{:.0}\">\n",
        view_x, view_y, view_width, view_height, width, height
    ));
    svg.push_str(&format!(
        "  <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#ffffff\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
        view_x, view_y, view_width, view_height
    ));
    for sheet in sheets {
        svg.push_str(&format!("  <g data-sheet=\"{}\">\n", escape_xml(&sheet.id)));
        for wire in &sheet.wires {
            svg.push_str(&format!(
                "    <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#0f766e\" stroke-width=\"0.5\"/>\n",
                wire.from[0], wire.from[1], wire.to[0], wire.to[1]
            ));
        }
        for junction in &sheet.junctions {
            svg.push_str(&format!(
                "    <circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"1.4\" fill=\"#1f2933\" data-junction=\"true\"/>\n",
                junction.x, junction.y
            ));
        }
        for text in &sheet.texts {
            svg.push_str(&format!(
                "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" text-anchor=\"start\">{}</text>\n",
                text.x,
                text.y,
                escape_xml(&text.text)
            ));
        }
        for component in &sheet.components {
            svg.push_str(&format!(
                "    <g transform=\"rotate({:.2} {:.2} {:.2})\" data-ref=\"{}\">\n",
                component.rotation,
                component.x,
                component.y,
                escape_xml(&component.reference)
            ));
            svg.push_str(&format!(
                "      <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n",
                component.x - COMPONENT_HALF_WIDTH,
                component.y - COMPONENT_HALF_HEIGHT,
                COMPONENT_HALF_WIDTH * 2.0,
                COMPONENT_HALF_HEIGHT * 2.0
            ));
            svg.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" text-anchor=\"middle\">{}</text>\n",
                component.x,
                component.y - 0.6,
                escape_xml(&component.reference)
            ));
            svg.push_str(&format!(
                "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" text-anchor=\"middle\">{}</text>\n",
                component.x,
                component.y + 3.0,
                escape_xml(&component.value)
            ));
            svg.push_str("    </g>\n");
        }
        svg.push_str("  </g>\n");
    }
    svg.push_str("</svg>\n");
    SvgProjection { document: svg, width, height }
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// One BOM row: a value and footprint shared by the listed designators.
struct BomRow {
    comment: String,
    footprint: String,
    designators: Vec<String>,
}

/// Groups designators by value and footprint; both are the model's own text, so
/// a row never invents a part number. Rows and designators are sorted, making
/// the projection deterministic. A selected variant removes its DNP designators
/// and replaces the value text of the designators it overrides.
fn bom_rows(design: &Design, variant: Option<&Variant>) -> Vec<BomRow> {
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for sheet in &design.sheets {
        for component in &sheet.components {
            if variant.is_some_and(|variant| variant.dnp.iter().any(|reference| reference == &component.reference)) {
                continue;
            }
            let comment = variant
                .and_then(|variant| variant.values.get(&component.reference).cloned())
                .unwrap_or_else(|| component.value.clone());
            groups.entry((comment, component.footprint.clone())).or_default().push(component.reference.clone());
        }
    }
    groups
        .into_iter()
        .map(|((comment, footprint), mut designators)| {
            designators.sort();
            BomRow { comment, footprint, designators }
        })
        .collect()
}

fn render_bom(rows: &[BomRow]) -> String {
    let mut text = String::from("Comment,Footprint,Designator,Quantity\n");
    for row in rows {
        text.push_str(&csv_field(&row.comment));
        text.push(',');
        text.push_str(&csv_field(&row.footprint));
        text.push(',');
        text.push_str(&csv_field(&designator_ranges(&row.designators).join(",")));
        text.push(',');
        text.push_str(&row.designators.len().to_string());
        text.push('\n');
    }
    text
}

/// Splits a reference designator into its letter prefix and trailing number.
///
/// A designator without trailing digits, for example `TP_A`, is kept whole and
/// never participates in a range.
fn split_designator(reference: &str) -> (&str, Option<u64>) {
    let digits = reference.len() - reference.trim_end_matches(|character: char| character.is_ascii_digit()).len();
    if digits == 0 { (reference, None) } else { (reference.split_at(digits).0, reference[digits..].parse::<u64>().ok()) }
}

/// Compresses consecutive designators into inclusive ranges.
///
/// Only runs of three or more are collapsed, so `R1 R2 R5 R6 R7` becomes
/// `R1,R2,R5-R7` and short pairs stay explicit. The input must be sorted.
fn designator_ranges(designators: &[String]) -> Vec<String> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < designators.len() {
        let (prefix, number) = split_designator(&designators[index]);
        let Some(first) = number
        else {
            ranges.push(designators[index].clone());
            index += 1;
            continue;
        };
        let start = index;
        let mut last = first;
        while index + 1 < designators.len() {
            let (next_prefix, next_number) = split_designator(&designators[index + 1]);
            if next_prefix == prefix && next_number == Some(last + 1) {
                last += 1;
                index += 1;
            }
            else {
                break;
            }
        }
        if index - start + 1 >= 3 {
            ranges.push(format!("{prefix}{first}-{prefix}{last}"));
        }
        else {
            ranges.extend(designators[start..=index].iter().cloned());
        }
        index += 1;
    }
    ranges
}

/// Projects the BOM rows to deterministic JSON, including the compressed
/// designator ranges the CSV writer emits.
fn render_bom_json(design: &Design, rows: &[BomRow]) -> Result<String, ServiceError> {
    let items = rows
        .iter()
        .map(|row| {
            let ranges = designator_ranges(&row.designators);
            json!({
                "comment": row.comment,
                "footprint": row.footprint,
                "quantity": row.designators.len(),
                "designators": row.designators,
                "designatorRanges": ranges,
            })
        })
        .collect::<Vec<_>>();
    let projection = json!({
        "format": BOM_PROJECTION_FORMAT,
        "version": BOM_PROJECTION_VERSION,
        "design": design.id,
        "revision": design.revision,
        "items": items,
    });
    let mut encoded = serde_json::to_string_pretty(&projection)
        .map_err(|error| invalid(format!("the BOM projection could not be encoded: {error}")))?;
    encoded.push('\n');
    Ok(encoded)
}

/// Projects the BOM rows to a self-contained, deterministic HTML page.
///
/// Every field is escaped, so a value or footprint is shown verbatim instead of
/// being read as markup. A selected variant names itself and its DNP count in
/// the header; it never changes the rows beyond what [`bom_rows`] already did.
fn render_bom_html(design: &Design, rows: &[BomRow], variant: Option<&Variant>) -> String {
    let title = design.title.as_deref().filter(|title| !title.is_empty()).unwrap_or("OpenEDA BOM");
    let mut html = String::from("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!("<title>{}</title>\n", escape_xml(&format!("{title} — BOM"))));
    html.push_str("</head>\n<body>\n");
    html.push_str(&format!("<h1>{}</h1>\n", escape_xml(&format!("{title} — BOM"))));
    let mut properties =
        vec![format!("Design: {}", design.id), format!("Revision: {}", design.revision), format!("Units: {}", design.units)];
    if let Some(variant) = variant {
        properties.push(format!("Variant: {}", variant.name));
        properties.push(format!("DNP: {}", variant.dnp.len()));
    }
    html.push_str(&format!("<p>{}</p>\n", escape_xml(&properties.join(" · "))));
    html.push_str(
        "<table>\n<thead>\n<tr><th>Comment</th><th>Footprint</th><th>Designator</th><th>Quantity</th></tr>\n</thead>\n<tbody>\n",
    );
    for row in rows {
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            escape_xml(&row.comment),
            escape_xml(&row.footprint),
            escape_xml(&designator_ranges(&row.designators).join(", ")),
            row.designators.len()
        ));
    }
    html.push_str("</tbody>\n</table>\n</body>\n</html>\n");
    html
}

/// Renders the deterministic Markdown design report.
///
/// The document reports the design header, one object-count table, one net
/// table per sheet and every issue produced by the supported checks grouped by
/// severity. The returned tuple is the Markdown, the number of sections emitted
/// and the issue count used by the artifact summary.
fn render_report(design: &Design) -> (String, usize, usize) {
    let checks: Vec<String> = SUPPORTED_CHECKS.iter().map(|check| (*check).to_string()).collect();
    let issues = inspect_design(design, &checks);
    let mut sections = 0usize;
    let mut text = String::from("# OpenEDA design report\n\n");
    text.push_str(&format!("- Design: `{}`\n", design.id));
    text.push_str(&format!(
        "- Title: {}\n",
        design.title.as_deref().map(markdown_cell).unwrap_or_else(|| "(untitled)".to_string())
    ));
    text.push_str(&format!("- Revision: {}\n", design.revision));
    if !design.rev.is_empty() {
        text.push_str(&format!("- Drawing revision: {}\n", markdown_cell(&design.rev)));
    }
    if !design.company.is_empty() {
        text.push_str(&format!("- Company: {}\n", markdown_cell(&design.company)));
    }
    if !design.date.is_empty() {
        text.push_str(&format!("- Date: {}\n", markdown_cell(&design.date)));
    }
    text.push_str(&format!("- Units: {}\n", design.units));
    text.push_str(&format!("- Sheets: {}\n\n", design.sheets.len()));

    sections += 1;
    text.push_str("## Objects\n\n| Sheet | Components | Nets | Wires | Junctions | Texts |\n");
    text.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for sheet in &design.sheets {
        text.push_str(&format!(
            "| {} (`{}`) | {} | {} | {} | {} | {} |\n",
            markdown_cell(&sheet.name),
            sheet.id,
            sheet.components.len(),
            sheet.nets.len(),
            sheet.wires.len(),
            sheet.junctions.len(),
            sheet.texts.len()
        ));
    }
    text.push('\n');

    if !design.rules.net_classes.is_empty() {
        sections += 1;
        text.push_str("## Net classes\n\n| Class | Width | Clearance | Via | Nets | Buses |\n");
        text.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for net_class in &design.rules.net_classes {
            text.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                markdown_cell(&net_class.name),
                format_number(net_class.width),
                format_number(net_class.clearance),
                format_number(net_class.via),
                markdown_cell(&net_class.nets.join(", ")),
                markdown_cell(&net_class.buses.join(", ")),
            ));
        }
        text.push('\n');
    }

    if !design.rules.reference_prefixes.is_empty() {
        sections += 1;
        text.push_str("## Reference prefixes\n\n| Class | Prefix | Components |\n| --- | --- | --- |\n");
        for rule in &design.rules.reference_prefixes {
            let references = design
                .sheets
                .iter()
                .flat_map(|sheet| sheet.components.iter())
                .filter(|component| design.rules.component_classes.get(&component.id) == Some(&rule.class))
                .map(|component| component.reference.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&format!(
                "| {} | {} | {} |\n",
                markdown_cell(&rule.class),
                markdown_cell(&rule.prefix),
                markdown_cell(&references)
            ));
        }
        text.push('\n');
    }

    sections += 1;
    text.push_str("## Nets\n");
    for sheet in &design.sheets {
        sections += 1;
        text.push_str(&format!("\n### {} (`{}`)\n\n", markdown_cell(&sheet.name), sheet.id));
        if sheet.nets.is_empty() {
            text.push_str("_No nets._\n");
            continue;
        }
        text.push_str("| Net | Nodes | Pins |\n| --- | --- | --- |\n");
        for net in &sheet.nets {
            let pins = net.nodes.iter().map(|node| format!("{}.{}", node.reference, node.pin)).collect::<Vec<_>>().join(", ");
            text.push_str(&format!("| {} | {} | {} |\n", markdown_cell(&net.name), net.nodes.len(), markdown_cell(&pins)));
        }
    }
    text.push('\n');

    sections += 1;
    text.push_str("## Inspection\n");
    for severity in ["error", "warning", "info"] {
        sections += 1;
        let matching: Vec<&Value> = issues.iter().filter(|issue| issue["severity"] == severity).collect();
        text.push_str(&format!("\n### {}\n\n", severity_title(severity)));
        if matching.is_empty() {
            text.push_str("_None._\n");
            continue;
        }
        for issue in matching {
            let code = issue["code"].as_str().unwrap_or("");
            let sheet = issue["sheetId"].as_str().unwrap_or("");
            let message = issue["message"].as_str().unwrap_or("");
            text.push_str(&format!("- `{code}` ({sheet}): {}\n", markdown_cell(message)));
        }
    }
    (text, sections, issues.len())
}

/// Projects one design to a deterministic, read-only JSON document.
///
/// The projection carries the design header and, for every sheet in document
/// order, its components, nets, wires, junctions and text annotations. Object
/// arrays keep the insertion order of the model and object keys are serialized
/// in a fixed order, so exporting one document twice is byte-identical. The
/// counters and identities used internally to allocate new objects are not
/// part of the projection.
fn render_json_projection(design: &Design) -> Result<String, ServiceError> {
    let sheets = design
        .sheets
        .iter()
        .map(|sheet| {
            json!({
                "id": sheet.id,
                "name": sheet.name,
                "components": sheet
                    .components
                    .iter()
                    .map(|component| json!({
                        "id": component.id,
                        "ref": component.reference,
                        "value": component.value,
                        "footprint": component.footprint,
                        "x": component.x,
                        "y": component.y,
                        "rotation": component.rotation,
                        "pins": component.pins,
                        "pinTypes": component.pin_types,
                    }))
                    .collect::<Vec<_>>(),
                "nets": sheet
                    .nets
                    .iter()
                    .map(|net| json!({
                        "id": net.id,
                        "name": net.name,
                        "nodes": net
                            .nodes
                            .iter()
                            .map(|node| json!({ "ref": node.reference, "pin": node.pin }))
                            .collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>(),
                "wires": sheet
                    .wires
                    .iter()
                    .map(|wire| json!({
                        "id": wire.id,
                        "net": wire.net,
                        "from": wire.from,
                        "to": wire.to,
                    }))
                    .collect::<Vec<_>>(),
                "junctions": sheet
                    .junctions
                    .iter()
                    .map(|junction| json!({ "x": junction.x, "y": junction.y }))
                    .collect::<Vec<_>>(),
                "noConnects": sheet
                    .no_connects
                    .iter()
                    .map(|no_connect| match &no_connect.node {
                        Some(node) => json!({
                            "x": no_connect.x,
                            "y": no_connect.y,
                            "node": { "ref": node.reference, "pin": node.pin },
                        }),
                        None => json!({ "x": no_connect.x, "y": no_connect.y }),
                    })
                    .collect::<Vec<_>>(),
                "texts": sheet
                    .texts
                    .iter()
                    .map(|text| json!({ "text": text.text, "x": text.x, "y": text.y }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let hierarchy = design
        .hierarchy
        .iter()
        .map(|sheet| {
            json!({
                "id": sheet.id,
                "name": sheet.name,
                "file": sheet.file,
                "x": sheet.x,
                "y": sheet.y,
                "width": sheet.width,
                "height": sheet.height,
                "ports": sheet
                    .ports
                    .iter()
                    .map(|port| json!({
                        "name": port.name,
                        "direction": port.direction,
                        "x": port.x,
                        "y": port.y,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let projection = json!({
        "format": JSON_PROJECTION_FORMAT,
        "version": JSON_PROJECTION_VERSION,
        "design": design.id,
        "title": design.title,
        "revision": design.revision,
        "rev": design.rev,
        "company": design.company,
        "date": design.date,
        "units": design.units,
        "sheets": sheets,
        "hierarchy": hierarchy,
        "netClasses": design
            .rules
            .net_classes
            .iter()
            .map(|net_class| json!({
                "id": net_class.id,
                "name": net_class.name,
                "width": net_class.width,
                "clearance": net_class.clearance,
                "via": net_class.via,
                "nets": net_class.nets,
                "buses": net_class.buses,
            }))
            .collect::<Vec<_>>(),
        "referencePrefixes": design
            .rules
            .reference_prefixes
            .iter()
            .map(|rule| json!({ "class": rule.class, "prefix": rule.prefix }))
            .collect::<Vec<_>>(),
        "componentClasses": design
            .sheets
            .iter()
            .flat_map(|sheet| sheet.components.iter())
            .filter_map(|component| {
                design.rules.component_classes.get(&component.id).map(|class| {
                    json!({ "componentId": component.id, "reference": component.reference, "class": class })
                })
            })
            .collect::<Vec<_>>(),
    });
    let mut encoded = serde_json::to_string_pretty(&projection)
        .map_err(|error| invalid(format!("the JSON projection could not be encoded: {error}")))?;
    encoded.push('\n');
    Ok(encoded)
}

/// Formats a numeric attribute without redundant trailing zeros.
fn format_number(value: f64) -> String {
    let text = format!("{value:.4}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" { "0".to_string() } else { trimmed.to_string() }
}

fn severity_title(severity: &str) -> String {
    let mut chars = severity.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Escapes text so it stays inside one Markdown table cell.
fn markdown_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Quotes a field only when the CSV grammar needs it.
fn csv_field(text: &str) -> String {
    if text.chars().any(|character| matches!(character, ',' | '"' | '\n' | '\r')) {
        format!("\"{}\"", text.replace('"', "\"\""))
    }
    else {
        text.to_string()
    }
}

const PDF_PAGE_WIDTH: f64 = 842.0;
const PDF_PAGE_HEIGHT: f64 = 595.0;
const PDF_MARGIN: f64 = 36.0;

/// Writes a one-page, A4 landscape vector PDF of every sheet.
///
/// The writer emits the catalog, page tree, font and content stream with a
/// cross-reference table, so the file opens in a viewer instead of only
/// starting with a PDF header.
fn render_pdf(design: &Design) -> String {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for sheet in &design.sheets {
        for component in &sheet.components {
            any = true;
            min_x = min_x.min(component.x - COMPONENT_HALF_WIDTH);
            max_x = max_x.max(component.x + COMPONENT_HALF_WIDTH);
            min_y = min_y.min(component.y - COMPONENT_HALF_HEIGHT);
            max_y = max_y.max(component.y + COMPONENT_HALF_HEIGHT);
        }
        for wire in &sheet.wires {
            any = true;
            for point in [wire.from, wire.to] {
                min_x = min_x.min(point[0]);
                max_x = max_x.max(point[0]);
                min_y = min_y.min(point[1]);
                max_y = max_y.max(point[1]);
            }
        }
    }
    if !any {
        min_x = 0.0;
        min_y = 0.0;
        max_x = 100.0;
        max_y = 60.0;
    }
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let scale = ((PDF_PAGE_WIDTH - 2.0 * PDF_MARGIN) / span_x).min((PDF_PAGE_HEIGHT - 2.0 * PDF_MARGIN) / span_y).max(0.001);
    let map_x = |x: f64| PDF_MARGIN + (x - min_x) * scale;
    let map_y = |y: f64| PDF_MARGIN + (max_y - y) * scale;
    let label_size = (7.0f64).max((3.2 * scale).min(9.0));

    let mut content = String::new();
    content.push_str(&format!("0.4 w 0 0 {PDF_PAGE_WIDTH:.2} {PDF_PAGE_HEIGHT:.2} re S\n"));
    let sheets_label =
        design.sheets.iter().map(|sheet| format!("{}: {}", sheet.id, sheet.name)).collect::<Vec<_>>().join(" | ");
    content.push_str(&format!(
        "BT /F1 9 Tf {:.2} {:.2} Td {} Tj ET\n",
        PDF_MARGIN,
        PDF_PAGE_HEIGHT - 16.0,
        pdf_text(&format!("OpenEDA {} revision {} | {}", design.id, design.revision, sheets_label))
    ));
    for sheet in &design.sheets {
        for wire in &sheet.wires {
            content.push_str(&format!(
                "0.6 w {:.2} {:.2} m {:.2} {:.2} l S\n",
                map_x(wire.from[0]),
                map_y(wire.from[1]),
                map_x(wire.to[0]),
                map_y(wire.to[1])
            ));
        }
        for component in &sheet.components {
            let x = map_x(component.x - COMPONENT_HALF_WIDTH);
            let y = map_y(component.y + COMPONENT_HALF_HEIGHT);
            let width = COMPONENT_HALF_WIDTH * 2.0 * scale;
            let height = COMPONENT_HALF_HEIGHT * 2.0 * scale;
            content.push_str(&format!("0.7 w {x:.2} {y:.2} {width:.2} {height:.2} re S\n"));
            content.push_str(&format!(
                "BT /F1 {label_size:.2} Tf {:.2} {:.2} Td {} Tj ET\n",
                map_x(component.x),
                map_y(component.y) + 2.0,
                pdf_text(&component.reference)
            ));
            if !component.value.is_empty() {
                content.push_str(&format!(
                    "BT /F1 {:.2} Tf {:.2} {:.2} Td {} Tj ET\n",
                    label_size * 0.85,
                    map_x(component.x),
                    map_y(component.y) - label_size - 1.0,
                    pdf_text(&component.value)
                ));
            }
        }
    }

    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PDF_PAGE_WIDTH:.0} {PDF_PAGE_HEIGHT:.0}] \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
    ];

    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
    pdf.push_str("0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));
    pdf
}

/// Escapes a string for a PDF literal string, dropping characters the byte
/// offsets cannot represent.
fn pdf_text(text: &str) -> String {
    let mut escaped = String::from("(");
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '(' => escaped.push_str("\\("),
            ')' => escaped.push_str("\\)"),
            '\n' | '\r' | '\t' => escaped.push(' '),
            character if character.is_ascii() && !character.is_ascii_control() => escaped.push(character),
            _ => escaped.push('?'),
        }
    }
    escaped.push(')');
    escaped
}

fn write_atomic(path: &Path, contents: &str, overwrite: bool) -> Result<usize, ServiceError> {
    if path.exists() && !overwrite {
        return Err(invalid(format!("`{}` already exists; pass overwrite to replace it", path.display())));
    }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(invalid(format!("directory `{}` does not exist", parent.display())));
    }
    let target =
        if path.parent().is_some_and(|parent| !parent.as_os_str().is_empty()) { path.to_path_buf() } else { parent.join(path) };
    boost_session::save_atomic(&target, contents.as_bytes())
        .map_err(|error| invalid(format!("cannot write `{}`: {error}", target.display())))?;
    Ok(contents.len())
}

#[cfg(test)]
mod tests {
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
            let expected_export = if matches!(format, "net" | "spice" | "svg" | "pdf" | "bom" | "kicad_sch") {
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
        assert_eq!(matrix["checks"]["nets"], "supported");
        assert_eq!(matrix["checks"]["connectivity"], "supported");
        assert_eq!(matrix["checks"]["footprints"], "supported");
        assert_eq!(matrix["checks"]["power"], "supported");
        assert_eq!(matrix["checks"]["drc"], "unsupported");
        assert_eq!(matrix["checks"]["clearance"], "unsupported");
        assert_eq!(matrix["renderers"]["svg"], "supported");
        assert_eq!(matrix["renderers"]["png"], "unsupported");
        assert_eq!(matrix["renderers"]["pdf"], "unsupported");
        assert_eq!(matrix["exportTargets"]["net"], "supported");
        assert_eq!(matrix["exportTargets"]["net_csv"], "supported");
        assert_eq!(matrix["exportTargets"]["net_hier"], "supported");
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
        assert_eq!(matrix["exportTargets"]["kicad_pcb"], "unsupported");
        assert_eq!(matrix["exportTargets"]["gbr"], "unsupported");
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

        update_component(handle, UpdateComponentRequest { sheet: None, reference: "R1".to_string(), value: "22k".to_string() })
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
        assert!(
            report["right"]["revision"].as_u64().expect("revision") > report["left"]["revision"].as_u64().expect("revision")
        );

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
                    },
                ],
                nets: Vec::new(),
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
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
            }],
            buses: Vec::new(),
            variants: Vec::new(),
            rules: DesignRules::default(),
            hierarchy: Vec::new(),
            next_component: 2,
            next_net: 3,
            next_wire: 1,
        };
        let handle = insert_design(design);
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
            UpdateComponentRequest { sheet: None, reference: "R1".to_string(), value: "22k".to_string() },
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
        let projection: Value =
            serde_json::from_str(&fs::read_to_string(&json_path).expect("read bom json")).expect("bom json");
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
                }],
                nets: Vec::new(),
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
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
            fs::write(&path, json!({ "format": "open-eda-design", "version": 1, "document": design }).to_string())
                .expect("write");
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
        assert_eq!(duplicate.code, "object/duplicate");
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
        assert_eq!(missing.code, "object/not-found");

        let removed = delete_no_connect(handle, 0, None).expect("delete flag");
        assert_eq!(removed["remaining"], 0);
        let restored =
            inspect_with_options(handle, InspectOptions { checks: Some(vec!["erc".to_string()]), ..InspectOptions::default() })
                .expect("inspect");
        assert_eq!(unconnected(&restored), 1);
        assert_eq!(delete_no_connect(handle, 0, None).expect_err("out of range").code, "object/not-found");
        close(handle).expect("close");
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
        assert!(codes(&conflict).contains(&"erc/output-output-conflict".to_string()));
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
        assert!(!codes(&unpowered_cleared).contains(&"erc/output-output-conflict".to_string()));
        assert!(!codes(&unpowered_cleared).contains(&"erc/power-in-unpowered".to_string()));
        assert!(!codes(&unpowered_cleared).contains(&"erc/input-input-conflict".to_string()));

        let input_revision = set("U1", "1", Some(PinType::Input))["revision"].as_u64().expect("revision");
        let inputs = inspect_with_options(
            handle,
            InspectOptions { checks: Some(vec!["electrical".to_string()]), ..InspectOptions::default() },
        )
        .expect("inspect");
        assert!(codes(&inputs).contains(&"erc/input-input-conflict".to_string()));

        let no_op = set("U1", "1", Some(PinType::Input));
        assert_eq!(no_op["revision"].as_u64().expect("revision"), input_revision);
        let cleared = set("U1", "1", None);
        assert_eq!(cleared["pinType"], Value::Null);
        let after_clear = inspect_with_options(
            handle,
            InspectOptions { checks: Some(vec!["electrical".to_string()]), ..InspectOptions::default() },
        )
        .expect("inspect");
        assert!(!codes(&after_clear).contains(&"erc/input-input-conflict".to_string()));

        let error = set_pin_type(
            handle,
            SetPinTypeRequest {
                sheet: None,
                reference: "U1".to_string(),
                pin: "9".to_string(),
                pin_type: Some(PinType::Output),
            },
        )
        .expect_err("missing pin");
        assert_eq!(error.code, "object/not-found");
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
            UpdateDesignRequest {
                title: Some("Blinker II".to_string()),
                rev: Some("B".to_string()),
                company: None,
                date: None,
            },
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
            SetPinTypeRequest {
                sheet: None,
                reference: "U1".to_string(),
                pin: "1".to_string(),
                pin_type: Some(PinType::Output),
            },
        )
        .expect("type pin 1");
        set_pin_type(
            handle,
            SetPinTypeRequest {
                sheet: None,
                reference: "U1".to_string(),
                pin: "2".to_string(),
                pin_type: Some(PinType::PowerIn),
            },
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
            assert!(detail.contains("sheet-local net label"));
        }

        let sheet = &imported["design"]["sheets"][0];
        assert_eq!(sheet["no_connects"], json!([{ "x": 50.0, "y": 60.0 }]));
        let nets = sheet["nets"].as_array().expect("nets");
        let names: Vec<&str> = nets.iter().filter_map(|net| net["name"].as_str()).collect();
        assert_eq!(names, vec!["VCC", "SIG"]);
        assert_eq!(nets[0]["nodes"], json!([{ "ref": "R1", "pin": "1" }]));
        assert_eq!(nets[1]["nodes"], json!([]));

        let design: Design = serde_json::from_value(imported["design"].clone()).expect("design");
        let rendered = kicad::render(&design);
        assert!(rendered.contains("(no_connect (at 50 60)"));
        assert!(rendered.contains("(label \"VCC\""));

        let again = import_kicad_sch(&rendered).expect("reimport");
        assert_eq!(again["losses"], json!([]));
        assert_eq!(again["design"]["sheets"][0]["no_connects"], json!([{ "x": 50.0, "y": 60.0 }]));
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
            ConnectNetRequest {
                sheet: Some("Power Supply".to_string()),
                net: "VCC".to_string(),
                nodes: vec!["R2.1".to_string()],
            },
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
        assert_eq!(move_junction(handle, 0, None, 0.0, 0.0).unwrap_err().code, "object/not-found");
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

        let disconnected = disconnect_net(
            handle,
            DisconnectNetRequest { sheet: None, net: "VCC".to_string(), nodes: vec!["R1.2".to_string()] },
        )
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
                }],
                nets: vec![Net { id: "net-1".to_string(), name: "SPARE".to_string(), nodes: Vec::new() }],
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
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

        let added = add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 4, aliases: vec!["D".to_string()] })
            .expect("add bus");
        assert_eq!(added["bus"], json!({ "id": "bus-1", "name": "DATA", "width": 4, "aliases": ["D"] }));
        assert_eq!(added["revision"], 3);
        assert_eq!(
            add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2, aliases: Vec::new() }).unwrap_err().code,
            "object/duplicate"
        );
        assert_eq!(
            add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 2, aliases: vec!["D".to_string()] })
                .unwrap_err()
                .code,
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
        assert_eq!(
            set_reference_prefix(handle, rule("resistor", "TOOLONGPREFIX")).unwrap_err().code,
            "session/argument-invalid"
        );
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
        let cleared = set_component_class(
            handle,
            ComponentClassRequest { sheet: None, reference: "R1".to_string(), class: String::new() },
        )
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
        update_component(handle, UpdateComponentRequest { sheet: None, reference: "R1".to_string(), value: "12k".to_string() })
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
        assert_eq!(
            export_with_variant(handle, "net", &default_bom, Some("lite")).unwrap_err().code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_variant(
                handle,
                VariantRequest { name: "lite".to_string(), dnp: vec!["R9".to_string()], values: BTreeMap::new() }
            )
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
        annotate(handle).expect("annotate");
        let variant = &list_variants(handle).expect("list")["variants"][0];
        assert_eq!(variant["dnp"], json!(["R1"]));
        assert_eq!(variant["values"]["R2"], "22k");
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
                SetPinTypeRequest {
                    sheet: None,
                    reference: reference.to_string(),
                    pin: pin.to_string(),
                    pin_type: Some(pin_type),
                },
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

        set_pin_type(
            handle,
            SetPinTypeRequest { sheet: None, reference: "U2".to_string(), pin: "1".to_string(), pin_type: None },
        )
        .expect("clear");
        let single = inspect(handle, Some(vec!["power".to_string()])).expect("inspect power");
        let codes =
            single["issues"].as_array().expect("issues").iter().filter_map(|issue| issue["code"].as_str()).collect::<Vec<_>>();
        assert!(!codes.contains(&"power/driver-conflict"));
        assert!(codes.contains(&"power/output-driver-conflict"));
        close(handle).expect("close");
    }
}
