/**
 * Authoring types of `@wae/open-eda`.
 *
 * This is the frozen P0 shape of the authoring surface: `new OpenEDA(options?)`
 * creates a session, `create`/`open` return an editable `Design`, and every
 * design and sheet carries a stable ID plus a revision.
 *
 * Geometry never enters this package. Schematics, boards and fabrication data
 * are described by the engine, not by the TypeScript surface.
 */

import type { DesignFormat, DesignIntent } from './formats.js';

/** Opaque design identity. */
export type DesignId = string;
/** Opaque sheet identity; the index is order only. */
export type SheetId = string;
/** Opaque object identity. */
export type ObjectId = string;

/** Hard limits applied before work starts. */
export interface ResourceBudgets {
    readonly maxDocumentBytes?: number;
    readonly maxObjects?: number;
    readonly maxSheets?: number;
}

/** Structured diagnostic emitted during import, inspection, render or export. */
export interface DesignDiagnostic {
    readonly code: string;
    readonly severity: 'info' | 'warning' | 'error';
    readonly revision: number;
    readonly sheetId?: SheetId;
    readonly objectId?: ObjectId;
    readonly recoverable: boolean;
    readonly message: string;
}

/** Where undo history lives between `Design` handles. */
export type DesignHistoryMode = 'memory' | 'sidecar';

/** Session options. Constructing a session opens no window, file or socket. */
export interface OpenEDAOptions {
    /** BCP-47 tag used for diagnostics and default copy. Defaults to `en`. */
    readonly locale?: string;
    /**
     * `memory` keeps the native per-handle history only. `sidecar` additionally
     * records every replacement of the opened document in `<path>.history.json`,
     * so `undo`/`redo` survive separate processes.
     */
    readonly history?: DesignHistoryMode;
    readonly budgets?: ResourceBudgets;
    readonly diagnostics?: (event: DesignDiagnostic) => void;
}

/** Disclosure level. `partial` always ships a loss path and diagnostic codes. */
export type SupportLevel = 'supported' | 'partial' | 'unsupported';

/** Inspection rules understood by `design.inspect()`. */
export type InspectCheck =
    | 'erc'
    | 'electrical'
    | 'drivers'
    | 'bus'
    | 'drc'
    | 'nets'
    | 'connectivity'
    | 'footprints'
    | 'clearance'
    | 'power'
    | 'references';

/** Render targets declared for preview surfaces. */
export type RenderFormat = 'svg' | 'png' | 'pdf';

/** Exchange targets `design.export()` declares. */
export type ExportTarget =
    | 'kicad_sch'
    | 'kicad_sym'
    | 'kicad_pcb'
    | 'net'
    | 'net_csv'
    | 'net_hier'
    | 'net_class'
    | 'spice'
    | 'spice_subckt'
    | 'gbr'
    | 'pdf'
    | 'svg'
    | 'bom'
    | 'bom_json'
    | 'bom_html'
    | 'report'
    | 'json';

/** What the installed engine can do, per format. */
export interface FormatCapability {
    readonly import: SupportLevel;
    readonly edit: SupportLevel;
    readonly export: SupportLevel;
    /** Codes reported when a `partial` path loses something. */
    readonly lossCodes: readonly string[];
}

/** Result of `OpenEDA.capabilities()`. Read this instead of assuming a method works. */
export interface DesignCapabilities {
    readonly engine: { readonly package: string; readonly version: string };
    readonly project: {
        readonly import: SupportLevel;
        readonly edit: SupportLevel;
        readonly export: SupportLevel;
    };
    readonly formats: Readonly<Record<DesignFormat, FormatCapability>>;
    readonly checks: Readonly<Record<InspectCheck | 'hierarchy', SupportLevel>>;
    readonly renderers: Readonly<Record<RenderFormat, SupportLevel>>;
    readonly exportTargets: Readonly<Record<ExportTarget, SupportLevel>>;
    /** Editing surfaces that are implemented rather than read-only. */
    readonly edits: {
        readonly junctions: SupportLevel;
        readonly noConnects: SupportLevel;
        readonly pins: SupportLevel;
        readonly annotate: SupportLevel;
    };
    /** Read-only queries that are not rule checks, such as the pin trace. */
    readonly queries: { readonly trace: SupportLevel };
}

/** Options of `OpenEDA.create()`. */
export interface DesignCreateOptions {
    readonly title?: string;
    /** Drawing revision label, for example `A`. */
    readonly rev?: string;
    /** Company or organisation of the title block. */
    readonly company?: string;
    /** Drawing date text. */
    readonly date?: string;
    readonly template?: string;
}

/** Drawing properties `design.setProperties()` may change. */
export interface DesignPropertiesInput {
    /** New title; an empty string clears it. */
    readonly title?: string;
    /** New drawing revision label; an empty string clears it. */
    readonly rev?: string;
    /** New company text; an empty string clears it. */
    readonly company?: string;
    /** New date text; an empty string clears it. */
    readonly date?: string;
}

/** Result of `design.setProperties()`. */
export interface DesignPropertiesResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly title: string | null;
    readonly rev: string;
    readonly company: string;
    readonly date: string;
}

/**
 * Input of `OpenEDA.open()`.
 *
 * Exactly one of `path` or `project` must be present, so the engine never
 * guesses whether a string is a file name or serialized content.
 */
export type DesignInput =
    | { readonly path: string; readonly intent?: DesignIntent }
    | { readonly project: string; readonly intent?: DesignIntent };

/** One sheet of a design summary. */
export interface DesignSummarySheet {
    readonly id: SheetId;
    readonly index: number;
    /** Sheet name, as shown in the studio. */
    readonly name: string;
    /** Alias of {@link name}, kept for callers that already read `title`. */
    readonly title: string;
    readonly objects: number;
    readonly components: number;
    readonly nets: number;
    readonly wires: number;
}

/** Object counts across the whole design. */
export interface DesignCounts {
    readonly sheets: number;
    readonly components: number;
    readonly nets: number;
    readonly wires: number;
}

/** Bounded per-handle undo/redo state. */
export interface DesignHistory {
    readonly undo: number;
    readonly redo: number;
    /** Snapshots kept per open handle. */
    readonly limit: number;
}

/** Summary of one design at its committed revision. */
export interface DesignSummary {
    readonly id: DesignId;
    readonly revision: number;
    readonly title: string | null;
    /** Drawing revision label from the title block, for example `A`. */
    readonly rev: string;
    /** Company or organisation from the title block. */
    readonly company: string;
    /** Drawing date text from the title block. */
    readonly date: string;
    readonly units: string;
    readonly sheets: readonly DesignSummarySheet[];
    readonly counts: DesignCounts;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly history: DesignHistory;
}

/** One schematic component placed on a sheet. */
/** Electrical role of one pin; names match the KiCad pin electrical types. */
export type PinType =
    | 'input'
    | 'output'
    | 'bidirectional'
    | 'tri_state'
    | 'passive'
    | 'power_in'
    | 'power_out'
    | 'open_collector'
    | 'open_emitter'
    | 'unspecified';

export interface Component {
    readonly id: ObjectId;
    /** Reference designator, unique inside the design. */
    readonly ref: string;
    readonly value: string;
    /** Assigned footprint, for example `Resistor_SMD:R_0805_2012Metric`; empty when unassigned. */
    readonly footprint: string;
    readonly x: number;
    readonly y: number;
    readonly rotation: number;
    readonly pins: readonly string[];
    /** Electrical role of the typed pins, keyed by pin name; absent when no pin is typed. */
    readonly pinTypes?: Readonly<Record<string, PinType>>;
    /** Custom metadata such as a manufacturer part number, keyed by property name; absent when none is set. */
    readonly properties?: Readonly<Record<string, string>>;
}

/** One end of a net. */
export interface NetNode {
    readonly ref: string;
    readonly pin: string;
}

/** One electrical net. */
export interface Net {
    readonly id: ObjectId;
    readonly name: string;
    readonly nodes: readonly NetNode[];
}

