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
const INSPECT_CHECKS: &[&str] = &["erc", "drc", "nets", "connectivity", "footprints", "clearance", "power"];

/// Rule checks this build actually runs.
const SUPPORTED_CHECKS: &[&str] = &["erc", "nets", "connectivity", "footprints", "power"];

/// Render targets declared for preview surfaces.
const RENDER_FORMATS: &[&str] = &["svg", "png", "pdf"];

/// Export targets declared by the command line.
const EXPORT_TARGETS: &[&str] = &["kicad_sch", "kicad_pcb", "net", "spice", "gbr", "pdf", "svg", "bom", "report", "json"];

/// Export targets this build actually writes.
const SUPPORTED_EXPORTS: &[&str] = &["kicad_sch", "net", "spice", "svg", "pdf", "bom", "report", "json"];

/// Tag written into the deterministic JSON document projection.
const JSON_PROJECTION_FORMAT: &str = "open-eda-projection";

/// Version written into the deterministic JSON document projection.
const JSON_PROJECTION_VERSION: u64 = 1;

/// Loss code reported when a SPICE element pin has no net and gets an NC node.
const SPICE_OPEN_PIN: &str = "spice/open-pin";

/// Loss code reported when an element has fewer pins than its SPICE mapping needs.
const SPICE_PIN_COUNT: &str = "spice/pin-count";

/// Import sources this build actually reads.
const SUPPORTED_IMPORTS: &[&str] = &["kicad_sch"];

/// Loss codes the KiCad schematic importer reports.
const KICAD_LOSS_CODES: &[&str] =
    &[kicad::UNSUPPORTED_ELEMENT, kicad::DUPLICATE_REFERENCE, kicad::MISSING_REFERENCE, kicad::LABEL_SCOPE];

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
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
    /// Rotation in degrees.
    pub rotation: f64,
    /// Pin names this component exposes.
    pub pins: Vec<String>,
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
/// keeps its placement so the flag survives a KiCad round trip; it does not
/// attach the flag to a component pin, because the document stores placement
/// but not per-pin geometry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoConnect {
    /// Placement X coordinate in design units.
    pub x: f64,
    /// Placement Y coordinate in design units.
    pub y: f64,
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
    /// Committed revision; every successful edit increments it.
    pub revision: u64,
    /// Design units, `mm` or `mil`.
    pub units: String,
    /// Schematic sheets; a created design always has one.
    pub sheets: Vec<Sheet>,
    /// Declared buses; absent in older documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<Bus>,
    /// Next component number used to build deterministic object identities.
    pub next_component: u64,
    /// Next net number used to build deterministic object identities.
    pub next_net: u64,
    /// Next wire number used to build deterministic object identities.
    pub next_wire: u64,
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

