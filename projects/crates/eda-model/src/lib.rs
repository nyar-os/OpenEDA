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
mod dangling;
mod diff;
mod drawings;
mod kicad;
mod net_audit;
mod png;

pub use net_audit::net_audit;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fmt, fs,
    path::{Component as PathComponent, Path, PathBuf},
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
const INSPECT_CHECKS: &[&str] = &[
    "erc",
    "electrical",
    "drivers",
    "pin_types",
    "bus",
    "drc",
    "nets",
    "connectivity",
    "dangling",
    "footprints",
    "clearance",
    "power",
    "references",
];

/// Rule checks this build actually runs.
const SUPPORTED_CHECKS: &[&str] = &[
    "erc",
    "electrical",
    "drivers",
    "pin_types",
    "bus",
    "nets",
    "connectivity",
    "dangling",
    "footprints",
    "clearance",
    "power",
    "references",
];

/// Render targets declared for preview surfaces.
const RENDER_FORMATS: &[&str] = &["svg", "png", "pdf"];

/// Export targets declared by the command line.
const EXPORT_TARGETS: &[&str] = &[
    "kicad_sch",
    "kicad_sym",
    "kicad_net",
    "kicad_pcb",
    "net",
    "net_csv",
    "net_hier",
    "net_class",
    "net_length",
    "spice",
    "spice_subckt",
    "gbr",
    "pdf",
    "svg",
    "png",
    "block_diagram",
    "sheet_overview",
    "pinout_chart",
    "symbol_library",
    "net_topology",
    "power_distribution",
    "bom_drawing",
    "bom_drawing_pdf",
    "net_xref",
    "net_xref_pdf",
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
    "kicad_net",
    "net",
    "net_csv",
    "net_hier",
    "net_class",
    "net_length",
    "spice",
    "spice_subckt",
    "svg",
    "pdf",
    "png",
    "block_diagram",
    "sheet_overview",
    "pinout_chart",
    "symbol_library",
    "net_topology",
    "power_distribution",
    "bom_drawing",
    "bom_drawing_pdf",
    "net_xref",
    "net_xref_pdf",
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

/// Tag written into the deterministic statistics report.
const STATS_FORMAT: &str = "open-eda-stats";
/// Version written into the deterministic statistics report.
const STATS_VERSION: u64 = 1;

/// Tag written into the read-only hierarchy report.
const HIERARCHY_REPORT_FORMAT: &str = "open-eda-hierarchy";
/// Version written into the read-only hierarchy report.
const HIERARCHY_REPORT_VERSION: u64 = 1;
/// Deepest child document the hierarchy check opens; the root document is depth 0.
const HIERARCHY_MAX_DEPTH: usize = 32;

/// Tag written into the read-only pin trace report.
const TRACE_REPORT_FORMAT: &str = "open-eda-trace";
/// Version written into the read-only pin trace report.
const TRACE_REPORT_VERSION: u64 = 1;

/// Tag written into the read-only component detail report.
const COMPONENT_REPORT_FORMAT: &str = "open-eda-component";
/// Version written into the read-only component detail report.
const COMPONENT_REPORT_VERSION: u64 = 1;

/// Tag written into the read-only net detail report.
const NET_REPORT_FORMAT: &str = "open-eda-net";
/// Version written into the read-only net detail report.
const NET_REPORT_VERSION: u64 = 1;

/// Loss code reported when a SPICE element pin has no net and gets an NC node.
const SPICE_OPEN_PIN: &str = "spice/open-pin";

/// Loss code reported when an element has fewer pins than its SPICE mapping needs.
const SPICE_PIN_COUNT: &str = "spice/pin-count";

/// Loss code reported when the PDF writer replaces characters the base-14
/// Helvetica encoding cannot carry.
const PDF_TEXT_ENCODING: &str = "pdf/text-encoding";

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
pub(crate) const COMPONENT_HALF_WIDTH: f64 = 12.7;
/// Component box half height, in design units, used by the SVG projection.
const COMPONENT_HALF_HEIGHT: f64 = 7.62;
/// Vertical pitch between adjacent symbol pins, in design units.
const PIN_PITCH: f64 = 2.54;
/// Length of the pin stub drawn outside the symbol body, in design units.
pub(crate) const PIN_LENGTH: f64 = 2.54;
/// Font size of the pin names printed inside the symbol body.
const PIN_NAME_SIZE: f64 = 2.0;

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
    /// Mirror axis applied in the component's local frame, before rotation.
    ///
    /// `Some("x")` flips the symbol left-right about its own vertical centre
    /// line, `Some("y")` flips it top-bottom about its horizontal centre line.
    /// `None`, the field absent from the document, keeps the symbol as drawn.
    /// The projections mirror the body artwork and the pin stubs; the reference,
    /// value and pin-name texts stay readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
    /// Pin names this component exposes.
    pub pins: Vec<String>,
    /// Electrical role of the pins that have one, keyed by pin name.
    ///
    /// An empty map means no pin has been typed yet. The `electrical` check
    /// only reads pins listed here and ignores the rest.
    #[serde(default, rename = "pinTypes", skip_serializing_if = "BTreeMap::is_empty")]
    pub pin_types: BTreeMap<String, PinType>,
    /// Custom key/value metadata, such as a manufacturer part number.
    ///
    /// Keys are unique inside the component and are stored verbatim after
    /// trimming; values are stored exactly as passed. The map is absent from
    /// older documents. Custom properties are model metadata: the schematics
    /// projections keep them in the JSON document but no exchange writer emits
    /// them yet.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
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

/// Cross-sheet scope of one net label.
///
/// The names match the KiCad label nodes (`label`, `global_label`,
/// `hierarchical_label`). The model stores the scope as a label attribute; it
/// does not merge nets across sheets, so a global label still names a net on the
/// sheet that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelScope {
    /// Ordinary sheet-local net label.
    Local,
    /// Label intended to name a net across the whole design.
    Global,
    /// Label intended to cross a hierarchical sheet boundary.
    Hierarchical,
}

/// One net label placed on a schematic sheet.
///
/// A label is an annotation, not a net: its text is an independent statement of
/// the intended net name at that point. The model keeps it separate from the
/// declared [`Net`] list so a designer can label a wire before deriving or
/// renaming the net, and so import never silently rewrites a label into a net.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetLabel {
    /// Stable object identity, for example `label-1`.
    pub id: String,
    /// Label text, the net name it announces.
    pub text: String,
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
    /// Cross-sheet scope of the label.
    pub scope: LabelScope,
    /// Rotation in degrees; absent when the label is horizontal.
    #[serde(default)]
    pub rotation: f64,
    /// Horizontal justification, one of `left`, `right` or `center`.
    #[serde(default = "default_justify")]
    pub justify: String,
}

/// Default horizontal justification of a net label.
fn default_justify() -> String {
    "left".to_string()
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
/// A symbol names the child sheet file and its ports. Editing never reads that
/// child file, so the symbol is a declaration: it survives import and export,
/// participates in validation and is reported by the JSON projection, but its
/// contents are not merged into the flat `sheets` list. Only the read-only
/// [`check_hierarchy`] follows the file to compare ports with the child labels.
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
    /// Net labels drawn on this sheet; absent in older documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<NetLabel>,
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

/// Request of [`split_wire`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitWireRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Stable wire identity, for example `wire-1`.
    pub id: String,
    /// Split point X coordinate, on the segment.
    pub x: f64,
    /// Split point Y coordinate, on the segment.
    pub y: f64,
}

/// Request of [`translate_objects`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Components to move, by reference designator.
    #[serde(default)]
    pub references: Vec<String>,
    /// Wire segments to move, by stable id.
    #[serde(default)]
    pub wires: Vec<String>,
    /// Offset added to every selected X coordinate.
    pub dx: f64,
    /// Offset added to every selected Y coordinate.
    pub dy: f64,
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

/// Request of [`update_text`].
///
/// Every field except the index is optional, so a request carries only what it
/// changes. A request without `text`, `x` or `y` is rejected instead of adding a
/// revision.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTextRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// 0-based position inside the sheet's text list.
    pub index: usize,
    /// New text content; trimmed for the empty check and stored verbatim.
    #[serde(default)]
    pub text: Option<String>,
    /// New placement X coordinate; the current value is kept when omitted.
    #[serde(default)]
    pub x: Option<f64>,
    /// New placement Y coordinate; the current value is kept when omitted.
    #[serde(default)]
    pub y: Option<f64>,
}

/// Request of [`add_label`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLabelRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Label text, the net name the label announces.
    pub text: String,
    /// Placement X coordinate.
    pub x: f64,
    /// Placement Y coordinate.
    pub y: f64,
    /// Cross-sheet scope; `local` when omitted.
    #[serde(default)]
    pub scope: Option<LabelScope>,
    /// Rotation in degrees; zero when omitted.
    #[serde(default)]
    pub rotation: Option<f64>,
    /// Horizontal justification, one of `left`, `right` or `center`; `left` when omitted.
    #[serde(default)]
    pub justify: Option<String>,
}

/// Request of [`update_label`].
///
/// Every field except the label identity is optional, so a request carries only
/// what it changes. A request that changes nothing is rejected.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLabelRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Stable label identity, for example `label-1`.
    pub id: String,
    /// New label text; trimmed for the empty check and stored verbatim.
    #[serde(default)]
    pub text: Option<String>,
    /// New placement X coordinate; the current value is kept when omitted.
    #[serde(default)]
    pub x: Option<f64>,
    /// New placement Y coordinate; the current value is kept when omitted.
    #[serde(default)]
    pub y: Option<f64>,
    /// New cross-sheet scope; the current value is kept when omitted.
    #[serde(default)]
    pub scope: Option<LabelScope>,
    /// New rotation in degrees; the current value is kept when omitted.
    #[serde(default)]
    pub rotation: Option<f64>,
    /// New horizontal justification; the current value is kept when omitted.
    #[serde(default)]
    pub justify: Option<String>,
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
///
/// Every field besides the identity is optional and only the fields a request
/// carries change, so one call can update the value, the footprint and the
/// placement without touching the rest.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// New value text; kept when omitted.
    #[serde(default)]
    pub value: Option<String>,
    /// New footprint text; the current assignment is kept when omitted and an
    /// empty string clears it.
    #[serde(default)]
    pub footprint: Option<String>,
    /// New placement X coordinate; kept when omitted.
    #[serde(default)]
    pub x: Option<f64>,
    /// New placement Y coordinate; kept when omitted.
    #[serde(default)]
    pub y: Option<f64>,
    /// New rotation in degrees; kept when omitted.
    #[serde(default)]
    pub rotation: Option<f64>,
}

/// Request of [`rotate_component`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Relative rotation in degrees, added to the current rotation.
    pub angle: f64,
}

/// Request of [`mirror_component`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Mirror axis in the component's local frame: `x` flips left-right, `y`
    /// flips top-bottom.
    pub axis: String,
}

/// Request of [`copy_component`].
///
/// The copy stays on the source sheet, keeps the source value, footprint,
/// rotation, mirror, pins, pin roles and custom properties, and moves by
/// `dx`/`dy`. Net nodes, wires and no-connect flags are never copied.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component to copy.
    pub reference: String,
    /// Reference designator of the copy; unique across every sheet.
    pub new_reference: String,
    /// Offset added to the source X coordinate; defaults to zero.
    #[serde(default)]
    pub dx: f64,
    /// Offset added to the source Y coordinate; defaults to zero.
    #[serde(default)]
    pub dy: f64,
}

/// Request of [`duplicate_component`].
///
/// The duplicate stays on the source sheet, keeps the source value, footprint,
/// rotation, mirror, pins, pin roles, custom properties and component class,
/// and moves by `dx`/`dy`. The reference is chosen automatically as the next free number
/// of the source reference prefix.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateComponentRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component to duplicate.
    pub reference: String,
    /// Offset added to the source X coordinate; defaults to zero.
    #[serde(default)]
    pub dx: f64,
    /// Offset added to the source Y coordinate; defaults to zero.
    #[serde(default)]
    pub dy: f64,
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

/// Request of [`set_component_property`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetComponentPropertyRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Property key, for example `MPN`.
    pub key: String,
    /// Property value; an empty string stores an empty value.
    #[serde(default)]
    pub value: String,
}

/// Request of [`remove_component_property`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveComponentPropertyRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Property key to remove.
    pub key: String,
}

/// Request of [`add_hierarchy`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddHierarchyRequest {
    /// Sheet name shown on the parent symbol; unique in the design.
    pub name: String,
    /// Child schematic file name, for example `power.kicad_sch`.
    pub file: String,
    /// Symbol top-left X coordinate; defaults to 0.
    #[serde(default)]
    pub x: f64,
    /// Symbol top-left Y coordinate; defaults to 0.
    #[serde(default)]
    pub y: f64,
    /// Symbol width; defaults to 60.
    #[serde(default = "default_hierarchy_width")]
    pub width: f64,
    /// Symbol height; defaults to 40.
    #[serde(default = "default_hierarchy_height")]
    pub height: f64,
}

/// Request of [`add_hierarchy_port`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddHierarchyPortRequest {
    /// Hierarchical sheet id or name.
    pub sheet: String,
    /// Port name, unique inside the symbol.
    pub name: String,
    /// Electrical direction; defaults to `passive`.
    #[serde(default = "default_port_direction")]
    pub direction: PortDirection,
    /// Port placement X coordinate; defaults to 0.
    #[serde(default)]
    pub x: f64,
    /// Port placement Y coordinate; defaults to 0.
    #[serde(default)]
    pub y: f64,
}

/// Request of [`remove_hierarchy_port`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveHierarchyPortRequest {
    /// Hierarchical sheet id or name.
    pub sheet: String,
    /// Port name to remove.
    pub name: String,
}

const fn default_hierarchy_width() -> f64 {
    60.0
}

const fn default_hierarchy_height() -> f64 {
    40.0
}

const fn default_port_direction() -> PortDirection {
    PortDirection::Passive
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

/// Request of [`add_pin`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddPinRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Pin name to add.
    pub pin: String,
}

/// Request of [`remove_pin`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovePinRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Pin name to remove.
    pub pin: String,
}

/// Request of [`rename_pin`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePinRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// Current pin name.
    pub from: String,
    /// New pin name.
    pub to: String,
}

/// Request of [`swap_pins`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapPinsRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Reference designator of the placed component.
    pub reference: String,
    /// First pin name as declared by the component.
    pub pin_a: String,
    /// Second pin name as declared by the component.
    pub pin_b: String,
}

/// Request of [`rename_net`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameNetRequest {
    /// Sheet that identifies the net; the first sheet is used when omitted.
    /// The rename itself follows the old name across every sheet.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Current net id or name.
    pub net: String,
    /// New net name.
    pub name: String,
}

/// Request of [`merge_nets`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeNetsRequest {
    /// Target sheet id or name; the first sheet is used when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Net that survives the merge, by id or name.
    pub target: String,
    /// Net absorbed into the target, by id or name.
    pub source: String,
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

/// Request of [`update_bus`].
///
/// Every field is optional and only the fields a request carries change, so one
/// call can rename a bus, change its width, replace its aliases or do all three
/// without touching the rest. Supplying `aliases` replaces the whole list, and an
/// empty list clears every alias.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBusRequest {
    /// New bus name; the current name is kept when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// New member count; must be at least 1 and keeps the current width when omitted.
    #[serde(default)]
    pub width: Option<u64>,
    /// Replacement alias list; the current aliases are kept when omitted and an
    /// empty list clears them.
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
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
/// SVG, BOM (CSV, JSON and HTML), PDF, report, JSON document, net-length,
/// KiCad schematic and KiCad symbol library
/// projections are supported,
/// KiCad schematics are also imported, and every other exchange format and rule
/// check is reported as unsupported. Design-level buses are edited through
/// `add_bus`, `remove_bus` and `list_buses`; a bus carries a width and optional
/// aliases that resolve to the same expansion. ERC reports the stable codes
/// `erc/duplicate-net-name`, `erc/duplicate-pin`, `erc/duplicate-reference`,
/// `erc/single-node-net`, `erc/dangling-net`, `erc/unconnected-pin`,
/// `erc/floating-wire`, `erc/reference-illegal`, `erc/pin-name-illegal`,
/// `erc/net-name-illegal` and `erc/unconnected-hierarchical-port`
/// next to the legacy codes. A no-connect flag that names a component pin
/// removes `erc/unconnected-pin` and `unreferenced-pin` for that pin, because
/// the open pin is deliberate. The `nets` check also reports `net-bus-member-collision` when a
/// declared net would be written twice as a bus member, and
/// `net-bus-alias-collision` when two nets on one sheet resolve to the same bus
/// through an alias. The JSON keys of this
/// report are unchanged. The `drivers` check reads typed pins only and reports
/// `erc/undriven-input` for a net whose inputs no driving pin reaches, and
/// `erc/unconnected-power-pin` for a typed power input that no net and no
/// no-connect flag covers. The `pin_types` check reads typed pins only and
/// reports `erc/output-power-in-conflict` when one net joins a logic `output`
/// pin and a `power_in` pin, and `erc/power-output-driver-conflict` when a
/// `power_out` pin shares a net with a `bidirectional`, `tri_state`,
/// `open_collector` or `open_emitter` pin; the conflicts the `electrical` and
/// `power` checks already report are not repeated.
///
/// Junctions and no-connect flags are editable, not read-only: the matrix
/// reports `edits.junctions` and `edits.noConnects` as `supported` because
/// `add_junction`/`delete_junction` and `add_no_connect`/`delete_no_connect`
/// exist and are covered by tests. Component pins are editable too:
/// `edits.pins` is `supported` because [`add_pin`], [`remove_pin`] and
/// [`rename_pin`] exist and are covered by tests. Reference annotation is an
/// editing surface as well: `edits.annotate` is `supported` because [`annotate`]
/// exists and is covered by tests. Wire splitting reports `edits.wireSplit` as
/// `supported` because [`split_wire`] exists and is covered by tests. Component
/// transforms report `edits.rotate` and `edits.mirror` as `supported` because
/// [`rotate_component`] and [`mirror_component`] exist and are covered by tests.
///
/// Read-only queries live under `queries`, next to `checks`: `queries.trace`
/// reports the pin-to-pin path query of [`trace_pins`] and `queries.net` reports
/// the net detail query of [`net_show`]. `queries.netAudit` reports the per-net
/// health report of [`net_audit`]. They are queries, not rule checks, so they
/// never appear in the `inspect --checks` value.
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
    checks.insert("hierarchy".to_string(), Value::String(level("check.hierarchy".to_string()).to_string()));

    let mut renderers = Map::new();
    for &renderer in RENDER_FORMATS {
        renderers.insert(renderer.to_string(), Value::String(level(format!("renderer.{renderer}")).to_string()));
    }

    let mut export_targets = Map::new();
    for &target in EXPORT_TARGETS {
        export_targets.insert(target.to_string(), Value::String(level(format!("export.{target}")).to_string()));
    }

    let edits = json!({
        "components": level("edit.components".to_string()),
        "junctions": level("edit.junctions".to_string()),
        "noConnects": level("edit.noConnects".to_string()),
        "pins": level("edit.pins".to_string()),
        "pinSwap": level("edit.pin-swap".to_string()),
        "annotate": level("edit.annotate".to_string()),
        "netRename": level("edit.net-rename".to_string()),
        "wireSplit": level("edit.wire-split".to_string()),
        "duplicate": level("edit.component-duplicate".to_string()),
        "rotate": level("edit.component-rotate".to_string()),
        "mirror": level("edit.component-mirror".to_string()),
    });

    let mut queries = Map::new();
    queries.insert("component".to_string(), Value::String(level("query.component".to_string()).to_string()));
    queries.insert("net".to_string(), Value::String(level("query.net".to_string()).to_string()));
    queries.insert("trace".to_string(), Value::String(level("query.trace".to_string()).to_string()));
    queries.insert("netAudit".to_string(), Value::String(level("query.netAudit".to_string()).to_string()));

    json!({
        "engine": { "package": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
        "project": { "import": "partial", "edit": "partial", "export": "partial" },
        "formats": Value::Object(formats),
        "checks": Value::Object(checks),
        "renderers": Value::Object(renderers),
        "exportTargets": Value::Object(export_targets),
        "edits": edits,
        "queries": Value::Object(queries),
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
    matrix.insert(Capability::new("check.hierarchy", "check", Support::Supported));
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
    matrix.insert(Capability::new("edit.components", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.junctions", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.noConnects", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.pins", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.pin-swap", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.annotate", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.net-rename", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.wire-split", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.component-duplicate", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.component-rotate", "edit", Support::Supported));
    matrix.insert(Capability::new("edit.component-mirror", "edit", Support::Supported));
    matrix.insert(Capability::new("query.component", "query", Support::Supported));
    matrix.insert(Capability::new("query.net", "query", Support::Supported));
    matrix.insert(Capability::new("query.trace", "query", Support::Supported));
    matrix.insert(Capability::new("query.netAudit", "query", Support::Supported));
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
    /// Document path the design was opened from or last saved to.
    path: Option<PathBuf>,
}

impl Session {
    fn new(design: Design) -> Self {
        Self { design, history: History::new(HISTORY_LIMIT), path: None }
    }

    fn at(design: Design, path: PathBuf) -> Self {
        Self { design, history: History::new(HISTORY_LIMIT), path: Some(path) }
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

fn insert_session(session: Session) -> u64 {
    let handle = store().next_handle.fetch_add(1, Ordering::SeqCst);
    locked_sessions().insert(handle, session);
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
            labels: Vec::new(),
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
    let handle = insert_session(session);
    Ok(json!({ "handle": handle, "design": summary }))
}

/// Opens a native design document and returns its handle and summary.
pub fn open(path: &Path) -> Result<Value, ServiceError> {
    let design = read_design(path)?;
    let session = Session::at(design, path.to_path_buf());
    let summary = summary_of(&session);
    let handle = insert_session(session);
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
    if let Some(session) = locked_sessions().get_mut(&handle) {
        session.path = Some(path.to_path_buf());
    }
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

/// Reports one declared net by name and the objects that reference it.
///
/// `name` is trimmed and must not be empty (`session/argument-invalid`). A named
/// `sheet` resolves through [`sheet_index`] and limits the search to that sheet;
/// without it the query walks every sheet in document order. A name no searched
/// sheet declares is `net/not-found`. Each match reports the sheet identity, the
/// net id and its nodes, the wires on that sheet whose declared net name equals
/// `name` with their 0-based document index, and the net labels whose text equals
/// `name`. The report also resolves the declared bus whose name or alias equals
/// `name` and the net class whose `nets` list contains it, or `null` for each.
/// The report is deterministic and never mutates the design.
pub fn net_show(handle: u64, name: &str, sheet: Option<&str>) -> Result<Value, ServiceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a net name must not be empty"));
    }
    with_session(handle, |session| {
        let design = &session.design;
        let scope = match sheet {
            Some(needle) => Some((sheet_index(design, sheet)?, needle)),
            None => None,
        };
        let mut matches = Vec::new();
        let mut node_count = 0usize;
        for (index, current) in design.sheets.iter().enumerate() {
            if scope.is_some_and(|(only, _)| only != index) {
                continue;
            }
            for net in current.nets.iter().filter(|net| net.name == name) {
                node_count += net.nodes.len();
                let wires = current
                    .wires
                    .iter()
                    .enumerate()
                    .filter(|(_, wire)| wire.net.as_deref() == Some(name))
                    .map(|(wire_index, wire)| json!({ "index": wire_index, "from": wire.from, "to": wire.to }))
                    .collect::<Vec<_>>();
                let labels = current
                    .labels
                    .iter()
                    .filter(|label| label.text == name)
                    .map(|label| json!({ "text": label.text, "scope": label.scope, "x": label.x, "y": label.y }))
                    .collect::<Vec<_>>();
                matches.push(json!({
                    "sheet": { "id": current.id, "name": current.name },
                    "net": { "id": net.id, "name": net.name, "nodes": net.nodes },
                    "wires": wires,
                    "labels": labels,
                }));
            }
        }
        if matches.is_empty() {
            let message = match scope {
                Some((_, needle)) => format!("net `{name}` is not declared on sheet `{needle}`"),
                None => format!("net `{name}` is not declared in this design"),
            };
            return Err(ServiceError::new("net/not-found", message));
        }
        let bus = bus_for_net(design, name).map(|bus| {
            json!({
                "id": bus.id,
                "name": bus.name,
                "width": bus.width,
                "members": (0..bus.width).map(|index| format!("{}{index}", bus.name)).collect::<Vec<_>>(),
            })
        });
        let net_class = design.rules.net_classes.iter().find(|class| class.nets.iter().any(|net| net == name)).map(|class| {
            json!({
                "id": class.id,
                "name": class.name,
                "width": class.width,
                "clearance": class.clearance,
                "via": class.via,
            })
        });
        Ok(json!({
            "format": NET_REPORT_FORMAT,
            "version": NET_REPORT_VERSION,
            "revision": design.revision,
            "design": summary_of(session),
            "name": name,
            "sheets": matches,
            "nodeCount": node_count,
            "bus": bus,
            "netClass": net_class,
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

/// Updates one declared bus selected by id, name or alias.
///
/// A request carries only what it changes and needs at least one of `name`,
/// `width` or `aliases`; every field it does not name keeps its current value.
/// The name follows the same rules as [`add_bus`]: it is trimmed, must not be
/// empty and must not collide with another bus's name, aliases or member names
/// (`object/duplicate`). The width must be at least 1
/// (`session/argument-invalid`). Supplying `aliases` replaces the whole list, so
/// an empty list clears every alias; each alias follows the same legality,
/// uniqueness and collision rules as [`add_bus`]. A rename changes only the
/// declaration: nets, net nodes and net-class assignments are not rewritten.
/// Changing the width changes only the derived member names. A request that
/// leaves the declaration unchanged reports `changed: false` and adds no
/// revision.
pub fn update_bus(handle: u64, id: &str, request: UpdateBusRequest) -> Result<Value, ServiceError> {
    let width = request.width;
    if request.name.is_none() && width.is_none() && request.aliases.is_none() {
        return Err(ServiceError::new("session/argument-invalid", "a bus update needs `name`, `width` or `aliases`"));
    }
    let name = match request.name.as_deref() {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(ServiceError::new("session/argument-invalid", "a bus name must not be empty"));
            }
            Some(trimmed.to_string())
        }
        None => None,
    };
    if width == Some(0) {
        return Err(ServiceError::new("session/argument-invalid", "a bus width must be at least 1"));
    }
    let aliases = match request.aliases {
        Some(raw_aliases) => {
            let mut cleaned = Vec::new();
            for raw in raw_aliases {
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
                if cleaned.contains(&alias) {
                    return Err(ServiceError::new(
                        "session/argument-invalid",
                        format!("bus alias `{alias}` is declared more than once"),
                    ));
                }
                cleaned.push(alias);
            }
            Some(cleaned)
        }
        None => None,
    };
    mutate(handle, |design| {
        let position = design
            .buses
            .iter()
            .position(|entry| entry.id == id || entry.name == id || entry.aliases.iter().any(|alias| alias == id))
            .ok_or_else(|| not_found(format!("no bus `{id}` is declared")))?;
        let current = design.buses[position].clone();
        let next_name = name.clone().unwrap_or_else(|| current.name.clone());
        let next_width = width.unwrap_or(current.width);
        let next_aliases = aliases.clone().unwrap_or_else(|| current.aliases.clone());
        let declared = |candidate: &str| {
            design.buses.iter().enumerate().any(|(index, bus)| {
                index != position
                    && (bus.name == candidate
                        || bus.aliases.iter().any(|alias| alias == candidate)
                        || (0..bus.width).any(|member| format!("{}{member}", bus.name) == candidate))
            })
        };
        if declared(&next_name) {
            return Err(duplicate(format!("bus name `{next_name}` already exists")));
        }
        for alias in &next_aliases {
            if alias == &next_name {
                return Err(ServiceError::new("session/argument-invalid", "a bus alias must differ from its bus name"));
            }
            if declared(alias) {
                return Err(duplicate(format!("bus alias `{alias}` already resolves to a declared bus")));
            }
            if (0..next_width).any(|index| format!("{next_name}{index}") == *alias) {
                return Err(duplicate(format!("bus alias `{alias}` is already a member name of bus `{next_name}`")));
            }
        }
        let changed = next_name != current.name || next_width != current.width || next_aliases != current.aliases;
        if changed {
            let bus = &mut design.buses[position];
            bus.name = next_name;
            bus.width = next_width;
            bus.aliases = next_aliases;
            design.revision += 1;
        }
        let bus = design.buses[position].clone();
        let mut payload = Map::new();
        payload.insert("bus".to_string(), json!(bus));
        payload.insert("changed".to_string(), json!(changed));
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
            mirror: None,
            pins: request.pins,
            pin_types: BTreeMap::new(),
            properties: BTreeMap::new(),
        };
        sheet.components.push(component.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        Ok(payload)
    })
}

/// Copies one placed component under a new reference designator.
///
/// The copy stays on the source sheet, receives a fresh `component-N` identity
/// and inherits the source value, footprint, rotation, mirror, pins, pin roles,
/// custom properties and component class. It is placed at the source position plus
/// `dx`/`dy`. Net nodes, wires and no-connect flags are never copied, so the new
/// component starts electrically unconnected. The new reference must be unique
/// across every sheet and reuses the duplicate error code of [`add_component`].
pub fn copy_component(handle: u64, request: CopyComponentRequest) -> Result<Value, ServiceError> {
    let new_reference = request.new_reference.trim().to_string();
    if new_reference.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a component reference must not be empty"));
    }
    if !request.dx.is_finite() || !request.dy.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "component offsets must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let source = design.sheets[index]
            .components
            .iter()
            .find(|component| component.reference == request.reference)
            .cloned()
            .ok_or_else(|| not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)))?;
        if design.sheets.iter().any(|sheet| sheet.components.iter().any(|component| component.reference == new_reference)) {
            return Err(duplicate(format!("reference `{new_reference}` already exists in this design")));
        }
        let x = source.x + request.dx;
        let y = source.y + request.dy;
        if !x.is_finite() || !y.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "component coordinates must be finite numbers"));
        }
        let class = design.rules.component_classes.get(&source.id).cloned();
        let sheet = &mut design.sheets[index];
        let mut used: HashSet<String> = sheet.components.iter().map(|component| component.id.clone()).collect();
        let id = next_id(&mut design.next_component, "component", &mut used);
        let component = Component {
            id: id.clone(),
            reference: new_reference,
            value: source.value,
            footprint: source.footprint,
            x,
            y,
            rotation: source.rotation,
            mirror: source.mirror,
            pins: source.pins,
            pin_types: source.pin_types,
            properties: source.properties,
        };
        sheet.components.push(component.clone());
        if let Some(class) = class {
            design.rules.component_classes.insert(id, class);
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        Ok(payload)
    })
}

/// Duplicates one placed component on the same sheet under the next free reference.
///
/// The duplicate keeps the source value, footprint, rotation, mirror, pins,
/// pin roles, custom properties and component class, and is placed at the source position
/// plus `dx`/`dy`. Its reference is derived from the leading ASCII letters of
/// the source reference and takes the smallest positive number that no sheet
/// already uses with that prefix, so `R1` in a design holding `R1` and `R3`
/// duplicates to `R2`. Net nodes, wires and no-connect flags are never copied,
/// so the duplicate starts electrically unconnected. A source reference without
/// leading letters is refused (`session/argument-invalid`) because no prefix
/// can be derived, and a reference the sheet does not place is
/// `object/not-found`. One successful call adds exactly one revision.
pub fn duplicate_component(handle: u64, request: DuplicateComponentRequest) -> Result<Value, ServiceError> {
    if !request.dx.is_finite() || !request.dy.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "component offsets must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let source = design.sheets[index]
            .components
            .iter()
            .find(|component| component.reference == request.reference)
            .cloned()
            .ok_or_else(|| not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)))?;
        let prefix: String = source.reference.chars().take_while(char::is_ascii_alphabetic).collect();
        if prefix.is_empty() {
            return Err(ServiceError::new(
                "session/argument-invalid",
                format!("cannot derive a reference prefix from `{}`", source.reference),
            ));
        }
        let used: HashSet<String> =
            design.sheets.iter().flat_map(|sheet| &sheet.components).map(|component| component.reference.clone()).collect();
        let mut number = 1u64;
        let reference = loop {
            let candidate = format!("{prefix}{number}");
            if !used.contains(&candidate) {
                break candidate;
            }
            number += 1;
        };
        let x = source.x + request.dx;
        let y = source.y + request.dy;
        if !x.is_finite() || !y.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "component coordinates must be finite numbers"));
        }
        let class = design.rules.component_classes.get(&source.id).cloned();
        let sheet = &mut design.sheets[index];
        let mut ids: HashSet<String> = sheet.components.iter().map(|component| component.id.clone()).collect();
        let id = next_id(&mut design.next_component, "component", &mut ids);
        let component = Component {
            id: id.clone(),
            reference: reference.clone(),
            value: source.value,
            footprint: source.footprint,
            x,
            y,
            rotation: source.rotation,
            mirror: source.mirror,
            pins: source.pins,
            pin_types: source.pin_types,
            properties: source.properties,
        };
        sheet.components.push(component.clone());
        if let Some(class) = class {
            design.rules.component_classes.insert(id, class);
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("source".to_string(), json!(request.reference));
        payload.insert("reference".to_string(), json!(reference));
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

/// Updates the editable fields of one placed component.
///
/// A request carries only what it changes and needs at least one of `value`,
/// `footprint`, `x`, `y` or `rotation`; every field it does not name keeps its
/// current value. A footprint is trimmed and an empty footprint clears the
/// assignment, while the value is stored verbatim. Coordinates and rotation
/// must be finite, and rotation is stored in degrees exactly as passed without
/// wrapping (`450` stays `450`). The reference must be placed on the sheet
/// (`object/not-found`). One call adds one revision.
pub fn update_component(handle: u64, request: UpdateComponentRequest) -> Result<Value, ServiceError> {
    for coordinate in request.x.iter().chain(request.y.iter()).chain(request.rotation.iter()) {
        if !coordinate.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "component coordinates must be finite numbers"));
        }
    }
    if request.value.is_none()
        && request.footprint.is_none()
        && request.x.is_none()
        && request.y.is_none()
        && request.rotation.is_none()
    {
        return Err(ServiceError::new(
            "session/argument-invalid",
            "a component update needs `value`, `footprint`, `x`, `y` or `rotation`",
        ));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        if let Some(value) = request.value {
            component.value = value;
        }
        if let Some(footprint) = request.footprint {
            component.footprint = footprint.trim().to_string();
        }
        if let Some(x) = request.x {
            component.x = x;
        }
        if let Some(y) = request.y {
            component.y = y;
        }
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

/// Rotates one placed component by a relative angle.
///
/// The angle is added to the current rotation and the sum is stored exactly as
/// computed, without wrapping (`450` stays `450`), so repeated turns accumulate
/// predictably. A non-finite angle, or an angle that would overflow the stored
/// rotation, is refused (`session/argument-invalid`). A zero angle changes
/// nothing: the revision stays put, no undo step is recorded and the payload
/// reports `changed: false`. The reference must be placed on the sheet
/// (`object/not-found`). One turn that changes the rotation adds exactly one
/// revision.
pub fn rotate_component(handle: u64, request: RotateComponentRequest) -> Result<Value, ServiceError> {
    if !request.angle.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "component rotation must be a finite number"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        let rotation = component.rotation + request.angle;
        if !rotation.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "component rotation must be a finite number"));
        }
        let changed = rotation != component.rotation;
        component.rotation = rotation;
        let component = component.clone();
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("changed".to_string(), json!(changed));
        Ok(payload)
    })
}

