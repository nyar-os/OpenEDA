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
    AnnotateResult,
    Bus,
    BusInput,
    Component,
    ComponentInput,
    ConnectNetInput,
    ConnectivityReport,
    DeleteNetInput,
    DesignCapabilities,
    DesignCreateOptions,
    DesignDiffInput,
    DesignDiffReport,
    DesignObjects,
    DesignPropertiesInput,
    DesignPropertiesResult,
    DesignSummary,
    DeriveNetsResult,
    DisconnectNetInput,
    ExportReport,
    ExportTarget,
    HistoryResult,
    InspectOptions,
    InspectReport,
    Junction,
    JunctionInput,
    KicadImportResult,
    KicadSymbolImportResult,
    MoveComponentInput,
    Net,
    NoConnect,
    NoConnectInput,
    RenameNetInput,
    SaveReport,
    SetFootprintInput,
    SetPinTypeInput,
    SetPinTypeResult,
    Sheet,
    SymbolLibraryResult,
    TextInput,
    TextLabel,
    UpdateComponentInput,
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

/** Renames one net and every wire that named it. */
export function renameNet(handle: number, input: RenameNetInput): NetResult {
    return decode<NetResult>(() =>
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