/** One schematic wire segment. */
export interface Wire {
    readonly id: ObjectId;
    readonly net: string | null;
    readonly from: readonly [number, number];
    readonly to: readonly [number, number];
}

/**
 * One declared bus: a named group of member nets.
 *
 * Buses belong to the design. The netlist expands `DATA` with width 4 into
 * `DATA0`..`DATA3`, the bus name followed by the 0-based index.
 */
export interface Bus {
    readonly id: ObjectId;
    readonly name: string;
    /** Number of member nets; at least 1. */
    readonly width: number;
    /** Alternate names that resolve to this bus; members still use `name`. */
    readonly aliases?: readonly string[];
}

/** One symbol the design uses, grouped by the library id derived from its pins. */
export interface SymbolUsage {
    readonly libId: string;
    /** Number of pins the symbol declares. */
    readonly pins: number;
    /** Number of placed components that use the symbol. */
    readonly components: number;
    /** Sorted reference designators that place the symbol. */
    readonly references: readonly string[];
    /** Distinct assigned footprints, sorted; empty when none is assigned. */
    readonly footprints: readonly string[];
    /** Components that still have no footprint assigned. */
    readonly missingFootprint: number;
}

/** Result of `design.listSymbols()`. */
export interface SymbolLibraryResult {
    readonly revision: number;
    readonly symbols: readonly SymbolUsage[];
    readonly totals: {
        readonly symbols: number;
        readonly components: number;
        readonly missingFootprint: number;
    };
}

/** One net listed by `design.connectivity()`. */
export interface ConnectivityNet {
    readonly id: ObjectId;
    readonly name: string;
    /** Number of connected pins. */
    readonly nodes: number;
    /** Connected pins in `REF.PIN` form, in declaration order. */
    readonly pins: readonly string[];
    /** True when the net reaches exactly one pin. */
    readonly singleNode: boolean;
    /** True when the net declares no pins. */
    readonly dangling: boolean;
}

/** Wire-group, junction and label totals of one sheet. */
export interface ConnectivitySheetCounts {
    readonly wires: {
        readonly segments: number;
        readonly groups: number;
        readonly unassigned: number;
        readonly conflicts: number;
    };
    readonly junctions: { readonly total: number; readonly floating: number };
    readonly labels: {
        readonly total: number;
        readonly local: number;
        readonly global: number;
        readonly hierarchical: number;
    };
}

/** One pin no net reaches. */
export interface UnconnectedPin {
    readonly ref: string;
    readonly pin: string;
}

/** One sheet of `design.connectivity()`. */
export interface ConnectivitySheet extends ConnectivitySheetCounts {
    readonly id: SheetId;
    readonly name: string;
    readonly nets: readonly ConnectivityNet[];
    readonly noConnects: number;
    readonly unconnectedPins: readonly UnconnectedPin[];
}

/** Result of `design.connectivity()`, tagged `open-eda-connectivity`. */
export interface ConnectivityReport {
    readonly format: 'open-eda-connectivity';
    readonly version: number;
    readonly revision: number;
    readonly sheets: readonly ConnectivitySheet[];
    readonly totals: {
        readonly nets: number;
        readonly nodes: number;
        readonly singleNodeNets: number;
        readonly danglingNets: number;
        readonly unconnectedPins: number;
        readonly wireGroups: number;
        readonly unassignedWireGroups: number;
        readonly conflictingWireGroups: number;
        readonly floatingJunctions: number;
        /** Net labels across the design. */
        readonly labels: number;
    };
}

/** Object kind of one pin trace hop. */
export type TraceHopType = 'pin' | 'wire' | 'junction' | 'label' | 'sheet-port';

/** How a connected pin pair is joined. */
export type TraceVia = 'wire' | 'label' | 'direct' | 'none';

/**
 * One hop of a pin trace.
 *
 * `index` is the 0-based position inside its sheet list for wires, junctions,
 * labels and sheet ports. `name` is the pin `REF.PIN`, the label text or the
 * port name. Both are present on labels and sheet ports.
 */
export interface TraceHop {
    readonly type: TraceHopType;
    readonly sheet: string;
    readonly index?: number;
    readonly name?: string;
    readonly detail: string;
}

/** Result of `design.tracePins()`, tagged `open-eda-trace`. */
export interface TraceReport {
    readonly format: 'open-eda-trace';
    readonly version: number;
    readonly from: string;
    readonly to: string;
    readonly connected: boolean;
    readonly via: TraceVia;
    readonly hops: readonly TraceHop[];
    /** Sheets the reported path touches, in hop order. */
    readonly sheets: readonly string[];
}

/** One pin of a component detail report. */
export interface ComponentPinDetail {
    /** Pin name as the component declares it. */
    readonly name: string;
    /** Names of the nets that reach the pin, in document order without duplicates. */
    readonly nets: readonly string[];
    /** True when a no-connect flag marks this pin. */
    readonly noConnect: boolean;
}

/** Placement and metadata of one component detail report. */
export interface ComponentDetail {
    readonly id: ObjectId;
    readonly reference: string;
    readonly value: string;
    /** Assigned footprint; empty when none is assigned. */
    readonly footprint: string;
    readonly x: number;
    readonly y: number;
    readonly rotation: number;
    /** Custom properties sorted by key. */
    readonly properties: readonly ComponentProperty[];
}

/** Result of `design.componentReport()`, tagged `open-eda-component`. */
export interface ComponentReport {
    readonly format: 'open-eda-component';
    readonly version: number;
    readonly sheet: { readonly id: SheetId; readonly name: string };
    readonly component: ComponentDetail;
    /** One entry per declared pin, in declaration order. */
    readonly pins: readonly ComponentPinDetail[];
}

/** One wire segment of a net detail report, with its 0-based sheet document index. */
export interface NetWireDetail {
    readonly index: number;
    readonly from: readonly [number, number];
    readonly to: readonly [number, number];
}

/** One net label of a net detail report. */
export interface NetLabelDetail {
    readonly text: string;
    readonly scope: LabelScope;
    readonly x: number;
    readonly y: number;
}

/** One sheet of a net detail report. */
export interface NetSheetDetail {
    readonly sheet: { readonly id: SheetId; readonly name: string };
    /** The declared net on this sheet with its nodes. */
    readonly net: Net;
    /** Wire segments on this sheet whose declared net name equals the requested name. */
    readonly wires: readonly NetWireDetail[];
    /** Net labels on this sheet whose text equals the requested name. */
    readonly labels: readonly NetLabelDetail[];
}

/** A declared bus whose name or alias resolves to the requested net name. */
export interface NetBusDetail {
    readonly id: ObjectId;
    readonly name: string;
    readonly width: number;
    /** Canonical member net names, `name` followed by the 0-based index. */
    readonly members: readonly string[];
}

/** A declared net class whose `nets` list contains the requested name. */
export interface NetClassDetail {
    readonly id: ObjectId;
    readonly name: string;
    readonly width: number;
    readonly clearance: number;
    readonly via: number;
}

/** Result of `design.netShow()`, tagged `open-eda-net`. */
export interface NetReport {
    readonly format: 'open-eda-net';
    readonly version: number;
    readonly revision: number;
    readonly design: DesignSummary;
    /** Requested net name after trimming. */
    readonly name: string;
    /** One entry per matching declared net, in sheet document order. */
    readonly sheets: readonly NetSheetDetail[];
    /** Nodes across every matching sheet. */
    readonly nodeCount: number;
    readonly bus: NetBusDetail | null;
    readonly netClass: NetClassDetail | null;
}

/** One schematic junction (connection dot). */
export interface Junction {
    readonly x: number;
    readonly y: number;
}

/** One no-connect flag at a sheet position, optionally marking one pin. */
export interface NoConnect {
    readonly x: number;
    readonly y: number;
    /** Pin this flag deliberately leaves open; absent on coordinate-only flags. */
    readonly node?: NetNode;
}

/** One junction with its 0-based position on the sheet. */
export interface JunctionEntry {
    readonly index: number;
    readonly x: number;
    readonly y: number;
}

/** Result of `design.listJunctions()`. */
export interface JunctionListResult {
    readonly revision: number;
    readonly sheetId: SheetId;
    readonly junctions: readonly JunctionEntry[];
}

