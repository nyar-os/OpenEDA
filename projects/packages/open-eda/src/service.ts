/**
 * Orthogonal native operations.
 *
 * One function performs one operation and returns data; argument parsing, help
 * text and exit codes belong to `cli.ts` and to the authoring surface. Nothing
 * here knows about a command line.
 */

import { NotImplementedError, OpenEDAError, errorCodes, type OpenEDAErrorCode } from './errors.js';
import { nativeBinding } from './native.js';
import type {
    AddHierarchyInput,
    AddHierarchyPortInput,
    AddPinInput,
    AnnotateResult,
    Bus,
    BusInput,
    Component,
    ComponentClassInput,
    ComponentClassListResult,
    ComponentClassResult,
    ComponentInput,
    ComponentPropertyListResult,
    ComponentPropertyResult,
    ConnectNetInput,
    ConnectivityReport,
    CopyComponentInput,
    DeleteNetInput,
    DeleteNetClassResult,
    DeleteReferencePrefixResult,
    DesignCapabilities,
    DesignCreateOptions,
    DesignDiffInput,
    DesignDiffReport,
    DesignObjects,
    DesignPropertiesInput,
    DesignPropertiesResult,
    DesignStats,
    DesignSummary,
    DeriveNetsResult,
    DisconnectNetInput,
    ExportReport,
    ExportTarget,
    HistoryResult,
    HierarchyCheckReport,
    HierarchyListResult,
    HierarchyPortResult,
    HierarchyResult,
    InspectOptions,
    InspectReport,
    Junction,
    JunctionInput,
    JunctionListResult,
    KicadImportResult,
    KicadNetImportResult,
    KicadSymbolImportResult,
    LabelDeleteResult,
    LabelInput,
    LabelListResult,
    LabelUpdateInput,
    LabelUpdateResult,
    MergeNetsInput,
    MergeNetsResult,
    MoveComponentInput,
    Net,
    NetClassInput,
    NetClassListResult,
    NetClassResult,
    NetlistDiffReport,
    NoConnect,
    NoConnectListResult,
    NoConnectInput,
    ReferencePrefixInput,
    ReferencePrefixListResult,
    ReferencePrefixResult,
    RemoveComponentPropertyInput,
    RemoveHierarchyPortInput,
    RemoveHierarchyResult,
    RemovePinInput,
    RenameNetInput,
    RenamePinInput,
    SaveReport,
    SetComponentPropertyInput,
    SetFootprintInput,
    SetPinTypeInput,
    SetPinTypeResult,
    Sheet,
    SheetListResult,
    SplitWireInput,
    SplitWireResult,
    SymbolLibraryResult,
    TextInput,
    TextLabel,
    TextListResult,
    TextUpdateInput,
    TextUpdateResult,
    TraceReport,
    TranslateInput,
    TranslateResult,
    UpdateComponentInput,
    UpdateWireInput,
    Variant,
    VariantInput,
    VariantListResult,
    VariantResult,
    Wire,
    WireInput,
} from './types.js';

/** Result of creating or opening a design. */
export interface DesignHandle {
    readonly handle: number;
    readonly design: DesignSummary;
}

/** Revision and summary returned by every edit. */
export interface EditResult {
    readonly revision: number;
    readonly design: DesignSummary;
}

/** History state every edit also reports. */
export interface HistoryFlags {
    readonly canUndo: boolean;
    readonly canRedo: boolean;
}

/** Result of {@link addComponent}. */
export interface ComponentResult extends EditResult, HistoryFlags {
    readonly component: Component;
}

/** Result of {@link deleteComponent}. */
export interface DeleteResult extends EditResult, HistoryFlags {
    readonly deleted: string;
}

/** Result of {@link connectNet}. */
export interface NetResult extends EditResult, HistoryFlags {
    readonly net: Net;
}

/** Result of {@link addWire}. */
export interface WireResult extends EditResult, HistoryFlags {
    readonly wire: Wire;
}

/** Result of {@link splitWire}. */
export interface SplitWireEditResult extends EditResult, HistoryFlags, SplitWireResult {}

/** Result of {@link translate}. */
export interface TranslateEditResult extends EditResult, HistoryFlags, TranslateResult {}

/** Result of {@link listWires}. */
export interface WireListResult extends EditResult {
    readonly sheetId: string;
    readonly wires: readonly Wire[];
}