/// Mirrors one placed component about an axis of its local frame.
///
/// The mirror is applied before the rotation, in the component's own frame:
/// `x` flips the symbol left-right about its vertical centre line, `y` flips it
/// top-bottom about its horizontal centre line. Requesting the axis the
/// component already mirrors clears the mirror; requesting the other axis
/// replaces it and adds 180 degrees, the exact rotational equivalent of
/// flipping the second axis on top of the first. Every call changes the
/// document, so one call adds exactly one revision. An axis other than `x` or
/// `y` is refused (`session/argument-invalid`), as is a component the sheet
/// does not place (`object/not-found`).
pub fn mirror_component(handle: u64, request: MirrorComponentRequest) -> Result<Value, ServiceError> {
    if request.axis != "x" && request.axis != "y" {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("mirror axis `{}` must be `x` or `y`", request.axis),
        ));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        let (mirror, extra) = match component.mirror.as_deref() {
            Some(current) if current == request.axis => (None, 0.0),
            Some(_) => (None, 180.0),
            None => (Some(request.axis.clone()), 0.0),
        };
        let rotation = component.rotation + extra;
        if !rotation.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "component rotation must be a finite number"));
        }
        component.mirror = mirror;
        component.rotation = rotation;
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component.clone()));
        payload.insert("mirror".to_string(), json!(component.mirror));
        payload.insert("changed".to_string(), json!(true));
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

/// Assigns one custom key/value property of a placed component.
///
/// The key is trimmed and must not be empty or contain whitespace or a control
/// character, so it stays addressable from every surface. The value is stored
/// exactly as passed, including an empty value. Assigning the value a property
/// already has is a no-op and adds no revision.
pub fn set_component_property(handle: u64, request: SetComponentPropertyRequest) -> Result<Value, ServiceError> {
    let key = request.key.trim().to_string();
    if key.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a component property needs a key"));
    }
    if breaks_net_grammar(&key) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("component property key `{key}` contains whitespace or a control character"),
        ));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        let changed = component.properties.insert(key.clone(), request.value.clone()) != Some(request.value.clone());
        let component = component.clone();
        if changed {
            design.revision += 1;
        }
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("key".to_string(), json!(key));
        payload.insert("value".to_string(), json!(request.value));
        Ok(payload)
    })
}

/// Removes one custom property from a placed component.
///
/// A key the component does not carry is reported as `object/not-found`.
pub fn remove_component_property(handle: u64, request: RemoveComponentPropertyRequest) -> Result<Value, ServiceError> {
    let key = request.key.trim().to_string();
    if key.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a component property needs a key"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let component =
            sheet.components.iter_mut().find(|component| component.reference == request.reference).ok_or_else(|| {
                not_found(format!("no component with reference `{}` is placed on this sheet", request.reference))
            })?;
        if component.properties.remove(&key).is_none() {
            return Err(not_found(format!("component `{}` has no property `{key}`", request.reference)));
        }
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("key".to_string(), json!(key));
        Ok(payload)
    })
}

/// Lists the custom properties of one placed component, sorted by key.
pub fn list_component_properties(handle: u64, reference: &str, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let target = &design.sheets[index];
        let component = target
            .components
            .iter()
            .find(|component| component.reference == reference)
            .ok_or_else(|| not_found(format!("no component with reference `{reference}` is placed on this sheet")))?;
        let properties =
            component.properties.iter().map(|(key, value)| json!({ "key": key, "value": value })).collect::<Vec<_>>();
        Ok(json!({
            "revision": design.revision,
            "sheetId": target.id,
            "componentId": component.id,
            "reference": component.reference,
            "properties": properties,
        }))
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

/// Adds one pin to a placed component.
///
/// The name is trimmed and must not be empty or contain whitespace, a control
/// character or a dot, because a `REF.PIN` node cannot carry it. A pin the
/// component already declares is refused as `pin/duplicate`.
pub fn add_pin(handle: u64, request: AddPinRequest) -> Result<Value, ServiceError> {
    let pin = request.pin.trim().to_string();
    if pin.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a pin name must not be empty"));
    }
    if breaks_node_grammar(&pin) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("pin name `{pin}` contains whitespace, a control character or a dot; `REF.PIN` nodes cannot carry it"),
        ));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let position =
            design.sheets[index].components.iter().position(|component| component.reference == request.reference).ok_or_else(
                || not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)),
            )?;
        let component = &mut design.sheets[index].components[position];
        if component.pins.iter().any(|candidate| candidate == &pin) {
            return Err(ServiceError::new(
                "pin/duplicate",
                format!("component `{}` already declares pin `{pin}`", request.reference),
            ));
        }
        component.pins.push(pin.clone());
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("pin".to_string(), json!(pin));
        Ok(payload)
    })
}

/// Removes one pin from a placed component.
///
/// A pin any net node or no-connect flag still references is refused as
/// `pin/in-use`; the message names the net when a net node holds it. A pin the
/// component does not declare is `pin/not-found`. Removing a pin also drops its
/// electrical role, so the document never types a pin it no longer declares.
pub fn remove_pin(handle: u64, request: RemovePinRequest) -> Result<Value, ServiceError> {
    let pin = request.pin.trim().to_string();
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let position =
            design.sheets[index].components.iter().position(|component| component.reference == request.reference).ok_or_else(
                || not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)),
            )?;
        if !design.sheets[index].components[position].pins.iter().any(|candidate| candidate == &pin) {
            return Err(ServiceError::new(
                "pin/not-found",
                format!("component `{}` declares no pin `{pin}`", request.reference),
            ));
        }
        for sheet in &design.sheets {
            for net in &sheet.nets {
                if net.nodes.iter().any(|node| node.reference == request.reference && node.pin == pin) {
                    return Err(ServiceError::new(
                        "pin/in-use",
                        format!("pin `{}.{pin}` is still used by net `{}`", request.reference, net.name),
                    ));
                }
            }
            for flag in &sheet.no_connects {
                if flag.node.as_ref().is_some_and(|node| node.reference == request.reference && node.pin == pin) {
                    return Err(ServiceError::new(
                        "pin/in-use",
                        format!("pin `{}.{pin}` is still used by a no-connect flag", request.reference),
                    ));
                }
            }
        }
        let component = &mut design.sheets[index].components[position];
        component.pins.retain(|candidate| candidate != &pin);
        component.pin_types.remove(&pin);
        let component = component.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("deleted".to_string(), json!(pin));
        Ok(payload)
    })
}

/// Renames one pin and every `REF.PIN` reference that named it.
///
/// The new name is trimmed and validated like [`add_pin`]. A name the component
/// already declares is `pin/duplicate`, and a `from` the component does not
/// declare is `pin/not-found`. The rename rewrites the pin list, the electrical
/// role, every net node and every no-connect node that referenced the old name,
/// so connectivity survives. Renaming a pin to itself is a no-op and adds no
/// revision.
pub fn rename_pin(handle: u64, request: RenamePinRequest) -> Result<Value, ServiceError> {
    let from = request.from.trim().to_string();
    let to = request.to.trim().to_string();
    if to.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a pin name must not be empty"));
    }
    if breaks_node_grammar(&to) {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("pin name `{to}` contains whitespace, a control character or a dot; `REF.PIN` nodes cannot carry it"),
        ));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let position =
            design.sheets[index].components.iter().position(|component| component.reference == request.reference).ok_or_else(
                || not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)),
            )?;
        let component = &mut design.sheets[index].components[position];
        let pin_index = component.pins.iter().position(|candidate| candidate == &from).ok_or_else(|| {
            ServiceError::new("pin/not-found", format!("component `{}` declares no pin `{from}`", request.reference))
        })?;
        if from == to {
            let component = component.clone();
            let mut payload = Map::new();
            payload.insert("component".to_string(), json!(component));
            payload.insert("from".to_string(), json!(from));
            payload.insert("to".to_string(), json!(to));
            return Ok(payload);
        }
        if component.pins.iter().any(|candidate| candidate == &to) {
            return Err(ServiceError::new(
                "pin/duplicate",
                format!("component `{}` already declares pin `{to}`", request.reference),
            ));
        }
        component.pins[pin_index] = to.clone();
        if let Some(pin_type) = component.pin_types.remove(&from) {
            component.pin_types.insert(to.clone(), pin_type);
        }
        for sheet in &mut design.sheets {
            for net in &mut sheet.nets {
                for node in &mut net.nodes {
                    if node.reference == request.reference && node.pin == from {
                        node.pin = to.clone();
                    }
                }
            }
            for flag in &mut sheet.no_connects {
                if let Some(node) = &mut flag.node
                    && node.reference == request.reference
                    && node.pin == from
                {
                    node.pin = to.clone();
                }
            }
        }
        let component = design.sheets[index].components[position].clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("from".to_string(), json!(from));
        payload.insert("to".to_string(), json!(to));
        Ok(payload)
    })
}

/// Swaps the net connections of two pins of one placed component.
///
/// Each pin takes the other pin's connection: a net node that named `pin_a`
/// is rewritten to `pin_b` and a node that named `pin_b` to `pin_a`, so when
/// one pin is unconnected the connection of the other transfers to it.
/// No-connect flags follow the same swap, so the deliberately-open marking
/// moves with the connection state and the `erc` check keeps its verdict on
/// the right pin. The component must be placed on the target sheet, both
/// pins must be declared by it (`pin/not-found`) and the names must differ
/// (`session/argument-invalid`). A swap where neither pin carries a net node
/// or a no-connect flag is a no-op and adds no revision.
pub fn swap_pins(handle: u64, request: SwapPinsRequest) -> Result<Value, ServiceError> {
    let pin_a = request.pin_a.trim().to_string();
    let pin_b = request.pin_b.trim().to_string();
    if pin_a.is_empty() || pin_b.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "pin names must not be empty"));
    }
    if pin_a == pin_b {
        return Err(ServiceError::new("session/argument-invalid", "a pin cannot be swapped with itself"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let position =
            design.sheets[index].components.iter().position(|component| component.reference == request.reference).ok_or_else(
                || not_found(format!("no component with reference `{}` is placed on this sheet", request.reference)),
            )?;
        let component = &design.sheets[index].components[position];
        if !component.pins.contains(&pin_a) {
            return Err(ServiceError::new(
                "pin/not-found",
                format!("component `{}` declares no pin `{pin_a}`", request.reference),
            ));
        }
        if !component.pins.contains(&pin_b) {
            return Err(ServiceError::new(
                "pin/not-found",
                format!("component `{}` declares no pin `{pin_b}`", request.reference),
            ));
        }
        let mut changed = false;
        for sheet in &mut design.sheets {
            for net in &mut sheet.nets {
                for node in &mut net.nodes {
                    if node.reference != request.reference {
                        continue;
                    }
                    if node.pin == pin_a {
                        node.pin = pin_b.clone();
                        changed = true;
                    }
                    else if node.pin == pin_b {
                        node.pin = pin_a.clone();
                        changed = true;
                    }
                }
            }
            for flag in &mut sheet.no_connects {
                if let Some(node) = &mut flag.node
                    && node.reference == request.reference
                {
                    if node.pin == pin_a {
                        node.pin = pin_b.clone();
                        changed = true;
                    }
                    else if node.pin == pin_b {
                        node.pin = pin_a.clone();
                        changed = true;
                    }
                }
            }
        }
        if changed {
            design.revision += 1;
        }
        let component = design.sheets[index].components[position].clone();
        let mut payload = Map::new();
        payload.insert("component".to_string(), json!(component));
        payload.insert("pinA".to_string(), json!(pin_a));
        payload.insert("pinB".to_string(), json!(pin_b));
        payload.insert("changed".to_string(), json!(changed));
        Ok(payload)
    })
}

/// Renumbers every component reference designator deterministically.
///
/// Components are grouped by the letters at the start of their reference. Each
/// group is visited sheet by sheet, then top to bottom (`y`) and left to right
/// (`x`), and numbered from 1, so a designator becomes `<prefix><n>`. Net nodes,
/// no-connect pin attachments and BOM variants follow their component, and a
/// component-class assignment is keyed by the stable component id so it stays
/// attached.
///
/// A reference without leading letters is skipped and reported under `skipped`
/// instead of being guessed into a prefix. The whole call is one revision: when
/// every designator already matches its number the revision and undo history
/// stay untouched and `count`, `changed` and `annotated` are zero.
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
        let mut skipped = Vec::new();
        for (sheet_index, component_index) in targets {
            let component = &design.sheets[sheet_index].components[component_index];
            let prefix = designator_prefix(&component.reference);
            if prefix.is_empty() {
                skipped.push(json!({
                    "sheetId": design.sheets[sheet_index].id,
                    "componentId": component.id,
                    "reference": component.reference,
                }));
                continue;
            }
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
            for no_connect in &mut sheet.no_connects {
                if let Some(node) = &mut no_connect.node
                    && let Some(next) = renames.get(&node.reference)
                {
                    node.reference = next.clone();
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
        let changed = changes.len();
        let mut payload = Map::new();
        payload.insert("count".to_string(), json!(changed));
        payload.insert("changed".to_string(), json!(changed));
        payload.insert("annotated".to_string(), json!(changed));
        payload.insert("changes".to_string(), json!(changes));
        payload.insert("skipped".to_string(), json!(skipped));
        Ok(payload)
    })
}

/// ASCII letters at the start of a reference designator, or an empty string when
/// the reference does not start with a letter.
fn designator_prefix(reference: &str) -> String {
    reference.chars().take_while(|character| character.is_ascii_alphabetic()).collect()
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

/// Renames a declared net by its old name across every sheet of the design.
///
/// Every declaration that carries the old name follows the rename and keeps its
/// nodes, so the net's `REF.PIN` nodes move with it. Wires that named the old
/// net, net labels whose text is the old name, net-class `nets` assignments and
/// bus `aliases` that spelled the old name are rewritten in document order.
/// A name another net already declares is refused (`object/duplicate`), a net
/// absent from the target sheet is `net/not-found` and an empty name is
/// `session/argument-invalid`. A rename that changes the name adds exactly one
/// revision; a request whose new name already matches reports `changed: false`
/// and adds none.
pub fn rename_net(handle: u64, request: RenameNetRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a net name must not be empty"));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, request.sheet.as_deref())?;
        let target = design.sheets[sheet_position]
            .nets
            .iter()
            .position(|net| net.id == request.net || net.name == request.net)
            .ok_or_else(|| ServiceError::new("net/not-found", format!("no net `{}` is declared on this sheet", request.net)))?;
        let previous = design.sheets[sheet_position].nets[target].name.clone();
        let changed = previous != name;
        if changed && design.sheets.iter().any(|sheet| sheet.nets.iter().any(|net| net.name == name)) {
            return Err(duplicate(format!("net name `{name}` already exists in this design")));
        }
        if changed {
            for sheet in &mut design.sheets {
                for net in &mut sheet.nets {
                    if net.name == previous {
                        net.name = name.clone();
                    }
                }
                for wire in &mut sheet.wires {
                    if wire.net.as_deref() == Some(previous.as_str()) {
                        wire.net = Some(name.clone());
                    }
                }
                for label in &mut sheet.labels {
                    if label.text == previous {
                        label.text = name.clone();
                    }
                }
            }
            for net_class in &mut design.rules.net_classes {
                if net_class.nets.iter().any(|entry| entry == &previous) {
                    net_class.nets.retain(|entry| entry != &previous && entry != &name);
                    net_class.nets.push(name.clone());
                }
            }
            for bus in &mut design.buses {
                for alias in &mut bus.aliases {
                    if alias == &previous {
                        *alias = name.clone();
                    }
                }
            }
            design.revision += 1;
        }
        let net = design.sheets[sheet_position].nets[target].clone();
        let mut payload = Map::new();
        payload.insert("net".to_string(), json!(net));
        payload.insert("previousName".to_string(), json!(previous));
        payload.insert("changed".to_string(), json!(changed));
        Ok(payload)
    })
}

