/**
 * `new OpenEDA(options?)` authoring session.
 *
 * The constructor is synchronous and opens nothing: no network, no disk scan,
 * no window, no file. Native initialisation and imports happen inside the async
 * methods, so a failure is always awaited and always carries a stable code.
 *
 * The session is the API. `history: 'sidecar'` moves the cross-process undo
 * stack out of the CLI and into the SDK; the module that touches `node:fs` is
 * loaded lazily, only for sidecar history or an explicit import target.
 */

import { OpenEDAError } from './errors.js';
import type { DesignIntent } from './formats.js';
import * as service from './service.js';
import type {
    Bus,
    BusInput,
    Component,
    ComponentInput,
    ConnectNetInput,
    DeleteNetInput,
    Design,
    DesignCapabilities,
    DesignCreateOptions,
    DesignHistoryMode,
    DesignInput,
    DesignObjects,
    DesignSummary,
    DeriveNetsResult,
    DisconnectNetInput,
    ExportOptions,
    ExportReport,
    HistoryResult,
    InspectOptions,
    InspectReport,
    Junction,
    JunctionInput,
    KicadImportOptions,
    KicadImportResult,
    MoveComponentInput,
    Net,
    OpenEDAOptions,
    RenameNetInput,
    SaveOptions,
    SaveReport,
    SetFootprintInput,
    Sheet,
    SymbolLibraryResult,
    TextInput,
    TextLabel,
    UpdateComponentInput,
    Wire,
    WireInput,
} from './types.js';

type HistoryModule = typeof import('./history.js');

let historyModule: Promise<HistoryModule> | undefined;

/** Loads the sidecar and document helpers on first file-backed use. */
function loadHistory(): Promise<HistoryModule> {
    historyModule ??= import('./history.js');
    return historyModule;
}

/** Authoring session. One instance may manage several designs. */
export class OpenEDA {
    /** BCP-47 tag used for diagnostics and default copy. */
    readonly locale: string;
    /** Where undo history lives: native per handle, or also beside the document. */
    readonly history: DesignHistoryMode;
    private closed = false;
    private readonly handles = new Set<number>();

    constructor(options: OpenEDAOptions = {}) {
        if (options.history !== undefined && options.history !== 'memory' && options.history !== 'sidecar') {
            throw new OpenEDAError('session/argument-invalid', '`history` must be `memory` or `sidecar`');
        }
        this.locale = options.locale ?? 'en';
        this.history = options.history ?? 'memory';
    }

    /** Whether {@link dispose} has run. */
    get disposed(): boolean {
        return this.closed;
    }

    /**
     * Format, checker, renderer and export disclosure for the installed engine.
     * Read this instead of treating a method name as a capability.
     */
    async capabilities(): Promise<DesignCapabilities> {
        this.assertActive();
        return service.capabilities();
    }

    /** Creates an empty design with its own handle. */
    async create(options: DesignCreateOptions = {}): Promise<Design> {
        this.assertActive();
        assertCreateOptions(options);
        const created = service.create(options);
        this.handles.add(created.handle);
        return attach(created, {
            history: this.history,
            onClose: (handle) => this.handles.delete(handle),
            onHandleReplaced: (previous, next) => {
                this.handles.delete(previous);
                this.handles.add(next);
            },
        });
    }

    /** Opens a native design document or an in-memory document. */
    async open(input: DesignInput): Promise<Design> {
        this.assertActive();
        const path = openPath(input);
        const opened = service.open(path);
        this.handles.add(opened.handle);
        const counts =
            this.history === 'sidecar' ? (await loadHistory()).counts(path) : undefined;
        return attach(opened, {
            history: this.history,
            sourcePath: path,
            initialHistory: counts,
            onClose: (handle) => this.handles.delete(handle),
            onHandleReplaced: (previous, next) => {
                this.handles.delete(previous);
                this.handles.add(next);
            },
        });
    }

    /**
     * Imports a KiCad schematic from its text or a file path.
     *
     * Passing `target` writes the imported document as a native `.oeda` file and
     * clears any stale sidecar on that target.
     */
    async importKicadSch(options: KicadImportOptions): Promise<KicadImportResult> {
        this.assertActive();
        assertImportOptions(options);
        const imported = service.importKicadSch(options.source);
        if (options.target !== undefined) {
            const history = await loadHistory();
            history.writeDocument(options.target, imported.design);
            history.clearHistory(options.target);
        }
        return imported;
    }