/** One no-connect flag with its 0-based position on the sheet. */
export interface NoConnectEntry extends NoConnect {
    readonly index: number;
}

/** Result of `design.listNoConnects()`. */
export interface NoConnectListResult {
    readonly revision: number;
    readonly sheetId: SheetId;
    readonly noConnects: readonly NoConnectEntry[];
}

/** Deterministic object counts of one sheet. */
export interface SheetStats {
    readonly id: SheetId;
    readonly name: string;
    readonly components: number;
    readonly nets: number;
    readonly wires: number;
    readonly junctions: number;
    readonly labels: number;
    readonly noConnects: number;
    readonly texts: number;
}

/** Result of `design.stats()`, tagged `open-eda-stats`. */
export interface DesignStats {
    readonly format: 'open-eda-stats';
    readonly version: number;
    readonly revision: number;
    readonly sheets: readonly SheetStats[];
    readonly totals: {
        readonly sheets: number;
        readonly components: number;
        readonly nets: number;
        readonly wires: number;
        readonly junctions: number;
        readonly labels: number;
        readonly noConnects: number;
        readonly texts: number;
        /** Design-level bus declarations. */
        readonly buses: number;
        /** BOM rows using the default population. */
        readonly bomRows: number;
    };
}

/** Result of `design.listSheets()`. */
export interface SheetListResult {
    readonly revision: number;
    readonly sheets: readonly SheetStats[];
}

/** One free text annotation drawn on a sheet. */
export interface TextLabel {
    readonly text: string;
    readonly x: number;
    readonly y: number;
}

/** Cross-sheet scope of a net label. */
export type LabelScope = 'local' | 'global' | 'hierarchical';

/** Horizontal justification of a net label. */
export type LabelJustify = 'left' | 'right' | 'center';

/**
 * One net label placed on a sheet.
 *
 * A label is an annotation, not a net: it announces a net name at a point
 * without creating, renaming or merging the declared nets on the sheet.
 */
export interface NetLabel {
    readonly id: ObjectId;
    readonly text: string;
    readonly x: number;
    readonly y: number;
    readonly scope: LabelScope;
    readonly rotation: number;
    readonly justify: LabelJustify;
}

/** Input of `design.addLabel()`. */
export interface LabelInput {
    readonly sheet?: SheetId;
    readonly text: string;
    readonly x: number;
    readonly y: number;
    /** Cross-sheet scope; `local` when omitted. */
    readonly scope?: LabelScope;
    readonly rotation?: number;
    readonly justify?: LabelJustify;
}

/**
 * Input of `design.updateLabel()`.
 *
 * At least one of `text`, `x`, `y`, `scope`, `rotation` or `justify` must be
 * present; omitted fields keep their current value.
 */
export interface LabelUpdateInput {
    readonly sheet?: SheetId;
    readonly id: ObjectId;
    readonly text?: string;
    readonly x?: number;
    readonly y?: number;
    readonly scope?: LabelScope;
    readonly rotation?: number;
    readonly justify?: LabelJustify;
}

/** Result of `design.listLabels()`. */
export interface LabelListResult {
    readonly revision: number;
    readonly sheetId: SheetId;
    /** Net labels in document order. */
    readonly labels: readonly NetLabel[];
}

/** Result of `design.updateLabel()`. */
export interface LabelUpdateResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly label: NetLabel;
}

/** Result of `design.removeLabel()`. */
export interface LabelDeleteResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly label: NetLabel;
    readonly remaining: number;
}

/** Input of `design.addBus()`. */
export interface BusInput {
    /** Bus name, for example `DATA`. */
    readonly name: string;
    /** Number of member nets; at least 1. */
    readonly width: number;
    /**
     * Alternate names that resolve to this bus, for example `['D']`. A net named
     * with an alias expands into the bus's canonical members (`DATA0`...), and an
     * alias may not collide with a bus name, member name or another alias.
     */
    readonly aliases?: readonly string[];
}

/** Input of `design.updateBus()`; only the given fields change. */
export interface UpdateBusInput {
    /** Bus id, name or alias that selects the declaration to edit. */
    readonly id: string;
    /** New bus name; kept when omitted. */
    readonly name?: string;
    /** New number of member nets; at least 1 and kept when omitted. */
    readonly width?: number;
    /**
     * Replacement alias list; the current aliases are kept when omitted and an
     * empty list clears them. Aliases keep the `addBus` collision rules.
     */
    readonly aliases?: readonly string[];
}

/** Result of `design.updateBus()`. */
export interface UpdateBusResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly bus: Bus;
    /** `false` when the supplied fields already matched the declaration. */
    readonly changed: boolean;
}

/** Input of `design.addComponent()`. */
export interface ComponentInput {
    /** Target sheet id or name; the first sheet is used when omitted. */
    readonly sheet?: SheetId;
    readonly reference: string;
    readonly value: string;
    readonly x: number;
    readonly y: number;
    readonly rotation?: number;
    readonly pins: readonly string[];
}

/**
 * Input of `design.copyComponent()`.
 *
 * The copy stays on the source sheet and keeps the source value, footprint,
 * rotation, pins, pin roles, custom properties and component class. It moves by
 * `dx`/`dy` (zero when omitted). Net nodes, wires and no-connect flags are not
 * copied, so the new component starts electrically unconnected.
 */
export interface CopyComponentInput {
    readonly sheet?: SheetId;
    /** Reference designator of the placed component to copy. */
    readonly reference: string;
    /** Reference designator of the copy; unique across every sheet. */
    readonly newReference: string;
    /** Offset added to the source X coordinate; defaults to zero. */
    readonly dx?: number;
    /** Offset added to the source Y coordinate; defaults to zero. */
    readonly dy?: number;
}

/** One reference designator moved by `design.annotate()`. */
export interface AnnotateChange {
    readonly sheetId: SheetId;
    readonly componentId: ObjectId;
    readonly from: string;
    readonly to: string;
}

/** One component `design.annotate()` left untouched because its reference has no prefix. */
export interface AnnotateSkipped {
    readonly sheetId: SheetId;
    readonly componentId: ObjectId;
    readonly reference: string;
}

/** Result of `design.annotate()`. */
export interface AnnotateResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    /** Designators that changed; zero when the design was already annotated. */
    readonly annotated: number;
    /** Same value as `annotated`, reported under the `count` field. */
    readonly count: number;
    /** Same value as `annotated`, reported under the `changed` field. */
    readonly changed: number;
    readonly changes: readonly AnnotateChange[];
    /** Components skipped because their reference does not start with a letter. */
    readonly skipped: readonly AnnotateSkipped[];
}

/** One BOM variant: designators that are not populated plus value overrides. */
export interface Variant {
    readonly name: string;
    /** Reference designators that are not populated. */
    readonly dnp: readonly string[];
    /** Alternate value text keyed by reference designator. */
    readonly values: Readonly<Record<string, string>>;
}

/**
 * One net class: routing attributes shared by the nets and buses assigned to it.
 *
 * Width, clearance and via diameter use the design unit and are never converted.
 * The model declares and assigns these attributes; it does not route with them.
 */
export interface NetClass {
    readonly id: ObjectId;
    readonly name: string;
    /** Trace width; greater than zero. */
    readonly width: number;
    /** Clearance to neighbouring copper; zero means no declared requirement. */
    readonly clearance: number;
    /** Via diameter; zero means no via is declared. */
    readonly via: number;
    /** Assigned net names. */
    readonly nets?: readonly string[];
    /** Assigned buses, stored by canonical bus name. */
    readonly buses?: readonly string[];
}

/** Input of `design.addNetClass()`; replaces a class with the same name. */
export interface NetClassInput {
    readonly name: string;
    readonly width: number;
    readonly clearance?: number;
    readonly via?: number;
    /** Net names assigned to the class; every name must already be declared. */
    readonly nets?: readonly string[];
    /** Bus names or aliases assigned to the class. */
    readonly buses?: readonly string[];
}

/** Result of `design.addNetClass()`. */
export interface NetClassResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly netClass: NetClass;
}