/// Merges one net into another on the same sheet.
///
/// The target keeps its identity, name and nodes. Every node the source reaches
/// is appended to the target unless it is already there, wires that named the
/// source are repointed to the target, net-class assignments that listed the
/// source follow the target, and the source net is removed. Merging a net into
/// itself is rejected. The payload reports the surviving `net`, the absorbed
/// `source` name and the counts of `movedNodes` and `repointedWires`.
pub fn merge_nets(handle: u64, request: MergeNetsRequest) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[sheet_position];
        let target = sheet
            .nets
            .iter()
            .position(|net| net.id == request.target || net.name == request.target)
            .ok_or_else(|| not_found(format!("no net `{}` is declared on this sheet", request.target)))?;
        let source = sheet
            .nets
            .iter()
            .position(|net| net.id == request.source || net.name == request.source)
            .ok_or_else(|| not_found(format!("no net `{}` is declared on this sheet", request.source)))?;
        if target == source {
            return Err(ServiceError::new("session/argument-invalid", "a net cannot be merged into itself"));
        }
        let source_name = sheet.nets[source].name.clone();
        let target_name = sheet.nets[target].name.clone();
        let source_nodes = sheet.nets[source].nodes.clone();
        let mut moved_nodes = 0usize;
        for node in source_nodes {
            if !sheet.nets[target].nodes.contains(&node) {
                sheet.nets[target].nodes.push(node);
                moved_nodes += 1;
            }
        }
        let mut repointed_wires = 0usize;
        if source_name != target_name {
            for wire in &mut sheet.wires {
                if wire.net.as_deref() == Some(source_name.as_str()) {
                    wire.net = Some(target_name.clone());
                    repointed_wires += 1;
                }
            }
        }
        let merged = sheet.nets[target].clone();
        sheet.nets.remove(source);
        for net_class in &mut design.rules.net_classes {
            if net_class.nets.iter().any(|name| name == &source_name) {
                net_class.nets.retain(|name| name != &source_name && name != &target_name);
                net_class.nets.push(target_name.clone());
            }
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("net".to_string(), json!(merged));
        payload.insert("source".to_string(), json!(source_name));
        payload.insert("movedNodes".to_string(), json!(moved_nodes));
        payload.insert("repointedWires".to_string(), json!(repointed_wires));
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
/// already exist (`no-connect/pin-missing`), and one pin can only carry one
/// flag (`no-connect/duplicate`).
pub fn add_no_connect(handle: u64, request: AddNoConnectRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "no-connect coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &design.sheets[index];
        if let Some(node) = &request.node {
            let component =
                sheet.components.iter().find(|component| component.reference == node.reference).ok_or_else(|| {
                    ServiceError::new(
                        "no-connect/pin-missing",
                        format!("no component `{}` is placed on this sheet", node.reference),
                    )
                })?;
            if !component.pins.iter().any(|pin| pin == &node.pin) {
                return Err(ServiceError::new(
                    "no-connect/pin-missing",
                    format!("component `{}` declares no pin `{}`", node.reference, node.pin),
                ));
            }
            if sheet.no_connects.iter().any(|flag| flag.node.as_ref() == Some(node)) {
                return Err(ServiceError::new(
                    "no-connect/duplicate",
                    format!("pin `{}.{}` already carries a no-connect flag", node.reference, node.pin),
                ));
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
///
/// An out-of-range value is reported as `no-connect/not-found`.
pub fn delete_no_connect(handle: u64, index: usize, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[position];
        if index >= target.no_connects.len() {
            return Err(ServiceError::new(
                "no-connect/not-found",
                format!("no no-connect flag exists at index {index} on this sheet"),
            ));
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
/// value is reported as `junction/not-found`.
pub fn delete_junction(handle: u64, index: usize, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        if index >= target.junctions.len() {
            return Err(ServiceError::new("junction/not-found", format!("no junction at index {index} on this sheet")));
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
/// junction list (`junction/not-found`). The destination must be free, so
/// moving onto another junction is refused as `junction/duplicate`.
pub fn move_junction(handle: u64, index: usize, sheet: Option<&str>, x: f64, y: f64) -> Result<Value, ServiceError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "junction coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        if index >= target.junctions.len() {
            return Err(ServiceError::new("junction/not-found", format!("no junction at index {index} on this sheet")));
        }
        if target
            .junctions
            .iter()
            .enumerate()
            .any(|(position, junction)| position != index && junction.x == x && junction.y == y)
        {
            return Err(ServiceError::new(
                "junction/duplicate",
                format!("a junction already exists at ({x}, {y}) on this sheet"),
            ));
        }
        let junction = &mut target.junctions[index];
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

/// Lists the free text annotations of one sheet, in 0-based index order.
pub fn list_texts(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let target = &design.sheets[index];
        let texts = target
            .texts
            .iter()
            .enumerate()
            .map(|(index, text)| json!({ "index": index, "text": text.text, "x": text.x, "y": text.y }))
            .collect::<Vec<_>>();
        Ok(json!({
            "revision": design.revision,
            "sheetId": target.id,
            "texts": texts,
        }))
    })
}

/// Updates the content or placement of the text annotation at `index` (0-based).
///
/// A request carries only what it changes and needs at least one of `text`, `x`
/// or `y`. New text is trimmed for the empty check and stored verbatim; a blank
/// value is rejected with the same argument code as [`add_text`]. Coordinates
/// must be finite. The index must be inside the sheet's text list.
pub fn update_text(handle: u64, request: UpdateTextRequest) -> Result<Value, ServiceError> {
    if let Some(text) = &request.text
        && text.trim().is_empty()
    {
        return Err(ServiceError::new("session/argument-invalid", "a text annotation must not be empty"));
    }
    for coordinate in request.x.iter().chain(request.y.iter()) {
        if !coordinate.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "text coordinates must be finite numbers"));
        }
    }
    if request.text.is_none() && request.x.is_none() && request.y.is_none() {
        return Err(ServiceError::new("session/argument-invalid", "a text update needs `text`, `x` or `y`"));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, request.sheet.as_deref())?;
        let target = &mut design.sheets[sheet_position];
        let Some(text) = target.texts.get_mut(request.index)
        else {
            return Err(not_found(format!("no text at index {} on this sheet", request.index)));
        };
        if let Some(content) = request.text {
            text.text = content;
        }
        if let Some(x) = request.x {
            text.x = x;
        }
        if let Some(y) = request.y {
            text.y = y;
        }
        let updated = text.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("index".to_string(), json!(request.index));
        payload.insert("text".to_string(), json!(updated));
        Ok(payload)
    })
}

/// Adds one net label to a sheet.
///
/// The label text is trimmed for the empty check and stored verbatim. Scope
/// defaults to `local`, rotation to zero and justification to `left`. A label
/// is an annotation: adding it never creates, renames or merges a net.
pub fn add_label(handle: u64, request: AddLabelRequest) -> Result<Value, ServiceError> {
    if request.text.trim().is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a net label must not be empty"));
    }
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "label coordinates must be finite numbers"));
    }
    let rotation = request.rotation.unwrap_or(0.0);
    if !rotation.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "label rotation must be a finite number"));
    }
    let justify = request.justify.unwrap_or_else(default_justify);
    if !valid_justify(&justify) {
        return Err(ServiceError::new("session/argument-invalid", "label justification must be left, right or center"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let id = free_label_id(design);
        let label = NetLabel {
            id,
            text: request.text,
            x: request.x,
            y: request.y,
            scope: request.scope.unwrap_or(LabelScope::Local),
            rotation,
            justify,
        };
        design.sheets[index].labels.push(label.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("label".to_string(), json!(label));
        Ok(payload)
    })
}

/// Lists the net labels of one sheet in document order.
pub fn list_labels(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let target = &design.sheets[index];
        Ok(json!({
            "revision": design.revision,
            "sheetId": target.id,
            "labels": target.labels,
        }))
    })
}

/// Updates the text, placement, scope or style of one net label.
///
/// A request carries only what it changes and needs at least one field besides
/// the identity. A text is trimmed for the empty check and stored verbatim, and
/// a justification must be `left`, `right` or `center`.
pub fn update_label(handle: u64, request: UpdateLabelRequest) -> Result<Value, ServiceError> {
    if let Some(text) = &request.text
        && text.trim().is_empty()
    {
        return Err(ServiceError::new("session/argument-invalid", "a net label must not be empty"));
    }
    for coordinate in request.x.iter().chain(request.y.iter()).chain(request.rotation.iter()) {
        if !coordinate.is_finite() {
            return Err(ServiceError::new("session/argument-invalid", "label coordinates must be finite numbers"));
        }
    }
    if let Some(justify) = &request.justify
        && !valid_justify(justify)
    {
        return Err(ServiceError::new("session/argument-invalid", "label justification must be left, right or center"));
    }
    if request.text.is_none()
        && request.x.is_none()
        && request.y.is_none()
        && request.scope.is_none()
        && request.rotation.is_none()
        && request.justify.is_none()
    {
        return Err(ServiceError::new(
            "session/argument-invalid",
            "a label update needs `text`, `x`, `y`, `scope`, `rotation` or `justify`",
        ));
    }
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, request.sheet.as_deref())?;
        let target = &mut design.sheets[sheet_position];
        let Some(label) = target.labels.iter_mut().find(|label| label.id == request.id)
        else {
            return Err(not_found(format!("no label `{}` exists on this sheet", request.id)));
        };
        if let Some(text) = request.text {
            label.text = text;
        }
        if let Some(x) = request.x {
            label.x = x;
        }
        if let Some(y) = request.y {
            label.y = y;
        }
        if let Some(scope) = request.scope {
            label.scope = scope;
        }
        if let Some(rotation) = request.rotation {
            label.rotation = rotation;
        }
        if let Some(justify) = request.justify {
            label.justify = justify;
        }
        let updated = label.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("label".to_string(), json!(updated));
        Ok(payload)
    })
}

/// Deletes one net label by identity.
pub fn delete_label(handle: u64, id: &str, sheet: Option<&str>) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let sheet_position = sheet_index(design, sheet)?;
        let target = &mut design.sheets[sheet_position];
        let Some(position) = target.labels.iter().position(|label| label.id == id)
        else {
            return Err(not_found(format!("no label `{id}` exists on this sheet")));
        };
        let removed = target.labels.remove(position);
        let remaining = target.labels.len();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("label".to_string(), json!(removed));
        payload.insert("remaining".to_string(), json!(remaining));
        Ok(payload)
    })
}

/// Lists the junctions of one sheet in document order.
pub fn list_junctions(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let target = &design.sheets[index];
        let junctions = target
            .junctions
            .iter()
            .enumerate()
            .map(|(position, junction)| json!({ "index": position, "x": junction.x, "y": junction.y }))
            .collect::<Vec<_>>();
        Ok(json!({
            "revision": design.revision,
            "sheetId": target.id,
            "junctions": junctions,
        }))
    })
}

/// Lists the no-connect flags of one sheet in document order.
pub fn list_no_connects(handle: u64, sheet: Option<&str>) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let index = sheet_index(design, sheet)?;
        let target = &design.sheets[index];
        let flags = target
            .no_connects
            .iter()
            .enumerate()
            .map(|(position, flag)| {
                let mut entry = Map::new();
                entry.insert("index".to_string(), json!(position));
                entry.insert("x".to_string(), json!(flag.x));
                entry.insert("y".to_string(), json!(flag.y));
                if let Some(node) = &flag.node {
                    entry.insert("node".to_string(), json!(node));
                }
                Value::Object(entry)
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "revision": design.revision,
            "sheetId": target.id,
            "noConnects": flags,
        }))
    })
}

/// Lists every sheet with its identity, name and object counts.
pub fn list_sheets(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let report = stats_of(design);
        Ok(json!({
            "revision": design.revision,
            "sheets": report["sheets"].clone(),
        }))
    })
}

/// Whether a label justification is one of the supported horizontal values.
fn valid_justify(justify: &str) -> bool {
    matches!(justify, "left" | "right" | "center")
}

/// Stable spelling of one label scope, used by reports and writers.
pub(crate) fn label_scope_name(scope: LabelScope) -> &'static str {
    match scope {
        LabelScope::Local => "local",
        LabelScope::Global => "global",
        LabelScope::Hierarchical => "hierarchical",
    }
}

/// Picks a free `label-N` identity across every sheet of one design.
fn free_label_id(design: &Design) -> String {
    let used: HashSet<&str> =
        design.sheets.iter().flat_map(|sheet| sheet.labels.iter().map(|label| label.id.as_str())).collect();
    let mut counter = used.len() + 1;
    loop {
        let candidate = format!("label-{counter}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
        counter += 1;
    }
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

/// Splits one wire segment into two at a point on the segment.
///
/// The original segment keeps its identity and now ends at the split point; a
/// new segment starts there and keeps the old endpoint and net assignment. The
/// point must lie on the segment and strictly between its endpoints, so a split
/// always yields two non-zero-length segments. An unknown wire id is
/// `wire/not-found`; a point off the segment, on an endpoint or on a
/// zero-length wire is `wire/split-point-not-interior`, and a non-finite
/// coordinate is `session/argument-invalid`. The payload reports both segments
/// as `first` (the original id) and `second` (the new id).
pub fn split_wire(handle: u64, request: SplitWireRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "wire coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let position =
            sheet.wires.iter().position(|wire| wire.id == request.id).ok_or_else(|| {
                ServiceError::new("wire/not-found", format!("no wire `{}` is drawn on this sheet", request.id))
            })?;
        let original = sheet.wires[position].clone();
        let [x0, y0] = original.from;
        let [x1, y1] = original.to;
        let delta_x = x1 - x0;
        let delta_y = y1 - y0;
        let length_squared = delta_x * delta_x + delta_y * delta_y;
        if length_squared == 0.0 {
            return Err(ServiceError::new(
                "wire/split-point-not-interior",
                "a zero-length wire has no interior point to split at",
            ));
        }
        let length = length_squared.sqrt();
        let along = ((request.x - x0) * delta_x + (request.y - y0) * delta_y) / length_squared;
        let perpendicular = (((request.x - x0) * delta_y - (request.y - y0) * delta_x) / length).abs();
        let tolerance = 1.0e-9 * length.max(1.0);
        if perpendicular > tolerance || along <= 1.0e-9 || along >= 1.0 - 1.0e-9 {
            return Err(ServiceError::new(
                "wire/split-point-not-interior",
                "the split point must lie on the segment and strictly between its endpoints",
            ));
        }
        let point = [request.x, request.y];
        let end = original.to;
        let mut used: HashSet<String> = sheet.wires.iter().map(|wire| wire.id.clone()).collect();
        let second_id = next_id(&mut design.next_wire, "wire", &mut used);
        sheet.wires[position].to = point;
        let second = Wire { id: second_id, net: original.net, from: point, to: end };
        sheet.wires.push(second.clone());
        let first = sheet.wires[position].clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("first".to_string(), json!(first));
        payload.insert("second".to_string(), json!(second));
        Ok(payload)
    })
}

/// Merges two touching, collinear wires of the same net into one segment.
///
/// Both wires are addressed by their 0-based position in the `sheet` wire
/// list, so they always belong to the same sheet. They must name the same net;
/// an absent, empty or whitespace-only name counts as net-less, matching
/// [`update_wire`], which clears an empty assignment. They must share exactly
/// one endpoint within a `1e-9` tolerance scaled by the longer segment, and
/// must extend in opposite directions from that shared endpoint, so the
/// shared point is not left in the middle of the merged wire.
///
/// The first wire keeps its identity, order and net and spans the two far
/// endpoints; the second wire is removed. A junction at the shared point is
/// refused as `wire/merge-junction`, because it would sit inside the merged
/// segment. A different net, a missing shared endpoint, overlapping or
/// non-collinear segments are `wire/merge-mismatch`, and a result that would
/// have zero length is `session/argument-invalid`. The payload reports the
/// surviving `wire` and the `removed` wire id.
pub fn merge_wires(handle: u64, sheet: &str, first: usize, second: usize) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = sheet_index(design, Some(sheet))?;
        let target = &mut design.sheets[index];
        let first_wire =
            target.wires.get(first).cloned().ok_or_else(|| not_found(format!("no wire at index {first} on this sheet")))?;
        let second_wire =
            target.wires.get(second).cloned().ok_or_else(|| not_found(format!("no wire at index {second} on this sheet")))?;
        if first == second {
            return Err(ServiceError::new("wire/merge-mismatch", "a wire cannot be merged with itself"));
        }
        if wire_net_name(first_wire.net.as_deref()) != wire_net_name(second_wire.net.as_deref()) {
            return Err(ServiceError::new("wire/merge-mismatch", "the wires must name the same net before they can be merged"));
        }
        let first_length = segment_length(first_wire.from, first_wire.to);
        let second_length = segment_length(second_wire.from, second_wire.to);
        let tolerance = 1.0e-9 * first_length.max(second_length).max(1.0);
        if first_length <= tolerance || second_length <= tolerance {
            return Err(ServiceError::new("session/argument-invalid", "a zero-length wire cannot be merged"));
        }
        let first_ends = [first_wire.from, first_wire.to];
        let second_ends = [second_wire.from, second_wire.to];
        let mut touches = 0;
        let mut shared = None;
        for (first_end, first_point) in first_ends.iter().enumerate() {
            for (second_end, second_point) in second_ends.iter().enumerate() {
                if points_within(*first_point, *second_point, tolerance) {
                    touches += 1;
                    shared = Some((first_end, second_end, *first_point));
                }
            }
        }
        let (first_end, second_end, shared_point) = match (touches, shared) {
            (1, Some(shared)) => shared,
            (0, _) => {
                return Err(ServiceError::new("wire/merge-mismatch", "the wires do not share an endpoint"));
            }
            _ => {
                return Err(ServiceError::new("wire/merge-mismatch", "the wires overlap or share both endpoints"));
            }
        };
        if target.junctions.iter().any(|junction| points_within([junction.x, junction.y], shared_point, tolerance)) {
            return Err(ServiceError::new(
                "wire/merge-junction",
                format!("a junction at ({}, {}) would sit inside the merged wire", shared_point[0], shared_point[1]),
            ));
        }
        let first_far = if first_end == 0 { first_wire.to } else { first_wire.from };
        let second_far = if second_end == 0 { second_wire.to } else { second_wire.from };
        let first_direction = unit_direction(first_far, shared_point);
        let second_direction = unit_direction(second_far, shared_point);
        let cross = first_direction[0] * second_direction[1] - first_direction[1] * second_direction[0];
        let dot = first_direction[0] * second_direction[0] + first_direction[1] * second_direction[1];
        if cross.abs() > 1.0e-9 || dot >= 0.0 {
            return Err(ServiceError::new(
                "wire/merge-mismatch",
                "the wires must be collinear and extend in opposite directions from the shared endpoint",
            ));
        }
        let merged = Wire {
            id: first_wire.id.clone(),
            net: first_wire.net.clone(),
            from: if first_end == 0 { second_far } else { first_wire.from },
            to: if first_end == 1 { second_far } else { first_wire.to },
        };
        if segment_length(merged.from, merged.to) <= tolerance {
            return Err(ServiceError::new("session/argument-invalid", "the merged wire would be zero length"));
        }
        target.wires.remove(second);
        let first_position = if second < first { first - 1 } else { first };
        target.wires[first_position] = merged.clone();
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("wire".to_string(), json!(merged));
        payload.insert("removed".to_string(), json!(second_wire.id));
        Ok(payload)
    })
}

/// Translates the selected components and wires of one sheet by an offset.
///
/// Only the listed objects move; unlisted components, wires, junctions, text
/// and no-connect flags keep their placement, and a component keeps its
/// reference, value, rotation and pins. Every reference and wire id must exist
/// on the sheet before anything moves, so a request is all-or-nothing. A request
/// that names neither kind, or whose offset is zero on both axes, is rejected
/// instead of adding a revision.
pub fn translate_objects(handle: u64, request: TranslateRequest) -> Result<Value, ServiceError> {
    if !request.dx.is_finite() || !request.dy.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "the translate offset must be finite numbers"));
    }
    if request.references.is_empty() && request.wires.is_empty() {
        return Err(ServiceError::new(
            "session/argument-invalid",
            "a translate request needs at least one component reference or wire id",
        ));
    }
    if request.dx == 0.0 && request.dy == 0.0 {
        return Err(ServiceError::new("session/argument-invalid", "the translate offset must move at least one axis"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        let sheet = &mut design.sheets[index];
        let mut components = BTreeSet::new();
        for reference in &request.references {
            let position = sheet
                .components
                .iter()
                .position(|component| component.reference == *reference)
                .ok_or_else(|| not_found(format!("no component with reference `{reference}` is placed on this sheet")))?;
            components.insert(position);
        }
        let mut wires = BTreeSet::new();
        for wire in &request.wires {
            let position = sheet
                .wires
                .iter()
                .position(|candidate| candidate.id == *wire)
                .ok_or_else(|| not_found(format!("no wire `{wire}` is drawn on this sheet")))?;
            wires.insert(position);
        }
        for position in &components {
            sheet.components[*position].x += request.dx;
            sheet.components[*position].y += request.dy;
        }
        for position in &wires {
            sheet.wires[*position].from[0] += request.dx;
            sheet.wires[*position].from[1] += request.dy;
            sheet.wires[*position].to[0] += request.dx;
            sheet.wires[*position].to[1] += request.dy;
        }
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("components".to_string(), json!(components.len()));
        payload.insert("wires".to_string(), json!(wires.len()));
        payload.insert("dx".to_string(), json!(request.dx));
        payload.insert("dy".to_string(), json!(request.dy));
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
///
/// A junction is identified by its placement, so a second junction at the same
/// `(x, y)` is refused as `junction/duplicate` instead of stacking two dots.
pub fn add_junction(handle: u64, request: AddJunctionRequest) -> Result<Value, ServiceError> {
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "junction coordinates must be finite numbers"));
    }
    mutate(handle, |design| {
        let index = sheet_index(design, request.sheet.as_deref())?;
        if design.sheets[index].junctions.iter().any(|junction| junction.x == request.x && junction.y == request.y) {
            return Err(ServiceError::new(
                "junction/duplicate",
                format!("a junction already exists at ({}, {}) on this sheet", request.x, request.y),
            ));
        }
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

/// Adds one hierarchical sheet symbol to the parent schematic.
///
/// The name and the child file are trimmed and must not be empty, and the name
/// must be unique across the design. The size defaults to 60 by 40 design units
/// and must be positive and finite, so the document stays valid right after the
/// edit. Editing never reads or modifies the referenced child file; only
/// [`check_hierarchy`] opens it.
pub fn add_hierarchy(handle: u64, request: AddHierarchyRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a hierarchical sheet needs a name"));
    }
    let file = request.file.trim().to_string();
    if file.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a hierarchical sheet needs a child file name"));
    }
    if !request.x.is_finite() || !request.y.is_finite() || !request.width.is_finite() || !request.height.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "hierarchical sheet geometry must be finite"));
    }
    if request.width <= 0.0 || request.height <= 0.0 {
        return Err(ServiceError::new("session/argument-invalid", "a hierarchical sheet needs a positive size"));
    }
    mutate(handle, |design| {
        if design.hierarchy.iter().any(|sheet| sheet.name == name) {
            return Err(duplicate(format!("hierarchical sheet name `{name}` already exists")));
        }
        let symbol = HierarchicalSheet {
            id: free_hierarchy_id(design),
            name,
            file,
            x: request.x,
            y: request.y,
            width: request.width,
            height: request.height,
            ports: Vec::new(),
        };
        design.hierarchy.push(symbol.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("hierarchy".to_string(), json!(symbol));
        Ok(payload)
    })
}

/// Lists the hierarchical sheet symbols declared by one open design.
pub fn list_hierarchy(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        Ok(json!({
            "revision": design.revision,
            "design": summary_of(session),
            "count": design.hierarchy.len(),
            "hierarchy": design.hierarchy,
        }))
    })
}

/// Checks the declared hierarchy against the child documents on disk.
///
/// The check only opens children whose file extension is `.oeda`; any other
/// child is reported as `hierarchy/child-format-unsupported` and left alone.
/// A missing child is `hierarchy/file-missing`, a child that cannot be read or
/// parsed is `hierarchy/child-invalid`, and a normalised path that is already
/// in the current chain is `hierarchy/cycle`. The walk stops at that branch so
/// no file is visited twice in one chain, and stops below
/// [`HIERARCHY_MAX_DEPTH`] with `hierarchy/depth-exceeded`.
///
/// Ports of each symbol are compared with the hierarchical-scope labels of the
/// child's first sheet: a port without a matching label is
/// `hierarchy/port-without-label` (error), a hierarchical label without a
/// matching port is `hierarchy/label-without-port` (warning), and a port name
/// declared twice is `hierarchy/duplicate-port` (error).
///
/// The report is deterministic and never writes a document. `sheets` counts the
/// child documents that opened successfully and excludes the root, and `passed`
/// is false only when at least one reported issue is an error.
pub fn check_hierarchy(handle: u64) -> Result<Value, ServiceError> {
    let (design, document_path) = with_session(handle, |session| {
        let path = session.path.clone().ok_or_else(|| {
            ServiceError::new(
                "hierarchy/document-path-missing",
                "the open design has no document path; open it from a file or save it before running the hierarchy check",
            )
        })?;
        Ok((session.design.clone(), path))
    })?;
    let root = if document_path.is_file() {
        fs::canonicalize(&document_path).unwrap_or_else(|_| normalize_path(&document_path))
    }
    else {
        normalize_path(&document_path)
    };
    let mut found = HierarchyFindings { issues: Vec::new(), sheets: 0 };
    let mut ancestors = vec![root.clone()];
    walk_hierarchy(&root, &design, 0, &mut ancestors, &mut found);
    let passed = !found.issues.iter().any(|issue| issue["severity"] == "error");
    Ok(json!({
        "format": HIERARCHY_REPORT_FORMAT,
        "version": HIERARCHY_REPORT_VERSION,
        "sheets": found.sheets,
        "issues": found.issues,
        "passed": passed,
    }))
}