/** Result of {@link addJunction}. */
export interface JunctionResult extends EditResult, HistoryFlags {
    readonly junction: Junction;
}

/** Result of {@link addText}. */
export interface TextResult extends EditResult, HistoryFlags {
    readonly text: TextLabel;
}

/** Result of {@link addNoConnect}. */
export interface NoConnectResult extends EditResult, HistoryFlags {
    readonly noConnect: NoConnect;
}

/** Result of {@link addSheet}, {@link renameSheet} and {@link deleteSheet}. */
export interface SheetResult extends EditResult, HistoryFlags {
    readonly sheet: Sheet | { readonly id: string; readonly name: string };
}

/** Result of {@link listComponents}. */
export interface ComponentListResult extends EditResult {
    readonly sheetId: string;
    readonly components: readonly Component[];
}

/** Result of {@link listNets}. */
export interface NetListResult extends EditResult {
    readonly sheetId: string;
    readonly nets: readonly Net[];
}

/** Result of {@link deriveNets}. */
export interface DeriveNetsEditResult extends EditResult, HistoryFlags, DeriveNetsResult {}

/** Result of {@link disconnectNet}. */
export interface DisconnectResult extends EditResult, HistoryFlags {
    readonly net: Net;
    readonly remaining: number;
    readonly removed: number;
}

/** Result of {@link deleteNet}. */
export interface DeleteNetResult extends EditResult, HistoryFlags {
    readonly deleted: string;
    readonly detached: number;
}

/** Result of {@link mergeNets}. */
export interface MergeNetsEditResult extends EditResult, HistoryFlags, MergeNetsResult {}

/** Result of {@link addBus}. */
export interface BusResult extends EditResult, HistoryFlags {
    readonly bus: Bus;
}

/** Result of {@link removeBus}. */
export interface DeleteBusResult extends EditResult, HistoryFlags {
    readonly deleted: string;
    readonly remaining: number;
}

/** Result of {@link listBuses}. */
export interface BusListResult extends EditResult {
    readonly buses: readonly Bus[];
}

/** Result of {@link deleteJunction} and {@link deleteText}. */
export interface DeleteIndexResult extends EditResult, HistoryFlags {
    readonly index: number;
    readonly remaining: number;
}

/** Result of {@link moveJunction}. */
export interface JunctionEditResult extends EditResult, HistoryFlags {
    readonly index: number;
    readonly junction: Junction;
}

/** Result of {@link moveText}. */
export interface TextEditResult extends EditResult, HistoryFlags {
    readonly index: number;
    readonly text: TextLabel;
}

const knownCodes = new Set<string>(errorCodes);

function binding(feature: string) {
    const native = nativeBinding();
    if (!native) {
        throw new NotImplementedError(feature);
    }
    return native;
}

function decode<T>(work: () => string): T {
    try {
        return JSON.parse(work()) as T;
    } catch (error) {
        throw translate(error);
    }
}

function translate(error: unknown): OpenEDAError {
    if (error instanceof OpenEDAError) {
        return error;
    }
    const message = error instanceof Error ? error.message : String(error);
    const separator = message.indexOf(': ');
    const code = separator > 0 ? message.slice(0, separator) : '';
    if (knownCodes.has(code)) {
        return new OpenEDAError(code as OpenEDAErrorCode, message.slice(separator + 2), { details: { feature: code } });
    }
    return new OpenEDAError('format/invalid', message, { cause: error });
}

/** Reads the capability matrix of the installed addon. */
export function capabilities(): DesignCapabilities {
    return decode<DesignCapabilities>(() => binding('capabilities').capabilities());
}

/** Creates a design and returns its handle. */
export function create(request: DesignCreateOptions): DesignHandle {
    return decode<DesignHandle>(() =>
        binding('create').design_create(
            JSON.stringify({
                title: request.title,
                rev: request.rev,
                company: request.company,
                date: request.date,
                template: request.template,
            }),
        ),
    );
}

/** Opens a native design document and returns its handle. */
export function open(path: string): DesignHandle {
    return decode<DesignHandle>(() => binding('open').design_open(path));
}

/** Writes one open design to a native document path. */
export function save(handle: number, target: string, overwrite: boolean): SaveReport {
    return decode<SaveReport>(() => binding('save').design_save(handle, target, overwrite));
}

