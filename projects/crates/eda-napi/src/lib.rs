//! Thin Node-API surface for OpenEDA.
//!
//! Every export performs one orthogonal operation and returns a JSON string, so
//! the TypeScript layer owns argument parsing, help text and exit codes. Design
//! semantics stay in `open-eda-model`; this crate only maps values and owns the
//! WebView host.
//!
//! Supported operations map onto the model crate directly. Unsupported
//! capabilities keep the stable `capability/not-implemented` code instead of
//! returning a fabricated result.

#![deny(missing_docs)]

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use open_eda_model::{
    AddBusRequest, AddComponentRequest, AddJunctionRequest, AddNoConnectRequest, AddTextRequest, AddWireRequest,
    ComponentClassRequest, ConnectNetRequest, CreateRequest, DeleteNetRequest, DisconnectNetRequest, InspectOptions,
    MoveComponentRequest, NetClassRequest, ReferencePrefixRequest, RenameNetRequest, ServiceError, SetFootprintRequest,
    SetPinTypeRequest, SplitWireRequest, TranslateRequest, UpdateComponentRequest, UpdateDesignRequest, UpdateWireRequest,
};
use serde_json::{Value, json};
use wae::{
    CallContext, Error, JsValue, Result, function, optional_string_prop, register_module,
    webview::{DragRect, IpcHandler, IpcOutcome, IpcSink, OpenOptions, WebViewError, WebViewHost},
};

fn fail(error: ServiceError) -> Error {
    Error::new(format!("{}: {}", error.code, error.message))
}

fn argument(ctx: &CallContext, index: usize) -> Result<JsValue> {
    ctx.args().get(index).copied().ok_or_else(|| Error::new("session/argument-invalid: a required argument is missing"))
}

fn text(ctx: &CallContext, index: usize) -> Result<String> {
    argument(ctx, index)?.as_string()
}

fn handle(ctx: &CallContext, index: usize) -> Result<u64> {
    let raw = argument(ctx, index)?.as_i64()?;
    u64::try_from(raw).map_err(|_| Error::new("session/argument-invalid: the design handle must be a non-negative integer"))
}

fn position(ctx: &CallContext, index: usize) -> Result<usize> {
    let raw = argument(ctx, index)?.as_i64()?;
    usize::try_from(raw).map_err(|_| Error::new("session/argument-invalid: the index must be a non-negative integer"))
}

fn float(ctx: &CallContext, index: usize) -> Result<f64> {
    argument(ctx, index)?.as_f64()
}

fn respond(ctx: &CallContext, value: &Value) -> Result<JsValue> {
    let text = value.to_string();
    ctx.env().string(&text)
}

/// Library version of the native binding.
#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string(env!("CARGO_PKG_VERSION"))
}

/// Reports the declared design capability matrix.
///
/// The matrix comes from the same model crate the CLI reads, so neither entry
/// point can report different capabilities.
#[function]
fn capabilities(ctx: CallContext) -> Result<JsValue> {
    let matrix = open_eda_model::capabilities().to_string();
    ctx.env().string(&matrix)
}

/// Creates a design from a JSON request.
#[function]
fn design_create(ctx: CallContext) -> Result<JsValue> {
    let raw = text(&ctx, 0)?;
    let request: Value =
        serde_json::from_str(&raw).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))?;
    let created = open_eda_model::create(CreateRequest {
        title: request["title"].as_str().map(str::to_string),
        rev: request["rev"].as_str().map(str::to_string),
        company: request["company"].as_str().map(str::to_string),
        date: request["date"].as_str().map(str::to_string),
        template: request["template"].as_str().map(str::to_string),
    })
    .map_err(fail)?;
    respond(&ctx, &created)
}

/// Updates the drawing properties of one open design.
#[function]
fn design_set_properties(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: UpdateDesignRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::update_design(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Opens a native design document from a path.
#[function]
fn design_open(ctx: CallContext) -> Result<JsValue> {
    let path = text(&ctx, 0)?;
    let opened = open_eda_model::open(Path::new(&path)).map_err(fail)?;
    respond(&ctx, &opened)
}

/// Writes one open design to a native document path.
#[function]
fn design_save(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let path = text(&ctx, 1)?;
    let overwrite = ctx.args().get(2).map(|value| value.as_bool()).transpose()?.unwrap_or(false);
    let report = open_eda_model::save(handle, Path::new(&path), overwrite).map_err(fail)?;
    respond(&ctx, &report)
}

/// Runs the requested rule checks on one open design.
#[function]
fn design_inspect(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let checks = match ctx.args().get(1) {
        Some(value) => {
            let raw = value.as_string()?;
            let names: Vec<String> =
                serde_json::from_str(&raw).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))?;
            Some(names)
        }
        None => None,
    };
    let report = open_eda_model::inspect(handle, checks).map_err(fail)?;
    respond(&ctx, &report)
}

/// Runs rule checks with severity overrides and waivers from a JSON request.
#[function]
fn design_inspect_with(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let options: InspectOptions = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::inspect_with_options(handle, options).map_err(fail)?;
    respond(&ctx, &report)
}