/// Accumulated findings of one hierarchy walk.
struct HierarchyFindings {
    issues: Vec<Value>,
    sheets: usize,
}

fn hierarchy_issue(code: &str, severity: &str, sheet: String, detail: String) -> Value {
    json!({ "code": code, "severity": severity, "sheet": sheet, "detail": detail })
}

/// Resolves one child file against the document that declares it.
fn resolve_child(parent: &Path, file: &str) -> PathBuf {
    let joined = parent.parent().unwrap_or_else(|| Path::new("")).join(file);
    if joined.is_file() {
        fs::canonicalize(&joined).unwrap_or_else(|_| normalize_path(&joined))
    }
    else {
        normalize_path(&joined)
    }
}

/// Lexically normalises a path so `.` and `..` cannot hide a repeated file.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            PathComponent::CurDir => {}
            PathComponent::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Walks one document's hierarchy declarations depth-first in document order.
fn walk_hierarchy(
    document_path: &Path,
    design: &Design,
    depth: usize,
    ancestors: &mut Vec<PathBuf>,
    found: &mut HierarchyFindings,
) {
    for symbol in &design.hierarchy {
        let child = resolve_child(document_path, &symbol.file);
        let label = child.display().to_string();
        if ancestors.contains(&child) {
            found.issues.push(hierarchy_issue(
                "hierarchy/cycle",
                "error",
                label,
                format!("`{}` is already open in this hierarchy chain", symbol.file),
            ));
            continue;
        }
        let child_depth = depth + 1;
        if child_depth > HIERARCHY_MAX_DEPTH {
            found.issues.push(hierarchy_issue(
                "hierarchy/depth-exceeded",
                "error",
                label,
                format!("the hierarchy is deeper than {HIERARCHY_MAX_DEPTH} levels"),
            ));
            continue;
        }
        let native = child
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("oeda"));
        if !native {
            found.issues.push(hierarchy_issue(
                "hierarchy/child-format-unsupported",
                "warning",
                label,
                format!("`{}` is not an `.oeda` document, so the check did not open it", symbol.file),
            ));
            continue;
        }
        if !child.is_file() {
            found.issues.push(hierarchy_issue(
                "hierarchy/file-missing",
                "error",
                label,
                format!("`{}` does not exist", symbol.file),
            ));
            continue;
        }
        let child_design = match read_design(&child) {
            Ok(child_design) => child_design,
            Err(error) => {
                found.issues.push(hierarchy_issue("hierarchy/child-invalid", "error", label, error.to_string()));
                continue;
            }
        };
        found.sheets += 1;

        let mut seen = HashSet::new();
        let mut reported = HashSet::new();
        for port in &symbol.ports {
            if !seen.insert(port.name.as_str()) && reported.insert(port.name.as_str()) {
                found.issues.push(hierarchy_issue(
                    "hierarchy/duplicate-port",
                    "error",
                    label.clone(),
                    format!("port `{}` is declared more than once on `{}`", port.name, symbol.name),
                ));
            }
        }
        let root_sheet = child_design.sheets.first();
        let labels: HashSet<&str> = root_sheet
            .map(|sheet| {
                sheet
                    .labels
                    .iter()
                    .filter(|entry| entry.scope == LabelScope::Hierarchical)
                    .map(|entry| entry.text.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let mut checked = HashSet::new();
        for port in &symbol.ports {
            if !checked.insert(port.name.as_str()) {
                continue;
            }
            if !labels.contains(port.name.as_str()) {
                found.issues.push(hierarchy_issue(
                    "hierarchy/port-without-label",
                    "error",
                    label.clone(),
                    format!("port `{}` has no hierarchical label on the child root sheet", port.name),
                ));
            }
        }
        if let Some(sheet) = root_sheet {
            let ports: HashSet<&str> = symbol.ports.iter().map(|port| port.name.as_str()).collect();
            let mut reported_labels = HashSet::new();
            for entry in &sheet.labels {
                if entry.scope != LabelScope::Hierarchical {
                    continue;
                }
                if !ports.contains(entry.text.as_str()) && reported_labels.insert(entry.text.as_str()) {
                    found.issues.push(hierarchy_issue(
                        "hierarchy/label-without-port",
                        "warning",
                        label.clone(),
                        format!("hierarchical label `{}` has no matching port on `{}`", entry.text, symbol.name),
                    ));
                }
            }
        }

        ancestors.push(child.clone());
        walk_hierarchy(&child, &child_design, child_depth, ancestors, found);
        ancestors.pop();
    }
}

/// Removes one hierarchical sheet symbol by id or name.
///
/// The child file is not touched; only the parent declaration is removed.
pub fn remove_hierarchy(handle: u64, sheet: &str) -> Result<Value, ServiceError> {
    mutate(handle, |design| {
        let index = design
            .hierarchy
            .iter()
            .position(|entry| entry.id == sheet || entry.name == sheet)
            .ok_or_else(|| not_found(format!("no hierarchical sheet with id or name `{sheet}` is declared")))?;
        let removed = design.hierarchy.remove(index);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("removed".to_string(), json!(removed.id));
        Ok(payload)
    })
}

/// Adds one port to a hierarchical sheet symbol.
///
/// The port name is trimmed, must not be empty, and must be unique inside the
/// symbol. Coordinates must be finite. Editing never reads the child file; only
/// [`check_hierarchy`] opens it.
pub fn add_hierarchy_port(handle: u64, request: AddHierarchyPortRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a hierarchical port needs a name"));
    }
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(ServiceError::new("session/argument-invalid", "hierarchical port coordinates must be finite"));
    }
    mutate(handle, |design| {
        let index = design
            .hierarchy
            .iter()
            .position(|entry| entry.id == request.sheet || entry.name == request.sheet)
            .ok_or_else(|| not_found(format!("no hierarchical sheet with id or name `{}` is declared", request.sheet)))?;
        if design.hierarchy[index].ports.iter().any(|port| port.name == name) {
            return Err(duplicate(format!(
                "hierarchical sheet `{}` already declares port `{name}`",
                design.hierarchy[index].name
            )));
        }
        let port = SheetPort { name, direction: request.direction, x: request.x, y: request.y };
        design.hierarchy[index].ports.push(port.clone());
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("hierarchy".to_string(), json!(design.hierarchy[index]));
        payload.insert("port".to_string(), json!(port));
        Ok(payload)
    })
}