/** Updates the drawing properties of one open design. */
export function setDesignProperties(handle: number, input: DesignPropertiesInput): DesignPropertiesResult {
    return decode<DesignPropertiesResult>(() =>
        binding('design.setProperties').design_set_properties(
            handle,
            JSON.stringify({
                title: input.title,
                rev: input.rev,
                company: input.company,
                date: input.date,
            }),
        ),
    );
}

/** Assigns or clears the electrical role of one component pin. */
export function setPinType(handle: number, input: SetPinTypeInput): SetPinTypeResult {
    return decode<SetPinTypeResult>(() =>
        binding('design.setPinType').design_set_pin_type(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                reference: input.reference,
                pin: input.pin,
                pinType: input.pinType,
            }),
        ),
    );
}

/** Adds one pin to a placed component. */
export function addPin(handle: number, input: AddPinInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.addPin').design_add_pin(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, pin: input.pin }),
        ),
    );
}

/** Removes one unreferenced pin from a placed component. */
export function removePin(handle: number, input: RemovePinInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.removePin').design_remove_pin(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, pin: input.pin }),
        ),
    );
}

/** Renames one pin and every `REF.PIN` reference that named it. */
export function renamePin(handle: number, input: RenamePinInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.renamePin').design_rename_pin(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, from: input.from, to: input.to }),
        ),
    );
}

/** Renumbers every reference designator of one open design. */
export function annotate(handle: number): AnnotateResult {
    return decode<AnnotateResult>(() => binding('design.annotate').design_annotate(handle));
}

/** Runs the requested rule checks on one open design. */
export function inspect(handle: number, options?: InspectOptions): InspectReport {
    if (options?.severity !== undefined || options?.waivers !== undefined) {
        return decode<InspectReport>(() =>
            binding('design.inspectWith').design_inspect_with(
                handle,
                JSON.stringify({
                    checks: options.checks,
                    severity: options.severity ?? {},
                    waivers: options.waivers ?? [],
                }),
            ),
        );
    }
    const checks = options?.checks;
    return decode<InspectReport>(() =>
        checks === undefined
            ? binding('inspect').design_inspect(handle)
            : binding('inspect').design_inspect(handle, JSON.stringify([...checks])),
    );
}

/** Projects one open design to an exchange target. */
export function exportDesign(handle: number, format: ExportTarget, target: string, variant?: string): ExportReport {
    const native = binding('export');
    return decode<ExportReport>(() =>
        variant === undefined ? native.design_export(handle, format, target) : native.design_export(handle, format, target, variant),
    );
}

/**
 * Imports a KiCad schematic from its text or a file path.
 *
 * The result carries the imported document and every construct the reader could
 * not represent, so nothing is dropped without a loss entry.
 */
export function importKicadSch(source: string): KicadImportResult {
    return decode<KicadImportResult>(() => binding('import.kicadSch').import_kicad_sch(source));
}

/**
 * Reads a KiCad `.kicad_sym` symbol library.
 *
 * The result lists every declared symbol with its unit count and pins; nothing
 * is written to disk and no design handle is opened.
 */
export function importKicadSymbol(source: string): KicadSymbolImportResult {
    return decode<KicadSymbolImportResult>(() => binding('import.kicadSymbol').import_kicad_symbol(source));
}

/**
 * Imports a KiCad netlist.
 *
 * The result carries the imported document, its totals and every construct the
 * reader could not represent; nothing is written to disk.
 */
export function importKicadNet(source: string): KicadNetImportResult {
    return decode<KicadNetImportResult>(() => binding('import.kicadNet').import_kicad_net(source));
}

/**
 * Compares one open design with a KiCad netlist.
 *
 * The design is the reference side, so an entry that exists only in the netlist
 * is reported as added. Netlist losses are returned next to the report.
 */
export function compareNetlist(handle: number, source: string): NetlistDiffReport {
    return decode<NetlistDiffReport>(() => binding('design.compareNetlist').design_compare_netlist(handle, source));
}

/**
 * Compares two native documents without opening either for editing.
 *
 * The report lists added, removed and modified sheets, components, nets, wires
 * and buses; nothing is written to disk.
 */