/// What this build can actually do.
///
/// The matrix is the single source of truth for the CLI, the SDK and the
/// studio, so a method name is never treated as a capability. Netlist, SPICE,
/// SVG, BOM, PDF, report, JSON and KiCad schematic projections are supported,
/// KiCad schematics are also imported, and every other exchange format and rule
/// check is reported as unsupported. Design-level buses are edited through
/// `add_bus`, `remove_bus` and `list_buses`, and ERC reports the stable codes
/// `erc/duplicate-net-name`, `erc/duplicate-pin`, `erc/single-node-net`,
/// `erc/dangling-net`, `erc/unconnected-pin` and `erc/floating-wire` next to the
/// legacy codes. The `nets` check also reports `net-bus-member-collision` when a
/// declared net would be written twice as a bus member. The JSON keys of this
/// report are unchanged.
pub fn capabilities() -> Value {
    let matrix = capability_matrix();
    let level = |id: String| if matrix.supports(&id) { "supported" } else { "unsupported" };

    let mut formats = Map::new();
    for &format in DESIGN_FORMATS {
        let import = level(format!("format.{format}.import"));
        let edit = level(format!("format.{format}.edit"));
        let export = level(format!("format.{format}.export"));
        let loss_codes = if SUPPORTED_IMPORTS.contains(&format) { json!(KICAD_LOSS_CODES) } else { json!([]) };
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

    let session = Session::new(design);
    let summary = summary_of(&session);
    let handle = insert_design(session.design);
    Ok(json!({ "handle": handle, "design": summary }))
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

/// Runs the requested rule checks on one open design.
pub fn inspect(handle: u64, checks: Option<Vec<String>>) -> Result<Value, ServiceError> {
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
    let selected = checks.unwrap_or_else(|| SUPPORTED_CHECKS.iter().map(|name| (*name).to_string()).collect());
    for check in &selected {
        if !INSPECT_CHECKS.contains(&check.as_str()) {
            return Err(not_implemented(&format!("inspect check `{check}`")));
        }
        if !SUPPORTED_CHECKS.contains(&check.as_str()) {
            return Err(not_implemented(&format!("inspect check `{check}`")));
        }
    }
    let issues = inspect_design(&design, &selected);
    let passed = !issues.iter().any(|issue| issue["severity"] == "error");
    Ok(json!({
        "revision": design.revision,
        "checkerVersion": format!("open-eda-erc/{}", env!("CARGO_PKG_VERSION")),
        "passed": passed,
        "issues": issues,
    }))
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
/// as `session/argument-invalid`.
pub fn add_bus(handle: u64, request: AddBusRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ServiceError::new("session/argument-invalid", "a bus name must not be empty"));
    }
    if request.width == 0 {
        return Err(ServiceError::new("session/argument-invalid", "a bus width must be at least 1"));
    }
    mutate(handle, |design| {
        if design.buses.iter().any(|bus| bus.name == name) {
            return Err(duplicate(format!("bus name `{name}` already exists")));
        }
        let id = free_bus_id(design);
        let bus = Bus { id, name: name.clone(), width: request.width };
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
            .position(|entry| entry.id == bus || entry.name == bus)
            .ok_or_else(|| not_found(format!("no bus `{bus}` is declared")))?;
        let removed = design.buses.remove(position);
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.name));
        payload.insert("remaining".to_string(), json!(design.buses.len()));
        Ok(payload)
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
            x: request.x,
            y: request.y,
            rotation: request.rotation,
            pins: request.pins,
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
        let target = &mut design.sheets[index];
        let before = target.components.len();
        target.components.retain(|component| component.reference != reference);
        if target.components.len() == before {
            return Err(not_found(format!("no component with reference `{reference}` is placed on this sheet")));
        }
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
        design.revision += 1;
        let mut payload = Map::new();
        payload.insert("deleted".to_string(), json!(removed.name));
        payload.insert("detached".to_string(), json!(detached));
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
    if !SUPPORTED_EXPORTS.contains(&format) {
        return Err(not_implemented(&format!("export `{format}`")));
    }
    let design = with_session(handle, |session| Ok(session.design.clone()))?;
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
        "svg" => {
            let artifact = json!({
                "kind": "svg",
                "components": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": design.sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
            });
            (render_svg(&design), artifact)
        }
        "bom" => {
            let rows = bom_rows(&design);
            let artifact = json!({
                "kind": "bom",
                "rows": rows.len(),
                "components": rows.iter().map(|(_, designators)| designators.len()).sum::<usize>(),
            });
            (render_bom(&rows), artifact)
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
            });
            (render_json_projection(&design)?, artifact)
        }
        "kicad_sch" => {
            let artifact = json!({
                "kind": "kicad_sch",
                "symbols": design.sheets.iter().map(|sheet| sheet.components.len()).sum::<usize>(),
                "wires": design.sheets.iter().map(|sheet| sheet.wires.len()).sum::<usize>(),
                "labels": design.sheets.iter().map(|sheet| sheet.nets.len()).sum::<usize>(),
            });
            (kicad::render(&design), artifact)
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
    if format == "spice" {
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
        for no_connect in &sheet.no_connects {
            if !no_connect.x.is_finite() || !no_connect.y.is_finite() {
                return Err(invalid(format!("sheet `{}` has a no-connect with a non-finite coordinate", sheet.id)));
            }
        }
        for text in &sheet.texts {
            if !text.x.is_finite() || !text.y.is_finite() {
                return Err(invalid(format!("sheet `{}` has a text with a non-finite coordinate", sheet.id)));
            }
        }
    }
    Ok(())
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
            for component in &sheet.components {
                for pin in &component.pins {
                    if !connected.contains(&(component.reference.clone(), pin.clone())) {
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

/// Projects one design to the deterministic text netlist.
///
/// Every declared bus is announced as `BUS <name> <width>`. A net whose name
/// equals a bus name expands into the bus member nets using the fixed rule
/// `DATA` with width 4 -> `DATA0`..`DATA3` (name plus 0-based index, no
/// separator); each member repeats the parent net's nodes. A declared bus with
/// no such net still lists its member names, with no nodes, after the sheets.
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
            if let Some(bus) = design.buses.iter().find(|bus| bus.name == net.name) {
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

/// Groups designators by value; the model has no footprint field, so a BOM row
/// is one distinct value.
fn bom_rows(design: &Design) -> Vec<(String, Vec<String>)> {
    let mut groups: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for sheet in &design.sheets {
        for component in &sheet.components {
            groups.entry(component.value.clone()).or_default().push(component.reference.clone());
        }
    }
    groups
        .into_iter()
        .map(|(value, mut designators)| {
            designators.sort();
            (value, designators)
        })
        .collect()
}

fn render_bom(rows: &[(String, Vec<String>)]) -> String {
    let mut text = String::from("Comment,Designator,Quantity\n");
    for (comment, designators) in rows {
        text.push_str(&csv_field(comment));
        text.push(',');
        text.push_str(&csv_field(&designators.join(" ")));
        text.push(',');
        text.push_str(&designators.len().to_string());
        text.push('\n');
    }
    text
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
                        "x": component.x,
                        "y": component.y,
                        "rotation": component.rotation,
                        "pins": component.pins,
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
                    .map(|no_connect| json!({ "x": no_connect.x, "y": no_connect.y }))
                    .collect::<Vec<_>>(),
                "texts": sheet
                    .texts
                    .iter()
                    .map(|text| json!({ "text": text.text, "x": text.x, "y": text.y }))
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
        "units": design.units,
        "sheets": sheets,
    });
    let mut encoded = serde_json::to_string_pretty(&projection)
        .map_err(|error| invalid(format!("the JSON projection could not be encoded: {error}")))?;
    encoded.push('\n');
    Ok(encoded)
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
            let expected_import = if format == "kicad_sch" { "supported" } else { "unsupported" };
            let expected_losses = if format == "kicad_sch" { json!(KICAD_LOSS_CODES) } else { json!([]) };
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
        assert_eq!(matrix["exportTargets"]["spice"], "supported");
        assert_eq!(matrix["exportTargets"]["svg"], "supported");
        assert_eq!(matrix["exportTargets"]["pdf"], "supported");
        assert_eq!(matrix["exportTargets"]["bom"], "supported");
        assert_eq!(matrix["exportTargets"]["report"], "supported");
        assert_eq!(matrix["exportTargets"]["json"], "supported");
        assert_eq!(matrix["exportTargets"]["kicad_sch"], "supported");
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

        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
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
    fn duplicate_references_are_an_inspection_finding() {
        let design = Design {
            id: "design-1".to_string(),
            title: None,
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
                        x: 0.0,
                        y: 0.0,
                        rotation: 0.0,
                        pins: vec!["1".to_string()],
                    },
                    Component {
                        id: "component-2".to_string(),
                        reference: "R1".to_string(),
                        value: "2k".to_string(),
                        x: 10.0,
                        y: 0.0,
                        rotation: 0.0,
                        pins: vec!["1".to_string()],
                    },
                ],
                nets: Vec::new(),
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
            }],
            buses: Vec::new(),
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
            next_component: 1,
            next_net: 3,
            next_wire: 1,
        };
        let issues = inspect_design(&design, &["erc".to_string()]);
        assert!(issues.iter().any(|issue| issue["code"] == "duplicate-net-name" && issue["severity"] == "error"));
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
    fn bom_export_groups_values_and_sorts_designators() {
        let dir = TempDir::new("bom");
        let path = dir.join("blinker.csv");
        let created = create(CreateRequest::default()).expect("create");
        let handle = created["handle"].as_u64().expect("handle");
        for (reference, value) in [("R2", "10k"), ("R1", "10k"), ("C1", "4k7")] {
            add_component(
                handle,
                AddComponentRequest {
                    sheet: None,
                    reference: reference.to_string(),
                    value: value.to_string(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string()],
                },
            )
            .expect("add");
        }
        let report = export(handle, "bom", &path).expect("export bom");
        assert_eq!(report["format"], "bom");
        assert_eq!(report["artifact"]["rows"], 2);
        assert_eq!(report["artifact"]["components"], 3);
        assert_eq!(report["bytes"], fs::metadata(&path).expect("metadata").len() as usize);
        let text = fs::read_to_string(&path).expect("read bom");
        assert_eq!(text, "Comment,Designator,Quantity\n10k,R1 R2,2\n4k7,C1,1\n");
        close(handle).expect("close");
    }

    #[test]
    fn pdf_export_writes_a_complete_vector_document() {
        let dir = TempDir::new("pdf");
        let path = dir.join("blinker.pdf");
        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
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
            revision: 1,
            units: "mm".to_string(),
            sheets: vec![Sheet {
                id: "sheet-1".to_string(),
                name: "Sheet 1".to_string(),
                components: vec![Component {
                    id: "component-1".to_string(),
                    reference: "R1".to_string(),
                    value: "1k".to_string(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string()],
                }],
                nets: Vec::new(),
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
            }],
            buses: Vec::new(),
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
        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
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
        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
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
    fn kicad_sch_export_is_deterministic_and_round_trips() {
        let dir = TempDir::new("kicad");
        let first = dir.join("blinker.kicad_sch");
        let second = dir.join("blinker-again.kicad_sch");

        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
        let handle = created["handle"].as_u64().expect("handle");
        blinker(handle);

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
        assert_eq!(components[1]["ref"], "C1");

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
    fn report_export_renders_object_and_net_tables() {
        let dir = TempDir::new("report");
        let path = dir.join("report.md");
        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
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
        let created = create(CreateRequest { title: Some("Blinker".to_string()), template: None }).expect("create");
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
            revision: 2,
            units: "mm".to_string(),
            sheets: vec![Sheet {
                id: "sheet-1".to_string(),
                name: "Sheet 1".to_string(),
                components: vec![Component {
                    id: "component-1".to_string(),
                    reference: "U1".to_string(),
                    value: "opamp".to_string(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string(), "1".to_string(), "2".to_string()],
                }],
                nets: vec![Net { id: "net-1".to_string(), name: "SPARE".to_string(), nodes: Vec::new() }],
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
            }],
            buses: Vec::new(),
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
    fn nets_check_reports_bus_member_collisions() {
        let design = Design {
            id: "design-1".to_string(),
            title: None,
            revision: 2,
            units: "mm".to_string(),
            sheets: vec![Sheet {
                id: "sheet-1".to_string(),
                name: "Sheet 1".to_string(),
                components: vec![Component {
                    id: "component-1".to_string(),
                    reference: "U1".to_string(),
                    value: "buffer".to_string(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    pins: vec!["1".to_string(), "2".to_string()],
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
            buses: vec![Bus { id: "bus-1".to_string(), name: "DATA".to_string(), width: 4 }],
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

        let added = add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 4 }).expect("add bus");
        assert_eq!(added["bus"], json!({ "id": "bus-1", "name": "DATA", "width": 4 }));
        assert_eq!(added["revision"], 3);
        assert_eq!(add_bus(handle, AddBusRequest { name: "DATA".to_string(), width: 2 }).unwrap_err().code, "object/duplicate");
        assert_eq!(
            add_bus(handle, AddBusRequest { name: "ADDR".to_string(), width: 0 }).unwrap_err().code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_bus(handle, AddBusRequest { name: "  ".to_string(), width: 1 }).unwrap_err().code,
            "session/argument-invalid"
        );

        export(handle, "net", &path).expect("export declared bus");
        let text = fs::read_to_string(&path).expect("read netlist");
        assert!(text.contains("BUS DATA 4\n"));
        for index in 0..4 {
            assert!(text.contains(&format!("NET DATA{index}\n\n")), "member {index} is listed without nodes");
        }

        connect_net(
            handle,
            ConnectNetRequest { sheet: None, net: "DATA".to_string(), nodes: vec!["R1.1".to_string(), "R1.2".to_string()] },
        )
        .expect("connect DATA");
        export(handle, "net", &path).expect("export expanded bus");
        let text = fs::read_to_string(&path).expect("read expanded netlist");
        for index in 0..4 {
            assert!(
                text.contains(&format!("NET DATA{index}\nNODE R1.1\nNODE R1.2\n\n")),
                "member {index} repeats the parent nodes"
            );
        }

        let listed = list_buses(handle).expect("list buses");
        assert_eq!(listed["buses"].as_array().expect("buses").len(), 1);
        assert_eq!(listed["buses"][0]["name"], "DATA");

        let removed = remove_bus(handle, "DATA").expect("remove by name");
        assert_eq!(removed["deleted"], "DATA");
        assert_eq!(removed["remaining"], 0);
        assert_eq!(list_buses(handle).expect("list again")["buses"], json!([]));
        assert_eq!(remove_bus(handle, "DATA").unwrap_err().code, "object/not-found");
        close(handle).expect("close");
    }
}