/** Result of `design.removeNetClass()`. */
export interface DeleteNetClassResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly deleted: string;
    readonly remaining: number;
}

/** Result of `design.listNetClasses()`. */
export interface NetClassListResult {
    readonly revision: number;
    readonly netClasses: readonly NetClass[];
}

/** One reference designator prefix rule for a component class. */
export interface ReferencePrefixRule {
    readonly class: string;
    /** Expected uppercase letter prefix, for example `R`. */
    readonly prefix: string;
}

/** One component class assignment with its resolved reference. */
export interface ComponentClassAssignment {
    readonly componentId: ObjectId;
    readonly reference: string;
    readonly sheetId: SheetId;
    readonly class: string;
}

/** Input of `design.setReferencePrefix()`. */
export interface ReferencePrefixInput {
    readonly class: string;
    readonly prefix: string;
}

/** Result of `design.setReferencePrefix()`. */
export interface ReferencePrefixResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly referencePrefix: ReferencePrefixRule;
}

/** Result of `design.removeReferencePrefix()`. */
export interface DeleteReferencePrefixResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly deleted: string;
    readonly remaining: number;
}

/** Result of `design.listReferencePrefixes()`. */
export interface ReferencePrefixListResult {
    readonly revision: number;
    readonly referencePrefixes: readonly ReferencePrefixRule[];
}

/** Input of `design.setComponentClass()`; an empty class clears the assignment. */
export interface ComponentClassInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    readonly class: string;
}

/** Result of `design.setComponentClass()`. */
export interface ComponentClassResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly reference: string;
    readonly componentId: ObjectId;
    readonly class: string | null;
}

/** Result of `design.listComponentClasses()`. */
export interface ComponentClassListResult {
    readonly revision: number;
    readonly componentClasses: readonly ComponentClassAssignment[];
}

/** Input of `design.addVariant()`; replaces a variant with the same name. */
export interface VariantInput {
    readonly name: string;
    readonly dnp?: readonly string[];
    readonly values?: Readonly<Record<string, string>>;
}

/** Result of `design.addVariant()`. */
export interface VariantResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly variant: Variant;
}

/** Result of `design.listVariants()`. */
export interface VariantListResult {
    readonly revision: number;
    readonly variants: readonly Variant[];
}

/** Input of `design.setPinType()`. */
export interface SetPinTypeInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    readonly pin: string;
    /** Role to assign; omit or pass null to clear the role. */
    readonly pinType?: PinType | null;
}

/** Result of `design.setPinType()`. */
export interface SetPinTypeResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly reference: string;
    readonly pin: string;
    readonly pinType: PinType | null;
}

/** One custom key/value property of a placed component. */
export interface ComponentProperty {
    readonly key: string;
    readonly value: string;
}

/** Input of `design.setComponentProperty()`. */
export interface SetComponentPropertyInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    /** Property key; trimmed and must not contain whitespace. */
    readonly key: string;
    /** Property value; an empty string is stored as an empty value. */
    readonly value: string;
}

/** Input of `design.removeComponentProperty()`. */
export interface RemoveComponentPropertyInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    readonly key: string;
}

/** Result of `design.setComponentProperty()` and `design.removeComponentProperty()`. */
export interface ComponentPropertyResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly component: Component;
    readonly key: string;
    /** Value that was assigned; absent when a property was removed. */
    readonly value?: string;
}

/** Result of `design.listComponentProperties()`. */
export interface ComponentPropertyListResult {
    readonly revision: number;
    readonly sheetId: SheetId;
    readonly componentId: ObjectId;
    readonly reference: string;
    /** Properties sorted by key. */
    readonly properties: readonly ComponentProperty[];
}

/** Input of `design.connectNet()`. */
export interface ConnectNetInput {
    readonly sheet?: SheetId;
    readonly net: string;
    /** Nodes in `REF.PIN` form. */
    readonly nodes: readonly string[];
}

/** Input of `design.addWire()`. */
export interface WireInput {
    readonly sheet?: SheetId;
    readonly net?: string;
    readonly from: readonly [number, number];
    readonly to: readonly [number, number];
}

/** Result of `design.splitWire()`: the original segment and the new one. */
export interface SplitWireResult {
    /** Original segment, now ending at the split point and keeping its identity. */
    readonly first: Wire;
    /** New segment running from the split point to the old endpoint. */
    readonly second: Wire;
}

/** Input of `design.splitWire()`. */
export interface SplitWireInput {
    readonly sheet?: SheetId;
    /** Stable wire identity, for example `wire-1`. */
    readonly id: ObjectId;
    /** Split point X coordinate, on the segment. */
    readonly x: number;
    /** Split point Y coordinate, on the segment. */
    readonly y: number;
}

/** Result of `design.mergeWires()`: the surviving segment and the removed one. */
export interface MergeWireResult {
    /** First segment, now spanning both far endpoints and keeping its identity. */
    readonly wire: Wire;
    /** Wire id removed by the merge. */
    readonly removed: ObjectId;
}

/** Input of `design.mergeWires()`; both indices are 0-based on one sheet. */
export interface MergeWireInput {
    /** Sheet that holds both wires. */
    readonly sheet: SheetId;
    /** 0-based position of the wire that keeps its identity. */
    readonly a: number;
    /** 0-based position of the wire that is removed. */
    readonly b: number;
}

/** Result of `design.translate()`. */
export interface TranslateResult {
    /** Components moved in this call. */
    readonly components: number;
    /** Wire segments moved in this call. */
    readonly wires: number;
    readonly dx: number;
    readonly dy: number;
}

/** Input of `design.translate()`; only the listed objects move. */
export interface TranslateInput {
    readonly sheet?: SheetId;
    /** Components to move, by reference designator; an empty list selects none. */
    readonly references?: readonly string[];
    /** Wire segments to move, by stable id; an empty list selects none. */
    readonly wires?: readonly ObjectId[];
    /** Offset added to every selected X coordinate. */
    readonly dx: number;
    /** Offset added to every selected Y coordinate. */
    readonly dy: number;
}

/** Input of `design.updateWire()`. An empty `net` clears the assignment. */
export interface UpdateWireInput {
    readonly sheet?: SheetId;
    /** Stable wire identity, for example `wire-1`. */
    readonly id: ObjectId;
    readonly net?: string;
    readonly from?: readonly [number, number];
    readonly to?: readonly [number, number];
}

/** Input of `design.addJunction()`. */
export interface JunctionInput {
    readonly sheet?: SheetId;
    readonly x: number;
    readonly y: number;
}

/** Input of `design.addText()`. */
export interface TextInput {
    readonly sheet?: SheetId;
    readonly text: string;
    readonly x: number;
    readonly y: number;
}

/** One free text annotation with its 0-based position on the sheet. */
export interface TextEntry {
    readonly index: number;
    readonly text: string;
    readonly x: number;
    readonly y: number;
}

/** Result of `design.listTexts()`. */
export interface TextListResult {
    readonly revision: number;
    readonly sheetId: SheetId;
    /** Text annotations in 0-based index order. */
    readonly texts: readonly TextEntry[];
}

/**
 * Input of `design.updateText()`.
 *
 * At least one of `text`, `x` or `y` must be present; omitted fields keep their
 * current value. The text is trimmed for the empty check and stored verbatim.
 */
export interface TextUpdateInput {
    readonly sheet?: SheetId;
    /** 0-based position inside the sheet's text list. */
    readonly index: number;
    readonly text?: string;
    readonly x?: number;
    readonly y?: number;
}

/** Result of `design.updateText()`. */
export interface TextUpdateResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly index: number;
    readonly text: TextLabel;
}

/** Input of `design.addNoConnect()`. */
export interface NoConnectInput {
    readonly sheet?: SheetId;
    readonly x: number;
    readonly y: number;
    /** Pin to mark as deliberately open; the pin must already exist. */
    readonly node?: NetNode;
}

/** Input of `design.addPin()`. */
export interface AddPinInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    /** Pin name to add; must not be empty or break the `REF.PIN` grammar. */
    readonly pin: string;
}