export function diff(input: DesignDiffInput): DesignDiffReport {
    return decode<DesignDiffReport>(() => binding('diffDocuments').diff_documents(input.left, input.right));
}

/** Releases one design handle. */
export function close(handle: number): void {
    binding('design.close').design_close(handle);
}

/** Reads the current summary of one open design. */
export function summary(handle: number): DesignSummary {
    return decode<DesignSummary>(() => binding('design.summary').design_summary(handle));
}

/** Lists components, nets and wires of one open design. */
export function listObjects(handle: number): DesignObjects {
    return decode<DesignObjects>(() => binding('design.list').design_list(handle));
}

/** Lists the symbol library one open design uses. */
export function symbols(handle: number): SymbolLibraryResult {
    return decode<SymbolLibraryResult>(() => binding('design.symbols').design_symbols(handle));
}

/** Reports what each net of one open design connects, plus unconnected pins and totals. */
export function connectivity(handle: number): ConnectivityReport {
    return decode<ConnectivityReport>(() => binding('design.connectivity').design_connectivity(handle));
}

/** Traces the read-only connection path between two `REF.PIN` pins. */
export function tracePins(handle: number, from: string, to: string): TraceReport {
    return decode<TraceReport>(() => binding('design.trace').design_trace(handle, from, to));
}

/** Adds one component to an open design. */
export function addComponent(handle: number, input: ComponentInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.addComponent').design_add_component(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                reference: input.reference,
                value: input.value,
                x: input.x,
                y: input.y,
                rotation: input.rotation ?? 0,
                pins: [...input.pins],
            }),
        ),
    );
}

/** Deletes one component from an open design. */
export function deleteComponent(handle: number, reference: string, sheet?: string): DeleteResult {
    const native = binding('design.deleteComponent');
    return decode<DeleteResult>(() =>
        sheet === undefined
            ? native.design_delete_component(handle, reference)
            : native.design_delete_component(handle, reference, sheet),
    );
}

/** Connects nodes to a named net on an open design. */
export function connectNet(handle: number, input: ConnectNetInput): NetResult {
    return decode<NetResult>(() =>
        binding('design.connectNet').design_connect_net(
            handle,
            JSON.stringify({ sheet: input.sheet, net: input.net, nodes: [...input.nodes] }),
        ),
    );
}

/** Adds one wire segment to an open design. */
export function addWire(handle: number, input: WireInput): WireResult {
    return decode<WireResult>(() =>
        binding('design.addWire').design_add_wire(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                net: input.net,
                from: [input.from[0], input.from[1]],
                to: [input.to[0], input.to[1]],
            }),
        ),
    );
}

/** Updates the endpoints or net of one wire segment. */
export function updateWire(handle: number, input: UpdateWireInput): WireResult {
    return decode<WireResult>(() =>
        binding('design.updateWire').design_update_wire(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                id: input.id,
                net: input.net,
                from: input.from === undefined ? undefined : [input.from[0], input.from[1]],
                to: input.to === undefined ? undefined : [input.to[0], input.to[1]],
            }),
        ),
    );
}

/** Splits one wire segment into two at a point on the segment. */
export function splitWire(handle: number, input: SplitWireInput): SplitWireEditResult {
    return decode<SplitWireEditResult>(() =>
        binding('design.splitWire').design_split_wire(
            handle,
            JSON.stringify({ sheet: input.sheet, id: input.id, x: input.x, y: input.y }),
        ),
    );
}

/** Translates selected components and wires of one sheet by an offset. */
export function translateObjects(handle: number, input: TranslateInput): TranslateEditResult {
    return decode<TranslateEditResult>(() =>
        binding('design.translate').design_translate(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                references: [...(input.references ?? [])],
                wires: [...(input.wires ?? [])],
                dx: input.dx,
                dy: input.dy,
            }),
        ),
    );
}

/** Lists the wires drawn on one sheet. */
export function listWires(handle: number, sheet?: string): WireListResult {
    const native = binding('design.listWires');
    return decode<WireListResult>(() =>
        sheet === undefined ? native.design_list_wires(handle) : native.design_list_wires(handle, sheet),
    );
}

/** Adds one junction (connection dot) to an open design. */
export function addJunction(handle: number, input: JunctionInput): JunctionResult {
    return decode<JunctionResult>(() =>
        binding('design.addJunction').design_add_junction(
            handle,
            JSON.stringify({ sheet: input.sheet, x: input.x, y: input.y }),
        ),
    );
}