/// Projects one open design to an exchange target.
#[function]
fn design_export(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let format = text(&ctx, 1)?;
    let path = text(&ctx, 2)?;
    let variant = optional_text(&ctx, 3)?;
    let report = open_eda_model::export_with_variant(handle, &format, Path::new(&path), variant.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Imports a KiCad schematic from either its text or a file path.
///
/// A value that names an existing file is read from disk; anything else is
/// treated as the schematic itself, so a caller can pass editor contents.
#[function]
fn import_kicad_sch(ctx: CallContext) -> Result<JsValue> {
    let source = text(&ctx, 0)?;
    let contents = if Path::new(&source).is_file() {
        std::fs::read_to_string(&source)
            .map_err(|error| Error::new(format!("format/invalid: cannot read `{source}`: {error}")))?
    }
    else {
        source
    };
    let report = open_eda_model::import_kicad_sch(&contents).map_err(fail)?;
    respond(&ctx, &report)
}

/// Imports a KiCad symbol library from either its text or a file path.
#[function]
fn import_kicad_symbol(ctx: CallContext) -> Result<JsValue> {
    let source = text(&ctx, 0)?;
    let contents = if Path::new(&source).is_file() {
        std::fs::read_to_string(&source)
            .map_err(|error| Error::new(format!("format/invalid: cannot read `{source}`: {error}")))?
    }
    else {
        source
    };
    let report = open_eda_model::import_kicad_symbol(&contents).map_err(fail)?;
    respond(&ctx, &report)
}

/// Imports a KiCad netlist from either its text or a file path.
#[function]
fn import_kicad_net(ctx: CallContext) -> Result<JsValue> {
    let source = text(&ctx, 0)?;
    let contents = if Path::new(&source).is_file() {
        std::fs::read_to_string(&source)
            .map_err(|error| Error::new(format!("format/invalid: cannot read `{source}`: {error}")))?
    }
    else {
        source
    };
    let report = open_eda_model::import_kicad_net(&contents).map_err(fail)?;
    respond(&ctx, &report)
}

/// Compares one open design with a KiCad netlist from its text or a file path.
#[function]
fn design_compare_netlist(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let source = text(&ctx, 1)?;
    let contents = if Path::new(&source).is_file() {
        std::fs::read_to_string(&source)
            .map_err(|error| Error::new(format!("format/invalid: cannot read `{source}`: {error}")))?
    }
    else {
        source
    };
    let report = open_eda_model::compare_kicad_net(handle, &contents).map_err(fail)?;
    respond(&ctx, &report)
}

/// Compares two native design documents without opening either for editing.
#[function]
fn diff_documents(ctx: CallContext) -> Result<JsValue> {
    let left = text(&ctx, 0)?;
    let right = text(&ctx, 1)?;
    let report = open_eda_model::diff_documents(Path::new(&left), Path::new(&right)).map_err(fail)?;
    respond(&ctx, &report)
}

/// Assigns or clears the electrical role of one component pin.
#[function]
fn design_set_pin_type(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: SetPinTypeRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::set_pin_type(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Renumbers every component reference designator of one open design.
#[function]
fn design_annotate(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::annotate(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Closes one open design and releases its handle.
#[function]
fn design_close(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    open_eda_model::close(handle).map_err(fail)?;
    ctx.env().boolean(true)
}

/// Reads the summary of one open design.
#[function]
fn design_summary(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::summary(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists components, nets and wires of one open design.
#[function]
fn design_list(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::list(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Adds one component to an open design.
#[function]
fn design_add_component(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: AddComponentRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_component(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes one component from an open design.
#[function]
fn design_delete_component(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let reference = text(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let report = open_eda_model::delete_component(handle, &reference, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Connects nodes to a named net on an open design.
#[function]
fn design_connect_net(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: ConnectNetRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::connect_net(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Adds one wire segment to an open design.
#[function]
fn design_add_wire(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: AddWireRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_wire(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Updates the endpoints or net of one wire segment.
#[function]
fn design_update_wire(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: UpdateWireRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::update_wire(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Splits one wire segment into two at a point on the segment.
#[function]
fn design_split_wire(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: SplitWireRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::split_wire(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Translates the selected components and wires of one sheet by an offset.
#[function]
fn design_translate(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: TranslateRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::translate_objects(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists the wires drawn on one sheet.
#[function]
fn design_list_wires(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = optional_text(&ctx, 1)?;
    let report = open_eda_model::list_wires(handle, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Adds one junction (connection dot) to an open design.
#[function]
fn design_add_junction(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: AddJunctionRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_junction(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Adds one free text annotation to an open design.
#[function]
fn design_add_text(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: AddTextRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_text(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Adds one no-connect flag to an open design, optionally marking a pin.
#[function]
fn design_add_no_connect(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: AddNoConnectRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_no_connect(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes one no-connect flag at a sheet index.
#[function]
fn design_delete_no_connect(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let index = position(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let report = open_eda_model::delete_no_connect(handle, index, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists components of one open design.
#[function]
fn design_list_components(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = optional_text(&ctx, 1)?;
    let report = open_eda_model::list_components(handle, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists nets of one open design.
#[function]
fn design_list_nets(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = optional_text(&ctx, 1)?;
    let report = open_eda_model::list_nets(handle, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Declares one design-level bus.
#[function]
fn design_add_bus(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: AddBusRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_bus(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Removes one design-level bus by id or name.
#[function]
fn design_remove_bus(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let bus = text(&ctx, 1)?;
    let report = open_eda_model::remove_bus(handle, &bus).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists the symbol library one open design uses.
#[function]
fn design_symbols(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::symbols(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Reports what each net of one open design connects, plus open pins and totals.
#[function]
fn design_connectivity(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::connectivity_report(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Declares or replaces one BOM variant on an open design.
#[function]
fn design_add_variant(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: open_eda_model::VariantRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_variant(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Removes one BOM variant by name.
#[function]
fn design_remove_variant(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let name = text(&ctx, 1)?;
    let report = open_eda_model::remove_variant(handle, &name).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists the BOM variants declared by one open design.
#[function]
fn design_list_variants(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::list_variants(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists the declared buses of one open design.
#[function]
fn design_list_buses(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::list_buses(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Declares or replaces one net class with its routing attributes and assignments.
#[function]
fn design_add_net_class(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: NetClassRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::add_net_class(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Removes one net class by id or name.
#[function]
fn design_remove_net_class(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let net_class = text(&ctx, 1)?;
    let report = open_eda_model::remove_net_class(handle, &net_class).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists the net classes declared by one open design.
#[function]
fn design_list_net_classes(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::list_net_classes(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Declares or replaces one reference designator prefix rule.
#[function]
fn design_set_reference_prefix(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: ReferencePrefixRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::set_reference_prefix(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Removes one reference prefix rule by component class.
#[function]
fn design_remove_reference_prefix(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let class = text(&ctx, 1)?;
    let report = open_eda_model::remove_reference_prefix(handle, &class).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists the reference prefix rules declared by one open design.
#[function]
fn design_list_reference_prefixes(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::list_reference_prefixes(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Assigns or clears the component class of one placed component.
#[function]
fn design_set_component_class(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: ComponentClassRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::set_component_class(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Lists component class assignments with their resolved references.
#[function]
fn design_list_component_classes(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::list_component_classes(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Restores the snapshot taken before the newest edit.
#[function]
fn design_undo(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::undo(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Reapplies the snapshot the newest undo displaced.
#[function]
fn design_redo(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let report = open_eda_model::redo(handle).map_err(fail)?;
    respond(&ctx, &report)
}

/// Adds an empty sheet to an open design.
#[function]
fn design_add_sheet(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let name = optional_text(&ctx, 1)?;
    let report = open_eda_model::add_sheet(handle, name.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Renames one sheet of an open design.
#[function]
fn design_rename_sheet(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = text(&ctx, 1)?;
    let name = text(&ctx, 2)?;
    let report = open_eda_model::rename_sheet(handle, &sheet, &name).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes one empty sheet of an open design.
#[function]
fn design_delete_sheet(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = text(&ctx, 1)?;
    let report = open_eda_model::delete_sheet(handle, &sheet).map_err(fail)?;
    respond(&ctx, &report)
}

/// Moves one placed component.
#[function]
fn design_move_component(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: MoveComponentRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::move_component(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Updates the value text of one placed component.
#[function]
fn design_update_component(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: UpdateComponentRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::update_component(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Assigns or clears the footprint text of one placed component.
#[function]
fn design_set_component_footprint(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: SetFootprintRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::set_component_footprint(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Renames one net and every wire that referenced it.
#[function]
fn design_rename_net(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: RenameNetRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::rename_net(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes one wire segment.
#[function]
fn design_delete_wire(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let wire = text(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let report = open_eda_model::delete_wire(handle, &wire, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Derives net names for the connected wire groups of one sheet.
#[function]
fn design_derive_nets(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = optional_text(&ctx, 1)?;
    let report = open_eda_model::derive_nets(handle, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Removes nodes from one net without deleting it.
#[function]
fn design_disconnect_net(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: DisconnectNetRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::disconnect_net(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes one net and detaches the wires that named it.
#[function]
fn design_delete_net(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request: DeleteNetRequest = decode(serde_json::from_str(&text(&ctx, 1)?))?;
    let report = open_eda_model::delete_net(handle, request).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes the junction at one sheet index.
#[function]
fn design_delete_junction(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let index = position(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let report = open_eda_model::delete_junction(handle, index, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Moves the junction at one sheet index to a new placement.
#[function]
fn design_move_junction(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let index = position(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let x = float(&ctx, 3)?;
    let y = float(&ctx, 4)?;
    let report = open_eda_model::move_junction(handle, index, sheet.as_deref(), x, y).map_err(fail)?;
    respond(&ctx, &report)
}

/// Deletes the text annotation at one sheet index.
#[function]
fn design_delete_text(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let index = position(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let report = open_eda_model::delete_text(handle, index, sheet.as_deref()).map_err(fail)?;
    respond(&ctx, &report)
}

/// Moves the text annotation at one sheet index to a new placement.
#[function]
fn design_move_text(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let index = position(&ctx, 1)?;
    let sheet = optional_text(&ctx, 2)?;
    let x = float(&ctx, 3)?;
    let y = float(&ctx, 4)?;
    let report = open_eda_model::move_text(handle, index, sheet.as_deref(), x, y).map_err(fail)?;
    respond(&ctx, &report)
}

fn decode<T>(result: std::result::Result<T, serde_json::Error>) -> Result<T> {
    result.map_err(|error| Error::new(format!("session/argument-invalid: {error}")))
}

fn optional_text(ctx: &CallContext, index: usize) -> Result<Option<String>> {
    match ctx.args().get(index) {
        Some(value) if !value.is_undefined()? => Ok(Some(value.as_string()?)),
        _ => Ok(None),
    }
}

// One WebView host per process, owned by the thread that opened it.
thread_local! {
    static GUI: RefCell<Option<GuiState>> = const { RefCell::new(None) };
}

const GUI_CLOSED: &str = "gui_closed";
const GUI_HOST_EXITED: &str = "gui_host_exited";
const GUI_NOT_OPEN: &str = "gui_not_open";

/// Window operations queued by the IPC worker and applied on the window thread.
#[derive(Debug, Clone, Copy)]
enum WindowCommand {
    Minimize,
    ToggleMaximize,
}

/// Latest drag rects reported by the page; `None` when no update is pending.
static DRAG_REGIONS: Mutex<Option<Vec<DragRect>>> = Mutex::new(None);
/// Window commands queued by the IPC worker and drained before each pump.
static WINDOW_COMMANDS: Mutex<Vec<WindowCommand>> = Mutex::new(Vec::new());

struct GuiState {
    host: WebViewHost,
    backend: &'static str,
    opened_at: Instant,
    /// Set when the page asks the host to close, so a dead host is
    /// distinguishable from one that never processed the request.
    close_requested: Arc<AtomicBool>,
}

struct DesignIpcHandler {
    close_requested: Arc<AtomicBool>,
}

impl IpcHandler for DesignIpcHandler {
    fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
        let Ok(value) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::Ignored;
        };
        let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        match ty {
            "close" | "window.close" => {
                self.close_requested.store(true, Ordering::SeqCst);
                IpcOutcome::CloseWindow
            }
            "window.dragRegions" => {
                let rects = parse_drag_rects(&value);
                let applied = rects.len();
                if let Ok(mut pending) = DRAG_REGIONS.lock() {
                    *pending = Some(rects);
                }
                reply(id, "windowDragRegionsResult", json!({ "applied": applied }))
            }
            "window.minimize" => {
                if let Ok(mut queue) = WINDOW_COMMANDS.lock() {
                    queue.push(WindowCommand::Minimize);
                }
                reply(id, "windowMinimizeResult", json!({ "queued": true }))
            }
            "window.toggleMaximize" => {
                if let Ok(mut queue) = WINDOW_COMMANDS.lock() {
                    queue.push(WindowCommand::ToggleMaximize);
                }
                reply(id, "windowToggleMaximizeResult", json!({ "queued": true }))
            }
            "ready" => reply(
                id,
                "readyResult",
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": platform_name(),
                    "window": { "customChrome": cfg!(target_os = "windows") },
                }),
            ),
            "capabilities" => reply(id, "capabilitiesResult", json!({ "capabilities": open_eda_model::capabilities() })),
            "design.create" => {
                let request = value.get("request");
                let title = request.and_then(|request| request.get("title")).and_then(Value::as_str).map(str::to_string);
                let rev = request.and_then(|request| request.get("rev")).and_then(Value::as_str).map(str::to_string);
                let company = request.and_then(|request| request.get("company")).and_then(Value::as_str).map(str::to_string);
                let date = request.and_then(|request| request.get("date")).and_then(Value::as_str).map(str::to_string);
                let template = request.and_then(|request| request.get("template")).and_then(Value::as_str).map(str::to_string);
                match open_eda_model::create(CreateRequest { title, rev, company, date, template }) {
                    Ok(payload) => reply(id, "createResult", payload),
                    Err(error) => failed(id, &error),
                }
            }
            "design.open" => match ipc_string(&value, "path") {
                Ok(path) => match open_eda_model::open(Path::new(&path)) {
                    Ok(payload) => reply(id, "openResult", payload),
                    Err(error) => failed(id, &error),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.save" => match ipc_handle(&value) {
                Ok(handle) => match ipc_string(&value, "path") {
                    Ok(path) => {
                        let overwrite = value.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
                        match open_eda_model::save(handle, Path::new(&path), overwrite) {
                            Ok(payload) => reply(id, "saveResult", json!({ "report": payload })),
                            Err(error) => failed(id, &error),
                        }
                    }
                    Err(message) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.inspect" => match ipc_handle(&value) {
                Ok(handle) => {
                    let checks = value
                        .get("checks")
                        .and_then(Value::as_array)
                        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
                    match open_eda_model::inspect(handle, checks) {
                        Ok(payload) => reply(id, "inspectResult", json!({ "report": payload })),
                        Err(error) => failed(id, &error),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.export" => match ipc_handle(&value) {
                Ok(handle) => match ipc_string(&value, "format") {
                    Ok(format) => {
                        let target =
                            value.get("target").or_else(|| value.get("path")).and_then(Value::as_str).unwrap_or("").to_string();
                        if target.is_empty() {
                            failure(id, "error", "session/argument-invalid: an export target path is required".to_string())
                        }
                        else {
                            let variant = value.get("variant").and_then(Value::as_str);
                            match open_eda_model::export_with_variant(handle, &format, Path::new(&target), variant) {
                                Ok(payload) => reply(id, "exportResult", json!({ "report": payload })),
                                Err(error) => failed(id, &error),
                            }
                        }
                    }
                    Err(message) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.close" => match ipc_handle(&value) {
                Ok(handle) => match open_eda_model::close(handle) {
                    Ok(()) => reply(id, "closeResult", json!({ "closed": true })),
                    Err(error) => failed(id, &error),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.summary" => match ipc_handle(&value) {
                Ok(handle) => match open_eda_model::summary(handle) {
                    Ok(payload) => reply(id, "summaryResult", payload),
                    Err(error) => failed(id, &error),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.list" => match ipc_handle(&value) {
                Ok(handle) => match open_eda_model::list(handle) {
                    Ok(payload) => reply(id, "listResult", payload),
                    Err(error) => failed(id, &error),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.addComponent" => match ipc_handle(&value) {
                Ok(handle) => {
                    let component = value.get("component").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<AddComponentRequest>(component) {
                        Ok(request) => match open_eda_model::add_component(handle, request) {
                            Ok(payload) => reply(id, "addComponentResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.deleteComponent" => match ipc_handle(&value) {
                Ok(handle) => match ipc_string(&value, "ref") {
                    Ok(reference) => {
                        let sheet = value.get("sheet").and_then(Value::as_str);
                        match open_eda_model::delete_component(handle, &reference, sheet) {
                            Ok(payload) => reply(id, "deleteComponentResult", payload),
                            Err(error) => failed(id, &error),
                        }
                    }
                    Err(message) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.connectNet" => match ipc_handle(&value) {
                Ok(handle) => match ipc_string(&value, "net") {
                    Ok(net) => {
                        let nodes = value
                            .get("nodes")
                            .and_then(Value::as_array)
                            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                            .unwrap_or_default();
                        let sheet = value.get("sheet").and_then(Value::as_str).map(str::to_string);
                        let request = ConnectNetRequest { sheet, net, nodes };
                        match open_eda_model::connect_net(handle, request) {
                            Ok(payload) => reply(id, "connectNetResult", payload),
                            Err(error) => failed(id, &error),
                        }
                    }
                    Err(message) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.addWire" => match ipc_handle(&value) {
                Ok(handle) => {
                    let wire = value.get("wire").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<AddWireRequest>(wire) {
                        Ok(request) => match open_eda_model::add_wire(handle, request) {
                            Ok(payload) => reply(id, "addWireResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.undo" => match ipc_handle(&value) {
                Ok(handle) => match open_eda_model::undo(handle) {
                    Ok(payload) => reply(id, "undoResult", payload),
                    Err(error) => failed(id, &error),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.redo" => match ipc_handle(&value) {
                Ok(handle) => match open_eda_model::redo(handle) {
                    Ok(payload) => reply(id, "redoResult", payload),
                    Err(error) => failed(id, &error),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.addSheet" => match ipc_handle(&value) {
                Ok(handle) => {
                    let name = value.get("name").and_then(Value::as_str);
                    match open_eda_model::add_sheet(handle, name) {
                        Ok(payload) => reply(id, "addSheetResult", payload),
                        Err(error) => failed(id, &error),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.renameSheet" => match ipc_handle(&value) {
                Ok(handle) => match (ipc_string(&value, "sheet"), ipc_string(&value, "name")) {
                    (Ok(sheet), Ok(name)) => match open_eda_model::rename_sheet(handle, &sheet, &name) {
                        Ok(payload) => reply(id, "renameSheetResult", payload),
                        Err(error) => failed(id, &error),
                    },
                    (Err(message), _) | (_, Err(message)) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.deleteSheet" => match ipc_handle(&value) {
                Ok(handle) => match ipc_string(&value, "sheet") {
                    Ok(sheet) => match open_eda_model::delete_sheet(handle, &sheet) {
                        Ok(payload) => reply(id, "deleteSheetResult", payload),
                        Err(error) => failed(id, &error),
                    },
                    Err(message) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.moveComponent" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<MoveComponentRequest>(request) {
                        Ok(request) => match open_eda_model::move_component(handle, request) {
                            Ok(payload) => reply(id, "moveComponentResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.updateComponent" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<UpdateComponentRequest>(request) {
                        Ok(request) => match open_eda_model::update_component(handle, request) {
                            Ok(payload) => reply(id, "updateComponentResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.setProperties" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<UpdateDesignRequest>(request) {
                        Ok(request) => match open_eda_model::update_design(handle, request) {
                            Ok(payload) => reply(id, "setPropertiesResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.setPinType" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<SetPinTypeRequest>(request) {
                        Ok(request) => match open_eda_model::set_pin_type(handle, request) {
                            Ok(payload) => reply(id, "setPinTypeResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.renameNet" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<RenameNetRequest>(request) {
                        Ok(request) => match open_eda_model::rename_net(handle, request) {
                            Ok(payload) => reply(id, "renameNetResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.deleteWire" => match ipc_handle(&value) {
                Ok(handle) => match ipc_string(&value, "wire") {
                    Ok(wire) => {
                        let sheet = value.get("sheet").and_then(Value::as_str);
                        match open_eda_model::delete_wire(handle, &wire, sheet) {
                            Ok(payload) => reply(id, "deleteWireResult", payload),
                            Err(error) => failed(id, &error),
                        }
                    }
                    Err(message) => failure(id, "error", message),
                },
                Err(message) => failure(id, "error", message),
            },
            "design.deriveNets" => match ipc_handle(&value) {
                Ok(handle) => {
                    let sheet = value.get("sheet").and_then(Value::as_str);
                    match open_eda_model::derive_nets(handle, sheet) {
                        Ok(payload) => reply(id, "deriveNetsResult", payload),
                        Err(error) => failed(id, &error),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.disconnectNet" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<DisconnectNetRequest>(request) {
                        Ok(request) => match open_eda_model::disconnect_net(handle, request) {
                            Ok(payload) => reply(id, "disconnectNetResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.deleteNet" => match ipc_handle(&value) {
                Ok(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    match serde_json::from_value::<DeleteNetRequest>(request) {
                        Ok(request) => match open_eda_model::delete_net(handle, request) {
                            Ok(payload) => reply(id, "deleteNetResult", payload),
                            Err(error) => failed(id, &error),
                        },
                        Err(error) => failure(id, "error", format!("session/argument-invalid: {error}")),
                    }
                }
                Err(message) => failure(id, "error", message),
            },
            "design.deleteJunction" => match (ipc_handle(&value), ipc_index(&value)) {
                (Ok(handle), Ok(index)) => {
                    let sheet = value.get("sheet").and_then(Value::as_str);
                    match open_eda_model::delete_junction(handle, index, sheet) {
                        Ok(payload) => reply(id, "deleteJunctionResult", payload),
                        Err(error) => failed(id, &error),
                    }
                }
                (Err(message), _) | (_, Err(message)) => failure(id, "error", message),
            },
            "design.moveJunction" => {
                match (ipc_handle(&value), ipc_index(&value), ipc_float(&value, "x"), ipc_float(&value, "y")) {
                    (Ok(handle), Ok(index), Ok(x), Ok(y)) => {
                        let sheet = value.get("sheet").and_then(Value::as_str);
                        match open_eda_model::move_junction(handle, index, sheet, x, y) {
                            Ok(payload) => reply(id, "moveJunctionResult", payload),
                            Err(error) => failed(id, &error),
                        }
                    }
                    (Err(message), _, _, _) | (_, Err(message), _, _) | (_, _, Err(message), _) | (_, _, _, Err(message)) => {
                        failure(id, "error", message)
                    }
                }
            }
            "design.deleteText" => match (ipc_handle(&value), ipc_index(&value)) {
                (Ok(handle), Ok(index)) => {
                    let sheet = value.get("sheet").and_then(Value::as_str);
                    match open_eda_model::delete_text(handle, index, sheet) {
                        Ok(payload) => reply(id, "deleteTextResult", payload),
                        Err(error) => failed(id, &error),
                    }
                }
                (Err(message), _) | (_, Err(message)) => failure(id, "error", message),
            },
            "design.moveText" => {
                match (ipc_handle(&value), ipc_index(&value), ipc_float(&value, "x"), ipc_float(&value, "y")) {
                    (Ok(handle), Ok(index), Ok(x), Ok(y)) => {
                        let sheet = value.get("sheet").and_then(Value::as_str);
                        match open_eda_model::move_text(handle, index, sheet, x, y) {
                            Ok(payload) => reply(id, "moveTextResult", payload),
                            Err(error) => failed(id, &error),
                        }
                    }
                    (Err(message), _, _, _) | (_, Err(message), _, _) | (_, _, Err(message), _) | (_, _, _, Err(message)) => {
                        failure(id, "error", message)
                    }
                }
            }
            _ => IpcOutcome::Ignored,
        }
    }
}

/// Parses page-reported drag regions into backend rects, skipping malformed rows.
fn parse_drag_rects(value: &Value) -> Vec<DragRect> {
    value
        .get("rects")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|rect| {
                    Some(DragRect {
                        x: rect.get("x")?.as_i64()? as i32,
                        y: rect.get("y")?.as_i64()? as i32,
                        width: rect.get("width")?.as_i64()? as i32,
                        height: rect.get("height")?.as_i64()? as i32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn ipc_handle(value: &Value) -> std::result::Result<u64, String> {
    value
        .get("handle")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session/argument-invalid: a design handle is required".to_string())
}

fn ipc_string(value: &Value, key: &str) -> std::result::Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("session/argument-invalid: `{key}` is required"))
}

fn ipc_index(value: &Value) -> std::result::Result<usize, String> {
    let index = value
        .get("index")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session/argument-invalid: `index` must be a non-negative integer".to_string())?;
    usize::try_from(index).map_err(|_| "session/argument-invalid: `index` is too large".to_string())
}

fn ipc_float(value: &Value, key: &str) -> std::result::Result<f64, String> {
    value.get(key).and_then(Value::as_f64).ok_or_else(|| format!("session/argument-invalid: `{key}` must be a number"))
}

fn failed(id: Value, error: &ServiceError) -> IpcOutcome {
    failure(id, "error", format!("{}: {}", error.code, error.message))
}

fn reply(id: Value, kind: &str, payload: Value) -> IpcOutcome {
    let mut body = payload;
    body["type"] = Value::String(kind.to_string());
    body["id"] = id;
    IpcOutcome::ReplyToPage(body.to_string())
}

fn failure(id: Value, kind: &str, message: String) -> IpcOutcome {
    reply(id, kind, json!({ "message": message }))
}

fn platform_name() -> &'static str {
    if cfg!(windows) {
        "win32"
    }
    else if cfg!(target_os = "macos") {
        "darwin"
    }
    else {
        "linux"
    }
}

fn gui_status(
    state: &'static str,
    code: Option<&'static str>,
    backend: Option<&'static str>,
    uptime_ms: u64,
    message: Option<String>,
) -> Value {
    json!({
        "state": state,
        "code": code,
        "backend": backend,
        "platform": platform_name(),
        "uptimeMs": uptime_ms,
        "message": message,
    })
}

fn gui_open_result(ok: bool, status: Value, error: Option<&WebViewError>) -> Value {
    let (stage, hresult, attempted) = match error {
        Some(error) => {
            let attempted = match error {
                WebViewError::Host(host) => host.attempted.clone(),
                _ => Vec::new(),
            };
            (error.stage(), error.hresult(), attempted)
        }
        None => (None, None, Vec::new()),
    };
    json!({
        "ok": ok,
        "status": status,
        "stage": stage,
        "hresult": hresult,
        "attempted": attempted,
    })
}

/// Resolves the GUI content directory: explicit override, then the built tree.
fn resolve_gui_dir() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("OPEN_EDA_GUI") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
        if path.is_file() {
            return path.parent().map(Path::to_path_buf);
        }
    }

    let manifest_relative = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("projects")
        .join("packages")
        .join("open-eda")
        .join("dist")
        .join("gui");
    if manifest_relative.join("index.html").is_file() {
        return Some(manifest_relative);
    }

    if let Ok(cwd) = std::env::current_dir() {
        for candidate in [cwd.join("projects/packages/open-eda/dist/gui"), cwd.join("dist/gui"), cwd.join("gui")] {
            if candidate.join("index.html").is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Opens the OpenEDA Studio window, or focuses the live one.
#[function(js_name = "openGui")]
fn open_gui(ctx: CallContext) -> Result<JsValue> {
    let mut title = "OpenEDA Studio".to_string();
    let mut content_dir: Option<String> = None;
    let mut width = 1280i32;
    let mut height = 820i32;

    if let Some(arg) = ctx.args().first().copied()
        && !arg.is_undefined()?
    {
        if let Some(value) = optional_string_prop(arg, "title")? {
            title = value;
        }
        if let Some(value) = optional_string_prop(arg, "contentDir")? {
            content_dir = Some(value);
        }
        if let Ok(value) = arg.get_named("width")
            && let Ok(number) = value.as_i64()
        {
            width = number as i32;
        }
        if let Ok(value) = arg.get_named("height")
            && let Ok(number) = value.as_i64()
        {
            height = number as i32;
        }
    }

    let live = GUI.with(|slot| {
        let guard = slot.borrow();
        guard
            .as_ref()
            .filter(|state| state.host.is_alive())
            .map(|state| (state.backend, state.opened_at.elapsed().as_millis() as u64))
    });
    if let Some((backend, uptime_ms)) = live {
        GUI.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                let _ = state.host.focus();
            }
        });
        let status = gui_status("open", None, Some(backend), uptime_ms, None);
        return respond(&ctx, &gui_open_result(true, status, None));
    }
    GUI.with(|slot| {
        *slot.borrow_mut() = None;
    });

    let dir = match content_dir {
        Some(dir) => PathBuf::from(dir),
        None => match resolve_gui_dir() {
            Some(dir) => dir,
            None => {
                let status = gui_status(
                    "failed",
                    Some("gui_content_missing"),
                    None,
                    0,
                    Some("the WebView content directory is missing; build the GUI first".to_string()),
                );
                return respond(&ctx, &gui_open_result(false, status, None));
            }
        },
    };

    let close_requested = Arc::new(AtomicBool::new(false));
    let mut options = OpenOptions::from_content_dir(dir.display().to_string(), title);
    options.width = width;
    options.height = height;
    options.accept_file_drop = true;
    // Windows draws its own window chrome in the page; the other backends keep
    // the native decorations.
    options.decorations = !cfg!(target_os = "windows");

    match WebViewHost::open(options, Box::new(DesignIpcHandler { close_requested: Arc::clone(&close_requested) })) {
        Ok(host) => {
            let backend = host.backend_name();
            GUI.with(|slot| {
                *slot.borrow_mut() = Some(GuiState { host, backend, opened_at: Instant::now(), close_requested });
            });
            let status = gui_status("open", None, Some(backend), 0, None);
            respond(&ctx, &gui_open_result(true, status, None))
        }
        Err(error) => {
            let status = gui_status("failed", error.code(), None, 0, Some(error.to_string()));
            respond(&ctx, &gui_open_result(false, status, Some(&error)))
        }
    }
}

/// Applies queued drag regions and window commands before the next pump.
///
/// The IPC worker reports into these queues; failures are ignored because the
/// page can always report again and the window lifetime is not affected.
fn apply_window_updates(state: &mut GuiState) {
    if let Ok(mut pending) = DRAG_REGIONS.lock()
        && let Some(rects) = pending.take()
    {
        let _ = state.host.set_drag_regions(&rects);
    }
    let commands: Vec<WindowCommand> = match WINDOW_COMMANDS.lock() {
        Ok(mut queue) => queue.drain(..).collect(),
        Err(_) => Vec::new(),
    };
    for command in commands {
        let _ = match command {
            WindowCommand::Minimize => state.host.minimize(),
            WindowCommand::ToggleMaximize => state.host.toggle_maximize(),
        };
    }
}

/// Pumps the host message loop once and reports the observed lifecycle state.
#[function(js_name = "pumpEvents")]
fn pump_events(ctx: CallContext) -> Result<JsValue> {
    let status = GUI.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut()
        else {
            return gui_status("closed", Some(GUI_NOT_OPEN), None, 0, Some("no OpenEDA window is open".to_string()));
        };
        let backend = state.backend;
        let uptime_ms = state.opened_at.elapsed().as_millis() as u64;
        apply_window_updates(state);
        if state.host.pump() {
            return gui_status("open", None, Some(backend), uptime_ms, None);
        }
        let requested = state.close_requested.swap(false, Ordering::SeqCst);
        *guard = None;
        if requested {
            gui_status("closed", Some(GUI_CLOSED), Some(backend), uptime_ms, None)
        }
        else {
            gui_status(
                "closed",
                Some(GUI_HOST_EXITED),
                Some(backend),
                uptime_ms,
                Some(format!("the {backend} webview host stopped without a page close request")),
            )
        }
    });
    respond(&ctx, &status)
}

/// Closes the OpenEDA window; the result distinguishes "already closed".
#[function(js_name = "closeGui")]
fn close_gui(ctx: CallContext) -> Result<JsValue> {
    let result = GUI.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut()
        else {
            return json!({
                "closed": false,
                "status": gui_status("not_open", Some(GUI_NOT_OPEN), None, 0, Some("no OpenEDA window is open".to_string())),
            });
        };
        let backend = state.backend;
        let uptime_ms = state.opened_at.elapsed().as_millis() as u64;
        match state.host.close() {
            Ok(()) => {
                *guard = None;
                json!({ "closed": true, "status": gui_status("closed", None, Some(backend), uptime_ms, None) })
            }
            Err(error) => json!({
                "closed": false,
                "status": gui_status("open", Some("gui_close_failed"), Some(backend), uptime_ms, Some(error.to_string())),
            }),
        }
    });
    respond(&ctx, &result)
}

/// Reports whether the OpenEDA window is alive.
#[function(js_name = "isGuiOpen")]
fn is_gui_open(ctx: CallContext) -> Result<JsValue> {
    let open = GUI.with(|slot| slot.borrow().as_ref().is_some_and(|state| state.host.is_alive()));
    ctx.env().boolean(open)
}

register_module![
    version,
    capabilities,
    design_create,
    design_open,
    design_save,
    design_set_properties,
    design_set_pin_type,
    design_annotate,
    design_inspect,
    design_inspect_with,
    design_export,
    import_kicad_sch,
    import_kicad_symbol,
    import_kicad_net,
    design_compare_netlist,
    diff_documents,
    design_close,
    design_summary,
    design_list,
    design_add_component,
    design_delete_component,
    design_connect_net,
    design_add_wire,
    design_update_wire,
    design_split_wire,
    design_translate,
    design_list_wires,
    design_add_junction,
    design_add_text,
    design_add_no_connect,
    design_delete_no_connect,
    design_list_components,
    design_list_nets,
    design_add_bus,
    design_remove_bus,
    design_list_buses,
    design_add_net_class,
    design_remove_net_class,
    design_list_net_classes,
    design_set_reference_prefix,
    design_remove_reference_prefix,
    design_list_reference_prefixes,
    design_set_component_class,
    design_list_component_classes,
    design_add_variant,
    design_remove_variant,
    design_list_variants,
    design_symbols,
    design_connectivity,
    design_undo,
    design_redo,
    design_add_sheet,
    design_rename_sheet,
    design_delete_sheet,
    design_move_component,
    design_update_component,
    design_set_component_footprint,
    design_rename_net,
    design_delete_wire,
    design_derive_nets,
    design_disconnect_net,
    design_delete_net,
    design_delete_junction,
    design_move_junction,
    design_delete_text,
    design_move_text,
    open_gui,
    pump_events,
    close_gui,
    is_gui_open
];
