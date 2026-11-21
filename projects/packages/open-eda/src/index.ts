/**
 * Authoring surface of `@wae/open-eda`.
 *
 * The native model implements the schematic document core: create and open a
 * `.oeda` document, edit sheets, components, nets and wires with bounded
 * undo/redo, list objects, run ERC/net/footprint checks and project to a
 * netlist, SVG, BOM or PDF. A KiCad schematic import/export covers the same
 * subset. Board layout, clearance checks, Gerber exchange and the interactive
 * renderer remain unimplemented and are reported as unsupported. Design rules,
 * inspection, rendering and exchange formats belong upstream and are never
 * re-implemented in this package.
 */

export { OpenEDA } from './open-eda.js';
export { NotImplementedError, OpenEDAError, errorCodes, isOpenEDAError } from './errors.js';
export type { OpenEDAErrorCode, OpenEDAErrorDetails, OpenEDAErrorOptions } from './errors.js';
export { defaultIntentFor, designFormats, formatFromPath, formatInfo, formatRegistry } from './formats.js';
export type {
    DesignFormat,
    DesignFormatInfo,
    DesignIntent,
    FormatFamily,
    FormatMarkup,
    FormatOpenKind,
} from './formats.js';
export { GuiError, closeGui, defaultGuiContentDir, isGuiOpen, openGui, pumpEvents, runGui } from './gui.js';
export type { GuiCloseResult, GuiOpenResult, GuiStatus, OpenGuiOptions } from './gui.js';
export type {
    BusListResult,
    BusResult,
    ComponentListResult,
    ComponentResult,
    DeleteBusResult,
    DeleteResult,
    DesignHandle,
    EditResult,
    HistoryFlags,
    JunctionResult,
    NetListResult,
    NetResult,
    SheetResult,
    TextResult,
    WireResult,
} from './service.js';
export type * from './types.js';