/** Adds one free text annotation to an open design. */
export function addText(handle: number, input: TextInput): TextResult {
    return decode<TextResult>(() =>
        binding('design.addText').design_add_text(
            handle,
            JSON.stringify({ sheet: input.sheet, text: input.text, x: input.x, y: input.y }),
        ),
    );
}

/** Adds one net label to a sheet. */
export function addLabel(handle: number, input: LabelInput): LabelUpdateResult {
    return decode<LabelUpdateResult>(() =>
        binding('design.addLabel').design_add_label(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                text: input.text,
                x: input.x,
                y: input.y,
                scope: input.scope,
                rotation: input.rotation,
                justify: input.justify,
            }),
        ),
    );
}

/** Lists the net labels of one sheet. */
export function listLabels(handle: number, sheet?: string): LabelListResult {
    const native = binding('design.listLabels');
    return decode<LabelListResult>(() =>
        sheet === undefined ? native.design_list_labels(handle) : native.design_list_labels(handle, sheet),
    );
}

/** Updates one net label by identity. */
export function updateLabel(handle: number, input: LabelUpdateInput): LabelUpdateResult {
    return decode<LabelUpdateResult>(() =>
        binding('design.updateLabel').design_update_label(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                id: input.id,
                text: input.text,
                x: input.x,
                y: input.y,
                scope: input.scope,
                rotation: input.rotation,
                justify: input.justify,
            }),
        ),
    );
}

/** Deletes one net label by identity. */
export function removeLabel(handle: number, id: string, sheet?: string): LabelDeleteResult {
    return decode<LabelDeleteResult>(() => binding('design.removeLabel').design_delete_label(handle, id, sheet));
}

/** Lists the junctions of one sheet. */
export function listJunctions(handle: number, sheet?: string): JunctionListResult {
    const native = binding('design.listJunctions');
    return decode<JunctionListResult>(() =>
        sheet === undefined ? native.design_list_junctions(handle) : native.design_list_junctions(handle, sheet),
    );
}

/** Lists the no-connect flags of one sheet. */
export function listNoConnects(handle: number, sheet?: string): NoConnectListResult {
    const native = binding('design.listNoConnects');
    return decode<NoConnectListResult>(() =>
        sheet === undefined ? native.design_list_no_connects(handle) : native.design_list_no_connects(handle, sheet),
    );
}

/** Lists every sheet with its identity, name and object counts. */
export function listSheets(handle: number): SheetListResult {
    return decode<SheetListResult>(() => binding('design.listSheets').design_list_sheets(handle));
}

/** Reports deterministic per-sheet and design-wide object counts. */
export function stats(handle: number): DesignStats {
    return decode<DesignStats>(() => binding('design.stats').design_stats(handle));
}

/** Copies one placed component under a new reference designator. */
export function copyComponent(handle: number, input: CopyComponentInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.copyComponent').design_copy_component(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                reference: input.reference,
                newReference: input.newReference,
                dx: input.dx ?? 0,
                dy: input.dy ?? 0,
            }),
        ),
    );
}

/** Lists the free text annotations of one sheet. */
export function listTexts(handle: number, sheet?: string): TextListResult {
    const native = binding('design.listTexts');
    return decode<TextListResult>(() =>
        sheet === undefined ? native.design_list_texts(handle) : native.design_list_texts(handle, sheet),
    );
}

/** Updates the content or placement of one text annotation. */
export function updateText(handle: number, input: TextUpdateInput): TextUpdateResult {
    return decode<TextUpdateResult>(() =>
        binding('design.updateText').design_update_text(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                index: input.index,
                text: input.text,
                x: input.x,
                y: input.y,
            }),
        ),
    );
}

/** Adds one no-connect flag, optionally marking a component pin. */
export function addNoConnect(handle: number, input: NoConnectInput): NoConnectResult {
    return decode<NoConnectResult>(() =>
        binding('design.addNoConnect').design_add_no_connect(
            handle,
            JSON.stringify({ sheet: input.sheet, x: input.x, y: input.y, node: input.node }),
        ),
    );
}