/** Input of `design.removePin()`. */
export interface RemovePinInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    /** Pin name to remove; a pin any net or no-connect flag uses is refused. */
    readonly pin: string;
}

/** Input of `design.renamePin()`; every `REF.PIN` reference follows the new name. */
export interface RenamePinInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    /** Current pin name. */
    readonly from: string;
    /** New pin name; must not be empty or break the `REF.PIN` grammar. */
    readonly to: string;
}

/** Input of `design.swapPins()`; the two pins exchange their net connections. */
export interface SwapPinsInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    /** First pin name. */
    readonly pinA: string;
    /** Second pin name. */
    readonly pinB: string;
}

/** Result of `design.swapPins()`. */
export interface SwapPinsResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    /** The component whose pins were swapped; its declaration list is unchanged. */
    readonly component: Component;
    readonly pinA: string;
    readonly pinB: string;
    /** `false` when neither pin carried a net node or no-connect flag; no revision was added. */
    readonly changed: boolean;
}

/** Input of `design.moveComponent()`. */
export interface MoveComponentInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    readonly x: number;
    readonly y: number;
    /** Rotation in degrees; the current rotation is kept when omitted. */
    readonly rotation?: number;
}

/** Input of `design.updateComponent()`; only the given fields change. */
export interface UpdateComponentInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    /** New value text; kept when omitted. */
    readonly value?: string;
    /** New footprint text; kept when omitted, and an empty string clears it. */
    readonly footprint?: string;
    /** New placement X coordinate; kept when omitted. */
    readonly x?: number;
    /** New placement Y coordinate; kept when omitted. */
    readonly y?: number;
    /** New rotation in degrees; kept when omitted. */
    readonly rotation?: number;
}

/** Input of `design.setFootprint()`; an empty footprint clears the assignment. */
export interface SetFootprintInput {
    readonly sheet?: SheetId;
    readonly reference: string;
    readonly footprint: string;
}

/**
 * Input of `design.renameNet()`. The rename follows the old name across every
 * sheet: declarations keep their nodes, and wires, net labels, net-class `nets`
 * and bus `aliases` that spelled the old name are rewritten. The `sheet`, when
 * given, only identifies the net.
 */
export interface RenameNetInput {
    readonly sheet?: SheetId;
    /** Current net id or name. */
    readonly net: string;
    /** New net name. */
    readonly name: string;
}

/** Result of `design.renameNet()`. */
export interface RenameNetResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly net: Net;
    /** The name the net carried before this call; equal to `net.name` when unchanged. */
    readonly previousName: string;
    /** `false` when the net already carried the requested name; no revision was added. */
    readonly changed: boolean;
}

/** Input of `design.disconnectNet()`. The net and its wires are kept. */
export interface DisconnectNetInput {
    readonly sheet?: SheetId;
    /** Net id or name. */
    readonly net: string;
    /** Nodes in `REF.PIN` form; every node is removed when empty. */
    readonly nodes: readonly string[];
}

/** Input of `design.deleteNet()`. */
export interface DeleteNetInput {
    readonly sheet?: SheetId;
    /** Net id or name. */
    readonly net: string;
}

/** Result of `design.mergeNets()`. */
export interface MergeNetsResult {
    /** Net that survived the merge, with the absorbed nodes appended. */
    readonly net: Net;
    /** Name of the absorbed net. */
    readonly source: string;
    /** Nodes appended to the target; nodes it already reached are not counted. */
    readonly movedNodes: number;
    /** Wires repointed from the absorbed name to the target. */
    readonly repointedWires: number;
}

/** Input of `design.mergeNets()`; both nets must be on the same sheet. */
export interface MergeNetsInput {
    readonly sheet?: SheetId;
    /** Net that survives the merge, by id or name. */
    readonly target: string;
    /** Net absorbed into the target, by id or name. */
    readonly source: string;
}

/** Counts returned by `design.deriveNets()`. */
export interface DeriveNetsResult {
    /** Wire groups that received a net in this call. */
    readonly assigned: number;
    /** Net objects created because the label was new. */
    readonly created: number;
    /** Connected wire groups found on the sheet. */
    readonly groups: number;
}

/** Objects of one open design, read through `design.listObjects()`. */
export interface DesignObjects {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly components: readonly Component[];
    readonly nets: readonly Net[];
    readonly wires: readonly Wire[];
}

/** One finding of an inspection run. */
export interface InspectIssue {
    readonly code: string;
    readonly severity: 'info' | 'warning' | 'error';
    readonly revision: number;
    readonly sheetId?: SheetId;
    readonly objectId?: ObjectId;
    readonly recoverable: boolean;
    readonly message: string;
    /** True only on findings returned in `InspectReport.waived`. */
    readonly waived?: boolean;
}

/** Result of `design.inspect()`. Rules only, never an aesthetic score. */
export interface InspectReport {
    readonly revision: number;
    readonly checkerVersion: string;
    readonly passed: boolean;
    /** Active findings; accepted findings are in `waived`. */
    readonly issues: readonly InspectIssue[];
    /** Findings a waiver accepted for this run. */
    readonly waived: readonly InspectIssue[];
    readonly counts: { readonly active: number; readonly waived: number };
}

/** Level one finding code may be promoted, demoted or silenced to. */
export type InspectSeverity = 'error' | 'warning' | 'info' | 'off';

/**
 * One accepted finding.
 *
 * With no `sheetId` or `objectId` the waiver accepts the code everywhere on the
 * design; each field narrows it to one sheet or object.
 */
export interface InspectWaiver {
    readonly code: string;
    readonly sheetId?: SheetId;
    readonly objectId?: ObjectId;
}

/** Options of `design.inspect()`. */
export interface InspectOptions {
    readonly checks?: readonly InspectCheck[];
    /** Per-code severity overrides; `off` drops the finding. */
    readonly severity?: Readonly<Record<string, InspectSeverity>>;
    /** Findings accepted for this run; they move from `issues` to `waived`. */
    readonly waivers?: readonly InspectWaiver[];
}

/** Options of `design.save()` (native document, not an exchange format). */
export interface SaveOptions {
    readonly target?: string;
    readonly overwrite?: boolean;
}

/** Result of `design.save()`. */
export interface SaveReport {
    readonly path: string;
    readonly revision: number;
    readonly bytes: number;
}

/** Options of `design.export()`. */
export interface ExportOptions {
    readonly format: ExportTarget;
    readonly target: string;
    readonly overwrite?: boolean;
    /** BOM variant to apply; only `bom`, `bom_json` and `bom_html` accept one. */
    readonly variant?: string;
}

/** One construct an export could not represent verbatim. */
export interface ExportLoss {
    /** Stable loss code, for example `spice/open-pin`. */
    readonly code: string;
    /** Human-readable detail naming what was approximated or left open. */
    readonly detail: string;
}

/** Result of `design.export()`. Reports what the target actually kept. */
export interface ExportReport {
    readonly revision: number;
    readonly format: ExportTarget;
    readonly path: string;
    readonly bytes: number;
    /** False for projections such as PDF that cannot be reopened for editing. */
    readonly editable: boolean;
    /** Small summary of what the writer emitted; shape depends on the format. */
    readonly artifact: Readonly<Record<string, string | number>>;
    /** Losses reported by writers that can approximate, such as SPICE. */
    readonly losses?: readonly ExportLoss[];
}

/** Result of `design.undo()` and `design.redo()`. */
export interface HistoryResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
}

/**
 * A design document as stored in a `.oeda` file, returned by the KiCad importer.
 *
 * Field names mirror the document schema, including its snake_case counters, so
 * the object can be written back without translation.
 */
export interface ImportedDesign {
    readonly id: DesignId;
    readonly title: string | null;
    readonly revision: number;
    readonly units: string;
    readonly sheets: readonly Sheet[];
    /** Hierarchical sheet symbols; absent when the document declares none. */
    readonly hierarchy?: readonly HierarchicalSheet[];
    readonly next_component: number;
    readonly next_net: number;
    readonly next_wire: number;
}

