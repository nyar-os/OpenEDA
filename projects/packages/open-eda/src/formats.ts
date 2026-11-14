/**
 * Design format family.
 *
 * EDA files split by what they describe (schematic capture, board layout,
 * netlist, simulation deck, fabrication output) and by container (KiCad
 * S-expression, generic text, fixed-layout vector or raster). The registry below
 * classifies extensions and fixes the default open intent.
 *
 * Classification is **not** a support claim. What the installed engine can read,
 * write or export is reported by `OpenEDA.capabilities()`; an extension listed
 * here may still be `unsupported`.
 */

/** Design intent applied when the caller does not override it. */
export type DesignIntent = 'edit' | 'inspect' | 'exchange';

/** Grouping used for capability reporting and user-facing wording. */
export type FormatFamily = 'schematic' | 'board' | 'netlist' | 'simulation' | 'fabrication' | 'exchange' | 'image';

/** Container encoding behind an extension. `unknown` means the tool is not fixed. */
export type FormatMarkup = 'sexpr' | 'text' | 'xml' | 'fixed' | 'raster' | 'step' | 'unknown';

/** What an extension can be consumed as. `none` means it never becomes a design. */
export type FormatOpenKind = 'design' | 'netlist' | 'exchange' | 'none';

/** Every extension the product must classify. */
export const designFormats = [
    'kicad_sch',
    'kicad_pcb',
    'sch',
    'brd',
    'net',
    'edif',
    'spice',
    'cir',
    'gbr',
    'drl',
    'step',
    'svg',
    'pdf',
    'png',
    'bom',
] as const;

/** Union of {@link designFormats}. */
export type DesignFormat = (typeof designFormats)[number];

/** What a format is, independent of whether it is implemented. */
export interface DesignFormatInfo {
    readonly format: DesignFormat;
    readonly family: FormatFamily;
    readonly markup: FormatMarkup;
    /** Whether the container may carry executable macros. Such parts are never executed. */
    readonly macroEnabled: boolean;
    readonly opensAs: FormatOpenKind;
    /** Intent applied to `open()` when the extension arrives without one. */
    readonly defaultIntent: DesignIntent | null;
}

type FormatRegistryEntry = Omit<DesignFormatInfo, 'format'>;

/** Extension to classification. Values describe the format, never a capability. */
export const formatRegistry: Readonly<Record<DesignFormat, FormatRegistryEntry>> = Object.freeze({
    kicad_sch: { family: 'schematic', markup: 'sexpr', macroEnabled: false, opensAs: 'design', defaultIntent: 'edit' },
    kicad_pcb: { family: 'board', markup: 'sexpr', macroEnabled: false, opensAs: 'design', defaultIntent: 'edit' },
    sch: { family: 'schematic', markup: 'unknown', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    brd: { family: 'board', markup: 'unknown', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    net: { family: 'netlist', markup: 'text', macroEnabled: false, opensAs: 'netlist', defaultIntent: 'exchange' },
    edif: { family: 'netlist', markup: 'sexpr', macroEnabled: false, opensAs: 'netlist', defaultIntent: 'exchange' },
    spice: { family: 'simulation', markup: 'text', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    cir: { family: 'simulation', markup: 'text', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    gbr: { family: 'fabrication', markup: 'text', macroEnabled: false, opensAs: 'exchange', defaultIntent: null },
    drl: { family: 'fabrication', markup: 'text', macroEnabled: false, opensAs: 'exchange', defaultIntent: null },
    step: { family: 'exchange', markup: 'step', macroEnabled: false, opensAs: 'exchange', defaultIntent: null },
    svg: { family: 'exchange', markup: 'xml', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    pdf: { family: 'exchange', markup: 'fixed', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    png: { family: 'image', markup: 'raster', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    bom: { family: 'fabrication', markup: 'text', macroEnabled: false, opensAs: 'none', defaultIntent: null },
});

/** Returns the full classification of one format. */
export function formatInfo(format: DesignFormat): DesignFormatInfo {
    return { format, ...formatRegistry[format] };
}

/** Extracts a classified extension from a path, ignoring case. */
export function formatFromPath(path: string): DesignFormat | undefined {
    const extension = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
    return (designFormats as readonly string[]).includes(extension) ? (extension as DesignFormat) : undefined;
}

/** Default open intent of a format, or `null` when it never opens as a design. */
export function defaultIntentFor(format: DesignFormat): DesignIntent | null {
    return formatRegistry[format].defaultIntent;
}