/** Deletes the no-connect flag at a 0-based sheet index. */
export function deleteNoConnect(handle: number, index: number, sheet?: string): DeleteIndexResult {
    return decode<DeleteIndexResult>(() => binding('design.deleteNoConnect').design_delete_no_connect(handle, index, sheet));
}

/** Restores the snapshot taken before the newest edit. */
export function undo(handle: number): HistoryResult {
    return decode<HistoryResult>(() => binding('design.undo').design_undo(handle));
}

/** Reapplies the snapshot the newest undo displaced. */
export function redo(handle: number): HistoryResult {
    return decode<HistoryResult>(() => binding('design.redo').design_redo(handle));
}

/** Moves one placed component. */
export function moveComponent(handle: number, input: MoveComponentInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.moveComponent').design_move_component(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                reference: input.reference,
                x: input.x,
                y: input.y,
                rotation: input.rotation,
            }),
        ),
    );
}

/** Replaces the value text of one component. */
export function updateComponent(handle: number, input: UpdateComponentInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.updateComponent').design_update_component(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, value: input.value }),
        ),
    );
}

/** Assigns or clears the footprint text of one component. */
export function setComponentFootprint(handle: number, input: SetFootprintInput): ComponentResult {
    return decode<ComponentResult>(() =>
        binding('design.setComponentFootprint').design_set_component_footprint(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, footprint: input.footprint }),
        ),
    );
}

/** Assigns one custom key/value property of a placed component. */
export function setComponentProperty(handle: number, input: SetComponentPropertyInput): ComponentPropertyResult {
    return decode<ComponentPropertyResult>(() =>
        binding('design.setComponentProperty').design_set_component_property(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                reference: input.reference,
                key: input.key,
                value: input.value,
            }),
        ),
    );
}

/** Removes one custom property of a placed component. */
export function removeComponentProperty(
    handle: number,
    input: RemoveComponentPropertyInput,
): ComponentPropertyResult {
    return decode<ComponentPropertyResult>(() =>
        binding('design.removeComponentProperty').design_remove_component_property(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, key: input.key }),
        ),
    );
}

/** Lists the custom properties of one placed component, sorted by key. */
export function listComponentProperties(
    handle: number,
    reference: string,
    sheet?: string,
): ComponentPropertyListResult {
    return decode<ComponentPropertyListResult>(() =>
        binding('design.listComponentProperties').design_list_component_properties(handle, reference, sheet),
    );
}

/** Renames one net and every wire that named it. */
export function renameNet(handle: number, input: RenameNetInput): NetResult {    return decode<NetResult>(() =>
        binding('design.renameNet').design_rename_net(
            handle,
            JSON.stringify({ sheet: input.sheet, net: input.net, name: input.name }),
        ),
    );
}

/** Deletes one wire segment. */
export function deleteWire(handle: number, wire: string, sheet?: string): DeleteResult {
    const native = binding('design.deleteWire');
    return decode<DeleteResult>(() =>
        sheet === undefined ? native.design_delete_wire(handle, wire) : native.design_delete_wire(handle, wire, sheet),
    );
}

/** Adds an empty sheet. */
export function addSheet(handle: number, name?: string): SheetResult {
    const native = binding('design.addSheet');
    return decode<SheetResult>(() =>
        name === undefined ? native.design_add_sheet(handle) : native.design_add_sheet(handle, name),
    );
}

/** Renames one sheet. */
export function renameSheet(handle: number, sheet: string, name: string): SheetResult {
    return decode<SheetResult>(() => binding('design.renameSheet').design_rename_sheet(handle, sheet, name));
}

/** Deletes one empty sheet. */
export function deleteSheet(handle: number, sheet: string): SheetResult {
    return decode<SheetResult>(() => binding('design.deleteSheet').design_delete_sheet(handle, sheet));
}

/** Adds one hierarchical sheet symbol. */
export function addHierarchy(handle: number, input: AddHierarchyInput): HierarchyResult {
    return decode<HierarchyResult>(() =>
        binding('design.addHierarchy').design_add_hierarchy(
            handle,
            JSON.stringify({
                name: input.name,
                file: input.file,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
            }),
        ),
    );
}

/** Lists the hierarchical sheet symbols of one design. */
export function listHierarchy(handle: number): HierarchyListResult {
    return decode<HierarchyListResult>(() => binding('design.listHierarchy').design_list_hierarchy(handle));
}