    /** Inspects an input without keeping a design handle. */
    async inspect(input: DesignInput, options?: InspectOptions): Promise<InspectReport> {
        this.assertActive();
        const opened = service.open(openPath(input));
        try {
            return service.inspect(opened.handle, options?.checks);
        } finally {
            service.close(opened.handle);
        }
    }

    /** Releases the session and every handle it owns. Idempotent. */
    async dispose(): Promise<void> {
        if (this.closed) {
            return;
        }
        this.closed = true;
        for (const handle of this.handles) {
            try {
                service.close(handle);
            } catch {
                // A handle that already vanished must not keep the rest alive.
            }
        }
        this.handles.clear();
    }

    /** @internal */
    assertActive(): void {
        if (this.closed) {
            throw new OpenEDAError('session/disposed', 'the OpenEDA session has been disposed; create a new instance');
        }
    }
}

/**
 * Validates an open input and returns the native path it addresses.
 *
 * Nothing is opened here; `open()` and the transient `inspect()` path share the
 * same validation so both reject with the same codes.
 */
function openPath(input: DesignInput): string {
    assertDesignInput(input);
    if (!('path' in input)) {
        throw new OpenEDAError(
            'format/unsupported',
            'opening a serialized `project` needs the document reader; write it to a path and pass `path`',
            { details: { feature: 'open.project' } },
        );
    }
    const intent: DesignIntent = input.intent ?? 'edit';
    if (intent === 'exchange') {
        throw new OpenEDAError(
            'format/unsupported',
            'native documents open for editing; `exchange` needs the import pipeline',
            { details: { feature: 'open.intent.exchange' } },
        );
    }
    return input.path;
}

interface AttachOptions {
    readonly history: DesignHistoryMode;
    readonly sourcePath?: string;
    readonly initialHistory?: { readonly undo: number; readonly redo: number };
    readonly onClose?: (handle: number) => void;
    readonly onHandleReplaced?: (previous: number, next: number) => void;
}

/**
 * Wraps a native handle in the `Design` contract.
 *
 * `id`, `revision` and `summary` are getters, so an edit performed through one
 * of the methods is visible on the same object. In sidecar mode every mutation
 * writes the document back to `sourcePath` and records the replaced content;
 * `undo`/`redo` swap that snapshot, bump the revision, close the old handle and
 * reopen the document so the object reflects the restored state.
 */