/// Removes one port from a hierarchical sheet symbol.
pub fn remove_hierarchy_port(handle: u64, request: RemoveHierarchyPortRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a hierarchical port needs a name"));
    }
    mutate(handle, |design| {
        let index = design
            .hierarchy
            .iter()
            .position(|entry| entry.id == request.sheet || entry.name == request.sheet)
            .ok_or_else(|| not_found(format!("no hierarchical sheet with id or name `{}` is declared", request.sheet)))?;
        let port_index = design.hierarchy[index].ports.iter().position(|port| port.name == name).ok_or_else(|| {
            not_found(format!("hierarchical sheet `{}` declares no port `{name}`", design.hierarchy[index].name))
        })?;
        let removed = design.hierarchy[index].ports.remove(port_index);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("hierarchy".to_string(), json!(design.hierarchy[index]));
        payload.insert("removed".to_string(), json!(removed.name));
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
            labels: Vec::new(),
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
    export_with_options(handle, format, path, None, None)
}

/// Projects one open design to an exchange target, optionally through a BOM
/// variant.
///
/// A variant only changes `bom`, `bom_json` and `bom_html`: its DNP designators
/// are left out
/// and its value overrides replace the component value. Any other format rejects
/// the variant instead of silently ignoring it.
pub fn export_with_variant(handle: u64, format: &str, path: &Path, variant: Option<&str>) -> Result<Value, ServiceError> {
    export_with_options(handle, format, path, variant, None)
}

/// Projects one open design to an exchange target, optionally through a BOM
/// variant and optionally limited to one drawing sheet.
///
/// A sheet selection applies to the drawing projections `svg`, `pdf` and `png`:
/// a selected export paints only that sheet, while the default paints every
/// sheet. PDF gives every sheet its own page, and the multi-sheet SVG and PNG
/// stack every sheet into its own labelled region so sheets that share design
/// coordinates never overlap. Formats that do not paint a drawing reject the
/// sheet instead of silently ignoring it.
pub fn export_with_options(
    handle: u64,
    format: &str,
    path: &Path,
    variant: Option<&str>,
    sheet: Option<&str>,
) -> Result<Value, ServiceError> {
    if !SUPPORTED_EXPORTS.contains(&format) {
        return Err(not_implemented(&format!("export `{format}`")));
    }
    if variant.is_some() && format != "bom" && format != "bom_json" && format != "bom_html" {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("a BOM variant applies to `bom`, `bom_json` and `bom_html`, not `{format}`"),
        ));
    }
    if sheet.is_some() && format != "svg" && format != "pdf" && format != "png" {
        return Err(ServiceError::new(
            "session/argument-invalid",
            format!("a sheet selection applies to `svg`, `pdf` and `png`, not `{format}`"),
        ));
    }
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let sheets: Vec<&Sheet> = match sheet {
        Some(needle) => vec![&design.sheets[sheet_index(&design, Some(needle))?]],
        None => design.sheets.iter().collect(),
    };
    if format == "png" {
        let bytes = png::render_png(&design, &sheets);
        let written = write_atomic_bytes(path, &bytes, true)?;
        return Ok(json!({
            "revision": design.revision,
            "format": format,
            "path": path.display().to_string(),
            "bytes": written,
            "editable": false,
            "artifact": json!({
                "kind": "png",
                "width": png::WIDTH,
                "height": png::HEIGHT,
                "components": sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
            }),
        }));
    }
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
        "net_class" => {
            let (contents, rows, unclassified) = render_net_class_csv(&design);
            let artifact = json!({
                "kind": "net_class",
                "classes": design.rules.net_classes.len(),
                "rows": rows,
                "unclassifiedRows": unclassified,
            });
            (contents, artifact)
        }
        "net_length" => {
            let (contents, rows, unrouted, wires, length) = render_net_length_csv(&design);
            let artifact = json!({
                "kind": "net_length",
                "nets": rows,
                "wires": wires,
                "unrouted": unrouted,
                "length": format_number(length),
            });
            (contents, artifact)
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
                "components": sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
            });
            (render_svg_for(&design, &sheets), artifact)
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
            let (contents, dropped) = render_pdf_for(&design, &sheets);
            if dropped > 0 {
                losses.push(json!({
                    "code": PDF_TEXT_ENCODING,
                    "detail": format!("{dropped} character(s) outside printable ASCII were written as `?` in the PDF; export SVG for full Unicode text"),
                }));
            }
            let artifact = json!({
                "kind": "pdf",
                "pages": sheets.len().max(1),
                "mediaBox": "a4-landscape",
                "components": sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
            });
            (contents, artifact)
        }
        "block_diagram" => {
            let drawing = drawings::render_block_diagram(&design);
            let artifact = json!({
                "kind": "block_diagram",
                "nodes": drawing.nodes,
                "edges": drawing.edges,
                "nets": drawing.nets,
            });
            (drawing.svg, artifact)
        }
        "sheet_overview" => {
            let drawing = drawings::render_sheet_overview(&design);
            let artifact = json!({
                "kind": "sheet_overview",
                "sheets": drawing.sheets,
                "columns": drawing.columns,
                "rows": drawing.rows,
            });
            (drawing.svg, artifact)
        }
        "pinout_chart" => {
            let drawing = drawings::render_pinout_chart(&design);
            let artifact = json!({
                "kind": "pinout_chart",
                "components": drawing.components,
                "rows": drawing.rows,
            });
            (drawing.svg, artifact)
        }
        "symbol_library" => {
            let drawing = drawings::render_symbol_library(&design);
            let artifact = json!({
                "kind": "symbol_library",
                "symbols": drawing.symbols,
                "components": drawing.components,
            });
            (drawing.svg, artifact)
        }
        "net_topology" => {
            let drawing = drawings::render_net_topology(&design);
            let artifact = json!({
                "kind": "net_topology",
                "components": drawing.components,
                "nets": drawing.nets,
                "edges": drawing.edges,
            });
            (drawing.svg, artifact)
        }
        "power_distribution" => {
            let drawing = drawings::render_power_distribution(&design);
            let artifact = json!({
                "kind": "power_distribution",
                "powerNets": drawing.power_nets,
                "groundNets": drawing.ground_nets,
                "components": drawing.components,
                "connections": drawing.connections,
            });
            (drawing.svg, artifact)
        }
        "bom_drawing" => {
            let drawing = drawings::render_bom_drawing(&design);
            let artifact = json!({
                "kind": "bom_drawing",
                "rows": drawing.rows,
                "components": drawing.components,
            });
            (drawing.svg, artifact)
        }
        "bom_drawing_pdf" => {
            let (contents, dropped, pages) = render_bom_pdf(&design);
            if dropped > 0 {
                losses.push(json!({
                    "loss": "pdf-font",
                    "detail": format!("{dropped} characters are outside the PDF base-14 font"),
                }));
            }
            let rows = bom_rows(&design, None);
            let artifact = json!({
                "kind": "bom_drawing_pdf",
                "pages": pages,
                "rows": rows.len(),
                "components": rows.iter().map(|row| row.designators.len()).sum::<usize>(),
                "mediaBox": "a4-landscape",
            });
            (contents, artifact)
        }
        "net_xref" => {
            let drawing = drawings::render_net_xref(&design);
            let artifact = json!({
                "kind": "net_xref",
                "nets": drawing.nets,
                "nodes": drawing.nodes,
                "pages": drawing.pages,
            });
            (drawing.svg, artifact)
        }
        "net_xref_pdf" => {
            let (contents, dropped, pages, nets) = render_net_xref_pdf(&design);
            if dropped > 0 {
                losses.push(json!({
                    "loss": "pdf-font",
                    "detail": format!("{dropped} characters are outside the PDF base-14 font"),
                }));
            }
            let artifact = json!({
                "kind": "net_xref_pdf",
                "pages": pages,
                "nets": nets,
                "mediaBox": "a4-landscape",
            });
            (contents, artifact)
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
        "kicad_net" => {
            let artifact = json!({
                "kind": "kicad_net",
                "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "nets": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
                "nodes": design
                    .sheets
                    .iter()
                    .flat_map(|sheet| &sheet.nets)
                    .map(|net| net.nodes.len())
                    .sum::<usize>(),
            });
            (kicad::render_netlist(&design), artifact)
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
    if format == "spice" || format == "spice_subckt" || format == "pdf" {
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

/// Validates one KiCad `.kicad_sym` symbol library.
///
/// The check reads the same declaration subset [`import_kicad_symbol`] reads and
/// never writes a file. It reports `library/duplicate-symbol`,
/// `library/symbol-without-pins`, `library/duplicate-pin-number` and
/// `library/pin-without-name` with the symbol, pin number and unit each finding
/// concerns, and returns the reader's `kicad_sym/unsupported-element` losses next
/// to the findings. `passed` is false only when a finding is an error, so an
/// unsupported construct that the reader skipped does not fail the check.
pub fn check_kicad_symbol(text: &str) -> Result<Value, ServiceError> {
    kicad::check_symbol_library(text)
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
            .ok_or_else(|| ServiceError::new("sheet/not-found", format!("sheet `{needle}` is not part of this design")))?,
    };
    let target = &design.sheets[index];
    let projection = svg_projection(&design, &[target]);
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
            .ok_or_else(|| ServiceError::new("sheet/not-found", format!("sheet `{needle}` is not part of this design"))),
    }
}

/// Net identity used by [`merge_wires`]: an absent, empty or whitespace-only
/// name means the wire belongs to no net.
fn wire_net_name(net: Option<&str>) -> Option<&str> {
    net.map(str::trim).filter(|name| !name.is_empty())
}

/// Whether two points lie within `tolerance` on both axes.
fn points_within(left: [f64; 2], right: [f64; 2], tolerance: f64) -> bool {
    (left[0] - right[0]).abs() <= tolerance && (left[1] - right[1]).abs() <= tolerance
}

/// Length of one straight segment.
fn segment_length(from: [f64; 2], to: [f64; 2]) -> f64 {
    let delta_x = to[0] - from[0];
    let delta_y = to[1] - from[1];
    (delta_x * delta_x + delta_y * delta_y).sqrt()
}

/// Unit vector pointing from `from` toward `toward`; the caller rules out a
/// zero-length segment, so the divisor is never zero.
fn unit_direction(from: [f64; 2], toward: [f64; 2]) -> [f64; 2] {
    let delta_x = toward[0] - from[0];
    let delta_y = toward[1] - from[1];
    let length = (delta_x * delta_x + delta_y * delta_y).sqrt();
    [delta_x / length, delta_y / length]
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

/// Picks a free `hierarchy-N` identity without adding a document counter field.
fn free_hierarchy_id(design: &Design) -> String {
    let used: HashSet<&str> = design.hierarchy.iter().map(|sheet| sheet.id.as_str()).collect();
    let mut counter = design.hierarchy.len() + 1;
    loop {
        let candidate = format!("hierarchy-{counter}");
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
    let mut label_ids = HashSet::new();
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
            if let Some(axis) = &component.mirror
                && axis != "x"
                && axis != "y"
            {
                return Err(invalid(format!("component `{}` mirror axis `{axis}` must be `x` or `y`", component.reference)));
            }
            for pin in component.pin_types.keys() {
                if !component.pins.iter().any(|declared| declared == pin) {
                    return Err(invalid(format!("component `{}` types missing pin `{pin}`", component.reference)));
                }
            }
            for key in component.properties.keys() {
                if key.trim().is_empty() {
                    return Err(invalid(format!("component `{}` has an empty property key", component.reference)));
                }
                if breaks_net_grammar(key) {
                    return Err(invalid(format!(
                        "component `{}` property key `{key}` contains whitespace or a control character",
                        component.reference
                    )));
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
        for label in &sheet.labels {
            if !label_ids.insert(label.id.clone()) {
                return Err(invalid(format!("label id `{}` appears more than once", label.id)));
            }
            if label.text.trim().is_empty() {
                return Err(invalid(format!("label `{}` has empty text", label.id)));
            }
            if !label.x.is_finite() || !label.y.is_finite() || !label.rotation.is_finite() {
                return Err(invalid(format!("label `{}` has a non-finite coordinate", label.id)));
            }
            if !valid_justify(&label.justify) {
                return Err(invalid(format!("label `{}` has justification `{}`", label.id, label.justify)));
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

/// Whether two component bodies overlap in their interior.
///
/// The body is the axis-aligned rectangle the SVG preview draws: centred on the
/// placement with [`COMPONENT_HALF_WIDTH`] and [`COMPONENT_HALF_HEIGHT`] as the
/// half extents. Rotation is deliberately ignored so the check and the preview
/// agree. Strict comparisons keep rectangles that only touch an edge apart.
fn component_bodies_overlap(left: &Component, right: &Component) -> bool {
    (left.x - COMPONENT_HALF_WIDTH) < (right.x + COMPONENT_HALF_WIDTH)
        && (right.x - COMPONENT_HALF_WIDTH) < (left.x + COMPONENT_HALF_WIDTH)
        && (left.y - COMPONENT_HALF_HEIGHT) < (right.y + COMPONENT_HALF_HEIGHT)
        && (right.y - COMPONENT_HALF_HEIGHT) < (left.y + COMPONENT_HALF_HEIGHT)
}

/// Whether a bus declaration claims a net name.
///
/// A bus claims its own name and every alias, because a net with that name
/// expands into the members; it also claims each member name it expands to.
/// Two buses claiming one name would give that net two meanings.
fn bus_claims_net(bus: &Bus, name: &str) -> bool {
    bus.name == name
        || bus.aliases.iter().any(|alias| alias == name)
        || (0..bus.width).any(|index| format!("{}{index}", bus.name) == name)
}

fn inspect_design(design: &Design, checks: &[String]) -> Vec<Value> {
    let mut issues = CheckReport::new();
    // Dangling endpoints keep the required sheet, wire and endpoint order, so
    // they are appended after the shared report normalisation instead of being
    // re-sorted by code and message.
    let mut appended = Vec::new();
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
        // One reference designates one component, so two placed components that
        // share it make every `REF.PIN` node naming that reference ambiguous.
        // Components are grouped by the exact stored reference across the whole
        // design, because a reference is design-wide. An empty reference is not
        // a shared designator and is already reported above. Each group with more
        // than one member reports once, at its first component in document order,
        // and lists every component id in the same order.
        let mut shared: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
        for sheet in &design.sheets {
            for component in &sheet.components {
                if component.reference.trim().is_empty() {
                    continue;
                }
                shared.entry(component.reference.as_str()).or_default().push((&sheet.id, &component.id));
            }
        }
        for (reference, members) in shared {
            if members.len() < 2 {
                continue;
            }
            let (sheet_id, object_id) = members[0];
            let ids = members.iter().map(|(_, id)| format!("`{id}`")).collect::<Vec<_>>().join(", ");
            issues.push(issue(
                "erc/duplicate-reference",
                "error",
                design.revision,
                sheet_id,
                object_id,
                true,
                format!("reference `{reference}` is shared by {} components: {ids}", members.len()),
            ));
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
    if checks.iter().any(|check| check == "drivers") {
        // Only typed pins carry a direction, so an untyped pin is never counted
        // as a driver or as a load. A driving pin sources or sinks the net: an
        // output, a bidirectional, tri-state or open-collector/emitter pin, and
        // a power output. An input that no driver reaches is an undriven input.
        for sheet in &design.sheets {
            let mut open_pins: HashSet<(&str, &str)> = HashSet::new();
            for flag in &sheet.no_connects {
                if let Some(node) = &flag.node {
                    open_pins.insert((node.reference.as_str(), node.pin.as_str()));
                }
            }
            for net in &sheet.nets {
                let mut inputs = Vec::new();
                let mut drivers = Vec::new();
                for node in &net.nodes {
                    let Some(component) = sheet.components.iter().find(|component| component.reference == node.reference)
                    else {
                        continue;
                    };
                    let Some(pin_type) = component.pin_types.get(&node.pin).copied()
                    else {
                        continue;
                    };
                    match pin_type {
                        PinType::Input => inputs.push(format!("{}.{}", node.reference, node.pin)),
                        PinType::Output
                        | PinType::Bidirectional
                        | PinType::TriState
                        | PinType::PowerOut
                        | PinType::OpenCollector
                        | PinType::OpenEmitter => drivers.push(format!("{}.{}", node.reference, node.pin)),
                        PinType::PowerIn | PinType::Passive | PinType::Unspecified => {}
                    }
                }
                if !inputs.is_empty() && drivers.is_empty() {
                    issues.push(issue(
                        "erc/undriven-input",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!("net `{}` connects input pin(s) {} but no pin drives it", net.name, inputs.join(", ")),
                    ));
                }
            }
            // A power input that reaches no net is open. A no-connect flag that
            // names the pin makes that deliberate, so only an unmarked pin is
            // reported.
            for component in &sheet.components {
                for (pin, pin_type) in &component.pin_types {
                    if *pin_type != PinType::PowerIn {
                        continue;
                    }
                    let connected = sheet
                        .nets
                        .iter()
                        .any(|net| net.nodes.iter().any(|node| node.reference == component.reference && node.pin == *pin));
                    if !connected && !open_pins.contains(&(component.reference.as_str(), pin.as_str())) {
                        issues.push(issue(
                            "erc/unconnected-power-pin",
                            "error",
                            design.revision,
                            &sheet.id,
                            &component.id,
                            true,
                            format!(
                                "power-input pin `{pin}` of component `{}` is not connected to any net",
                                component.reference
                            ),
                        ));
                    }
                }
            }
        }
    }
    if checks.iter().any(|check| check == "pin_types") {
        // Only a role pair that cannot share one net is a conflict here. The
        // pairs the `electrical` and `power` checks already report
        // (input/input, output/output, power_out/power_out and
        // power_out/output) are left out so one net never produces two
        // findings for the same short.
        for sheet in &design.sheets {
            for net in &sheet.nets {
                let mut outputs = Vec::new();
                let mut power_inputs = Vec::new();
                let mut power_outputs = Vec::new();
                let mut logic_drivers = Vec::new();
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
                        PinType::Output => outputs.push(label),
                        PinType::PowerIn => power_inputs.push(label),
                        PinType::PowerOut => power_outputs.push(label),
                        PinType::Bidirectional | PinType::TriState | PinType::OpenCollector | PinType::OpenEmitter => {
                            logic_drivers.push(label)
                        }
                        PinType::Input | PinType::Passive | PinType::Unspecified => {}
                    }
                }
                if !outputs.is_empty() && !power_inputs.is_empty() {
                    issues.push(issue(
                        "erc/output-power-in-conflict",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` connects logic output {} and power input {}; a logic output cannot supply a power input",
                            net.name,
                            outputs.join(", "),
                            power_inputs.join(", ")
                        ),
                    ));
                }
                if !power_outputs.is_empty() && !logic_drivers.is_empty() {
                    issues.push(issue(
                        "erc/power-output-driver-conflict",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` connects power output {} and logic driver {}; the supply would drive against the logic pin",
                            net.name,
                            power_outputs.join(", "),
                            logic_drivers.join(", ")
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
    if checks.iter().any(|check| check == "bus") {
        // Buses live on the design, not on a sheet, so their findings carry an
        // empty `sheetId`; only `bus/shared-net` names the sheet of the net.
        let mut names: HashSet<&str> = HashSet::new();
        for bus in &design.buses {
            if !names.insert(bus.name.as_str()) {
                issues.push(issue(
                    "bus/duplicate-name",
                    "error",
                    design.revision,
                    "",
                    &bus.id,
                    true,
                    format!("bus name `{}` is declared more than once", bus.name),
                ));
            }
        }
        let declared_nets: HashSet<&str> =
            design.sheets.iter().flat_map(|sheet| sheet.nets.iter()).map(|net| net.name.as_str()).collect();
        for bus in &design.buses {
            if bus.width == 0 {
                issues.push(issue(
                    "bus/empty",
                    "warning",
                    design.revision,
                    "",
                    &bus.id,
                    true,
                    format!("bus `{}` declares zero member nets", bus.name),
                ));
                continue;
            }
            // One net named the bus or an alias already provides every member,
            // so individual member nets are only required when nothing expands.
            let expands =
                declared_nets.iter().any(|name| *name == bus.name.as_str() || bus.aliases.iter().any(|alias| alias == *name));
            let missing: Vec<String> = (0..bus.width)
                .map(|index| format!("{}{index}", bus.name))
                .filter(|member| !expands && !declared_nets.contains(member.as_str()))
                .collect();
            if !missing.is_empty() {
                issues.push(issue(
                    "bus/member-missing",
                    "error",
                    design.revision,
                    "",
                    &bus.id,
                    true,
                    format!(
                        "bus `{}` declares member net(s) {} that no net in the document provides",
                        bus.name,
                        missing.join(", ")
                    ),
                ));
            }
        }
        for sheet in &design.sheets {
            for net in &sheet.nets {
                let claims: Vec<&str> =
                    design.buses.iter().filter(|bus| bus_claims_net(bus, &net.name)).map(|bus| bus.name.as_str()).collect();
                if claims.len() > 1 {
                    issues.push(issue(
                        "bus/shared-net",
                        "error",
                        design.revision,
                        &sheet.id,
                        &net.id,
                        true,
                        format!(
                            "net `{}` is claimed by buses {}; one net may belong to only one bus",
                            net.name,
                            claims.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
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
    if checks.iter().any(|check| check == "clearance") {
        for sheet in &design.sheets {
            let mut components: Vec<&Component> = sheet.components.iter().collect();
            components.sort_by(|left, right| left.reference.cmp(&right.reference).then_with(|| left.id.cmp(&right.id)));
            for (index, left) in components.iter().enumerate() {
                for right in &components[index + 1..] {
                    if component_bodies_overlap(left, right) {
                        issues.push(
                            issue(
                                "clearance/component-overlap",
                                "warning",
                                design.revision,
                                &sheet.id,
                                &left.id,
                                true,
                                format!(
                                    "components `{}` and `{}` have overlapping bodies; the preview draws both as axis-aligned boxes centred on their placement",
                                    left.reference, right.reference
                                ),
                            )
                            .with_detail("references", format!("{},{}", left.reference, right.reference)),
                        );
                    }
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
    if checks.iter().any(|check| check == "dangling") {
        for (index, sheet) in design.sheets.iter().enumerate() {
            for finding in dangling::findings(design, index) {
                appended.push(issue_json(
                    &issue(
                        dangling::DANGLING_ENDPOINT,
                        "warning",
                        design.revision,
                        &sheet.id,
                        &finding.wire_id,
                        true,
                        finding.message,
                    )
                    .with_detail("endpoint", finding.endpoint.to_string())
                    .with_detail("x", finding.x.to_string())
                    .with_detail("y", finding.y.to_string()),
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
    let mut values: Vec<Value> = issues.issues.iter().map(issue_json).collect();
    values.extend(appended);
    values
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
///
/// `endpoint`, `x` and `y` appear only on findings that carry them, such as the
/// dangling-endpoint check; the other findings keep the same shape as before.
fn issue_json(issue: &Issue) -> Value {
    let mut value = json!({
        "code": issue.code,
        "severity": issue.severity.as_str(),
        "revision": issue.details.get("revision").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
        "sheetId": issue.details.get("sheetId"),
        "objectId": issue.details.get("objectId"),
        "recoverable": issue.details.get("recoverable").is_some_and(|value| value == "true"),
        "message": issue.message,
    });
    let Some(map) = value.as_object_mut()
    else {
        return value;
    };
    if let Some(endpoint) = issue.details.get("endpoint").and_then(|value| value.parse::<u64>().ok()) {
        map.insert("endpoint".to_string(), json!(endpoint));
    }
    for key in ["x", "y"] {
        if let Some(number) = issue.details.get(key).and_then(|value| value.parse::<f64>().ok()) {
            map.insert(key.to_string(), json!(number));
        }
    }
    value
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

/// Reports the full detail of one placed component by reference.
///
/// The lookup walks the sheets in document order; a reference that no sheet
/// places is `object/not-found`. The report gathers the component identity,
/// placement, value, footprint and custom properties, and one entry per declared
/// pin with the nets that reach it and whether a no-connect flag marks it. Pins
/// keep the component's declaration order and net names keep document order
/// without duplicates, so one document state always produces the same report.
/// The report never mutates the design.
pub fn component_report(handle: u64, reference: &str) -> Result<Value, ServiceError> {
    with_session(handle, |session| {
        let design = &session.design;
        let (sheet, component) = design
            .sheets
            .iter()
            .find_map(|sheet| {
                sheet.components.iter().find(|component| component.reference == reference).map(|component| (sheet, component))
            })
            .ok_or_else(|| not_found(format!("no component with reference `{reference}` is placed in this design")))?;

        let mut marked: HashSet<(&str, &str)> = HashSet::new();
        for flag in &sheet.no_connects {
            if let Some(node) = &flag.node {
                marked.insert((node.reference.as_str(), node.pin.as_str()));
            }
        }

        let pins = component
            .pins
            .iter()
            .map(|pin| {
                let mut nets: Vec<&str> = Vec::new();
                for net in &sheet.nets {
                    let reaches = net.nodes.iter().any(|node| node.reference == component.reference && node.pin == *pin);
                    if reaches && !nets.contains(&net.name.as_str()) {
                        nets.push(net.name.as_str());
                    }
                }
                json!({
                    "name": pin,
                    "nets": nets,
                    "noConnect": marked.contains(&(component.reference.as_str(), pin.as_str())),
                })
            })
            .collect::<Vec<_>>();

        let properties =
            component.properties.iter().map(|(key, value)| json!({ "key": key, "value": value })).collect::<Vec<_>>();

        Ok(json!({
            "format": COMPONENT_REPORT_FORMAT,
            "version": COMPONENT_REPORT_VERSION,
            "sheet": { "id": sheet.id, "name": sheet.name },
            "component": {
                "id": component.id,
                "reference": component.reference,
                "value": component.value,
                "footprint": component.footprint,
                "x": component.x,
                "y": component.y,
                "rotation": component.rotation,
                "mirror": component.mirror,
                "properties": properties,
            },
            "pins": pins,
        }))
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
    let mut label_total = 0usize;

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

            let local_labels = sheet.labels.iter().filter(|label| label.scope == LabelScope::Local).count();
            let global_labels = sheet.labels.iter().filter(|label| label.scope == LabelScope::Global).count();
            let hierarchical_labels = sheet.labels.iter().filter(|label| label.scope == LabelScope::Hierarchical).count();
            label_total += sheet.labels.len();

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
                "labels": {
                    "total": sheet.labels.len(),
                    "local": local_labels,
                    "global": global_labels,
                    "hierarchical": hierarchical_labels,
                },
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
            "labels": label_total,
        },
    })
}

/// One node of the read-only connection graph behind [`trace_pins`].
struct TraceNode {
    kind: &'static str,
    sheet: usize,
    sheet_name: String,
    index: Option<usize>,
    name: Option<String>,
    detail: String,
    ordinal: usize,
}

/// Deterministic node order: sheet, kind, 0-based index or name, then creation order.
fn compare_trace_nodes(left: &TraceNode, right: &TraceNode) -> std::cmp::Ordering {
    left.sheet_name
        .cmp(&right.sheet_name)
        .then_with(|| left.kind.cmp(right.kind))
        .then_with(|| match (left.index, right.index) {
            (Some(left_index), Some(right_index)) => left_index.cmp(&right_index),
            _ => std::cmp::Ordering::Equal,
        })
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.ordinal.cmp(&right.ordinal))
}

/// Undirected graph of the objects one trace may walk through.
#[derive(Default)]
struct TraceGraph {
    nodes: Vec<TraceNode>,
    edges: Vec<BTreeSet<usize>>,
}

impl TraceGraph {
    fn push(
        &mut self,
        kind: &'static str,
        sheet: usize,
        sheet_name: &str,
        index: Option<usize>,
        name: Option<String>,
        detail: String,
    ) -> usize {
        let ordinal = self.nodes.len();
        self.nodes.push(TraceNode { kind, sheet, sheet_name: sheet_name.to_string(), index, name, detail, ordinal });
        self.edges.push(BTreeSet::new());
        ordinal
    }

    fn link(&mut self, left: usize, right: usize) {
        if left != right {
            self.edges[left].insert(right);
            self.edges[right].insert(left);
        }
    }
}

/// Splits one `REF.PIN` argument without inventing a second grammar.
fn split_trace_pin(text: &str) -> Result<(&str, &str), ServiceError> {
    text.rsplit_once('.')
        .filter(|(reference, pin)| !reference.is_empty() && !pin.is_empty())
        .ok_or_else(|| ServiceError::new("session/argument-invalid", format!("`{text}` is not a `REF.PIN` node")))
}

/// Resolves one `REF.PIN` to its sheet, component and pin name.
fn resolve_trace_pin<'a>(design: &'a Design, text: &'a str) -> Result<(usize, &'a Component, &'a str), ServiceError> {
    let (reference, pin) = split_trace_pin(text)?;
    for (sheet_index, sheet) in design.sheets.iter().enumerate() {
        let Some(component) = sheet.components.iter().find(|component| component.reference == reference)
        else {
            continue;
        };
        if !component.pins.iter().any(|candidate| candidate == pin) {
            return Err(ServiceError::new("trace/pin-missing", format!("component `{reference}` has no pin `{pin}`")));
        }
        return Ok((sheet_index, component, pin));
    }
    Err(ServiceError::new(
        "trace/component-missing",
        format!("no component with reference `{reference}` is placed in this design"),
    ))
}

/// Resolves one hop sheet; a hop may never point outside the document.
fn trace_sheet_name(design: &Design, index: usize) -> Result<&str, ServiceError> {
    design
        .sheets
        .get(index)
        .map(|sheet| sheet.name.as_str())
        .ok_or_else(|| ServiceError::new("sheet/not-found", format!("sheet index {index} is not part of this design")))
}

/// Stable spelling of one port direction, matching the port editor vocabulary.
fn port_direction_name(direction: PortDirection) -> &'static str {
    match direction {
        PortDirection::Input => "input",
        PortDirection::Output => "output",
        PortDirection::Bidirectional => "bidirectional",
        PortDirection::TriState => "tri_state",
        PortDirection::Passive => "passive",
    }
}

/// Stable spelling of one pin electrical role, matching the document schema and
/// the KiCad pin types.
fn pin_role_name(pin_type: PinType) -> &'static str {
    match pin_type {
        PinType::Input => "input",
        PinType::Output => "output",
        PinType::Bidirectional => "bidirectional",
        PinType::TriState => "tri_state",
        PinType::Passive => "passive",
        PinType::PowerIn => "power_in",
        PinType::PowerOut => "power_out",
        PinType::OpenCollector => "open_collector",
        PinType::OpenEmitter => "open_emitter",
        PinType::Unspecified => "unspecified",
    }
}

/// Builds the connection graph and the pin lookup of one design.
///
/// Pins join the wires of their declared net, wires join by the same endpoint
/// and T-junction rule as the connectivity check, junctions join every wire
/// they lie on, and labels join the wire they sit on. Two labels join when
/// their text matches and their scope permits it: any pair on one sheet, and
/// `global` labels across the whole design. A hierarchical label also joins a
/// sheet port of the same name, because hierarchy declarations are
/// document-level. No child document is opened.
fn build_trace_graph(design: &Design) -> (TraceGraph, HashMap<(String, String), usize>) {
    let mut graph = TraceGraph::default();
    let mut pin_nodes: HashMap<(String, String), usize> = HashMap::new();
    let mut sheet_wires: Vec<Vec<usize>> = Vec::with_capacity(design.sheets.len());
    let mut sheet_junctions: Vec<Vec<usize>> = Vec::with_capacity(design.sheets.len());
    let mut sheet_labels: Vec<Vec<usize>> = Vec::with_capacity(design.sheets.len());

    for (sheet_index, sheet) in design.sheets.iter().enumerate() {
        let name = sheet.name.as_str();
        for component in &sheet.components {
            for pin in &component.pins {
                let key = format!("{}.{}", component.reference, pin);
                let detail = format!("pin {key} of {} ({})", component.reference, component.value);
                let node = graph.push("pin", sheet_index, name, None, Some(key.clone()), detail);
                pin_nodes.insert((component.reference.clone(), pin.clone()), node);
            }
        }
        let mut wires = Vec::with_capacity(sheet.wires.len());
        for (index, wire) in sheet.wires.iter().enumerate() {
            let net = wire.net.as_deref().filter(|net| !net.is_empty()).unwrap_or("(no net)");
            let detail =
                format!("{} {} ({}, {}) -> ({}, {})", wire.id, net, wire.from[0], wire.from[1], wire.to[0], wire.to[1]);
            wires.push(graph.push("wire", sheet_index, name, Some(index), None, detail));
        }
        let mut junctions = Vec::with_capacity(sheet.junctions.len());
        for (index, junction) in sheet.junctions.iter().enumerate() {
            let detail = format!("junction at ({}, {})", junction.x, junction.y);
            junctions.push(graph.push("junction", sheet_index, name, Some(index), None, detail));
        }
        let mut labels = Vec::with_capacity(sheet.labels.len());
        for (index, label) in sheet.labels.iter().enumerate() {
            let detail = format!("{} label \"{}\" at ({}, {})", label_scope_name(label.scope), label.text, label.x, label.y);
            labels.push(graph.push("label", sheet_index, name, Some(index), Some(label.text.clone()), detail));
        }
        sheet_wires.push(wires);
        sheet_junctions.push(junctions);
        sheet_labels.push(labels);
    }

    let mut port_nodes = Vec::new();
    let root_name = design.sheets.first().map(|sheet| sheet.name.as_str()).unwrap_or("");
    for symbol in &design.hierarchy {
        for (index, port) in symbol.ports.iter().enumerate() {
            let detail = format!(
                "{} port \"{}\" of hierarchical sheet \"{}\" ({})",
                port_direction_name(port.direction),
                port.name,
                symbol.name,
                symbol.file
            );
            port_nodes.push(graph.push("sheet-port", 0, root_name, Some(index), Some(port.name.clone()), detail));
        }
    }

    for (sheet_index, sheet) in design.sheets.iter().enumerate() {
        let wire_nodes = &sheet_wires[sheet_index];
        let junction_nodes = &sheet_junctions[sheet_index];
        let label_nodes = &sheet_labels[sheet_index];

        let mut wires_by_net: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, wire) in sheet.wires.iter().enumerate() {
            if let Some(net) = wire.net.as_deref().filter(|net| !net.is_empty()) {
                wires_by_net.entry(net).or_default().push(index);
            }
        }
        for net in &sheet.nets {
            let attached = wires_by_net
                .get(net.name.as_str())
                .map(|indices| indices.iter().map(|&index| wire_nodes[index]).collect::<Vec<_>>())
                .unwrap_or_default();
            let pins = net
                .nodes
                .iter()
                .filter_map(|node| pin_nodes.get(&(node.reference.clone(), node.pin.clone())).copied())
                .collect::<Vec<_>>();
            if attached.is_empty() {
                for left in 0..pins.len() {
                    for right in left + 1..pins.len() {
                        graph.link(pins[left], pins[right]);
                    }
                }
            }
            else {
                for &pin in &pins {
                    for &wire in &attached {
                        graph.link(pin, wire);
                    }
                }
            }
        }

        for left in 0..sheet.wires.len() {
            for right in left + 1..sheet.wires.len() {
                if connectivity::connected(&sheet.wires[left], &sheet.wires[right]) {
                    graph.link(wire_nodes[left], wire_nodes[right]);
                }
            }
        }
        for (index, junction) in sheet.junctions.iter().enumerate() {
            let point = [junction.x, junction.y];
            for (wire_index, wire) in sheet.wires.iter().enumerate() {
                if connectivity::on_wire(point, wire) {
                    graph.link(junction_nodes[index], wire_nodes[wire_index]);
                }
            }
        }
        for (index, label) in sheet.labels.iter().enumerate() {
            let point = [label.x, label.y];
            for (wire_index, wire) in sheet.wires.iter().enumerate() {
                if connectivity::on_wire(point, wire) {
                    graph.link(label_nodes[index], wire_nodes[wire_index]);
                }
            }
        }
    }

    let all_labels = design
        .sheets
        .iter()
        .enumerate()
        .flat_map(|(sheet_index, sheet)| {
            let nodes = &sheet_labels[sheet_index];
            sheet
                .labels
                .iter()
                .enumerate()
                .map(move |(index, label)| (sheet_index, nodes[index], label.scope, label.text.clone()))
        })
        .collect::<Vec<_>>();
    for left in 0..all_labels.len() {
        for right in left + 1..all_labels.len() {
            let (left_sheet, left_node, left_scope, left_text) = &all_labels[left];
            let (right_sheet, right_node, right_scope, right_text) = &all_labels[right];
            if left_text != right_text {
                continue;
            }
            let same_sheet = left_sheet == right_sheet;
            let global_pair = *left_scope == LabelScope::Global && *right_scope == LabelScope::Global;
            if same_sheet || global_pair {
                graph.link(*left_node, *right_node);
            }
        }
    }
    for &port in &port_nodes {
        let port_name = graph.nodes[port].name.clone().unwrap_or_default();
        for (_, label, scope, text) in &all_labels {
            if *scope == LabelScope::Hierarchical && *text == port_name {
                graph.link(port, *label);
            }
        }
    }

    (graph, pin_nodes)
}

/// Shortest path by hop count, then lexicographic node order for ties.
fn shortest_trace_path(graph: &TraceGraph, start: usize, target: usize) -> Option<Vec<usize>> {
    let mut distance = vec![usize::MAX; graph.nodes.len()];
    distance[target] = 0;
    let mut queue = VecDeque::from([target]);
    while let Some(node) = queue.pop_front() {
        for &neighbor in &graph.edges[node] {
            if distance[neighbor] == usize::MAX {
                distance[neighbor] = distance[node] + 1;
                queue.push_back(neighbor);
            }
        }
    }
    if distance[start] == usize::MAX {
        return None;
    }
    let mut path = vec![start];
    let mut current = start;
    while current != target {
        let next = graph.edges[current]
            .iter()
            .copied()
            .filter(|&neighbor| distance[neighbor] + 1 == distance[current])
            .min_by(|&left, &right| compare_trace_nodes(&graph.nodes[left], &graph.nodes[right]))
            .expect("a shortest path always has a next node");
        path.push(next);
        current = next;
    }
    Some(path)
}

/// Whether two pins stay connected without walking a label or a sheet port.
fn trace_without_labels(graph: &TraceGraph, start: usize, target: usize) -> bool {
    let mut seen = vec![false; graph.nodes.len()];
    seen[start] = true;
    let mut queue = VecDeque::from([start]);
    while let Some(node) = queue.pop_front() {
        if node == target {
            return true;
        }
        for &neighbor in &graph.edges[node] {
            if matches!(graph.nodes[neighbor].kind, "label" | "sheet-port") || seen[neighbor] {
                continue;
            }
            seen[neighbor] = true;
            queue.push_back(neighbor);
        }
    }
    false
}

/// Projects one node to the hop object the trace report exposes.
fn trace_hop_json(node: &TraceNode) -> Value {
    let mut hop = Map::new();
    hop.insert("type".to_string(), json!(node.kind));
    hop.insert("sheet".to_string(), json!(node.sheet_name));
    if let Some(index) = node.index {
        hop.insert("index".to_string(), json!(index));
    }
    if let Some(name) = &node.name {
        hop.insert("name".to_string(), json!(name));
    }
    hop.insert("detail".to_string(), json!(node.detail));
    Value::Object(hop)
}

/// Traces the read-only connection path between two pins of one open design.
///
/// Both arguments are `REF.PIN` strings. The report is tagged
/// `open-eda-trace` and lists the shortest hop sequence between the two pins,
/// with ties resolved by `(sheet, kind, index or name)` order. A pin that
/// names itself returns `via: "direct"` with no hops. Two pins that no wire,
/// junction, label or sheet port reaches return `connected: false` and are not
/// an error. `via` is `"label"` only when every path needs a label hop; `"wire"`
/// covers net membership, wires and junctions. The trace never writes the
/// document and never opens a child document.
pub fn trace_pins(handle: u64, from: &str, to: &str) -> Result<Value, ServiceError> {
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let (from_sheet, from_component, from_pin) = resolve_trace_pin(&design, from)?;
    let (to_sheet, to_component, to_pin) = resolve_trace_pin(&design, to)?;
    trace_sheet_name(&design, from_sheet)?;
    trace_sheet_name(&design, to_sheet)?;

    let from_key = (from_component.reference.clone(), from_pin.to_string());
    let to_key = (to_component.reference.clone(), to_pin.to_string());
    let (graph, pin_nodes) = build_trace_graph(&design);
    let from_node = *pin_nodes.get(&from_key).expect("a resolved pin always has a trace node");
    let to_node = *pin_nodes.get(&to_key).expect("a resolved pin always has a trace node");

    let same_pin = from_node == to_node;
    let path = if same_pin { None } else { shortest_trace_path(&graph, from_node, to_node) };
    let connected = same_pin || path.is_some();
    let via = if same_pin {
        "direct"
    }
    else if !connected {
        "none"
    }
    else if trace_without_labels(&graph, from_node, to_node) {
        "wire"
    }
    else {
        "label"
    };
    let hops = path
        .as_ref()
        .map(|path| path.iter().map(|&node| trace_hop_json(&graph.nodes[node])).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut sheets = Vec::new();
    let mut seen_sheets = BTreeSet::new();
    if let Some(path) = &path {
        for &node in path {
            if seen_sheets.insert(graph.nodes[node].sheet) {
                sheets.push(graph.nodes[node].sheet_name.clone());
            }
        }
    }

    Ok(json!({
        "format": TRACE_REPORT_FORMAT,
        "version": TRACE_REPORT_VERSION,
        "from": from,
        "to": to,
        "connected": connected,
        "via": via,
        "hops": hops,
        "sheets": sheets,
    }))
}

/// Reports deterministic per-sheet and design-wide object counts.
///
/// The counts come from the model arrays themselves, so the report can never
/// disagree with `list` or the JSON projection. BOM rows are counted with the
/// default population, and the bus count is the number of design-level bus
/// declarations.
pub fn stats(handle: u64) -> Result<Value, ServiceError> {
    with_session(handle, |session| Ok(stats_of(&session.design)))
}

fn stats_of(design: &Design) -> Value {
    let mut components_total = 0usize;
    let mut nets_total = 0usize;
    let mut wires_total = 0usize;
    let mut junctions_total = 0usize;
    let mut labels_total = 0usize;
    let mut no_connects_total = 0usize;
    let mut texts_total = 0usize;

    let sheets = design
        .sheets
        .iter()
        .map(|sheet| {
            components_total += sheet.components.len();
            nets_total += sheet.nets.len();
            wires_total += sheet.wires.len();
            junctions_total += sheet.junctions.len();
            labels_total += sheet.labels.len();
            no_connects_total += sheet.no_connects.len();
            texts_total += sheet.texts.len();
            json!({
                "id": sheet.id,
                "name": sheet.name,
                "components": sheet.components.len(),
                "nets": sheet.nets.len(),
                "wires": sheet.wires.len(),
                "junctions": sheet.junctions.len(),
                "labels": sheet.labels.len(),
                "noConnects": sheet.no_connects.len(),
                "texts": sheet.texts.len(),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "format": STATS_FORMAT,
        "version": STATS_VERSION,
        "revision": design.revision,
        "sheets": sheets,
        "totals": {
            "sheets": design.sheets.len(),
            "components": components_total,
            "nets": nets_total,
            "wires": wires_total,
            "junctions": junctions_total,
            "labels": labels_total,
            "noConnects": no_connects_total,
            "texts": texts_total,
            "buses": design.buses.len(),
            "bomRows": bom_rows(design, None).len(),
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

/// Projects one design to a CSV pin-level netlist grouped by net class.
///
/// The header is `Class,Sheet,Net,Ref,Pin,Role`. Rows are grouped by the declared
/// net classes in declaration order, and the nets no class assigns come last
/// with an empty `Class`. A class claims a net it explicitly assigns or, through
/// a declared bus it assigns, every member of that bus. A net whose name equals
/// a bus name or alias expands into the same member nets the other netlists
/// write, so all projections agree. `Role` is the pin's electrical role in the
/// snake_case vocabulary, or empty for an untyped pin.
///
/// The returned tuple is the CSV, its data-row count and how many of those rows
/// belong to the unclassified group.
fn render_net_class_csv(design: &Design) -> (String, usize, usize) {
    let mut text = String::from("Class,Sheet,Net,Ref,Pin,Role\n");
    let mut rows = 0usize;
    let mut unclassified = 0usize;
    for group in 0..=design.rules.net_classes.len() {
        let group_name = design.rules.net_classes.get(group).map(|class| class.name.as_str());
        for sheet in &design.sheets {
            for net in &sheet.nets {
                if net_class_for_net(design, &net.name).map(|class| class.name.as_str()) != group_name {
                    continue;
                }
                let members: Vec<String> = match bus_for_net(design, &net.name) {
                    Some(bus) => (0..bus.width).map(|index| format!("{}{index}", bus.name)).collect(),
                    None => vec![net.name.clone()],
                };
                for member in members {
                    for node in &net.nodes {
                        let role = sheet
                            .components
                            .iter()
                            .find(|component| component.reference == node.reference)
                            .and_then(|component| component.pin_types.get(&node.pin))
                            .map(|pin_type| pin_role_name(*pin_type))
                            .unwrap_or("");
                        text.push_str(&format!(
                            "{},{},{},{},{},{}\n",
                            csv_field(group_name.unwrap_or("")),
                            csv_field(&sheet.name),
                            csv_field(&member),
                            csv_field(&node.reference),
                            csv_field(&node.pin),
                            role
                        ));
                        rows += 1;
                        if group_name.is_none() {
                            unclassified += 1;
                        }
                    }
                }
            }
        }
    }
    (text, rows, unclassified)
}

/// Projects one design to a CSV of drawn wire length per declared net.
///
/// The header is `Sheet,Net,Length,Wires,Unrouted`; every declared net of every
/// sheet becomes one row in document order. `Length` is the sum of the Euclidean
/// lengths of the wires on that sheet whose declared net equals the row's net,
/// in the document unit, and `Wires` counts those segments. `Unrouted` is `yes`
/// when no wire of the sheet names the net, so a net that only exists as node
/// membership is listed instead of assumed drawn. A bus is never expanded here:
/// the declaration has no geometry of its own, and the length of each member is
/// reported through the net that carries the member name.
///
/// The returned tuple is the CSV, its data-row count, how many rows are
/// unrouted, the number of contributing wires and the total length.
fn render_net_length_csv(design: &Design) -> (String, usize, usize, usize, f64) {
    let mut text = String::from("Sheet,Net,Length,Wires,Unrouted\n");
    let mut rows = 0usize;
    let mut unrouted = 0usize;
    let mut wires = 0usize;
    let mut total_length = 0.0;
    for sheet in &design.sheets {
        for net in &sheet.nets {
            let segments: Vec<&Wire> =
                sheet.wires.iter().filter(|wire| wire.net.as_deref() == Some(net.name.as_str())).collect();
            let length: f64 = segments.iter().map(|wire| segment_length(wire.from, wire.to)).sum();
            let count = segments.len();
            let is_unrouted = count == 0;
            text.push_str(&format!(
                "{},{},{},{},{}\n",
                csv_field(&sheet.name),
                csv_field(&net.name),
                format_number(length),
                count,
                if is_unrouted { "yes" } else { "no" }
            ));
            rows += 1;
            wires += count;
            total_length += length;
            if is_unrouted {
                unrouted += 1;
            }
        }
    }
    (text, rows, unrouted, wires, total_length)
}

/// Resolves the net class that claims one net name.
///
/// An explicit net assignment wins; otherwise a declared bus assigns the net
/// when the class names the bus and the net is that bus, one of its aliases or
/// one of its members. Classes are visited in declaration order, so the first
/// claim is stable.
fn net_class_for_net<'a>(design: &'a Design, name: &str) -> Option<&'a NetClass> {
    design.rules.net_classes.iter().find(|class| {
        class.nets.iter().any(|net| net == name)
            || design.buses.iter().any(|bus| {
                class.buses.contains(&bus.name)
                    && (bus.name == name
                        || bus.aliases.iter().any(|alias| alias == name)
                        || (0..bus.width).any(|index| format!("{}{index}", bus.name) == name))
            })
    })
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

/// One pin of a rendered schematic symbol.
struct PinShape {
    /// Pin name printed inside the body.
    name: String,
    /// Outer end of the stub, where a wire connects.
    end: [f64; 2],
    /// Point where the stub meets the body.
    root: [f64; 2],
    /// Whether the pin sits on the left side of the body.
    left: bool,
}

/// Half height of a component body; it grows so every pin keeps one pitch of
/// clearance inside the outline.
fn component_half_height(component: &Component) -> f64 {
    let rows = component.pins.len().div_ceil(2) as f64;
    COMPONENT_HALF_HEIGHT.max(rows * PIN_PITCH / 2.0 + PIN_PITCH)
}

/// Mirrors one local point about the component centre, before rotation.
///
/// `x` flips left-right about the vertical centre line, `y` flips top-bottom
/// about the horizontal one; any other value, including `None`, keeps the
/// point. The projections feed body artwork and pin stubs through this helper
/// so a mirrored symbol draws flipped while its texts stay readable.
fn mirror_local(component: &Component, point: [f64; 2]) -> [f64; 2] {
    match component.mirror.as_deref() {
        Some("x") => [2.0 * component.x - point[0], point[1]],
        Some("y") => [point[0], 2.0 * component.y - point[1]],
        _ => point,
    }
}

/// Pins of one symbol, split over the left and right sides on a fixed pitch.
///
/// The model stores pin names only, so the projection owns this layout. Left
/// pins take the first half of the list, right pins the rest, and every stub is
/// [`PIN_LENGTH`] long so wires can attach to [`PinShape::end`] without
/// overlapping the body. A component mirrored on `x` swaps the two sides, so
/// the [`PinShape::left`] flag and the stub directions follow the mirror.
fn component_pins(component: &Component) -> Vec<PinShape> {
    let count = component.pins.len();
    if count == 0 {
        return Vec::new();
    }
    let flipped = component.mirror.as_deref() == Some("x");
    let left_count = count.div_ceil(2);
    let right_count = count - left_count;
    let mut pins = Vec::with_capacity(count);
    for (index, name) in component.pins.iter().enumerate() {
        let left = index < left_count;
        let side_index = if left { index } else { index - left_count };
        let side_count = if left { left_count } else { right_count };
        let offset = (side_index as f64 - (side_count as f64 - 1.0) / 2.0) * PIN_PITCH;
        let y = component.y + offset;
        let root_x = if left { component.x - COMPONENT_HALF_WIDTH } else { component.x + COMPONENT_HALF_WIDTH };
        let end_x = if left { root_x - PIN_LENGTH } else { root_x + PIN_LENGTH };
        let root = mirror_local(component, [root_x, y]);
        let end = mirror_local(component, [end_x, y]);
        pins.push(PinShape { name: name.clone(), end, root, left: left != flipped });
    }
    pins
}

/// Rotates one point around the component origin.
fn rotate_about(component: &Component, point: [f64; 2]) -> [f64; 2] {
    let angle = component.rotation.to_radians();
    let (sin, cos) = (angle.sin(), angle.cos());
    let dx = point[0] - component.x;
    let dy = point[1] - component.y;
    [component.x + dx * cos - dy * sin, component.y + dx * sin + dy * cos]
}

/// Axis-aligned bounds of a component body and its pins, after rotation.
fn component_extents(component: &Component) -> ([f64; 2], [f64; 2]) {
    let half_height = component_half_height(component);
    let mut points = vec![
        [component.x - COMPONENT_HALF_WIDTH, component.y - half_height],
        [component.x + COMPONENT_HALF_WIDTH, component.y - half_height],
        [component.x + COMPONENT_HALF_WIDTH, component.y + half_height],
        [component.x - COMPONENT_HALF_WIDTH, component.y + half_height],
    ];
    for pin in component_pins(component) {
        points.push(pin.root);
        points.push(pin.end);
    }
    let mut min = [f64::INFINITY, f64::INFINITY];
    let mut max = [f64::NEG_INFINITY, f64::NEG_INFINITY];
    for point in points {
        let point = rotate_about(component, point);
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    (min, max)
}

/// Axis-aligned bounds of the painted symbol: its body and artwork plus every
/// pin stub, after mirroring and rotation.
///
/// Unlike [`component_extents`] this ignores the invisible generic box around
/// two-pin parts, so reference and value labels can sit just outside the
/// visible artwork instead of floating at the box margin.
fn component_visual_extents(component: &Component) -> ([f64; 2], [f64; 2]) {
    let (artwork, keeps_box) = symbol_artwork(component);
    let inside_box = keeps_box || artwork.is_empty();
    let mut points = Vec::new();
    if inside_box {
        let half_height = component_half_height(component);
        points.extend([
            [component.x - COMPONENT_HALF_WIDTH, component.y - half_height],
            [component.x + COMPONENT_HALF_WIDTH, component.y - half_height],
            [component.x + COMPONENT_HALF_WIDTH, component.y + half_height],
            [component.x - COMPONENT_HALF_WIDTH, component.y + half_height],
        ]);
    }
    for primitive in &artwork {
        match primitive {
            SymbolPrimitive::Stroke(primitive_points) | SymbolPrimitive::Fill(primitive_points) => {
                points.extend(primitive_points.iter().copied());
            }
        }
    }
    for pin in component_pins(component) {
        points.push(pin.root);
        points.push(pin.end);
    }
    let mut min = [f64::INFINITY, f64::INFINITY];
    let mut max = [f64::NEG_INFINITY, f64::NEG_INFINITY];
    for point in points {
        let point = rotate_about(component, point);
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    (min, max)
}

/// One horizontally painted reference or value label.
#[derive(Clone, Copy)]
struct ComponentLabel {
    /// Text baseline x.
    x: f64,
    /// Text baseline y.
    y: f64,
    /// SVG text anchor.
    anchor: &'static str,
    /// Font size in design units.
    size: f64,
}

impl ComponentLabel {
    /// The painted box `[min_x, min_y, max_x, max_y]` of the label.
    fn bounds(self, text: &str) -> [f64; 4] {
        let width = text.chars().count() as f64 * self.size * 0.6;
        let left = match self.anchor {
            "middle" => self.x - width / 2.0,
            "end" => self.x - width,
            _ => self.x,
        };
        [left, self.y - self.size * 0.75, left + width, self.y + self.size * 0.25]
    }
}

/// Places the reference and value labels outside the visible symbol.
///
/// Wide symbols prefer the free space above the reference and below the value;
/// tall (rotated) symbols stack both labels horizontally to the right so the
/// text never rotates with the body. Every candidate is checked against the
/// symbol bounds and the label already placed, so a label that would touch a
/// pin, the body or its sibling moves to the next side.
fn component_labels(component: &Component) -> (ComponentLabel, ComponentLabel) {
    let (min, max) = component_visual_extents(component);
    let (span_x, span_y) = (max[0] - min[0], max[1] - min[1]);
    let center_x = (min[0] + max[0]) / 2.0;
    let center_y = (min[1] + max[1]) / 2.0;
    let body = [min[0], min[1], max[0], max[1]];
    let overlaps = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    let top = ComponentLabel { x: center_x, y: min[1] - 1.96, anchor: "middle", size: 3.2 };
    let bottom = ComponentLabel { x: center_x, y: max[1] + 1.96, anchor: "middle", size: 2.6 };
    let right_reference = ComponentLabel { x: max[0] + 1.2, y: center_y - 0.6, anchor: "start", size: 3.2 };
    let right_value = ComponentLabel { x: max[0] + 1.2, y: center_y + 3.0, anchor: "start", size: 2.6 };
    let left_reference = ComponentLabel { x: min[0] - 1.2, y: center_y - 0.6, anchor: "end", size: 3.2 };
    let left_value = ComponentLabel { x: min[0] - 1.2, y: center_y + 3.0, anchor: "end", size: 2.6 };
    let reference = if span_x >= span_y {
        let candidate = if overlaps(top.bounds(&component.reference), body) { bottom } else { top };
        let candidate = if overlaps(candidate.bounds(&component.reference), body) { right_reference } else { candidate };
        if overlaps(candidate.bounds(&component.reference), body) { left_reference } else { candidate }
    }
    else {
        let candidate =
            if overlaps(right_reference.bounds(&component.reference), body) { left_reference } else { right_reference };
        if overlaps(candidate.bounds(&component.reference), body) { top } else { candidate }
    };
    let reference_box = reference.bounds(&component.reference);
    let value_candidates: [ComponentLabel; 4] =
        if span_x >= span_y { [bottom, top, right_value, left_value] } else { [right_value, left_value, bottom, top] };
    let value = value_candidates
        .into_iter()
        .find(|candidate| {
            !overlaps(candidate.bounds(&component.value), body) && !overlaps(candidate.bounds(&component.value), reference_box)
        })
        .unwrap_or(bottom);
    (reference, value)
}

/// Axis-aligned boxes of the reference, value and pin-name texts of one
/// component, in model coordinates.
///
/// The boxes mirror exactly where [`push_svg_component`] paints the texts, so
/// collision avoidance can reason about the printed labels instead of the
/// component outline alone. Each box is `[min_x, min_y, max_x, max_y]`.
fn component_text_boxes(component: &Component) -> Vec<[f64; 4]> {
    let (artwork, keeps_box) = symbol_artwork(component);
    let inside_box = keeps_box || artwork.is_empty();
    let (reference, value) = component_labels(component);
    let mut boxes = vec![reference.bounds(&component.reference), value.bounds(&component.value)];
    for pin in component_pins(component) {
        let anchor = rotate_about(
            component,
            [
                if pin.left { pin.root[0] + 0.9 } else { pin.root[0] - 0.9 },
                if inside_box { pin.root[1] + 0.7 } else { pin.root[1] - 1.2 },
            ],
        );
        let width = pin.name.chars().count() as f64 * PIN_NAME_SIZE * 0.6;
        let left = if pin.left { anchor[0] } else { anchor[0] - width };
        boxes.push([left, anchor[1] - PIN_NAME_SIZE * 0.75, left + width, anchor[1] + PIN_NAME_SIZE * 0.25]);
    }
    boxes
}

/// Schematic body class derived from the reference prefix.
#[derive(Clone, Copy)]
enum SymbolClass {
    Resistor,
    Capacitor,
    Inductor,
    Diode,
    /// Three-pin `Q`/`J` part: the enclosing circle replaces the generic box.
    Transistor,
    /// Generic multi-pin and `U` parts, painted exactly as before.
    Box,
}

/// One unrotated body primitive in design units, centred on the component.
enum SymbolPrimitive {
    /// Open polyline stroked in the symbol colour.
    Stroke(Vec<[f64; 2]>),
    /// Closed polygon filled and stroked in the symbol colour.
    Fill(Vec<[f64; 2]>),
}

/// Classifies one component from its first alphabetic reference character.
///
/// Only two-pin `R`/`C`/`L`/`D` parts and three-pin `Q`/`J` transistors leave
/// the generic box, so pin connection points never move for any other part.
fn symbol_class(component: &Component) -> SymbolClass {
    let prefix =
        component.reference.chars().find(|character| character.is_alphabetic()).map(|character| character.to_ascii_uppercase());
    match (component.pins.len(), prefix) {
        (2, Some('R')) => SymbolClass::Resistor,
        (2, Some('C')) => SymbolClass::Capacitor,
        (2, Some('L')) => SymbolClass::Inductor,
        (2, Some('D')) => SymbolClass::Diode,
        (3, Some('Q')) | (3, Some('J')) => SymbolClass::Transistor,
        _ => SymbolClass::Box,
    }
}

/// Body artwork for one symbol, with the component's mirror applied.
///
/// Every primitive point is fed through [`mirror_local`] so a mirrored symbol
/// draws flipped in the same local frame [`component_pins`] uses; the rotation
/// itself stays with the callers, which apply [`rotate_about`] or an SVG
/// `rotate` group exactly as before.
fn symbol_artwork(component: &Component) -> (Vec<SymbolPrimitive>, bool) {
    let (mut primitives, keeps_box) = symbol_artwork_unmirrored(component);
    if component.mirror.is_some() {
        for primitive in &mut primitives {
            let points = match primitive {
                SymbolPrimitive::Stroke(points) | SymbolPrimitive::Fill(points) => points,
            };
            for point in points {
                *point = mirror_local(component, *point);
            }
        }
    }
    (primitives, keeps_box)
}

/// Body artwork for one symbol class, in unrotated design units centred on
/// the component origin, and whether the generic box is still painted.
///
/// The two-pin bodies and their leads span about ten grid units between the
/// [`PIN_LENGTH`] stub roots, so the stubs and their [`PinShape::end`]
/// connection points stay exactly as [`component_pins`] places them.
fn symbol_artwork_unmirrored(component: &Component) -> (Vec<SymbolPrimitive>, bool) {
    let cx = component.x;
    let cy = component.y;
    let left_root = [cx - COMPONENT_HALF_WIDTH, cy];
    let right_root = [cx + COMPONENT_HALF_WIDTH, cy];
    let body = 5.08;
    match symbol_class(component) {
        SymbolClass::Box => (Vec::new(), false),
        SymbolClass::Transistor => {
            let radius = COMPONENT_HALF_WIDTH + PIN_LENGTH + 0.64;
            let ring = (0..=24)
                .map(|step| {
                    let angle = std::f64::consts::TAU * step as f64 / 24.0;
                    [cx + radius * angle.cos(), cy + radius * angle.sin()]
                })
                .collect();
            (vec![SymbolPrimitive::Stroke(ring)], false)
        }
        SymbolClass::Resistor => {
            let zigzag = vec![
                [cx - body, cy],
                [cx - 3.81, cy - 2.54],
                [cx - 1.27, cy + 2.54],
                [cx + 1.27, cy - 2.54],
                [cx + 3.81, cy + 2.54],
                [cx + body, cy],
            ];
            let primitives = vec![
                SymbolPrimitive::Stroke(vec![left_root, [cx - body, cy]]),
                SymbolPrimitive::Stroke(zigzag),
                SymbolPrimitive::Stroke(vec![[cx + body, cy], right_root]),
            ];
            (primitives, false)
        }
        SymbolClass::Capacitor => {
            let plate = 1.27;
            let mut primitives = vec![
                SymbolPrimitive::Stroke(vec![left_root, [cx - plate, cy]]),
                SymbolPrimitive::Stroke(vec![[cx - plate, cy - 2.54], [cx - plate, cy + 2.54]]),
                SymbolPrimitive::Stroke(vec![[cx + plate, cy - 2.54], [cx + plate, cy + 2.54]]),
                SymbolPrimitive::Stroke(vec![[cx + plate, cy], right_root]),
            ];
            if component.value.contains('+') {
                primitives.push(SymbolPrimitive::Stroke(vec![[cx - 5.08, cy - 3.81], [cx - 2.54, cy - 3.81]]));
                primitives.push(SymbolPrimitive::Stroke(vec![[cx - 3.81, cy - 5.08], [cx - 3.81, cy - 2.54]]));
            }
            (primitives, false)
        }
        SymbolClass::Inductor => {
            let mut coil = vec![[cx - body, cy]];
            for bump in 0..4 {
                for step in 1..=12 {
                    let angle = std::f64::consts::PI * (12.0 - step as f64) / 12.0;
                    coil.push([cx - body + 2.54 * bump as f64 + 1.27 + 1.27 * angle.cos(), cy - 1.27 * angle.sin()]);
                }
            }
            let primitives = vec![
                SymbolPrimitive::Stroke(vec![left_root, [cx - body, cy]]),
                SymbolPrimitive::Stroke(coil),
                SymbolPrimitive::Stroke(vec![[cx + body, cy], right_root]),
            ];
            (primitives, false)
        }
        SymbolClass::Diode => {
            let mut primitives = vec![
                SymbolPrimitive::Stroke(vec![left_root, [cx - 2.54, cy]]),
                SymbolPrimitive::Fill(vec![[cx - 2.54, cy - 2.54], [cx - 2.54, cy + 2.54], [cx + 2.54, cy]]),
                SymbolPrimitive::Stroke(vec![[cx + 2.54, cy - 2.54], [cx + 2.54, cy + 2.54]]),
                SymbolPrimitive::Stroke(vec![[cx + 2.54, cy], right_root]),
            ];
            if component.value.to_ascii_uppercase().contains("LED") {
                for offset in [0.0, 1.27] {
                    let tip = [cx + 3.81 + offset, cy - 5.08];
                    primitives.push(SymbolPrimitive::Stroke(vec![[cx + 1.27 + offset, cy - 2.54], tip]));
                    primitives.push(SymbolPrimitive::Stroke(vec![[tip[0] - 1.27, tip[1]], tip]));
                    primitives.push(SymbolPrimitive::Stroke(vec![[tip[0], tip[1] + 1.27], tip]));
                }
            }
            (primitives, false)
        }
    }
}

/// Paints one body primitive inside the component's already-rotated SVG group.
fn push_svg_symbol(svg: &mut String, primitive: &SymbolPrimitive) {
    let (points, filled) = match primitive {
        SymbolPrimitive::Stroke(points) => (points, false),
        SymbolPrimitive::Fill(points) => (points, true),
    };
    let rendered = points.iter().map(|point| format!("{:.2},{:.2}", point[0], point[1])).collect::<Vec<_>>().join(" ");
    let (tag, fill) = if filled { ("polygon", "#1f6feb") } else { ("polyline", "none") };
    svg.push_str(&format!("      <{tag} points=\"{rendered}\" fill=\"{fill}\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n"));
}

/// Projects exactly the listed sheets onto one SVG document.
///
/// The export writer passes every sheet unless the caller selects one, so the
/// default drawing keeps every sheet and a selected export paints only that
/// sheet.
fn render_svg_for(design: &Design, sheets: &[&Sheet]) -> String {
    svg_projection(design, sheets).document
}

/// Axis-aligned bounds of everything the SVG and PDF projections paint.
///
/// Returns `(min_x, min_y, max_x, max_y)` in design units. An empty sheet set
/// keeps the placeholder extent so a renderer still has a finite viewport.
pub(crate) fn drawing_bounds<'a>(sheets: impl IntoIterator<Item = &'a Sheet>) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for sheet in sheets {
        for component in &sheet.components {
            any = true;
            let (min, max) = component_extents(component);
            min_x = min_x.min(min[0]);
            max_x = max_x.max(max[0]);
            min_y = min_y.min(min[1]);
            max_y = max_y.max(max[1]);
            for text in component_text_boxes(component) {
                min_x = min_x.min(text[0]);
                max_x = max_x.max(text[2]);
                min_y = min_y.min(text[1]);
                max_y = max_y.max(text[3]);
            }
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
            let width = text.text.chars().count() as f64 * 3.2 * 0.6;
            min_x = min_x.min(text.x);
            max_x = max_x.max(text.x + width);
            min_y = min_y.min(text.y - 3.2);
            max_y = max_y.max(text.y);
        }
        for label in &sheet.labels {
            any = true;
            let width = label.text.chars().count() as f64 * 2.8 * 0.6 + 2.0;
            min_x = min_x.min(label.x);
            max_x = max_x.max(label.x + width);
            min_y = min_y.min(label.y - 4.0);
            max_y = max_y.max(label.y + 1.0);
        }
        for flag in &sheet.no_connects {
            any = true;
            min_x = min_x.min(flag.x - 1.0);
            max_x = max_x.max(flag.x + 1.0);
            min_y = min_y.min(flag.y - 1.0);
            max_y = max_y.max(flag.y + 1.0);
        }
        for (net, (from, to)) in net_name_anchors(sheet) {
            any = true;
            let [name_x, name_y] = net_name_point(sheet, from, to, &net);
            let half = net.chars().count() as f64 * 2.2 * 0.3;
            min_x = min_x.min(name_x - half);
            max_x = max_x.max(name_x + half);
            min_y = min_y.min(name_y - 2.2);
            max_y = max_y.max(name_y);
        }
    }
    if any { (min_x, min_y, max_x, max_y) } else { (0.0, 0.0, 100.0, 60.0) }
}

/// Height of the title-block strip reserved below the schematic content, in
/// design units. The strip carries the sheet name, design title, revision and
/// counts instead of a bare border.
pub(crate) const TITLE_BLOCK_HEIGHT: f64 = 14.0;

/// Projects the given sheets onto one SVG document.
///
/// The export writer passes every sheet unless a sheet is selected; the preview
/// passes exactly one, so the viewport and the painted geometry always describe
/// the same sheet set. The design supplies the title-block text; only the
/// listed sheets are painted. More than one sheet is stacked into separate
/// page regions instead of sharing one coordinate space.
fn svg_projection(design: &Design, sheets: &[&Sheet]) -> SvgProjection {
    if sheets.len() > 1 {
        return svg_paged_projection(design, sheets);
    }
    let (min_x, min_y, max_x, max_y) = drawing_bounds(sheets.iter().copied());
    // Reserve at least 8% of the final page width on every side. Solving
    // content + 2 * (0.08 * page) = page keeps the frame margin honest for
    // wide and narrow designs alike.
    let content_width = (max_x - min_x).max(1.0);
    let padding = content_width * 0.08 / 0.84;
    let view_x = min_x - padding;
    let view_y = min_y - padding;
    let view_width = content_width + 2.0 * padding;
    let view_height = (max_y - min_y).max(1.0) + 2.0 * padding + TITLE_BLOCK_HEIGHT;
    // The SVG viewBox counts millimetres. A standalone file opens at eight
    // pixels per millimetre so pin labels and references stay legible instead
    // of rendering as a postage stamp.
    let width = (view_width * 8.0).round();
    let height = (view_height * 8.0).round();

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
        push_svg_sheet_contents(&mut svg, sheet);
        svg.push_str("  </g>\n");
    }
    push_svg_title_block(&mut svg, design, sheets, view_x, view_y, view_width, view_height);
    svg.push_str("</svg>\n");
    SvgProjection { document: svg, width, height }
}

/// Projects several sheets onto one SVG document as stacked page regions.
///
/// Sheets routinely place parts at the same design coordinates, so painting
/// them into one shared space overlaps them. Instead every sheet is translated
/// into its own vertical band — separator frame, sheet-name header and the
/// shared sheet painter at unit scale — mirroring the one-page-per-sheet PDF
/// behaviour while staying a single vector document.
fn svg_paged_projection(design: &Design, sheets: &[&Sheet]) -> SvgProjection {
    let bounds: Vec<(f64, f64, f64, f64)> = sheets.iter().map(|sheet| drawing_bounds(std::iter::once(*sheet))).collect();
    let header = 6.0;
    let gap = 10.0;
    let content_width = bounds.iter().map(|bound| bound.2 - bound.0).fold(1.0, f64::max);
    let padding = content_width * 0.08 / 0.84;
    let view_x = -padding;
    let view_width = content_width + 2.0 * padding;
    // One full padding above the first band and one below the last, so every
    // sheet keeps the same eight-percent margin from the page border.
    let mut view_height = 2.0 * padding + TITLE_BLOCK_HEIGHT;
    for bound in &bounds {
        // Two extra units keep text descenders off the band frame.
        view_height += header + (bound.3 - bound.1).max(1.0) + 2.0 + gap;
    }
    let width = (view_width * 8.0).round();
    let height = (view_height * 8.0).round();

    let mut svg = String::new();
    svg.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{view_x:.2} {view_y:.2} {view_width:.2} {view_height:.2}\" width=\"{width:.0}\" height=\"{height:.0}\">\n",
        view_y = -padding,
    ));
    svg.push_str(&format!(
        "  <rect x=\"{view_x:.2}\" y=\"{:.2}\" width=\"{view_width:.2}\" height=\"{view_height:.2}\" fill=\"#ffffff\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
        -padding,
    ));
    let mut region_top = -padding;
    for (sheet, bound) in sheets.iter().zip(&bounds) {
        let (min_x, min_y, _, max_y) = *bound;
        let span_y = (max_y - min_y).max(1.0) + 2.0;
        svg.push_str(&format!(
            "  <rect x=\"{view_x:.2}\" y=\"{region_top:.2}\" width=\"{view_width:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#64748b\" stroke-width=\"0.4\" stroke-dasharray=\"2.4 1.6\"/>\n",
            header + span_y,
        ));
        svg.push_str(&format!(
            "  <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" fill=\"#1f2933\">{}</text>\n",
            view_x + 2.0,
            region_top + 4.4,
            escape_xml(&drawings::fit_text(&sheet.name, view_width - 4.0, 3.2)),
        ));
        svg.push_str(&format!(
            "  <g data-sheet=\"{}\" transform=\"translate({:.2} {:.2})\">\n",
            escape_xml(&sheet.id),
            -min_x,
            region_top + header - min_y,
        ));
        push_svg_sheet_contents(&mut svg, sheet);
        svg.push_str("  </g>\n");
        region_top += header + span_y + gap;
    }
    push_svg_title_block(&mut svg, design, sheets, view_x, -padding, view_width, view_height);
    svg.push_str("</svg>\n");
    SvgProjection { document: svg, width, height }
}

/// Paints one sheet's wires, junctions, no-connect flags, net annotations,
/// labels, texts and components into an SVG document at design coordinates.
///
/// Shared by the sheet projection and the multi-sheet overview drawing so both
/// paint byte-identical primitives; the caller owns the surrounding group.
pub(crate) fn push_svg_sheet_contents(svg: &mut String, sheet: &Sheet) {
    let pin_key = |point: [f64; 2]| -> (i64, i64) { ((point[0] * 100.0).round() as i64, (point[1] * 100.0).round() as i64) };
    let mut connected: BTreeSet<(i64, i64)> = BTreeSet::new();
    for wire in &sheet.wires {
        connected.insert(pin_key(wire.from));
        connected.insert(pin_key(wire.to));
    }
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
    for flag in &sheet.no_connects {
        svg.push_str(&format!(
                "    <path d=\"M {:.2} {:.2} L {:.2} {:.2} M {:.2} {:.2} L {:.2} {:.2}\" stroke=\"#b91c1c\" stroke-width=\"0.35\" fill=\"none\" data-no-connect=\"true\"/>\n",
                flag.x - 0.7,
                flag.y - 0.7,
                flag.x + 0.7,
                flag.y + 0.7,
                flag.x - 0.7,
                flag.y + 0.7,
                flag.x + 0.7,
                flag.y - 0.7
            ));
    }
    let labelled: BTreeSet<String> = sheet.labels.iter().map(|label| label.text.to_lowercase()).collect();
    for (net, (from, to)) in net_name_anchors(sheet) {
        if labelled.contains(&net.to_lowercase()) {
            continue;
        }
        let [mid_x, mid_y] = net_name_point(sheet, from, to, &net);
        let half = net.chars().count() as f64 * 2.2 * 0.3;
        svg.push_str(&format!(
            "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"2.8\" fill=\"#ffffff\" data-net-name-box=\"true\"/>\n",
            mid_x - half - 0.4,
            mid_y - 2.1,
            half * 2.0 + 0.8
        ));
        svg.push_str(&format!(
                "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"middle\" fill=\"#64748b\" data-net=\"{}\">{}</text>\n",
                mid_x,
                mid_y,
                escape_xml(&net),
                escape_xml(&net)
            ));
    }
    let mut painted_labels: BTreeSet<(String, String)> = BTreeSet::new();
    for label in &sheet.labels {
        let key = (format!("{:?}", label.scope), label.text.clone());
        if !painted_labels.insert(key) {
            continue;
        }
        let width = label.text.chars().count() as f64 * 2.8 * 0.6;
        match label.scope {
            LabelScope::Local => {
                svg.push_str(&format!(
                    "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"4.2\" fill=\"#ffffff\"/>\n",
                    label.x - 0.4,
                    label.y - 3.4,
                    width + 0.8
                ));
                svg.push_str(&format!(
                        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.8\" text-anchor=\"start\" fill=\"#0f766e\">{}</text>\n",
                        label.x,
                        label.y,
                        escape_xml(&label.text)
                    ));
                svg.push_str(&format!(
                    "    <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#0f766e\" stroke-width=\"0.3\"/>\n",
                    label.x,
                    label.y + 0.6,
                    label.x + width,
                    label.y + 0.6
                ));
            }
            LabelScope::Global => {
                svg.push_str(&format!(
                        "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"4.2\" rx=\"0.8\" fill=\"#ffffff\" stroke=\"#b91c1c\" stroke-width=\"0.35\"/>\n",
                        label.x - 0.6,
                        label.y - 3.4,
                        width + 1.2
                    ));
                svg.push_str(&format!(
                        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.8\" text-anchor=\"start\" fill=\"#b91c1c\">{}</text>\n",
                        label.x,
                        label.y,
                        escape_xml(&label.text)
                    ));
            }
            LabelScope::Hierarchical => {
                svg.push_str(&format!(
                        "    <polygon points=\"{:.2},{:.2} {:.2},{:.2} {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}\" fill=\"#ffffff\" stroke=\"#15803d\" stroke-width=\"0.35\"/>\n",
                        label.x,
                        label.y + 0.8,
                        label.x + 1.6,
                        label.y - 1.3,
                        label.x,
                        label.y - 3.4,
                        label.x + width + 1.2,
                        label.y - 3.4,
                        label.x + width + 1.2,
                        label.y + 0.8
                    ));
                svg.push_str(&format!(
                        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.8\" text-anchor=\"start\" fill=\"#15803d\">{}</text>\n",
                        label.x + 1.8,
                        label.y,
                        escape_xml(&label.text)
                    ));
            }
        }
    }
    for text in &sheet.texts {
        let width = text.text.chars().count() as f64 * 3.2 * 0.6;
        svg.push_str(&format!(
            "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"4.6\" fill=\"#ffffff\" data-text-box=\"true\"/>\n",
            text.x - 0.5,
            text.y - 3.6,
            width + 1.0
        ));
        svg.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" text-anchor=\"start\">{}</text>\n",
            text.x,
            text.y,
            escape_xml(&text.text)
        ));
    }
    for component in &sheet.components {
        push_svg_component_marked(svg, component, Some(&connected));
    }
}

/// Paints one component symbol and suppresses pin dots that coincide with a
/// wire endpoint in `connected` (rounded to 0.01 unit). `None` marks every pin.
pub(crate) fn push_svg_component_marked(svg: &mut String, component: &Component, connected: Option<&BTreeSet<(i64, i64)>>) {
    let (artwork, keeps_box) = symbol_artwork(component);
    let inside_box = keeps_box || artwork.is_empty();
    let (reference, value) = component_labels(component);
    svg.push_str(&format!(
        "    <g transform=\"rotate({:.2} {:.2} {:.2})\" data-ref=\"{}\">\n",
        component.rotation,
        component.x,
        component.y,
        escape_xml(&component.reference)
    ));
    if inside_box {
        svg.push_str(&format!(
                    "      <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#f6faff\" stroke=\"#1f6feb\" stroke-width=\"0.5\"/>\n",
                    component.x - COMPONENT_HALF_WIDTH,
                    component.y - component_half_height(component),
                    COMPONENT_HALF_WIDTH * 2.0,
                    component_half_height(component) * 2.0
                ));
    }
    for primitive in &artwork {
        push_svg_symbol(svg, primitive);
    }
    for pin in component_pins(component) {
        svg.push_str(&format!(
            "      <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#1f6feb\" stroke-width=\"0.35\"/>\n",
            pin.root[0], pin.root[1], pin.end[0], pin.end[1]
        ));
        let end = rotate_about(component, pin.end);
        let unconnected =
            connected.is_none_or(|set| !set.contains(&((end[0] * 100.0).round() as i64, (end[1] * 100.0).round() as i64)));
        if unconnected {
            svg.push_str(&format!(
                "      <circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"0.45\" fill=\"#1f6feb\"/>\n",
                pin.end[0], pin.end[1]
            ));
        }
        let (x, anchor) = if pin.left { (pin.root[0] + 0.9, "start") } else { (pin.root[0] - 0.9, "end") };
        let name_y = if inside_box { pin.root[1] + 0.7 } else { pin.root[1] - 1.2 };
        svg.push_str(&format!(
                    "      <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"{PIN_NAME_SIZE:.1}\" text-anchor=\"{anchor}\" fill=\"#334155\" transform=\"rotate({:.2} {:.2} {:.2})\">{}</text>\n",
                    x,
                    name_y,
                    -component.rotation,
                    x,
                    name_y,
                    escape_xml(&pin.name)
                ));
    }
    svg.push_str("    </g>\n");
    for (label, text) in [(reference, component.reference.as_str()), (value, component.value.as_str())] {
        let box_min = label.bounds(text);
        svg.push_str(&format!(
            "    <rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"#ffffff\" data-component-text=\"true\"/>\n",
            box_min[0] - 0.3,
            box_min[1] - 0.2,
            box_min[2] - box_min[0] + 0.6,
            box_min[3] - box_min[1] + 0.4
        ));
        svg.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"{:.1}\" text-anchor=\"{}\">{}</text>\n",
            label.x,
            label.y,
            label.size,
            label.anchor,
            escape_xml(text)
        ));
    }
}

/// One painted net name and the segment it annotates.
type NetAnchor = (String, ([f64; 2], [f64; 2]));

/// Picks one anchor wire per net: the longest segment carrying that net name.
///
/// The net name is painted once at the midpoint of that segment so the sheet
/// reads like an annotated schematic without repeating the label on every
/// wire. Nets without a name (free wires) are skipped. Deterministic: equal
/// lengths keep the first wire in document order.
fn net_name_anchors(sheet: &Sheet) -> Vec<NetAnchor> {
    type Anchor = (String, f64, ([f64; 2], [f64; 2]));
    let mut best: BTreeMap<&str, Anchor> = BTreeMap::new();
    for wire in &sheet.wires {
        let Some(net) = &wire.net
        else {
            continue;
        };
        let length = (wire.to[0] - wire.from[0]).powi(2) + (wire.to[1] - wire.from[1]).powi(2);
        match best.get(net.as_str()) {
            Some((_, longest, _)) if *longest >= length => {}
            _ => {
                best.insert(net.as_str(), (net.clone(), length, (wire.from, wire.to)));
            }
        }
    }
    best.into_values().map(|(net, _, segment)| (net, segment)).collect()
}

/// Baseline point of a net-name annotation in model coordinates.
///
/// The default spot is one unit above the wire midpoint; when that box would
/// collide with a label of any scope or with the reference, value or pin-name
/// text of a placed component, the annotation is lifted one text line per
/// collision (at most three lines) so the grey net name never sits on top of
/// a green local label, a boxed global label or a printed pin name. A vertical
/// anchor wire would otherwise run straight through a centred name, so the
/// annotation is shifted sideways past the wire end instead, clearing the
/// stroke by a label gap.
fn net_name_point(sheet: &Sheet, from: [f64; 2], to: [f64; 2], net: &str) -> [f64; 2] {
    let half = net.chars().count() as f64 * 2.2 * 0.3;
    let vertical = (to[0] - from[0]).abs() < (to[1] - from[1]).abs();
    let name_x = if vertical { (from[0] + to[0]) / 2.0 + half + 0.8 } else { (from[0] + to[0]) / 2.0 };
    let mut baseline = (from[1] + to[1]) / 2.0 - 1.0;
    let collides = |baseline: f64| {
        let label_hit = sheet.labels.iter().any(|label| {
            let width = label.text.chars().count() as f64 * 2.8 * 0.6;
            label.x < name_x + half + 0.4
                && label.x + width > name_x - half - 0.4
                && label.y - 2.8 < baseline
                && baseline - 2.2 < label.y + 0.6
        });
        let text_hit = sheet.components.iter().flat_map(component_text_boxes).any(|box_min| {
            box_min[0] < name_x + half + 0.4
                && box_min[2] > name_x - half - 0.4
                && box_min[1] < baseline + 0.4
                && baseline - 2.2 < box_min[3]
        });
        label_hit || text_hit
    };
    let mut lifts = 0;
    while collides(baseline) && lifts < 3 {
        baseline -= 3.0;
        lifts += 1;
    }
    [name_x, baseline]
}

/// Paints the title-block strip along the bottom of the sheet frame.
///
/// The strip is split into four cells: design title (with company below),
/// sheet identity, revision block and content counts. Every value comes from
/// the document itself, so the projection stays byte-deterministic.
pub(crate) fn push_svg_title_block(
    svg: &mut String,
    design: &Design,
    sheets: &[&Sheet],
    view_x: f64,
    view_y: f64,
    view_width: f64,
    view_height: f64,
) {
    let strip_y = view_y + view_height - TITLE_BLOCK_HEIGHT;
    let components: usize = sheets.iter().map(|sheet| sheet.components.len()).sum();
    let wires: usize = sheets.iter().map(|sheet| sheet.wires.len()).sum();
    let title = design.title.as_deref().filter(|title| !title.trim().is_empty()).unwrap_or(&design.id);
    let sheet_names = sheets.iter().map(|sheet| sheet.name.as_str()).collect::<Vec<_>>().join(", ");
    let rev = if design.rev.trim().is_empty() { "-" } else { design.rev.as_str() };
    let drawing_number = format!("DWG {}", design.id);
    let sheet_count = format!("{} sheet(s)", sheets.len());
    svg.push_str("  <g data-title-block=\"true\">\n");
    svg.push_str(&format!(
        "    <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
        view_x,
        strip_y,
        view_x + view_width,
        strip_y
    ));
    let columns = [0.42, 0.24, 0.14, 0.20];
    let mut cursor = view_x;
    for fraction in &columns[1..] {
        cursor += view_width * fraction;
        svg.push_str(&format!(
            "    <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#1f2933\" stroke-width=\"0.4\"/>\n",
            cursor,
            strip_y,
            cursor,
            view_y + view_height
        ));
    }
    let cell_x = |index: usize| view_x + view_width * columns[..index].iter().sum::<f64>() + 1.5;
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"3.2\" text-anchor=\"start\" fill=\"#1f2933\">{}</text>\n",
        cell_x(0),
        strip_y + 5.0,
        escape_xml(title)
    ));
    let subtitle = if design.company.trim().is_empty() && design.date.trim().is_empty() {
        None
    }
    else {
        let mut parts = Vec::new();
        if !design.company.trim().is_empty() {
            parts.push(design.company.as_str());
        }
        if !design.date.trim().is_empty() {
            parts.push(design.date.as_str());
        }
        Some(parts.join(" | "))
    };
    if let Some(subtitle) = subtitle {
        svg.push_str(&format!(
            "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"start\" fill=\"#64748b\">{}</text>\n",
            cell_x(0),
            strip_y + 9.5,
            escape_xml(&subtitle)
        ));
    }
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.4\" text-anchor=\"start\" fill=\"#1f2933\">{}</text>\n",
        cell_x(1),
        strip_y + 5.0,
        escape_xml(&sheet_names)
    ));
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"start\" fill=\"#64748b\">{} | {}</text>\n",
        cell_x(1),
        strip_y + 9.5,
        escape_xml(&sheet_count),
        escape_xml(&drawings::fit_text(&drawing_number, view_width * columns[1] - 3.0, 2.2))
    ));
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.6\" text-anchor=\"start\" fill=\"#1f2933\">REV {}</text>\n",
        cell_x(2),
        strip_y + 5.0,
        escape_xml(rev)
    ));
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"start\" fill=\"#64748b\">SCALE 1:1 | UNIT {}</text>\n",
        cell_x(2),
        strip_y + 9.5,
        escape_xml(&design.units)
    ));
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"start\" fill=\"#64748b\">{} parts / {} wires</text>\n",
        cell_x(3),
        strip_y + 5.0,
        components,
        wires
    ));
    svg.push_str(&format!(
        "    <text x=\"{:.2}\" y=\"{:.2}\" font-family=\"monospace\" font-size=\"2.2\" text-anchor=\"start\" fill=\"#64748b\">OpenEDA rev {}</text>\n",
        cell_x(3),
        strip_y + 9.5,
        design.revision
    ));
    svg.push_str("  </g>\n");
}

pub(crate) fn escape_xml(text: &str) -> String {
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
            designators.sort_by(|left, right| natural_compare(left, right));
            BomRow { comment, footprint, designators }
        })
        .collect()
}

/// Compares two designators the way a human reads them: digit runs compare by
/// numeric value instead of text, so `R2` precedes `R10` and adjacent numbers
/// stay together for [`designator_ranges`]. Equal numeric runs fall back to the
/// run length, keeping the order total and deterministic.
pub(crate) fn natural_compare(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut left = left;
    let mut right = right;
    loop {
        match (left.chars().next(), right.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let left_len = left.find(|character: char| !character.is_ascii_digit()).unwrap_or(left.len());
                    let right_len = right.find(|character: char| !character.is_ascii_digit()).unwrap_or(right.len());
                    let left_run = &left[..left_len];
                    let right_run = &right[..right_len];
                    let left_digits = left_run.trim_start_matches('0');
                    let right_digits = right_run.trim_start_matches('0');
                    let ordering = left_digits.len().cmp(&right_digits.len()).then_with(|| left_digits.cmp(right_digits));
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                    let ordering = left_len.cmp(&right_len);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                    left = &left[left_len..];
                    right = &right[right_len..];
                }
                else {
                    let ordering = left_char.cmp(&right_char);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                    left = &left[left_char.len_utf8()..];
                    right = &right[right_char.len_utf8()..];
                }
            }
        }
    }
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
    if digits == 0 {
        (reference, None)
    }
    else {
        let split = reference.len() - digits;
        (reference.split_at(split).0, reference[split..].parse::<u64>().ok())
    }
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
    text.push_str("## Objects\n\n| Sheet | Components | Nets | Wires | Junctions | Labels | Texts |\n");
    text.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for sheet in &design.sheets {
        text.push_str(&format!(
            "| {} (`{}`) | {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&sheet.name),
            sheet.id,
            sheet.components.len(),
            sheet.nets.len(),
            sheet.wires.len(),
            sheet.junctions.len(),
            sheet.labels.len(),
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

    if design.sheets.iter().any(|sheet| !sheet.labels.is_empty()) {
        sections += 1;
        text.push_str("## Labels\n");
        for sheet in &design.sheets {
            if sheet.labels.is_empty() {
                continue;
            }
            sections += 1;
            text.push_str(&format!("\n### {} (`{}`)\n\n", markdown_cell(&sheet.name), sheet.id));
            text.push_str("| Label | Scope | Placement |\n| --- | --- | --- |\n");
            for label in &sheet.labels {
                text.push_str(&format!(
                    "| {} | {} | ({}, {}) |\n",
                    markdown_cell(&label.text),
                    label_scope_name(label.scope),
                    format_number(label.x),
                    format_number(label.y)
                ));
            }
        }
        text.push('\n');
    }

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
                "labels": sheet
                    .labels
                    .iter()
                    .map(|label| json!({
                        "id": label.id,
                        "text": label.text,
                        "x": label.x,
                        "y": label.y,
                        "scope": label.scope,
                        "rotation": label.rotation,
                        "justify": label.justify,
                    }))
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

/// Height of the title-block strip reserved at the bottom of a PDF page, in
/// points. The strip mirrors the SVG title block so both exports carry the
/// same sheet identity, revision and counts.
const PDF_TITLE_BLOCK_PT: f64 = 40.0;

/// Lowest point a drawing may reach above the title-block strip, in points.
const PDF_CONTENT_BOTTOM: f64 = PDF_MARGIN + PDF_TITLE_BLOCK_PT;

/// Highest point a drawing may reach below the page header, in points.
const PDF_CONTENT_TOP: f64 = PDF_PAGE_HEIGHT - PDF_MARGIN;

/// Scale and origin that place one drawing inside the printable area.
#[derive(Debug, Clone, Copy)]
struct PdfLayout {
    scale: f64,
    offset_x: f64,
    offset_y: f64,
}

/// Fits `span_x` by `span_y` design units into the printable area and centres
/// the result between the margins and the title-block strip.
fn pdf_layout(span_x: f64, span_y: f64) -> PdfLayout {
    let usable_width = PDF_PAGE_WIDTH - 2.0 * PDF_MARGIN;
    let usable_height = PDF_CONTENT_TOP - PDF_CONTENT_BOTTOM;
    let scale = (usable_width / span_x).min(usable_height / span_y).max(0.001);
    PdfLayout {
        scale,
        offset_x: PDF_MARGIN + (usable_width - span_x * scale) / 2.0,
        offset_y: PDF_CONTENT_BOTTOM + (usable_height - span_y * scale) / 2.0,
    }
}

/// Writes an A4 landscape vector PDF with one page per listed sheet.
///
/// The writer emits the catalog, page tree, font and content streams with a
/// cross-reference table, so the file opens in a viewer instead of only
/// starting with a PDF header. Every page carries its own sheet and keeps the
/// drawing centred between the margins and the title-block strip.
fn render_pdf_for(design: &Design, sheets: &[&Sheet]) -> (String, usize) {
    let mut dropped = 0usize;
    let total = sheets.len().max(1);
    let mut pages = Vec::with_capacity(total);
    if sheets.is_empty() {
        pages.push(pdf_page(design, None, 0, 1, &mut dropped));
    }
    else {
        for (index, sheet) in sheets.iter().enumerate() {
            pages.push(pdf_page(design, Some(*sheet), index, total, &mut dropped));
        }
    }
    (assemble_pdf(&pages), dropped)
}

/// Writes the bill of materials as a paginated A4-landscape PDF table.
///
/// The rows reuse the shared BOM grouping with designators compressed into
/// ranges. Every cell is clipped to its column width so no text can run over
/// the page border or into the neighbouring cell, and each page carries a
/// ruled header, row separators and a footer title line.
fn render_bom_pdf(design: &Design) -> (String, usize, usize) {
    let rows = bom_rows(design, None);
    let parts: usize = rows.iter().map(|row| row.designators.len()).sum();
    let per_page = 24usize;
    let pages_count = rows.len().div_ceil(per_page).max(1);
    let title = design.title.as_deref().map(str::trim).filter(|text| !text.is_empty()).unwrap_or(design.id.as_str());
    let mut dropped = 0usize;
    let mut text_at = |text: &str, size: f64, x: f64, y: f64| -> String {
        let (literal, lost) = pdf_text(text);
        dropped += lost;
        format!("0 0 0 rg BT /F1 {size:.1} Tf {x:.2} {y:.2} Td ({literal}) Tj ET\n")
    };
    let clip = |text: &str, width: f64, size: f64| -> String {
        let max = (width / (size * 0.5)).floor() as usize;
        if text.chars().count() <= max {
            return text.to_string();
        }
        if max <= 3 {
            return text.chars().take(max).collect();
        }
        let mut cut: String = text.chars().take(max - 3).collect();
        cut.push_str("...");
        cut
    };
    let columns: [(f64, f64); 5] = [(40.0, 26.0), (66.0, 124.0), (190.0, 130.0), (320.0, 380.0), (700.0, 60.0)];
    let mut pages: Vec<String> = Vec::new();
    for page in 0..pages_count {
        let slice = &rows[page * per_page..((page + 1) * per_page).min(rows.len())];
        let mut content = String::new();
        content.push_str(&format!("0.12 0.16 0.2 RG 0.4 w 0 0 {PDF_PAGE_WIDTH:.2} {PDF_PAGE_HEIGHT:.2} re S\n"));
        content.push_str(&text_at(
            &format!(
                "OpenEDA {} revision {} | bill of materials (page {} of {})",
                design.id,
                design.revision,
                page + 1,
                pages_count
            ),
            9.0,
            PDF_MARGIN,
            PDF_PAGE_HEIGHT - 24.0,
        ));
        let header_y = PDF_PAGE_HEIGHT - 48.0;
        for (index, label) in ["#", "COMMENT", "FOOTPRINT", "DESIGNATORS", "QTY"].iter().enumerate() {
            content.push_str(&text_at(label, 8.0, columns[index].0, header_y));
        }
        content.push_str(&format!(
            "0.12 0.16 0.2 RG 0.4 w {PDF_MARGIN:.2} {:.2} m {:.2} {:.2} l S\n",
            header_y - 5.0,
            PDF_PAGE_WIDTH - PDF_MARGIN,
            header_y - 5.0
        ));
        if slice.is_empty() {
            content.push_str(&text_at("(no components)", 8.0, columns[1].0, header_y - 20.0));
        }
        for (index, row) in slice.iter().enumerate() {
            let y = header_y - 20.0 - index as f64 * 16.0;
            let designators = clip(&designator_ranges(&row.designators).join(", "), columns[3].1 - 4.0, 8.0);
            let cells = [
                format!("{}", page * per_page + index + 1),
                clip(&row.comment, columns[1].1 - 4.0, 8.0),
                clip(&row.footprint, columns[2].1 - 4.0, 8.0),
                designators,
                format!("{}", row.designators.len()),
            ];
            for (column, text) in cells.iter().enumerate() {
                content.push_str(&text_at(text, 8.0, columns[column].0, y));
            }
            content.push_str(&format!(
                "0.8 0.84 0.88 RG 0.3 w {PDF_MARGIN:.2} {:.2} m {:.2} {:.2} l S\n",
                y - 5.0,
                PDF_PAGE_WIDTH - PDF_MARGIN,
                y - 5.0
            ));
        }
        content.push_str(&format!(
            "0.12 0.16 0.2 RG 0.4 w {PDF_MARGIN:.2} {PDF_CONTENT_BOTTOM:.2} m {:.2} {PDF_CONTENT_BOTTOM:.2} l S\n",
            PDF_PAGE_WIDTH - PDF_MARGIN
        ));
        let footer_y = PDF_CONTENT_BOTTOM - 12.0;
        content.push_str(&text_at(&clip(title, 280.0, 9.0), 9.0, PDF_MARGIN + 4.0, footer_y));
        content.push_str(&text_at(&format!("REV {}", clip(&design.revision.to_string(), 60.0, 9.0)), 9.0, 340.0, footer_y));
        content.push_str(&text_at(&format!("{} rows / {} parts", rows.len(), parts), 9.0, 470.0, footer_y));
        content.push_str(&text_at(&format!("page {} of {}", page + 1, pages_count), 9.0, 700.0, footer_y));
        pages.push(content);
    }
    (assemble_pdf(&pages), dropped, pages_count)
}

/// Breaks a monospace PDF cell into lines of at most `max_chars` characters,
/// preferring a break after a comma or space so a long net member list stays
/// readable and aligned with the surrounding rows.
fn pdf_wrap_cell(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_inclusive([',', ' ']) {
        if !current.is_empty() && current.chars().count() + word.chars().count() > max_chars {
            lines.push(current.trim_end().to_string());
            current.clear();
        }
        if word.chars().count() > max_chars {
            for character in word.chars() {
                if current.chars().count() == max_chars {
                    lines.push(current.trim_end().to_string());
                    current.clear();
                }
                current.push(character);
            }
        }
        else {
            current.push_str(word);
        }
    }
    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Renders the net cross-reference table as paginated A4-landscape pages.
///
/// Each net lists its `REFERENCE.PIN` members wrapped inside the node column
/// and the one-based sheet numbers that name it. Rows are grouped into pages by
/// accumulated line height, never by a guessed row count, so wrapped networks
/// cannot overflow the printable area.
fn render_net_xref_pdf(design: &Design) -> (String, usize, usize, usize) {
    let rows = drawings::net_xref_rows(design);
    let nets = rows.len();
    let mut dropped = 0usize;
    let mut text_at = |text: &str, size: f64, x: f64, y: f64| -> String {
        let (literal, lost) = pdf_text(text);
        dropped += lost;
        format!("0 0 0 rg BT /F1 {size:.1} Tf {x:.2} {y:.2} Td ({literal}) Tj ET\n")
    };
    let title = design.title.as_deref().map(str::trim).filter(|text| !text.is_empty()).unwrap_or(design.id.as_str());
    let node_chars = 118usize;
    let prepared: Vec<(String, Vec<String>, String)> = rows
        .iter()
        .map(|row| {
            let node_lines = pdf_wrap_cell(&row.nodes.join(", "), node_chars);
            let sheets = row.sheets.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
            (row.name.clone(), node_lines, sheets)
        })
        .collect();
    let mut pages_rows: Vec<Vec<usize>> = Vec::new();
    let mut current = Vec::new();
    let mut used = 0.0;
    for (index, (_, node_lines, _)) in prepared.iter().enumerate() {
        let height = node_lines.len().max(1) as f64 * 11.0 + 4.0;
        if !current.is_empty() && used + height > PDF_PAGE_HEIGHT - 150.0 {
            pages_rows.push(std::mem::take(&mut current));
            used = 0.0;
        }
        used += height;
        current.push(index);
    }
    if pages_rows.is_empty() {
        pages_rows.push(current);
    }
    let pages_count = pages_rows.len().max(1);
    let mut pages = Vec::new();
    for (page, indexes) in pages_rows.iter().enumerate() {
        let mut content = String::new();
        content.push_str(&format!("0.12 0.16 0.2 RG 0.4 w 0 0 {PDF_PAGE_WIDTH:.2} {PDF_PAGE_HEIGHT:.2} re S\n"));
        content.push_str(&text_at(
            &format!(
                "OpenEDA {} revision {} | net cross reference (page {} of {})",
                design.id,
                design.revision,
                page + 1,
                pages_count
            ),
            9.0,
            PDF_MARGIN,
            PDF_PAGE_HEIGHT - 24.0,
        ));
        let header_y = PDF_PAGE_HEIGHT - 48.0;
        content.push_str(&text_at("NET", 8.0, 40.0, header_y));
        content.push_str(&text_at("NODES (REFERENCE.PIN)", 8.0, 120.0, header_y));
        content.push_str(&text_at("SHEET", 8.0, 700.0, header_y));
        content.push_str(&format!(
            "0.12 0.16 0.2 RG 0.4 w {PDF_MARGIN:.2} {:.2} m {:.2} {:.2} l S\n",
            header_y - 5.0,
            PDF_PAGE_WIDTH - PDF_MARGIN,
            header_y - 5.0
        ));
        let mut y = header_y - 20.0;
        for index in indexes {
            let (name, node_lines, sheets) = &prepared[*index];
            content.push_str(&text_at(&clip_pdf_text(name, 11.0, 7.0), 7.0, 40.0, y));
            for (line, text) in node_lines.iter().enumerate() {
                content.push_str(&text_at(&clip_pdf_text(text, 72.0, 7.0), 7.0, 120.0, y - line as f64 * 10.0));
            }
            content.push_str(&text_at(sheets, 7.0, 700.0, y));
            y -= node_lines.len().max(1) as f64 * 10.0 + 5.0;
            content.push_str(&format!(
                "0.8 0.84 0.88 RG 0.3 w {PDF_MARGIN:.2} {y:.2} m {:.2} {y:.2} l S\n",
                PDF_PAGE_WIDTH - PDF_MARGIN
            ));
            y -= 3.0;
        }
        content.push_str(&format!(
            "0.12 0.16 0.2 RG 0.4 w {PDF_MARGIN:.2} {PDF_CONTENT_BOTTOM:.2} m {:.2} {PDF_CONTENT_BOTTOM:.2} l S\n",
            PDF_PAGE_WIDTH - PDF_MARGIN
        ));
        let footer_y = PDF_CONTENT_BOTTOM - 12.0;
        content.push_str(&text_at(&clip_pdf_text(title, 280.0, 9.0), 9.0, PDF_MARGIN + 4.0, footer_y));
        content.push_str(&text_at(
            &format!("REV {}", clip_pdf_text(&design.revision.to_string(), 60.0, 9.0)),
            9.0,
            340.0,
            footer_y,
        ));
        content.push_str(&text_at(
            &format!("{nets} nets / {} nodes", rows.iter().map(|row| row.nodes.len()).sum::<usize>()),
            9.0,
            440.0,
            footer_y,
        ));
        content.push_str(&text_at(&format!("page {} of {}", page + 1, pages_count), 9.0, 700.0, footer_y));
        pages.push(content);
    }
    (assemble_pdf(&pages), dropped, pages_count, nets)
}

/// Clips a monospace PDF string to `width` points at `size`, appending dots so
/// a long cell cannot run into the next column.
fn clip_pdf_text(text: &str, width: f64, size: f64) -> String {
    let max = (width / (size * 0.5)).floor() as usize;
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 3 {
        return text.chars().take(max).collect();
    }
    let mut cut: String = text.chars().take(max - 3).collect();
    cut.push_str("...");
    cut
}

/// Builds one page content stream for one sheet.
fn pdf_page(design: &Design, sheet: Option<&Sheet>, index: usize, total: usize, dropped: &mut usize) -> String {
    let (min_x, min_y, max_x, max_y) = match sheet {
        Some(sheet) => drawing_bounds(std::iter::once(sheet)),
        None => (0.0, 0.0, 100.0, 60.0),
    };
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let layout = pdf_layout(span_x, span_y);
    let scale = layout.scale;
    let map_x = |x: f64| layout.offset_x + (x - min_x) * scale;
    let map_y = |y: f64| layout.offset_y + (max_y - y) * scale;
    let label_size = (7.0f64).max((3.2 * scale).min(9.0));

    let mut pdf_literal = |text: &str| {
        let (literal, count) = pdf_text(text);
        *dropped += count;
        literal
    };
    let mut content = String::new();
    content.push_str(&format!("0.12 0.16 0.2 RG 0.4 w 0 0 {PDF_PAGE_WIDTH:.2} {PDF_PAGE_HEIGHT:.2} re S\n"));
    let header = match sheet {
        Some(sheet) => format!(
            "OpenEDA {} revision {} | sheet {}: {} ({}/{})",
            design.id,
            design.revision,
            sheet.id,
            sheet.name,
            index + 1,
            total
        ),
        None => format!("OpenEDA {} revision {}", design.id, design.revision),
    };
    content.push_str(&format!(
        "0 0 0 rg BT /F1 9 Tf {:.2} {:.2} Td {} Tj ET\n",
        PDF_MARGIN,
        PDF_PAGE_HEIGHT - 16.0,
        pdf_literal(&header)
    ));
    for sheet in sheet.iter() {
        for wire in &sheet.wires {
            content.push_str(&format!(
                "0.06 0.46 0.43 RG 0.6 w {:.2} {:.2} m {:.2} {:.2} l S\n",
                map_x(wire.from[0]),
                map_y(wire.from[1]),
                map_x(wire.to[0]),
                map_y(wire.to[1])
            ));
        }
        for junction in &sheet.junctions {
            let side = (1.4 * scale).max(1.2);
            content.push_str(&format!(
                "0 0 0 rg {x:.2} {y:.2} {side:.2} {side:.2} re f\n",
                x = map_x(junction.x) - side / 2.0,
                y = map_y(junction.y) - side / 2.0,
            ));
        }
        for flag in &sheet.no_connects {
            let half = (0.7 * scale).max(2.0);
            content.push_str(&format!(
                "0.7 0.11 0.11 RG 0.5 w {:.2} {:.2} m {:.2} {:.2} l S {:.2} {:.2} m {:.2} {:.2} l S\n",
                map_x(flag.x) - half,
                map_y(flag.y) - half,
                map_x(flag.x) + half,
                map_y(flag.y) + half,
                map_x(flag.x) - half,
                map_y(flag.y) + half,
                map_x(flag.x) + half,
                map_y(flag.y) - half,
            ));
        }
        for (net, (from, to)) in net_name_anchors(sheet) {
            let size = (2.2 * scale).clamp(5.0, 7.0);
            let [name_x, name_y] = net_name_point(sheet, from, to, &net);
            content.push_str(&format!(
                "0.39 0.45 0.55 rg BT /F1 {size:.2} Tf {x:.2} {y:.2} Td {} Tj ET\n",
                pdf_literal(&net),
                x = map_x(name_x) - pdf_text_width(&net, size) / 2.0,
                y = map_y(name_y),
            ));
        }
        for label in &sheet.labels {
            let size = (2.8 * scale).clamp(5.0, 8.0);
            let text_x = map_x(label.x);
            let text_y = map_y(label.y);
            let text_w = pdf_text_width(&label.text, size);
            match label.scope {
                LabelScope::Local => {
                    content.push_str(&format!(
                        "0.06 0.46 0.43 rg BT /F1 {size:.2} Tf {text_x:.2} {text_y:.2} Td {} Tj ET\n",
                        pdf_literal(&label.text)
                    ));
                    content.push_str(&format!(
                        "0.06 0.46 0.43 RG 0.3 w {text_x:.2} {:.2} m {:.2} {:.2} l S\n",
                        text_y - size * 0.214,
                        text_x + text_w,
                        text_y - size * 0.214,
                    ));
                }
                LabelScope::Global => {
                    content.push_str(&format!(
                        "0.72 0.11 0.11 RG 0.35 w {:.2} {:.2} {:.2} {:.2} re S\n",
                        text_x - size * 0.214,
                        text_y - size * 0.286,
                        text_w + size * 0.429,
                        size * 1.5,
                    ));
                    content.push_str(&format!(
                        "0.72 0.11 0.11 rg BT /F1 {size:.2} Tf {text_x:.2} {text_y:.2} Td {} Tj ET\n",
                        pdf_literal(&label.text)
                    ));
                }
                LabelScope::Hierarchical => {
                    content.push_str(&format!(
                        "0.08 0.5 0.24 RG 0.35 w {text_x:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l h S\n",
                        text_y - size * 0.286,
                        text_x + size * 0.571,
                        text_y + size * 0.464,
                        text_x,
                        text_y + size * 1.214,
                        text_x + text_w + size * 0.429,
                        text_y + size * 1.214,
                        text_x + text_w + size * 0.429,
                        text_y - size * 0.286,
                    ));
                    content.push_str(&format!(
                        "0.08 0.5 0.24 rg BT /F1 {size:.2} Tf {:.2} {text_y:.2} Td {} Tj ET\n",
                        text_x + size * 0.643,
                        pdf_literal(&label.text)
                    ));
                }
            }
        }
        for text in &sheet.texts {
            let size = (3.2 * scale).clamp(5.0, 9.0);
            content.push_str(&format!(
                "0 0 0 rg BT /F1 {size:.2} Tf {:.2} {:.2} Td {} Tj ET\n",
                map_x(text.x),
                map_y(text.y),
                pdf_literal(&text.text)
            ));
        }
        for component in &sheet.components {
            let half_height = component_half_height(component);
            let (artwork, keeps_box) = symbol_artwork(component);
            let inside_box = keeps_box || artwork.is_empty();
            if inside_box {
                let corners = [
                    [component.x - COMPONENT_HALF_WIDTH, component.y - half_height],
                    [component.x + COMPONENT_HALF_WIDTH, component.y - half_height],
                    [component.x + COMPONENT_HALF_WIDTH, component.y + half_height],
                    [component.x - COMPONENT_HALF_WIDTH, component.y + half_height],
                ];
                let mut path = String::new();
                for (index, corner) in corners.iter().enumerate() {
                    let point = rotate_about(component, *corner);
                    if index == 0 {
                        path.push_str(&format!("{:.2} {:.2} m", map_x(point[0]), map_y(point[1])));
                    }
                    else {
                        path.push_str(&format!(" {:.2} {:.2} l", map_x(point[0]), map_y(point[1])));
                    }
                }
                content.push_str(&format!("0.12 0.44 0.92 RG 0.7 w {path} h S\n"));
            }
            for primitive in &artwork {
                let (points, closed) = match primitive {
                    SymbolPrimitive::Stroke(points) => (points, false),
                    SymbolPrimitive::Fill(points) => (points, true),
                };
                let mut path = String::new();
                for (index, point) in points.iter().enumerate() {
                    let rotated = rotate_about(component, *point);
                    if index == 0 {
                        path.push_str(&format!("{:.2} {:.2} m", map_x(rotated[0]), map_y(rotated[1])));
                    }
                    else {
                        path.push_str(&format!(" {:.2} {:.2} l", map_x(rotated[0]), map_y(rotated[1])));
                    }
                }
                if closed {
                    content.push_str(&format!("0.12 0.44 0.92 RG 0.12 0.44 0.92 rg 0.6 w {path} h B\n"));
                }
                else {
                    content.push_str(&format!("0.12 0.44 0.92 RG 0.6 w {path} S\n"));
                }
            }
            for pin in component_pins(component) {
                let root = rotate_about(component, pin.root);
                let end = rotate_about(component, pin.end);
                content.push_str(&format!(
                    "0.12 0.44 0.92 RG 0.5 w {:.2} {:.2} m {:.2} {:.2} l S\n",
                    map_x(root[0]),
                    map_y(root[1]),
                    map_x(end[0]),
                    map_y(end[1])
                ));
                content.push_str(&format!(
                    "0.12 0.44 0.92 rg {:.2} {:.2} 1.6 1.6 re f\n",
                    map_x(end[0]) - 0.8,
                    map_y(end[1]) - 0.8,
                ));
                let name_at = rotate_about(
                    component,
                    [
                        if pin.left { pin.root[0] + 0.9 } else { pin.root[0] - 0.9 },
                        if inside_box { pin.root[1] + 0.7 } else { pin.root[1] - 1.2 },
                    ],
                );
                let x =
                    if pin.left { map_x(name_at[0]) } else { map_x(name_at[0]) - pdf_text_width(&pin.name, label_size * 0.7) };
                content.push_str(&format!(
                    "0.2 0.25 0.33 rg BT /F1 {:.2} Tf {x:.2} {:.2} Td {} Tj ET\n",
                    label_size * 0.7,
                    map_y(name_at[1]),
                    pdf_literal(&pin.name)
                ));
            }
            content.push_str(&format!(
                "0 0 0 rg BT /F1 {label_size:.2} Tf {:.2} {:.2} Td {} Tj ET\n",
                map_x(component.x),
                map_y(component.y) + if inside_box { 2.0 } else { 4.5 * scale },
                pdf_literal(&component.reference)
            ));
            if !component.value.is_empty() {
                content.push_str(&format!(
                    "0 0 0 rg BT /F1 {:.2} Tf {:.2} {:.2} Td {} Tj ET\n",
                    label_size * 0.85,
                    map_x(component.x),
                    map_y(component.y) - if inside_box { label_size + 1.0 } else { 4.5 * scale + label_size * 0.85 + 1.0 },
                    pdf_literal(&component.value)
                ));
            }
        }
    }
    push_pdf_title_block(&mut content, design, sheet, index, total, dropped);
    content
}

/// Wraps the page content streams into a complete PDF document.
fn assemble_pdf(pages: &[String]) -> String {
    let page_count = pages.len();
    let font_object = 3 + page_count * 2;
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids = (0..page_count).map(|index| format!("{} 0 R", 3 + index * 2)).collect::<Vec<_>>().join(" ");
    objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>"));
    for (index, content) in pages.iter().enumerate() {
        let page_object = 3 + index * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PDF_PAGE_WIDTH:.0} {PDF_PAGE_HEIGHT:.0}] \
             /Resources << /Font << /F1 {font_object} 0 R >> >> /Contents {} 0 R >>",
            page_object + 1
        ));
        objects.push(format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

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

/// Appends the PDF title-block strip along the bottom of the page.
///
/// The strip mirrors the SVG title block: design title, sheet identity,
/// revision and content counts, all taken from the document so the page stays
/// byte-deterministic. It only names the sheet painted on this page, so a
/// multi-page export never advertises another page's content.
fn push_pdf_title_block(
    content: &mut String,
    design: &Design,
    sheet: Option<&Sheet>,
    page_index: usize,
    page_total: usize,
    dropped: &mut usize,
) {
    let strip_top = PDF_CONTENT_BOTTOM;
    let left = PDF_MARGIN;
    let right = PDF_PAGE_WIDTH - PDF_MARGIN;
    let width = right - left;
    let components = sheet.map_or(0, |sheet| sheet.components.len());
    let wires = sheet.map_or(0, |sheet| sheet.wires.len());
    let title = design.title.as_deref().filter(|title| !title.trim().is_empty()).unwrap_or(&design.id);
    let sheet_name = sheet.map_or("-", |sheet| sheet.name.as_str());
    let rev = if design.rev.trim().is_empty() { "-" } else { design.rev.as_str() };
    content.push_str(&format!("0.4 w 0 0 0 RG {left:.2} {strip_top:.2} m {right:.2} {strip_top:.2} l S\n"));
    let columns = [0.42, 0.24, 0.14, 0.20];
    let mut cursor = left;
    for fraction in &columns[1..] {
        cursor += width * fraction;
        content.push_str(&format!("{:.2} {PDF_MARGIN:.2} m {:.2} {strip_top:.2} l S\n", cursor, cursor));
    }
    let cell_x = |index: usize| left + width * columns[..index].iter().sum::<f64>() + 3.0;
    let mut line = |x: f64, y: f64, size: f64, gray: &str, text: &str| {
        let (literal, count) = pdf_text(text);
        *dropped += count;
        content.push_str(&format!("{gray} {gray} {gray} rg BT /F1 {size:.2} Tf {x:.2} {y:.2} Td {literal} Tj ET\n"));
    };
    line(cell_x(0), strip_top - 12.0, 9.0, "0", title);
    if !design.company.trim().is_empty() || !design.date.trim().is_empty() {
        let mut parts = Vec::new();
        if !design.company.trim().is_empty() {
            parts.push(design.company.as_str());
        }
        if !design.date.trim().is_empty() {
            parts.push(design.date.as_str());
        }
        line(cell_x(0), strip_top - 24.0, 7.0, "0.4", &parts.join(" | "));
    }
    line(cell_x(1), strip_top - 12.0, 7.5, "0", sheet_name);
    line(cell_x(1), strip_top - 24.0, 6.5, "0.4", &format!("page {} of {}", page_index + 1, page_total));
    line(cell_x(2), strip_top - 12.0, 8.0, "0", &format!("REV {rev}"));
    line(cell_x(2), strip_top - 24.0, 6.5, "0.4", &format!("units: {}", design.units));
    line(cell_x(3), strip_top - 12.0, 6.5, "0.4", &format!("{components} parts / {wires} wires"));
    line(cell_x(3), strip_top - 24.0, 6.5, "0.4", &format!("OpenEDA rev {}", design.revision));
}

/// Approximate width of a PDF text run in points.
fn pdf_text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * 0.5
}

/// Escapes a string for a PDF literal string and counts the characters the
/// base-14 font cannot encode; those are written as `?`.
fn pdf_text(text: &str) -> (String, usize) {
    let mut escaped = String::from("(");
    let mut dropped = 0;
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '(' => escaped.push_str("\\("),
            ')' => escaped.push_str("\\)"),
            '\n' | '\r' | '\t' => escaped.push(' '),
            character if character.is_ascii() && !character.is_ascii_control() => escaped.push(character),
            _ => {
                escaped.push('?');
                dropped += 1;
            }
        }
    }
    escaped.push(')');
    (escaped, dropped)
}

fn write_atomic(path: &Path, contents: &str, overwrite: bool) -> Result<usize, ServiceError> {
    write_atomic_bytes(path, contents.as_bytes(), overwrite)
}

/// Writes bytes through a temporary file and a rename, returning the length.
fn write_atomic_bytes(path: &Path, contents: &[u8], overwrite: bool) -> Result<usize, ServiceError> {
    if path.exists() && !overwrite {
        return Err(invalid(format!("`{}` already exists; pass overwrite to replace it", path.display())));
    }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(invalid(format!("directory `{}` does not exist", parent.display())));
    }
    let target =
        if path.parent().is_some_and(|parent| !parent.as_os_str().is_empty()) { path.to_path_buf() } else { parent.join(path) };
    boost_session::save_atomic(&target, contents)
        .map_err(|error| invalid(format!("cannot write `{}`: {error}", target.display())))?;
    Ok(contents.len())
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/wire_split.rs"]
mod wire_split_tests;

#[cfg(test)]
#[path = "../tests/unit/trace.rs"]
mod trace_tests;

#[cfg(test)]
#[path = "../tests/unit/pins.rs"]
mod pin_tests;

#[cfg(test)]
#[path = "../tests/unit/clearance.rs"]
mod clearance_tests;

#[cfg(test)]
#[path = "../tests/unit/bus.rs"]
mod bus_tests;

#[cfg(test)]
#[path = "../tests/unit/net_rename.rs"]
mod net_rename_tests;

#[cfg(test)]
#[path = "../tests/unit/net_audit.rs"]
mod net_audit_tests;

#[cfg(test)]
#[path = "../tests/unit/dangling.rs"]
mod dangling_tests;

#[cfg(test)]
#[path = "../tests/unit/duplicate.rs"]
mod duplicate_tests;

#[cfg(test)]
#[path = "../tests/unit/component_transform.rs"]
mod component_transform_tests;

#[cfg(test)]
#[path = "../tests/unit/drawings.rs"]
mod drawing_tests;