/** Checks the declared hierarchy against the child `.oeda` documents on disk. */
export function checkHierarchy(handle: number): HierarchyCheckReport {
    return decode<HierarchyCheckReport>(() => binding('design.checkHierarchy').design_check_hierarchy(handle));
}

/** Removes one hierarchical sheet symbol by id or name. */
export function removeHierarchy(handle: number, sheet: string): RemoveHierarchyResult {
    return decode<RemoveHierarchyResult>(() =>
        binding('design.removeHierarchy').design_remove_hierarchy(handle, sheet),
    );
}

/** Adds one port to a hierarchical sheet symbol. */
export function addHierarchyPort(handle: number, input: AddHierarchyPortInput): HierarchyPortResult {
    return decode<HierarchyPortResult>(() =>
        binding('design.addHierarchyPort').design_add_hierarchy_port(
            handle,
            JSON.stringify({
                sheet: input.sheet,
                name: input.name,
                direction: input.direction,
                x: input.x,
                y: input.y,
            }),
        ),
    );
}

/** Removes one port from a hierarchical sheet symbol. */
export function removeHierarchyPort(handle: number, input: RemoveHierarchyPortInput): RemoveHierarchyResult {
    return decode<RemoveHierarchyResult>(() =>
        binding('design.removeHierarchyPort').design_remove_hierarchy_port(handle, JSON.stringify(input)),
    );
}

/** Lists components of one open design. */
export function listComponents(handle: number, sheet?: string): ComponentListResult {
    const native = binding('design.listComponents');
    return decode<ComponentListResult>(() =>
        sheet === undefined ? native.design_list_components(handle) : native.design_list_components(handle, sheet),
    );
}

/** Lists nets of one open design. */
export function listNets(handle: number, sheet?: string): NetListResult {
    const native = binding('design.listNets');
    return decode<NetListResult>(() =>
        sheet === undefined ? native.design_list_nets(handle) : native.design_list_nets(handle, sheet),
    );
}

/** Names every connected wire group of one sheet and reports the counts. */
export function deriveNets(handle: number, sheet?: string): DeriveNetsEditResult {
    const native = binding('design.deriveNets');
    return decode<DeriveNetsEditResult>(() =>
        sheet === undefined ? native.design_derive_nets(handle) : native.design_derive_nets(handle, sheet),
    );
}

/** Removes `REF.PIN` nodes from a net while keeping the net and its wires. */
export function disconnectNet(handle: number, input: DisconnectNetInput): DisconnectResult {
    return decode<DisconnectResult>(() =>
        binding('design.disconnectNet').design_disconnect_net(
            handle,
            JSON.stringify({ sheet: input.sheet, net: input.net, nodes: [...input.nodes] }),
        ),
    );
}

/** Deletes a net and detaches the wires that named it. */
export function deleteNet(handle: number, input: DeleteNetInput): DeleteNetResult {
    return decode<DeleteNetResult>(() =>
        binding('design.deleteNet').design_delete_net(handle, JSON.stringify({ sheet: input.sheet, net: input.net })),
    );
}

/** Merges one net into another and repoints the wires that named it. */
export function mergeNets(handle: number, input: MergeNetsInput): MergeNetsEditResult {
    return decode<MergeNetsEditResult>(() =>
        binding('design.mergeNets').design_merge_nets(
            handle,
            JSON.stringify({ sheet: input.sheet, target: input.target, source: input.source }),
        ),
    );
}

/** Declares a design-level bus. */
export function addBus(handle: number, input: BusInput): BusResult {
    return decode<BusResult>(() =>
        binding('design.addBus').design_add_bus(
            handle,
            JSON.stringify({ name: input.name, width: input.width, aliases: input.aliases }),
        ),
    );
}

/** Removes one declared bus by id or name. */
export function removeBus(handle: number, bus: string): DeleteBusResult {
    return decode<DeleteBusResult>(() => binding('design.removeBus').design_remove_bus(handle, bus));
}

/** Lists the buses declared by one open design. */
export function listBuses(handle: number): BusListResult {
    return decode<BusListResult>(() => binding('design.listBuses').design_list_buses(handle));
}