function attach(opened: service.DesignHandle, options: AttachOptions): Design {
    const sidecar = options.history === 'sidecar';
    let handle = opened.handle;
    let sourcePath = options.sourcePath;
    let current: DesignSummary = opened.design;
    let counts = options.initialHistory;

    /** Summary with sidecar counts applied when the frontier is file-backed. */
    const view = (): DesignSummary =>
        counts === undefined
            ? current
            : {
                  ...current,
                  canUndo: counts.undo > 0,
                  canRedo: counts.redo > 0,
                  history: { ...current.history, undo: counts.undo, redo: counts.redo },
              };

    /** Reads the file a sidecar mutation is about to replace. */
    const before = async (): Promise<string | undefined> => {
        if (!sidecar || sourcePath === undefined) {
            return undefined;
        }
        return (await loadHistory()).readDocument(sourcePath);
    };

    /** Writes the mutated handle back in place and records the replaced file. */
    const after = async (previous: string | undefined): Promise<void> => {
        if (!sidecar || sourcePath === undefined) {
            return;
        }
        service.save(handle, sourcePath, true);
        if (previous !== undefined) {
            counts = (await loadHistory()).recordStep(sourcePath, previous);
        }
    };

    /** Runs one native mutation, persisting it when the session is file-backed. */
    const mutate = async <T>(apply: () => T): Promise<T> => {
        const previous = await before();
        const result = apply();
        await after(previous);
        return result;
    };

    /** Swaps one sidecar snapshot, then reopens so the handle matches the file. */
    const step = async (direction: 'undo' | 'redo'): Promise<HistoryResult> => {
        const path = sourcePath as string;
        const history = await loadHistory();
        const result = history.stepBack(path, direction);
        if (result === undefined) {
            throw new OpenEDAError(
                'history/empty',
                direction === 'undo' ? 'there is no earlier revision to restore' : 'there is no later revision to restore',
            );
        }
        const previous = handle;
        service.close(previous);
        const reopened = service.open(path);
        handle = reopened.handle;
        current = reopened.design;
        counts = { undo: result.undo, redo: result.redo };
        options.onHandleReplaced?.(previous, handle);
        return {
            revision: current.revision,
            design: view(),
            canUndo: result.undo > 0,
            canRedo: result.redo > 0,
        };
    };

    return {
        get id() {
            return current.id;
        },
        get revision() {
            return current.revision;
        },
        get summary() {
            return view();
        },
        get canUndo() {
            return counts === undefined ? current.canUndo : counts.undo > 0;
        },
        get canRedo() {
            return counts === undefined ? current.canRedo : counts.redo > 0;
        },
        async inspect(inspectOptions?: InspectOptions): Promise<InspectReport> {
            return service.inspect(handle, inspectOptions?.checks);
        },
        async save(saveOptions: SaveOptions = {}): Promise<SaveReport> {
            const target = saveOptions.target ?? sourcePath;
            if (target === undefined) {
                throw new OpenEDAError(
                    'session/argument-invalid',
                    '`target` is required because this design was not opened from a path',
                );
            }
            const report = service.save(handle, target, saveOptions.overwrite ?? (target === sourcePath));
            current = service.summary(handle);
            if (sidecar && target !== sourcePath) {
                sourcePath = target;
                const history = await loadHistory();
                history.clearHistory(target);
                counts = { undo: 0, redo: 0 };
            }
            return report;
        },
        async export(exportOptions: ExportOptions): Promise<ExportReport> {
            return service.exportDesign(handle, exportOptions.format, exportOptions.target);
        },
        async undo(): Promise<HistoryResult> {
            if (sidecar && sourcePath !== undefined) {
                return step('undo');
            }
            const result = service.undo(handle);
            current = result.design;
            return result;
        },
        async redo(): Promise<HistoryResult> {
            if (sidecar && sourcePath !== undefined) {
                return step('redo');
            }
            const result = service.redo(handle);
            current = result.design;
            return result;
        },
        async addComponent(input: ComponentInput): Promise<Component> {
            const result = await mutate(() => service.addComponent(handle, input));
            current = result.design;
            return result.component;
        },
        async deleteComponent(reference: string, deleteOptions?: { readonly sheet?: string }): Promise<void> {
            const result = await mutate(() => service.deleteComponent(handle, reference, deleteOptions?.sheet));
            current = result.design;
        },
        async connectNet(input: ConnectNetInput): Promise<Net> {
            const result = await mutate(() => service.connectNet(handle, input));
            current = result.design;
            return result.net;
        },
        async addWire(input: WireInput): Promise<Wire> {
            const result = await mutate(() => service.addWire(handle, input));
            current = result.design;
            return result.wire;
        },
        async addJunction(input: JunctionInput): Promise<Junction> {
            const result = await mutate(() => service.addJunction(handle, input));
            current = result.design;
            return result.junction;
        },
        async addText(input: TextInput): Promise<TextLabel> {
            const result = await mutate(() => service.addText(handle, input));
            current = result.design;
            return result.text;
        },
        async moveComponent(input: MoveComponentInput): Promise<Component> {
            const result = await mutate(() => service.moveComponent(handle, input));
            current = result.design;
            return result.component;
        },
        async updateComponent(input: UpdateComponentInput): Promise<Component> {
            const result = await mutate(() => service.updateComponent(handle, input));
            current = result.design;
            return result.component;
        },
        async setFootprint(input: SetFootprintInput): Promise<Component> {
            const result = await mutate(() => service.setComponentFootprint(handle, input));
            current = result.design;
            return result.component;
        },
        async renameNet(input: RenameNetInput): Promise<Net> {
            const result = await mutate(() => service.renameNet(handle, input));
            current = result.design;
            return result.net;
        },
        async deleteWire(wire: string, deleteOptions?: { readonly sheet?: string }): Promise<void> {
            const result = await mutate(() => service.deleteWire(handle, wire, deleteOptions?.sheet));
            current = result.design;
        },
        async deriveNets(sheet?: string): Promise<DeriveNetsResult> {
            const result = await mutate(() => service.deriveNets(handle, sheet));
            current = result.design;
            return { assigned: result.assigned, created: result.created, groups: result.groups };
        },
        async disconnectNet(input: DisconnectNetInput): Promise<Net> {
            const result = await mutate(() => service.disconnectNet(handle, input));
            current = result.design;
            return result.net;
        },
        async deleteNet(input: DeleteNetInput): Promise<void> {
            const result = await mutate(() => service.deleteNet(handle, input));
            current = result.design;
        },
        async addBus(input: BusInput): Promise<Bus> {
            const result = await mutate(() => service.addBus(handle, input));
            current = result.design;
            return result.bus;
        },
        async removeBus(bus: string): Promise<void> {
            const result = await mutate(() => service.removeBus(handle, bus));
            current = result.design;
        },
        async listBuses(): Promise<readonly Bus[]> {
            return service.listBuses(handle).buses;
        },
        async deleteJunction(index: number, deleteOptions?: { readonly sheet?: string }): Promise<void> {
            const result = await mutate(() => service.deleteJunction(handle, index, deleteOptions?.sheet));
            current = result.design;
        },
        async moveJunction(
            index: number,
            x: number,
            y: number,
            moveOptions?: { readonly sheet?: string },
        ): Promise<Junction> {
            const result = await mutate(() => service.moveJunction(handle, index, x, y, moveOptions?.sheet));
            current = result.design;
            return result.junction;
        },
        async deleteText(index: number, deleteOptions?: { readonly sheet?: string }): Promise<void> {
            const result = await mutate(() => service.deleteText(handle, index, deleteOptions?.sheet));
            current = result.design;
        },
        async moveText(
            index: number,
            x: number,
            y: number,
            moveOptions?: { readonly sheet?: string },
        ): Promise<TextLabel> {
            const result = await mutate(() => service.moveText(handle, index, x, y, moveOptions?.sheet));
            current = result.design;
            return result.text;
        },
        async addSheet(name?: string): Promise<Sheet> {
            const result = await mutate(() => service.addSheet(handle, name));
            current = result.design;
            return result.sheet as Sheet;
        },
        async renameSheet(sheet: string, name: string): Promise<Sheet> {
            const result = await mutate(() => service.renameSheet(handle, sheet, name));
            current = result.design;
            return result.sheet as Sheet;
        },
        async deleteSheet(sheet: string): Promise<void> {
            const result = await mutate(() => service.deleteSheet(handle, sheet));
            current = result.design;
        },
        async listObjects(): Promise<DesignObjects> {
            return service.listObjects(handle);
        },
        async listSymbols(): Promise<SymbolLibraryResult> {
            return service.symbols(handle);
        },
        async listComponents(): Promise<readonly Component[]> {
            return service.listComponents(handle).components;
        },
        async listNets(): Promise<readonly Net[]> {
            return service.listNets(handle).nets;
        },
        async close(): Promise<void> {
            service.close(handle);
            options.onClose?.(handle);
        },
    };
}