/** One construct a KiCad schematic import could not represent. */
export interface KicadImportLoss {
    /** Stable loss code, for example `kicad/unsupported-element`. */
    readonly code: string;
    /** Human-readable detail naming the construct that was skipped. */
    readonly detail: string;
}

/** Input of `OpenEDA.importKicadSch()`. */
export interface KicadImportOptions {
    /** KiCad schematic text or a path to a `.kicad_sch` file. */
    readonly source: string;
    /** Native `.oeda` document to write when the import should be materialized. */
    readonly target?: string;
}

/** Result of `importKicadSch()`. */
export interface KicadImportResult {
    /** Imported document, validated against the native schema. */
    readonly design: ImportedDesign;
    /** Constructs outside the imported subset; empty when nothing was lost. */
    readonly losses: readonly KicadImportLoss[];
}

/** Object counts of one imported KiCad netlist. */
export interface KicadNetTotals {
    readonly components: number;
    readonly nets: number;
    /** Pins recovered from net nodes; unconnected pins are not part of a netlist. */
    readonly pins: number;
}

/** Result of `OpenEDA.importKicadNet()`. */
export interface KicadNetImportResult {
    readonly kind: 'net';
    /** Imported document, validated against the native schema. */
    readonly design: ImportedDesign;
    readonly totals: KicadNetTotals;
    /** Netlist constructs outside the imported subset. */
    readonly losses: readonly KicadImportLoss[];
}

/** One component present on only one side of a netlist comparison. */
export interface NetlistComponentEntry {
    readonly reference: string;
    readonly value: string;
    readonly footprint: string;
}

/** One component whose value or footprint differs between design and netlist. */
export interface NetlistComponentChange {
    readonly reference: string;
    readonly changes: {
        readonly value: { readonly from: string; readonly to: string };
        readonly footprint: { readonly from: string; readonly to: string };
    };
}

/** One net present on only one side of a netlist comparison. */
export interface NetlistNetEntry {
    readonly name: string;
    /** Sorted `REF.PIN` nodes. */
    readonly nodes: readonly string[];
}

/** One net whose `REF.PIN` node set differs between design and netlist. */
export interface NetlistNetChange {
    readonly name: string;
    /** Nodes present only in the netlist. */
    readonly addedNodes: readonly string[];
    /** Nodes present only in the design. */
    readonly removedNodes: readonly string[];
}

/** Result of `design.compareNetlist()`, tagged `open-eda-netlist-diff`. */
export interface NetlistDiffReport {
    readonly format: 'open-eda-netlist-diff';
    readonly version: number;
    readonly equal: boolean;
    readonly components: {
        readonly added: readonly NetlistComponentEntry[];
        readonly removed: readonly NetlistComponentEntry[];
        readonly modified: readonly NetlistComponentChange[];
    };
    readonly nets: {
        readonly added: readonly NetlistNetEntry[];
        readonly removed: readonly NetlistNetEntry[];
        readonly modified: readonly NetlistNetChange[];
    };
    /** Netlist constructs the importer could not represent. */
    readonly losses: readonly KicadImportLoss[];
}

/** One pin of a symbol imported from a KiCad `.kicad_sym` library. */
export interface KicadSymbolPin {
    /** Pin number, for example `1`. */
    readonly number: string;
    /** Pin name, for example `VCC`. */
    readonly name: string;
    /** Electrical role read from the library, or `unspecified` when absent. */
    readonly type: PinType;
    /** Unit (gate) the pin belongs to; `1` when the symbol has one unit. */
    readonly unit: number;
}

/** One symbol declared by a KiCad `.kicad_sym` library. */
export interface KicadSymbol {
    /** Symbol name, for example `74HC00`. */
    readonly name: string;
    /** Number of units (gates) the symbol declares; at least 1. */
    readonly unitCount: number;
    readonly pins: readonly KicadSymbolPin[];
}

/** Result of `OpenEDA.importKicadSymbol()`. */
export interface KicadSymbolImportResult {
    readonly kind: 'kicad_sym';
    readonly symbols: readonly KicadSymbol[];
    readonly totals: { readonly symbols: number; readonly pins: number };
    /** Constructs outside the imported subset; empty when nothing was lost. */
    readonly losses: readonly KicadImportLoss[];
}

/** One finding of `OpenEDA.checkKicadSymbol()`. */
export interface KicadSymbolFinding {
    /** Stable code such as `library/duplicate-pin-number`. */
    readonly code: string;
    readonly severity: 'error' | 'warning';
    /** Symbol the finding belongs to. */
    readonly symbol: string;
    /** Pin number the finding belongs to, or `null` for a symbol-level finding. */
    readonly pin: string | null;
    /** Unit the pin belongs to, or `null` for a symbol-level finding. */
    readonly unit: number | null;
    readonly detail: string;
}

/** Result of `OpenEDA.checkKicadSymbol()`. */
export interface KicadSymbolCheckResult {
    readonly kind: 'kicad_sym';
    readonly format: 'open-eda-library-check';
    readonly version: 1;
    readonly symbols: number;
    readonly pins: number;
    readonly findings: readonly KicadSymbolFinding[];
    /** False only when at least one finding is an error. */
    readonly passed: boolean;
    /** Constructs the reader skipped; these never fail the check on their own. */
    readonly losses: readonly KicadImportLoss[];
}

/** Electrical direction of one hierarchical sheet port. */
export type PortDirection = 'input' | 'output' | 'bidirectional' | 'tri_state' | 'passive';

/** One port on a hierarchical sheet symbol. */
export interface SheetPort {
    readonly name: string;
    readonly direction: PortDirection;
    readonly x: number;
    readonly y: number;
}

/** One hierarchical sheet symbol stored by the document. */
export interface HierarchicalSheet {
    readonly id: ObjectId;
    readonly name: string;
    readonly file: string;
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
    /** Ports declared on this symbol; absent when the symbol has no port. */
    readonly ports?: readonly SheetPort[];
}

/** Input of `design.addHierarchy()`; size defaults to 60 by 40 design units. */
export interface AddHierarchyInput {
    readonly name: string;
    readonly file: string;
    readonly x?: number;
    readonly y?: number;
    readonly width?: number;
    readonly height?: number;
}

/** Input of `design.addHierarchyPort()`. */
export interface AddHierarchyPortInput {
    /** Hierarchical sheet id or name. */
    readonly sheet: string;
    readonly name: string;
    readonly direction?: PortDirection;
    readonly x?: number;
    readonly y?: number;
}

/** Input of `design.removeHierarchyPort()`. */
export interface RemoveHierarchyPortInput {
    /** Hierarchical sheet id or name. */
    readonly sheet: string;
    readonly name: string;
}

/** Result of `design.addHierarchy()`. */
export interface HierarchyResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly hierarchy: HierarchicalSheet;
}

/** Result of `design.listHierarchy()`. */
export interface HierarchyListResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly count: number;
    readonly hierarchy: readonly HierarchicalSheet[];
}

/** Result of `design.removeHierarchy()` and `design.removeHierarchyPort()`. */
export interface RemoveHierarchyResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly removed: string;
}

/** Result of `design.addHierarchyPort()`. */
export interface HierarchyPortResult {
    readonly revision: number;
    readonly design: DesignSummary;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
    readonly hierarchy: HierarchicalSheet;
    readonly port: SheetPort;
}

/** One finding of `design.checkHierarchy()`. */
export interface HierarchyCheckIssue {
    readonly code: string;
    readonly severity: 'error' | 'warning' | 'info';
    /** Child document path the finding belongs to. */
    readonly sheet: string;
    readonly detail: string;
}

/** Result of `design.checkHierarchy()`, tagged `open-eda-hierarchy`. */
export interface HierarchyCheckReport {
    readonly format: 'open-eda-hierarchy';
    readonly version: number;
    /** Child documents that opened successfully; the root is excluded. */
    readonly sheets: number;
    readonly issues: readonly HierarchyCheckIssue[];
    /** False only when at least one issue is an error. */
    readonly passed: boolean;
}