/** Declares or replaces one BOM variant. */
export function addVariant(handle: number, input: VariantInput): VariantResult {
    return decode<VariantResult>(() =>
        binding('design.addVariant').design_add_variant(
            handle,
            JSON.stringify({ name: input.name, dnp: [...(input.dnp ?? [])], values: input.values ?? {} }),
        ),
    );
}

/** Removes one BOM variant by name. */
export function removeVariant(handle: number, name: string): DeleteBusResult {
    return decode<DeleteBusResult>(() => binding('design.removeVariant').design_remove_variant(handle, name));
}

/** Lists the BOM variants declared by one open design. */
export function listVariants(handle: number): VariantListResult {
    return decode<VariantListResult>(() => binding('design.listVariants').design_list_variants(handle));
}

/** Declares or replaces one net class. */
export function addNetClass(handle: number, input: NetClassInput): NetClassResult {
    return decode<NetClassResult>(() =>
        binding('design.addNetClass').design_add_net_class(
            handle,
            JSON.stringify({
                name: input.name,
                width: input.width,
                clearance: input.clearance ?? 0,
                via: input.via ?? 0,
                nets: [...(input.nets ?? [])],
                buses: [...(input.buses ?? [])],
            }),
        ),
    );
}

/** Removes one net class by id or name. */
export function removeNetClass(handle: number, netClass: string): DeleteNetClassResult {
    return decode<DeleteNetClassResult>(() =>
        binding('design.removeNetClass').design_remove_net_class(handle, netClass),
    );
}

/** Lists the net classes declared by one open design. */
export function listNetClasses(handle: number): NetClassListResult {
    return decode<NetClassListResult>(() => binding('design.listNetClasses').design_list_net_classes(handle));
}

/** Declares or replaces one reference designator prefix rule. */
export function setReferencePrefix(handle: number, input: ReferencePrefixInput): ReferencePrefixResult {
    return decode<ReferencePrefixResult>(() =>
        binding('design.setReferencePrefix').design_set_reference_prefix(
            handle,
            JSON.stringify({ class: input.class, prefix: input.prefix }),
        ),
    );
}

/** Removes one reference prefix rule by component class. */
export function removeReferencePrefix(handle: number, componentClass: string): DeleteReferencePrefixResult {
    return decode<DeleteReferencePrefixResult>(() =>
        binding('design.removeReferencePrefix').design_remove_reference_prefix(handle, componentClass),
    );
}

/** Lists the reference prefix rules declared by one open design. */
export function listReferencePrefixes(handle: number): ReferencePrefixListResult {
    return decode<ReferencePrefixListResult>(() =>
        binding('design.listReferencePrefixes').design_list_reference_prefixes(handle),
    );
}

/** Assigns or clears the component class of one placed component. */
export function setComponentClass(handle: number, input: ComponentClassInput): ComponentClassResult {
    return decode<ComponentClassResult>(() =>
        binding('design.setComponentClass').design_set_component_class(
            handle,
            JSON.stringify({ sheet: input.sheet, reference: input.reference, class: input.class }),
        ),
    );
}

/** Lists component class assignments with their resolved references. */
export function listComponentClasses(handle: number): ComponentClassListResult {
    return decode<ComponentClassListResult>(() =>
        binding('design.listComponentClasses').design_list_component_classes(handle),
    );
}

/** Deletes the junction at a 0-based index on a sheet. */
export function deleteJunction(handle: number, index: number, sheet?: string): DeleteIndexResult {
    return decode<DeleteIndexResult>(() => binding('design.deleteJunction').design_delete_junction(handle, index, sheet));
}

/** Moves the junction at a 0-based index to a new placement. */
export function moveJunction(handle: number, index: number, x: number, y: number, sheet?: string): JunctionEditResult {
    return decode<JunctionEditResult>(() =>
        binding('design.moveJunction').design_move_junction(handle, index, sheet, x, y),
    );
}

/** Deletes the text annotation at a 0-based index on a sheet. */
export function deleteText(handle: number, index: number, sheet?: string): DeleteIndexResult {
    return decode<DeleteIndexResult>(() => binding('design.deleteText').design_delete_text(handle, index, sheet));
}

/** Moves the text annotation at a 0-based index to a new placement. */
export function moveText(handle: number, index: number, x: number, y: number, sheet?: string): TextEditResult {
    return decode<TextEditResult>(() => binding('design.moveText').design_move_text(handle, index, sheet, x, y));
}