function assertCreateOptions(options: DesignCreateOptions): void {
    if (options.title !== undefined && (typeof options.title !== 'string' || options.title.trim().length === 0)) {
        throw new OpenEDAError('session/argument-invalid', '`title` must be a non-empty string');
    }
    if (options.template !== undefined && (typeof options.template !== 'string' || options.template.trim().length === 0)) {
        throw new OpenEDAError('session/argument-invalid', '`template` must be a non-empty string');
    }
}

function assertImportOptions(options: KicadImportOptions): void {
    if (typeof options.source !== 'string' || options.source.trim().length === 0) {
        throw new OpenEDAError('session/argument-invalid', '`source` must be non-empty KiCad schematic text or a path');
    }
    if (options.target !== undefined && (typeof options.target !== 'string' || options.target.trim().length === 0)) {
        throw new OpenEDAError('session/argument-invalid', '`target` must be a non-empty path');
    }
}

function assertDesignInput(input: DesignInput): void {
    const candidate = input as unknown as Record<string, unknown>;
    const present = (['path', 'project'] as const).filter((key) => candidate[key] !== undefined);
    if (present.length !== 1) {
        throw new OpenEDAError(
            'session/argument-invalid',
            'DesignInput must contain exactly one of `path` or `project`',
        );
    }
    const value = candidate[present[0]];
    if (typeof value !== 'string' || value.length === 0) {
        throw new OpenEDAError('session/argument-invalid', `\`${present[0]}\` must be a non-empty string`);
    }
}