/** A sheet as returned by the sheet operations. */
export interface Sheet {
    readonly id: SheetId;
    readonly name: string;
    readonly components: readonly Component[];
    readonly nets: readonly Net[];
    readonly wires: readonly Wire[];
    readonly junctions: readonly Junction[];
    readonly no_connects: readonly NoConnect[];
    readonly texts: readonly TextLabel[];
    /** Net labels; absent in documents written before labels existed. */
    readonly labels?: readonly NetLabel[];
}

/** Input of `OpenEDA.diff()`. */
export interface DesignDiffInput {
    /** Path of the before document. */
    readonly left: string;
    /** Path of the after document. */
    readonly right: string;
}

/** One side of a diff report. */
export interface DesignDiffSide {
    readonly path: string;
    readonly id: DesignId;
    readonly revision: number;
    readonly title: string | null;
}

/** Added, removed and modified counters of one object family. */
export interface DesignDiffCounts {
    readonly added: number;
    readonly removed: number;
    readonly modified: number;
}

/** One field change of a compared object. */
export interface DesignDiffFieldChange {
    readonly from: unknown;
    readonly to: unknown;
}

/** One modified object, keyed by its reference or name. */
export interface DesignDiffObjectChange {
    readonly ref?: string;
    readonly name?: string;
    readonly id?: string;
    readonly changes: Readonly<Record<string, DesignDiffFieldChange>>;
}

/** Added, removed and modified lists of one object family. */
export interface DesignDiffSet<T> {
    readonly added: readonly T[];
    readonly removed: readonly T[];
    readonly modified: readonly DesignDiffObjectChange[];
}

/** One sheet entry of a diff report. */
export interface DesignDiffSheet {
    readonly id: SheetId;
    readonly name: string;
    readonly leftName: string | null;
    readonly rightName: string | null;
    readonly status: 'added' | 'removed' | 'modified' | 'unchanged';
    /** Present only when the sheet itself was renamed. */
    readonly changes?: Readonly<Record<string, DesignDiffFieldChange>>;
    readonly components: DesignDiffSet<Component>;
    readonly nets: DesignDiffSet<Net>;
    readonly wires: DesignDiffSet<Wire>;
    readonly junctions: Omit<DesignDiffSet<Junction>, 'modified'>;
    readonly noConnects: Omit<DesignDiffSet<NoConnect>, 'modified'>;
    readonly texts: Omit<DesignDiffSet<TextLabel>, 'modified'>;
}

/** Result of `OpenEDA.diff()`, tagged `open-eda-diff`. */
export interface DesignDiffReport {
    readonly format: 'open-eda-diff';
    readonly version: number;
    /** True when no sheet, object, bus or variant changed. */
    readonly equal: boolean;
    readonly left: DesignDiffSide;
    readonly right: DesignDiffSide;
    readonly summary: {
        readonly sheets: DesignDiffCounts & { readonly unchanged: number };
        readonly components: DesignDiffCounts;
        readonly nets: DesignDiffCounts;
        readonly wires: DesignDiffCounts;
        readonly junctions: DesignDiffCounts;
        readonly noConnects: DesignDiffCounts;
        readonly texts: DesignDiffCounts;
        readonly buses: DesignDiffCounts;
        readonly variants: DesignDiffCounts;
        readonly design: { readonly modified: number };
    };
    /** Drawing property changes; null when title, rev, company, date and units all match. */
    readonly design: { readonly changes: Readonly<Record<string, DesignDiffFieldChange>> | null };
    /** Only the sheets that changed. */
    readonly sheets: readonly DesignDiffSheet[];
    readonly buses: DesignDiffSet<Bus>;
    readonly variants: DesignDiffSet<Variant>;
}

