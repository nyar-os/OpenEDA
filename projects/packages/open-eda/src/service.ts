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
    Bus,
    BusInput,
    Component,
    ComponentInput,
    ConnectNetInput,
    DeleteNetInput,
    DesignCapabilities,
    DesignCreateOptions,
    DesignObjects,
    DesignSummary,
    DeriveNetsResult,
    DisconnectNetInput,
    ExportReport,
    ExportTarget,
    HistoryResult,
    InspectCheck,
    InspectReport,
    Junction,
    JunctionInput,
    KicadImportResult,
    MoveComponentInput,
    Net,
    RenameNetInput,
    SaveReport,
    Sheet,
    TextInput,
    TextLabel,
    UpdateComponentInput,
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
        binding('create').design_create(JSON.stringify({ title: request.title, template: request.template })),
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

/** Runs the requested rule checks on one open design. */
export function inspect(handle: number, checks?: readonly InspectCheck[]): InspectReport {
    return decode<InspectReport>(() =>
        checks === undefined
            ? binding('inspect').design_inspect(handle)
            : binding('inspect').design_inspect(handle, JSON.stringify(checks)),
    );
}

/** Projects one open design to an exchange target. */
export function exportDesign(handle: number, format: ExportTarget, target: string): ExportReport {
    return decode<ExportReport>(() => binding('export').design_export(handle, format, target));
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
        binding('design.addBus').design_add_bus(handle, JSON.stringify({ name: input.name, width: input.width })),
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