/** An open design document. Every operation keeps its own stable code. */
export interface Design {
    readonly id: DesignId;
    readonly revision: number;
    readonly summary: DesignSummary;
    /** True when an earlier snapshot is available on this handle. */
    readonly canUndo: boolean;
    /** True when an undo can be reapplied on this handle. */
    readonly canRedo: boolean;
    /** Runs the supported rule checks (`erc`, `nets`, `connectivity`, `footprints`, `power`). */
    inspect(options?: InspectOptions): Promise<InspectReport>;
    /** Writes the native `.oeda` document; defaults to the path it was opened from. */
    save(options?: SaveOptions): Promise<SaveReport>;
    /** Projects the design to `net`, `net_csv`, `spice`, `svg`, `bom`, `bom_json`, `bom_html`, `pdf`, `report`, `json`, `kicad_sch` or `kicad_sym`; other targets report `capability/not-implemented`. */
    export(options: ExportOptions): Promise<ExportReport>;
    /** Updates title, drawing revision, company and date; an empty string clears a field. */
    setProperties(properties: DesignPropertiesInput): Promise<DesignPropertiesResult>;
    /** Assigns or clears the electrical role of one pin; omitted `pinType` clears it. */
    setPinType(input: SetPinTypeInput): Promise<SetPinTypeResult>;
    /** Adds one pin to a placed component; an illegal or duplicate name is refused. */
    addPin(input: AddPinInput): Promise<Component>;
    /** Removes one pin that no net node or no-connect flag still references. */
    removePin(input: RemovePinInput): Promise<Component>;
    /** Renames one pin; every net node and no-connect flag that named it follows. */
    renamePin(input: RenamePinInput): Promise<Component>;
    /** Swaps the net connections of two pins of one component; each pin takes the other pin's connection. */
    swapPins(input: SwapPinsInput): Promise<SwapPinsResult>;
    /** Renumbers every reference designator by sheet and placement order; net nodes follow. */
    annotate(): Promise<AnnotateResult>;
    /** Restores the snapshot taken before the newest edit. */
    undo(): Promise<HistoryResult>;
    /** Reapplies the snapshot the newest undo displaced. */
    redo(): Promise<HistoryResult>;
    /** Adds one component and returns the created object. */
    addComponent(input: ComponentInput): Promise<Component>;
    /** Copies one placed component under a new reference; net nodes and wires are not copied. */
    copyComponent(input: CopyComponentInput): Promise<Component>;
    /** Deletes one component; its net nodes are removed and detached wires stay valid. */
    deleteComponent(reference: string, options?: { readonly sheet?: SheetId }): Promise<void>;
    /** Connects `REF.PIN` nodes to a net, creating it when needed. */
    connectNet(input: ConnectNetInput): Promise<Net>;
    /** Adds one wire segment. */
    addWire(input: WireInput): Promise<Wire>;
    /** Updates the endpoints or net of one wire segment. */
    updateWire(input: UpdateWireInput): Promise<Wire>;
    /** Splits one wire segment into two at a point on the segment. */
    splitWire(input: SplitWireInput): Promise<SplitWireResult>;
    /** Merges two touching collinear wires of the same net into one segment. */
    mergeWires(input: MergeWireInput): Promise<MergeWireResult>;
    /** Translates selected components and wires of a sheet by an offset. */
    translate(input: TranslateInput): Promise<TranslateResult>;
    /** Lists the wires drawn on a sheet. */
    listWires(options?: { readonly sheet?: SheetId }): Promise<readonly Wire[]>;
    /** Adds one junction (connection dot). */
    addJunction(input: JunctionInput): Promise<Junction>;
    /** Adds one free text annotation. */
    addText(input: TextInput): Promise<TextLabel>;
    /** Adds one no-connect flag; a `node` marks that pin as deliberately open. */
    addNoConnect(input: NoConnectInput): Promise<NoConnect>;
    /** Deletes the no-connect flag at a 0-based index on a sheet. */
    deleteNoConnect(index: number, options?: { readonly sheet?: SheetId }): Promise<void>;
    /** Moves one component to a new placement and optional rotation. */
    moveComponent(input: MoveComponentInput): Promise<Component>;
    /** Replaces the value text of one component. */
    updateComponent(input: UpdateComponentInput): Promise<Component>;
    /** Assigns or clears the footprint of one component; the BOM and KiCad writer use it verbatim. */
    setFootprint(input: SetFootprintInput): Promise<Component>;
    /** Assigns one custom key/value property of a component; an unchanged value adds no revision. */
    setComponentProperty(input: SetComponentPropertyInput): Promise<ComponentPropertyResult>;
    /** Removes one custom property of a component; an absent key reports `object/not-found`. */
    removeComponentProperty(input: RemoveComponentPropertyInput): Promise<ComponentPropertyResult>;
    /** Lists the custom properties of one component, sorted by key. */
    listComponentProperties(
        reference: string,
        options?: { readonly sheet?: SheetId },
    ): Promise<ComponentPropertyListResult>;
    /** Renames a declared net across every sheet; wires, labels, net-class and bus alias assignments follow; an already-matching name reports `changed: false` and adds no revision. */
    renameNet(input: RenameNetInput): Promise<RenameNetResult>;
    /** Deletes one wire segment. */
    deleteWire(wire: ObjectId, options?: { readonly sheet?: SheetId }): Promise<void>;
    /** Names every connected wire group on a sheet and reports the counts. */
    deriveNets(sheet?: SheetId): Promise<DeriveNetsResult>;
    /** Removes `REF.PIN` nodes from a net while keeping the net and its wires. */
    disconnectNet(input: DisconnectNetInput): Promise<Net>;
    /** Deletes a net and detaches the wires that named it. */
    deleteNet(input: DeleteNetInput): Promise<void>;
    /** Merges one net into another, appending its nodes and repointing its wires. */
    mergeNets(input: MergeNetsInput): Promise<MergeNetsResult>;
    /** Declares a design-level bus; a duplicate name is refused. */
    addBus(input: BusInput): Promise<Bus>;
    /** Renames, resizes or replaces the aliases of one declared bus. */
    updateBus(input: UpdateBusInput): Promise<UpdateBusResult>;
    /** Removes one declared bus by id or name. */
    removeBus(bus: ObjectId | string): Promise<void>;
    /** Lists the buses declared by the design. */
    listBuses(): Promise<readonly Bus[]>;
    /** Declares or replaces a BOM variant; every designator must be placed. */
    addVariant(input: VariantInput): Promise<Variant>;
    /** Removes one BOM variant by name. */
    removeVariant(name: string): Promise<void>;
    /** Lists the BOM variants declared by the design. */
    listVariants(): Promise<readonly Variant[]>;
    /** Declares or replaces a net class with routing attributes and net/bus assignments. */
    addNetClass(input: NetClassInput): Promise<NetClass>;
    /** Removes one net class by id or name. */
    removeNetClass(netClass: ObjectId | string): Promise<void>;
    /** Lists the net classes declared by the design. */
    listNetClasses(): Promise<readonly NetClass[]>;
    /** Declares or replaces the expected reference prefix of one component class. */
    setReferencePrefix(input: ReferencePrefixInput): Promise<ReferencePrefixRule>;
    /** Removes one reference prefix rule by component class. */
    removeReferencePrefix(componentClass: string): Promise<void>;
    /** Lists the reference prefix rules declared by the design. */
    listReferencePrefixes(): Promise<readonly ReferencePrefixRule[]>;
    /** Assigns or clears the component class of one placed component. */
    setComponentClass(input: ComponentClassInput): Promise<ComponentClassResult>;
    /** Lists component class assignments with their resolved references. */
    listComponentClasses(): Promise<readonly ComponentClassAssignment[]>;
    /**
     * Compares the design with a KiCad netlist.
     *
     * The design is the reference side: `added` entries exist only in the
     * netlist and `removed` entries exist only in the design.
     */
    compareNetlist(source: string): Promise<NetlistDiffReport>;
    /** Deletes the junction at a 0-based index on a sheet. */
    deleteJunction(index: number, options?: { readonly sheet?: SheetId }): Promise<void>;
    /** Moves the junction at a 0-based index to a new placement. */
    moveJunction(index: number, x: number, y: number, options?: { readonly sheet?: SheetId }): Promise<Junction>;
    /** Deletes the text annotation at a 0-based index on a sheet. */
    deleteText(index: number, options?: { readonly sheet?: SheetId }): Promise<void>;
    /** Moves the text annotation at a 0-based index to a new placement. */
    moveText(index: number, x: number, y: number, options?: { readonly sheet?: SheetId }): Promise<TextLabel>;
    /** Lists the text annotations of a sheet in 0-based index order. */
    listTexts(options?: { readonly sheet?: SheetId }): Promise<TextListResult>;
    /** Updates the content or placement of the text annotation at a 0-based index. */
    updateText(input: TextUpdateInput): Promise<TextUpdateResult>;
    /** Adds one net label; scope defaults to `local` and never changes the declared nets. */
    addLabel(input: LabelInput): Promise<NetLabel>;
    /** Lists the net labels of one sheet in document order. */
    listLabels(options?: { readonly sheet?: SheetId }): Promise<LabelListResult>;
    /** Updates one net label by identity; omitted fields keep their current value. */
    updateLabel(input: LabelUpdateInput): Promise<LabelUpdateResult>;
    /** Deletes one net label by identity. */
    removeLabel(id: ObjectId, options?: { readonly sheet?: SheetId }): Promise<void>;
    /** Lists the junctions of one sheet with 0-based positions. */
    listJunctions(options?: { readonly sheet?: SheetId }): Promise<JunctionListResult>;
    /** Lists the no-connect flags of one sheet with 0-based positions. */
    listNoConnects(options?: { readonly sheet?: SheetId }): Promise<NoConnectListResult>;
    /** Lists every sheet with its identity, name and deterministic object counts. */
    listSheets(): Promise<SheetListResult>;
    /** Reports deterministic per-sheet and design-wide object counts. */
    stats(): Promise<DesignStats>;
    /** Adds an empty sheet and returns it. */
    addSheet(name?: string): Promise<Sheet>;
    /** Renames one sheet. */
    renameSheet(sheet: SheetId, name: string): Promise<Sheet>;
    /** Deletes one empty sheet; the last sheet and sheets with objects are refused. */
    deleteSheet(sheet: SheetId): Promise<void>;
    /** Adds one hierarchical sheet symbol; editing does not read the child file. */
    addHierarchy(input: AddHierarchyInput): Promise<HierarchyResult>;
    /** Lists the hierarchical sheet symbols of the design. */
    listHierarchy(): Promise<HierarchyListResult>;
    /**
     * Checks the declared hierarchy against the child `.oeda` documents.
     *
     * Only `.oeda` children are opened; other extensions, missing files, parse
     * failures, cycles, excessive depth and port/label mismatches are reported
     * as deterministic issues. The check is read-only and needs the document
     * path of an opened or saved design.
     */
    checkHierarchy(): Promise<HierarchyCheckReport>;
    /** Removes one hierarchical sheet symbol by id or name. */
    removeHierarchy(sheet: string): Promise<RemoveHierarchyResult>;
    /** Adds one port to a hierarchical sheet symbol. */
    addHierarchyPort(input: AddHierarchyPortInput): Promise<HierarchyPortResult>;
    /** Removes one port from a hierarchical sheet symbol. */
    removeHierarchyPort(input: RemoveHierarchyPortInput): Promise<RemoveHierarchyResult>;
    /** Lists components, nets and wires of the open design. */
    listObjects(): Promise<DesignObjects>;
    /** Lists the symbol library the design uses, with pin counts and missing footprints. */
    listSymbols(): Promise<SymbolLibraryResult>;
    /** Reports what each net connects, the pins no net reaches and the per-sheet wire totals. */
    connectivity(): Promise<ConnectivityReport>;
    /** Traces the read-only connection path between two `REF.PIN` pins. */
    tracePins(from: string, to: string): Promise<TraceReport>;
    /** Reports one placed component's placement, properties, pins and reaching nets. */
    componentReport(reference: string): Promise<ComponentReport>;
    /** Reports one declared net's nodes, wires, labels and matching bus and net class. */
    netShow(name: string, options?: { readonly sheet?: SheetId }): Promise<NetReport>;
    listComponents(options?: { readonly sheet?: SheetId }): Promise<readonly Component[]>;
    listNets(options?: { readonly sheet?: SheetId }): Promise<readonly Net[]>;
    close(): Promise<void>;
}
