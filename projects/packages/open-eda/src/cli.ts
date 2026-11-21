#!/usr/bin/env node
/**
 * Command-line shell around the `OpenEDA` session.
 *
 * Every command only parses arguments, calls one session or `Design` method,
 * prints the result and returns an exit code. The session owns open/save and,
 * with `history: 'sidecar'`, the cross-process undo stack; the CLI never imports
 * the orthogonal service functions or touches `<file>.history.json` itself.
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { createCli } from '@wae/commander';

import { runGui } from './gui.js';
import { OpenEDAError } from './errors.js';
import { OpenEDA } from './open-eda.js';
import type {
    ConnectivityReport,
    Design,
    DesignCapabilities,
    DesignDiffReport,
    DesignObjects,
    DesignSummary,
    ExportReport,
    ExportTarget,
    InspectCheck,
    InspectOptions,
    InspectReport,
    InspectSeverity,
    InspectWaiver,
    KicadSymbolImportResult,
    PinType,
    SaveReport,
    SymbolLibraryResult,
} from './types.js';

const here = path.dirname(fileURLToPath(import.meta.url));

/** One session serves every command; sidecar mode keeps history on disk. */
const app = new OpenEDA({ history: 'sidecar' });

const cli = createCli('open-eda').locales(path.join(here, '..', 'locales')).intro('cli.intro');

// Exported for contract audits that import the built entry to inspect the
// command tree. Nothing runs until the file is executed as the program entry.
export { cli };

interface ParsedOptions {
    readonly _?: readonly string[];
    readonly [key: string]: unknown;
}

const text = (options: ParsedOptions, key: string): string | undefined =>
    typeof options[key] === 'string' ? (options[key] as string) : undefined;

const flag = (options: ParsedOptions, key: string): boolean => options[key] === true || options[key] === 'true';

const number = (options: ParsedOptions, key: string): number | undefined => {
    const raw = text(options, key);
    if (raw === undefined) {
        return undefined;
    }
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : Number.NaN;
};

const integer = (options: ParsedOptions, key: string): number | undefined => {
    const parsed = number(options, key);
    if (parsed === undefined) {
        return undefined;
    }
    return Number.isInteger(parsed) ? parsed : Number.NaN;
};

const list = (options: ParsedOptions, key: string): string[] =>
    text(options, key)
        ?.split(',')
        .map((item) => item.trim())
        .filter((item) => item.length > 0) ?? [];

const input = (options: ParsedOptions): string | undefined => options._?.[0];

/** Opens one document for the duration of a command and always releases it. */
async function withDesign<T>(file: string, run: (design: Design) => Promise<T>): Promise<T> {
    const design = await app.open({ path: file });
    try {
        return await run(design);
    } finally {
        await design.close();
    }
}

/** Export targets the CLI accepts, mirroring the declared capability matrix. */
const exportTargets: readonly ExportTarget[] = [
    'kicad_sch',
    'kicad_sym',
    'kicad_pcb',
    'net',
    'net_csv',
    'net_hier',
    'spice',
    'spice_subckt',
    'gbr',
    'pdf',
    'svg',
    'bom',
    'bom_json',
    'bom_html',
    'report',
    'json',
];

function asExportTarget(value: string | undefined): ExportTarget | undefined {
    return value !== undefined && (exportTargets as readonly string[]).includes(value) ? (value as ExportTarget) : undefined;
}

function describeCapabilities(matrix: DesignCapabilities): string {
    const formats = Object.entries(matrix.formats)
        .filter(([, entry]) => entry.import === 'supported')
        .map(([name]) => name);
    const exports = Object.entries(matrix.formats)
        .filter(([, entry]) => entry.export === 'supported')
        .map(([name]) => name);
    const checks = Object.entries(matrix.checks)
        .filter(([, level]) => level === 'supported')
        .map(([name]) => name);
    const targets = Object.entries(matrix.exportTargets)
        .filter(([, level]) => level === 'supported')
        .map(([name]) => name);
    return [
        `engine: ${matrix.engine.package} ${matrix.engine.version}`,
        `import formats: ${formats.length > 0 ? formats.join(', ') : 'none supported'}`,
        `export formats: ${exports.length > 0 ? exports.join(', ') : 'none supported'}`,
        `export targets: ${targets.length > 0 ? targets.join(', ') : 'none supported'}`,
        `checks: ${checks.length > 0 ? checks.join(', ') : 'none supported'}`,
        `project: import=${matrix.project.import} edit=${matrix.project.edit} export=${matrix.project.export}`,
    ].join('\n');
}

function describeDesign(design: DesignSummary): string {
    const title = design.title ?? 'untitled';
    const properties = [
        design.rev.length > 0 ? `rev=${design.rev}` : undefined,
        design.company.length > 0 ? `company=${design.company}` : undefined,
        design.date.length > 0 ? `date=${design.date}` : undefined,
    ].filter((entry): entry is string => entry !== undefined);
    const suffix = properties.length > 0 ? ` ${properties.join(' ')}` : '';
    const lines = [`design ${design.id} "${title}" revision=${design.revision}${suffix} sheets=${design.sheets.length}`];
    for (const sheet of design.sheets) {
        lines.push(`  [${sheet.index}] ${sheet.id} ${sheet.title} (${sheet.objects} objects)`);
    }
    return lines.join('\n');
}

function describeReport(report: InspectReport): string {
    const lines = [
        `inspect revision=${report.revision} checker=${report.checkerVersion} passed=${report.passed} active=${report.issues.length} waived=${report.waived.length}`,
    ];
    for (const issue of report.issues) {
        lines.push(`  ${issue.severity} ${issue.code} ${issue.sheetId ?? '-'} ${issue.message}`);
    }
    for (const issue of report.waived) {
        lines.push(`  (waived) ${issue.severity} ${issue.code} ${issue.sheetId ?? '-'} ${issue.message}`);
    }
    return lines.join('\n');
}

type Parsed<T> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly message: string };

const severityLevels: readonly InspectSeverity[] = ['error', 'warning', 'info', 'off'];

function parseSeverity(raw: string): Parsed<Record<string, InspectSeverity>> {
    const overrides: Record<string, InspectSeverity> = {};
    for (const entry of raw.split(',').map((item) => item.trim()).filter((item) => item.length > 0)) {
        const separator = entry.indexOf('=');
        const code = separator > 0 ? entry.slice(0, separator).trim() : '';
        const level = separator > 0 ? entry.slice(separator + 1).trim() : '';
        if (code.length === 0 || !(severityLevels as readonly string[]).includes(level)) {
            return { ok: false, message: '`--severity` entries must be `code=level` with level error, warning, info or off' };
        }
        overrides[code] = level as InspectSeverity;
    }
    return { ok: true, value: overrides };
}

function parseWaivers(raw: string): Parsed<InspectWaiver[]> {
    const waivers: InspectWaiver[] = [];
    for (const entry of raw.split(',').map((item) => item.trim()).filter((item) => item.length > 0)) {
        const separator = entry.indexOf('@');
        const code = separator > 0 ? entry.slice(0, separator).trim() : entry;
        const target = separator > 0 ? entry.slice(separator + 1).trim() : '';
        if (code.length === 0) {
            return { ok: false, message: '`--waive` entries must be `code`, `code@objectId` or `code@sheetId/objectId`' };
        }
        if (target.length === 0) {
            waivers.push({ code });
            continue;
        }
        const slash = target.indexOf('/');
        waivers.push(
            slash > 0
                ? { code, sheetId: target.slice(0, slash), objectId: target.slice(slash + 1) }
                : { code, objectId: target },
        );
    }
    return { ok: true, value: waivers };
}

function parseVariantValues(raw: string): Parsed<Record<string, string>> {
    const values: Record<string, string> = {};
    for (const entry of raw.split(',').map((item) => item.trim()).filter((item) => item.length > 0)) {
        const separator = entry.indexOf('=');
        const reference = separator > 0 ? entry.slice(0, separator).trim() : '';
        const value = separator > 0 ? entry.slice(separator + 1).trim() : '';
        if (reference.length === 0 || value.length === 0) {
            return { ok: false, message: '`--values` entries must be `ref=value` pairs' };
        }
        values[reference] = value;
    }
    return { ok: true, value: values };
}

function describeObjects(objects: DesignObjects): string {
    const lines = [describeDesign(objects.design), `components: ${objects.components.length}`];
    for (const component of objects.components) {
        const value = component.value.length > 0 ? component.value : '(no value)';
        lines.push(`  ${component.ref} ${value} at (${component.x}, ${component.y}) rotation=${component.rotation}`);
    }
    lines.push(`nets: ${objects.nets.length}`);
    for (const net of objects.nets) {
        const nodes = net.nodes.map((node) => `${node.ref}.${node.pin}`).join(', ');
        lines.push(`  ${net.name}: ${nodes}`);
    }
    lines.push(`wires: ${objects.wires.length}`);
    return lines.join('\n');
}

function describeSymbols(library: SymbolLibraryResult): string {
    const totals = library.totals;
    const lines = [
        `revision=${library.revision} symbols=${totals.symbols} components=${totals.components} missingFootprint=${totals.missingFootprint}`,
    ];
    for (const symbol of library.symbols) {
        const footprints = symbol.footprints.length > 0 ? symbol.footprints.join(', ') : '(none)';
        lines.push(
            `  ${symbol.libId} pins=${symbol.pins} components=${symbol.components} refs=${symbol.references.join(',')} footprints=${footprints} missingFootprint=${symbol.missingFootprint}`,
        );
    }
    return lines.join('\n');
}

function describeKicadSymbolLibrary(library: KicadSymbolImportResult): string {
    const totals = library.totals;
    const lines = [`kicad_sym symbols=${totals.symbols} pins=${totals.pins} losses=${library.losses.length}`];
    for (const symbol of library.symbols) {
        lines.push(`  ${symbol.name} units=${symbol.unitCount} pins=${symbol.pins.length}`);
    }
    return lines.join('\n');
}

const changeText = (changes: Readonly<Record<string, { readonly from: unknown; readonly to: unknown }>>): string =>
    Object.entries(changes)
        .map(([field, change]) => `${field} ${JSON.stringify(change.from)} -> ${JSON.stringify(change.to)}`)
        .join(', ');

function describeDiff(report: DesignDiffReport): string {
    const totals = report.summary;
    const lines = [
        `diff ${report.left.path} (revision ${report.left.revision}) -> ${report.right.path} (revision ${report.right.revision}) equal=${report.equal}`,
        `sheets +${totals.sheets.added} -${totals.sheets.removed} ~${totals.sheets.modified} unchanged=${totals.sheets.unchanged}`,
        `components +${totals.components.added} -${totals.components.removed} ~${totals.components.modified}`,
        `nets +${totals.nets.added} -${totals.nets.removed} ~${totals.nets.modified}`,
        `wires +${totals.wires.added} -${totals.wires.removed} ~${totals.wires.modified}`,
        `buses +${totals.buses.added} -${totals.buses.removed} ~${totals.buses.modified}`,
        `variants +${totals.variants.added} -${totals.variants.removed} ~${totals.variants.modified}`,
    ];
    if (report.design.changes !== null) {
        lines.push(`  design ${changeText(report.design.changes)}`);
    }
    for (const variant of report.variants.added) {
        lines.push(`  variant +${variant.name} dnp=${variant.dnp.join(',') || '-'}`);
    }
    for (const variant of report.variants.removed) {
        lines.push(`  variant -${variant.name} dnp=${variant.dnp.join(',') || '-'}`);
    }
    for (const variant of report.variants.modified) {
        lines.push(`  variant ~${variant.name ?? '?'} ${changeText(variant.changes)}`);
    }
    for (const bus of report.buses.added) {
        lines.push(`  bus +${bus.name} width=${bus.width}`);
    }
    for (const bus of report.buses.removed) {
        lines.push(`  bus -${bus.name} width=${bus.width}`);
    }
    for (const bus of report.buses.modified) {
        lines.push(`  bus ~${bus.name ?? '?'} ${changeText(bus.changes)}`);
    }
    for (const sheet of report.sheets) {
        lines.push(`  [${sheet.id}] ${sheet.name} ${sheet.status}`);
        for (const component of sheet.components.added) {
            lines.push(`    component +${component.ref} ${component.value}`);
        }
        for (const component of sheet.components.removed) {
            lines.push(`    component -${component.ref} ${component.value}`);
        }
        for (const component of sheet.components.modified) {
            lines.push(`    component ~${component.ref ?? '?'} ${changeText(component.changes)}`);
        }
        for (const net of sheet.nets.added) {
            lines.push(`    net +${net.name} nodes=${net.nodes.length}`);
        }
        for (const net of sheet.nets.removed) {
            lines.push(`    net -${net.name} nodes=${net.nodes.length}`);
        }
        for (const net of sheet.nets.modified) {
            lines.push(`    net ~${net.name ?? '?'} ${changeText(net.changes)}`);
        }
        const minor = [
            sheet.junctions.added.length + sheet.junctions.removed.length > 0
                ? `junctions +${sheet.junctions.added.length} -${sheet.junctions.removed.length}`
                : undefined,
            sheet.noConnects.added.length + sheet.noConnects.removed.length > 0
                ? `noConnects +${sheet.noConnects.added.length} -${sheet.noConnects.removed.length}`
                : undefined,
            sheet.texts.added.length + sheet.texts.removed.length > 0
                ? `texts +${sheet.texts.added.length} -${sheet.texts.removed.length}`
                : undefined,
            sheet.wires.added.length + sheet.wires.removed.length + sheet.wires.modified.length > 0
                ? `wires +${sheet.wires.added.length} -${sheet.wires.removed.length} ~${sheet.wires.modified.length}`
                : undefined,
        ].filter((entry): entry is string => entry !== undefined);
        if (minor.length > 0) {
            lines.push(`    ${minor.join(' ')}`);
        }
        if (sheet.changes !== undefined) {
            lines.push(`    ${changeText(sheet.changes)}`);
        }
    }
    return lines.join('\n');
}

function describeConnectivity(report: ConnectivityReport): string {    const totals = report.totals;
    const lines = [
        `connectivity revision=${report.revision} nets=${totals.nets} nodes=${totals.nodes} unconnectedPins=${totals.unconnectedPins} singleNodeNets=${totals.singleNodeNets} danglingNets=${totals.danglingNets} wireGroups=${totals.wireGroups} conflictingWireGroups=${totals.conflictingWireGroups} unassignedWireGroups=${totals.unassignedWireGroups} floatingJunctions=${totals.floatingJunctions}`,
    ];
    for (const sheet of report.sheets) {
        lines.push(
            `  [${sheet.id}] ${sheet.name}: nets=${sheet.nets.length} wires=${sheet.wires.segments} groups=${sheet.wires.groups} junctions=${sheet.junctions.total}`,
        );
        for (const net of sheet.nets) {
            const pins = net.pins.length > 0 ? net.pins.join(', ') : '(none)';
            lines.push(`    net ${net.name}: ${net.nodes} node(s) ${pins}`);
        }
        for (const pin of sheet.unconnectedPins) {
            lines.push(`    open ${pin.ref}.${pin.pin}`);
        }
    }
    return lines.join('\n');
}

/** Saves the already-mutated design and reports what the file now holds. */
async function savedReport(design: Design): Promise<SaveReport> {
    return design.save();
}

cli.command('capabilities', 'cli.cmd.capabilities').actionCtx(async (_options, _args, ctx) => {
    const matrix = await app.capabilities();
    ctx.emitResult(matrix, describeCapabilities(matrix));
    return 0;
});

cli
    .command('create', 'cli.cmd.create')
    .option('--out <file>', 'cli.opt.out')
    .option('--title <title>', 'cli.opt.title')
    .option('--rev <rev>', 'cli.opt.rev')
    .option('--company <company>', 'cli.opt.company')
    .option('--date <date>', 'cli.opt.date')
    .option('--template <name>', 'cli.opt.template')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-eda.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const design = await app.create({
            title: text(options, 'title'),
            rev: text(options, 'rev'),
            company: text(options, 'company'),
            date: text(options, 'date'),
            template: text(options, 'template'),
        });
        try {
            const report = await design.save({ target: out, overwrite: flag(options, 'overwrite') });
            ctx.emitResult(
                { ...report, design: design.summary },
                `created ${out} revision=${report.revision} sheets=${design.summary.sheets.length}`,
            );
        } finally {
            await design.close();
        }
        return 0;
    });

cli.command('annotate', 'cli.cmd.annotate').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
        return 2;
    }
    const { result, saved } = await withDesign(file, async (design) => {
        const result = await design.annotate();
        return { result, saved: await savedReport(design) };
    });
    ctx.emitResult(
        { ...saved, revision: saved.revision, annotated: result.annotated, changes: result.changes },
        `annotated ${result.annotated} component(s) in ${file} revision=${saved.revision}`,
    );
    return 0;
});

cli.command('open', 'cli.cmd.open').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
        return 2;
    }
    const design = await withDesign(file, async (opened) => opened.summary);
    ctx.emitResult(design, describeDesign(design));
    return 0;
});

cli.command('diff', 'cli.cmd.diff').actionCtx(async (options, _args, ctx) => {
    const [left, right] = options._ ?? [];
    if (left === undefined || right === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'two design documents are required' });
        return 2;
    }
    const report = await app.diff({ left, right });
    ctx.emitResult(report, describeDiff(report));
    return 0;
});

cli.command('list', 'cli.cmd.list').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
        return 2;
    }
    const objects = await withDesign(file, (design) => design.listObjects());
    ctx.emitResult(objects, describeObjects(objects));
    return 0;
});

cli.command('symbols', 'cli.cmd.symbols').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
        return 2;
    }
    const library = await withDesign(file, (design) => design.listSymbols());
    ctx.emitResult(library, describeSymbols(library));
    return 0;
});

cli.command('library', 'cli.cmd.library').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'a KiCad symbol library is required' });
        return 2;
    }
    const report = await app.importKicadSymbol(file);
    ctx.emitResult(report, describeKicadSymbolLibrary(report));
    return 0;
});

cli.command('connectivity', 'cli.cmd.connectivity').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
        return 2;
    }
    const report = await withDesign(file, (design) => design.connectivity());
    ctx.emitResult(report, describeConnectivity(report));
    return 0;
});

const designCommand = cli.command('design', 'cli.cmd.design');
designCommand
    .command('set', 'cli.cmd.design-set')
    .option('--title <title>', 'cli.opt.title')
    .option('--rev <rev>', 'cli.opt.rev')
    .option('--company <company>', 'cli.opt.company')
    .option('--date <date>', 'cli.opt.date')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const properties = {
            title: text(options, 'title'),
            rev: text(options, 'rev'),
            company: text(options, 'company'),
            date: text(options, 'date'),
        };
        if (Object.values(properties).every((entry) => entry === undefined)) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: 'at least one of `--title`, `--rev`, `--company` or `--date` is required',
            });
            return 2;
        }
        const { saved, title, rev, company, date } = await withDesign(file, async (design) => {
            const updated = await design.setProperties(properties);
            return { saved: await savedReport(design), title: updated.title, rev: updated.rev, company: updated.company, date: updated.date };
        });
        ctx.emitResult(
            { ...saved, title, rev, company, date },
            `set design properties in ${file} revision=${saved.revision} title=${title ?? '(cleared)'} rev=${rev.length > 0 ? rev : '(cleared)'} company=${company.length > 0 ? company : '(cleared)'} date=${date.length > 0 ? date : '(cleared)'}`,
        );
        return 0;
    });

const componentCommand = cli.command('component', 'cli.cmd.component');
componentCommand
    .command('add', 'cli.cmd.component-add')
    .option('--ref <ref>', 'cli.opt.ref')
    .option('--value <value>', 'cli.opt.value')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--rotation <degrees>', 'cli.opt.rotation')
    .option('--pins <list>', 'cli.opt.pins')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const reference = text(options, 'ref');
        const value = text(options, 'value');
        const x = number(options, 'x');
        const y = number(options, 'y');
        const rotation = number(options, 'rotation') ?? 0;
        if (
            reference === undefined ||
            reference.trim().length === 0 ||
            value === undefined ||
            x === undefined ||
            Number.isNaN(x) ||
            y === undefined ||
            Number.isNaN(y) ||
            Number.isNaN(rotation)
        ) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--ref`, `--value`, `--x` and `--y` are required; coordinates must be finite numbers',
            });
            return 2;
        }
        const { component, saved } = await withDesign(file, async (design) => {
            const component = await design.addComponent({
                sheet: text(options, 'sheet'),
                reference,
                value,
                x,
                y,
                rotation,
                pins: list(options, 'pins'),
            });
            return { component, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, component },
            `added ${component.ref} to ${file} revision=${saved.revision}`,
        );
        return 0;
    });

componentCommand
    .command('move', 'cli.cmd.component-move')
    .option('--ref <ref>', 'cli.opt.ref')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--rotation <degrees>', 'cli.opt.rotation')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const reference = text(options, 'ref');
        const x = number(options, 'x');
        const y = number(options, 'y');
        const rotation = number(options, 'rotation');
        if (
            reference === undefined ||
            reference.trim().length === 0 ||
            x === undefined ||
            Number.isNaN(x) ||
            y === undefined ||
            Number.isNaN(y) ||
            (rotation !== undefined && Number.isNaN(rotation))
        ) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--ref`, `--x` and `--y` are required; coordinates must be finite numbers',
            });
            return 2;
        }
        const { component, saved } = await withDesign(file, async (design) => {
            const component = await design.moveComponent({
                sheet: text(options, 'sheet'),
                reference,
                x,
                y,
                rotation,
            });
            return { component, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, component },
            `moved ${reference} to (${x}, ${y}) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

componentCommand
    .command('update', 'cli.cmd.component-update')
    .option('--ref <ref>', 'cli.opt.ref')
    .option('--value <value>', 'cli.opt.value')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const reference = text(options, 'ref');
        const value = text(options, 'value');
        if (reference === undefined || reference.trim().length === 0 || value === undefined) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--ref` and `--value` are required' });
            return 2;
        }
        const { component, saved } = await withDesign(file, async (design) => {
            const component = await design.updateComponent({
                sheet: text(options, 'sheet'),
                reference,
                value,
            });
            return { component, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, component },
            `updated ${reference} to ${value} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

componentCommand
    .command('footprint', 'cli.cmd.component-footprint')
    .option('--ref <ref>', 'cli.opt.ref')
    .option('--footprint <name>', 'cli.opt.footprint')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const reference = text(options, 'ref');
        if (reference === undefined || reference.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--ref` is required' });
            return 2;
        }
        const footprint = text(options, 'footprint') ?? '';
        const { component, saved } = await withDesign(file, async (design) => {
            const component = await design.setFootprint({
                sheet: text(options, 'sheet'),
                reference,
                footprint,
            });
            return { component, saved: await savedReport(design) };
        });
        const shown = component.footprint.length > 0 ? component.footprint : '(cleared)';
        ctx.emitResult(
            { ...saved, revision: saved.revision, component },
            `set footprint of ${reference} to ${shown} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

componentCommand
    .command('pin-type', 'cli.cmd.component-pin-type')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .option('--clear', 'cli.opt.clear')
    .actionCtx(async (options, _args, ctx) => {
        const [file, reference, pin, pinType] = options._ ?? [];
        if (file === undefined || reference === undefined || pin === undefined) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`<file> <ref> <pin> [type]` arguments are required',
            });
            return 2;
        }
        const clear = flag(options, 'clear');
        if (!clear && pinType === undefined) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: 'a pin electrical type is required unless `--clear` is passed',
            });
            return 2;
        }
        const { result, saved } = await withDesign(file, async (design) => {
            const result = await design.setPinType({
                sheet: text(options, 'sheet'),
                reference,
                pin,
                pinType: clear ? null : (pinType as PinType),
            });
            return { result, saved: await savedReport(design) };
        });
        const shown = result.pinType ?? '(cleared)';
        ctx.emitResult(
            { ...saved, revision: saved.revision, pinType: result },
            `set pin ${reference}.${pin} to ${shown} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

componentCommand
    .command('delete', 'cli.cmd.component-delete')
    .option('--ref <ref>', 'cli.opt.ref')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const reference = text(options, 'ref');
        if (reference === undefined || reference.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--ref` is required' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.deleteComponent(reference, { sheet: text(options, 'sheet') });
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: reference },
            `deleted ${reference} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const netCommand = cli.command('net', 'cli.cmd.net');
netCommand
    .command('connect', 'cli.cmd.net-connect')
    .option('--net <name>', 'cli.opt.net')
    .option('--nodes <list>', 'cli.opt.nodes')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const net = text(options, 'net');
        const nodes = list(options, 'nodes');
        if (net === undefined || net.trim().length === 0 || nodes.length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--net` and `--nodes REF.PIN,...` are required',
            });
            return 2;
        }
        const { net: connected, saved } = await withDesign(file, async (design) => {
            const connected = await design.connectNet({
                sheet: text(options, 'sheet'),
                net,
                nodes,
            });
            return { net: connected, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, net: connected },
            `connected ${net} (${connected.nodes.length} nodes) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

netCommand
    .command('rename', 'cli.cmd.net-rename')
    .option('--id <net>', 'cli.opt.net')
    .option('--name <name>', 'cli.opt.name')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const net = text(options, 'id');
        const name = text(options, 'name');
        if (net === undefined || net.trim().length === 0 || name === undefined || name.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--id` and `--name` are required' });
            return 2;
        }
        const { net: renamed, saved } = await withDesign(file, async (design) => {
            const renamed = await design.renameNet({ sheet: text(options, 'sheet'), net, name });
            return { net: renamed, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, net: renamed },
            `renamed net ${net} to ${name} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

netCommand
    .command('derive', 'cli.cmd.net-derive')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const { assigned, created, groups, saved } = await withDesign(file, async (design) => {
            const derived = await design.deriveNets(text(options, 'sheet'));
            return { ...derived, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, assigned, created, groups },
            `assigned ${assigned} group(s) and created ${created} net(s) of ${groups} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

netCommand
    .command('disconnect', 'cli.cmd.net-disconnect')
    .option('--net <name>', 'cli.opt.net')
    .option('--nodes <list>', 'cli.opt.nodes')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const net = text(options, 'net');
        if (net === undefined || net.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--net` is required' });
            return 2;
        }
        const { net: updated, saved } = await withDesign(file, async (design) => {
            const updated = await design.disconnectNet({
                sheet: text(options, 'sheet'),
                net,
                nodes: list(options, 'nodes'),
            });
            return { net: updated, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, net: updated },
            `disconnected nodes from ${net}, ${updated.nodes.length} remain in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

netCommand
    .command('delete', 'cli.cmd.net-delete')
    .option('--net <name>', 'cli.opt.net')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const net = text(options, 'net');
        if (net === undefined || net.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--net` is required' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.deleteNet({ sheet: text(options, 'sheet'), net });
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: net },
            `deleted net ${net} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const busCommand = cli.command('bus', 'cli.cmd.bus');

busCommand
    .command('add', 'cli.cmd.bus-add')
    .option('--name <name>', 'cli.opt.name')
    .option('--width <number>', 'cli.opt.width')
    .option('--aliases <names>', 'cli.opt.aliases')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const name = text(options, 'name');
        const width = integer(options, 'width');
        if (name === undefined || name.trim().length === 0 || width === undefined || Number.isNaN(width) || width < 1) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--name` and a positive integer `--width` are required',
            });
            return 2;
        }
        const aliases = list(options, 'aliases');
        const { bus, saved } = await withDesign(file, async (design) => {
            const bus = await design.addBus({ name, width, aliases });
            return { bus, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, bus },
            `declared bus ${bus.name}[0..${bus.width - 1}]${aliases.length > 0 ? ` aliased as ${aliases.join(',')}` : ''} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

busCommand
    .command('list', 'cli.cmd.bus-list')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const buses = await withDesign(file, (design) => design.listBuses());
        const lines = buses.map((bus) => `${bus.id} ${bus.name} width=${bus.width}`);
        ctx.emitResult({ buses }, [`${buses.length} bus(es) in ${file}`, ...lines].join('\n'));
        return 0;
    });

busCommand
    .command('remove', 'cli.cmd.bus-remove')
    .option('--id <bus>', 'cli.opt.bus')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const bus = text(options, 'id');
        if (bus === undefined || bus.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--id` is required' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.removeBus(bus);
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: bus },
            `removed bus ${bus} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const variantCommand = cli.command('variant', 'cli.cmd.variant');
variantCommand
    .command('add', 'cli.cmd.variant-add')
    .option('--name <name>', 'cli.opt.name')
    .option('--dnp <list>', 'cli.opt.dnp')
    .option('--values <pairs>', 'cli.opt.variant-values')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const name = text(options, 'name');
        if (name === undefined || name.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--name` is required' });
            return 2;
        }
        let values: Record<string, string> = {};
        const rawValues = text(options, 'values');
        if (rawValues !== undefined) {
            const parsed = parseVariantValues(rawValues);
            if (!parsed.ok) {
                ctx.fail('open-eda.cli.arguments-invalid', { message: parsed.message });
                return 2;
            }
            values = parsed.value;
        }
        const { variant, saved } = await withDesign(file, async (design) => {
            const variant = await design.addVariant({ name, dnp: list(options, 'dnp'), values });
            return { variant, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, variant },
            `declared variant ${variant.name} dnp=${variant.dnp.length} overrides=${Object.keys(variant.values).length} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

variantCommand
    .command('list', 'cli.cmd.variant-list')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const variants = await withDesign(file, (design) => design.listVariants());
        const lines = variants.map(
            (variant) => `${variant.name} dnp=${variant.dnp.join(',') || '-'} overrides=${Object.keys(variant.values).join(',') || '-'}`,
        );
        ctx.emitResult({ variants }, [`${variants.length} variant(s) in ${file}`, ...lines].join('\n'));
        return 0;
    });

variantCommand
    .command('remove', 'cli.cmd.variant-remove')
    .option('--name <name>', 'cli.opt.name')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const name = text(options, 'name');
        if (name === undefined || name.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--name` is required' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.removeVariant(name);
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: name },
            `removed variant ${name} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const wireCommand = cli.command('wire', 'cli.cmd.wire');
wireCommand
    .command('delete', 'cli.cmd.wire-delete')
    .option('--id <wire>', 'cli.opt.wire')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const wire = text(options, 'id');
        if (wire === undefined || wire.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--id` is required' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.deleteWire(wire, { sheet: text(options, 'sheet') });
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: wire },
            `deleted ${wire} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const junctionCommand = cli.command('junction', 'cli.cmd.junction');
junctionCommand
    .command('add', 'cli.cmd.junction-add')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const x = number(options, 'x');
        const y = number(options, 'y');
        if (x === undefined || Number.isNaN(x) || y === undefined || Number.isNaN(y)) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--x` and `--y` are required; coordinates must be finite numbers',
            });
            return 2;
        }
        const { junction, saved } = await withDesign(file, async (design) => {
            const junction = await design.addJunction({ sheet: text(options, 'sheet'), x, y });
            return { junction, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, junction },
            `added junction at (${x}, ${y}) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

junctionCommand
    .command('delete', 'cli.cmd.junction-delete')
    .option('--index <number>', 'cli.opt.index')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const index = integer(options, 'index');
        if (index === undefined || Number.isNaN(index) || index < 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--index` must be a non-negative integer' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.deleteJunction(index, { sheet: text(options, 'sheet') });
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: index },
            `deleted junction ${index} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

junctionCommand
    .command('move', 'cli.cmd.junction-move')
    .option('--index <number>', 'cli.opt.index')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const index = integer(options, 'index');
        const x = number(options, 'x');
        const y = number(options, 'y');
        if (
            index === undefined ||
            Number.isNaN(index) ||
            index < 0 ||
            x === undefined ||
            Number.isNaN(x) ||
            y === undefined ||
            Number.isNaN(y)
        ) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--index` must be a non-negative integer and `--x`/`--y` finite numbers',
            });
            return 2;
        }
        const { junction, saved } = await withDesign(file, async (design) => {
            const junction = await design.moveJunction(index, x, y, { sheet: text(options, 'sheet') });
            return { junction, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, junction },
            `moved junction ${index} to (${x}, ${y}) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const noConnectCommand = cli.command('no-connect', 'cli.cmd.no-connect');
noConnectCommand
    .command('add', 'cli.cmd.no-connect-add')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--ref <ref>', 'cli.opt.ref')
    .option('--pin <pin>', 'cli.opt.pin')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const x = number(options, 'x');
        const y = number(options, 'y');
        const reference = text(options, 'ref');
        const pin = text(options, 'pin');
        if (x === undefined || Number.isNaN(x) || y === undefined || Number.isNaN(y)) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--x` and `--y` are required; coordinates must be finite numbers',
            });
            return 2;
        }
        if ((reference === undefined) !== (pin === undefined)) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--ref` and `--pin` must be given together' });
            return 2;
        }
        const node = reference !== undefined && pin !== undefined ? { ref: reference, pin } : undefined;
        const { noConnect, saved } = await withDesign(file, async (design) => {
            const noConnect = await design.addNoConnect({ sheet: text(options, 'sheet'), x, y, node });
            return { noConnect, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, noConnect },
            `added no-connect at (${x}, ${y}) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

noConnectCommand
    .command('delete', 'cli.cmd.no-connect-delete')
    .option('--index <number>', 'cli.opt.index')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const index = integer(options, 'index');
        if (index === undefined || Number.isNaN(index) || index < 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--index` must be a non-negative integer' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.deleteNoConnect(index, { sheet: text(options, 'sheet') });
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: index },
            `deleted no-connect ${index} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const textCommand = cli.command('text', 'cli.cmd.text');
textCommand
    .command('add', 'cli.cmd.text-add')
    .option('--text <text>', 'cli.opt.text')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const content = text(options, 'text');
        const x = number(options, 'x');
        const y = number(options, 'y');
        if (
            content === undefined ||
            x === undefined ||
            Number.isNaN(x) ||
            y === undefined ||
            Number.isNaN(y)
        ) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--text`, `--x` and `--y` are required; coordinates must be finite numbers',
            });
            return 2;
        }
        const { text: annotation, saved } = await withDesign(file, async (design) => {
            const annotation = await design.addText({ sheet: text(options, 'sheet'), text: content, x, y });
            return { text: annotation, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, text: annotation },
            `added text "${content}" at (${x}, ${y}) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

textCommand
    .command('delete', 'cli.cmd.text-delete')
    .option('--index <number>', 'cli.opt.index')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const index = integer(options, 'index');
        if (index === undefined || Number.isNaN(index) || index < 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--index` must be a non-negative integer' });
            return 2;
        }
        const saved = await withDesign(file, async (design) => {
            await design.deleteText(index, { sheet: text(options, 'sheet') });
            return savedReport(design);
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, deleted: index },
            `deleted text ${index} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

textCommand
    .command('move', 'cli.cmd.text-move')
    .option('--index <number>', 'cli.opt.index')
    .option('--x <number>', 'cli.opt.x')
    .option('--y <number>', 'cli.opt.y')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const index = integer(options, 'index');
        const x = number(options, 'x');
        const y = number(options, 'y');
        if (
            index === undefined ||
            Number.isNaN(index) ||
            index < 0 ||
            x === undefined ||
            Number.isNaN(x) ||
            y === undefined ||
            Number.isNaN(y)
        ) {
            ctx.fail('open-eda.cli.arguments-invalid', {
                message: '`--index` must be a non-negative integer and `--x`/`--y` finite numbers',
            });
            return 2;
        }
        const { text: annotation, saved } = await withDesign(file, async (design) => {
            const annotation = await design.moveText(index, x, y, { sheet: text(options, 'sheet') });
            return { text: annotation, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, text: annotation },
            `moved text ${index} to (${x}, ${y}) in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

const sheetCommand = cli.command('sheet', 'cli.cmd.sheet');
sheetCommand
    .command('add', 'cli.cmd.sheet-add')
    .option('--name <name>', 'cli.opt.name')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const { sheet, saved } = await withDesign(file, async (design) => {
            const sheet = await design.addSheet(text(options, 'name'));
            return { sheet, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, sheet },
            `added sheet ${sheet.id} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

sheetCommand
    .command('rename', 'cli.cmd.sheet-rename')
    .option('--id <sheet>', 'cli.opt.sheet-id')
    .option('--name <name>', 'cli.opt.name')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const sheet = text(options, 'id');
        const name = text(options, 'name');
        if (sheet === undefined || sheet.trim().length === 0 || name === undefined || name.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--id` and `--name` are required' });
            return 2;
        }
        const { sheet: renamed, saved } = await withDesign(file, async (design) => {
            const renamed = await design.renameSheet(sheet, name);
            return { sheet: renamed, saved: await savedReport(design) };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, sheet: renamed },
            `renamed sheet ${sheet} to ${name} in ${file} revision=${saved.revision}`,
        );
        return 0;
    });

sheetCommand
    .command('delete', 'cli.cmd.sheet-delete')
    .option('--id <sheet>', 'cli.opt.sheet-id')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const sheet = text(options, 'id');
        if (sheet === undefined || sheet.trim().length === 0) {
            ctx.fail('open-eda.cli.arguments-invalid', { message: '`--id` is required' });
            return 2;
        }
        const { removed, saved } = await withDesign(file, async (design) => {
            const existing = design.summary.sheets.find((entry) => entry.id === sheet || entry.name === sheet);
            await design.deleteSheet(sheet);
            return {
                removed: { id: existing?.id ?? sheet, name: existing?.name ?? '' },
                saved: await savedReport(design),
            };
        });
        ctx.emitResult(
            { ...saved, revision: saved.revision, sheet: removed },
            `deleted sheet ${sheet} from ${file} revision=${saved.revision}`,
        );
        return 0;
    });

for (const [name, description] of [
    ['undo', 'cli.cmd.undo'],
    ['redo', 'cli.cmd.redo'],
] as const) {
    cli.command(name, description).actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const design = await withDesign(file, async (opened) => {
            try {
                return name === 'undo' ? await opened.undo() : await opened.redo();
            } catch (error) {
                if (error instanceof OpenEDAError && error.code === 'history/empty') {
                    return undefined;
                }
                throw error;
            }
        });
        if (design === undefined) {
            ctx.fail('history/empty', {
                message: `there is no ${name === 'undo' ? 'earlier' : 'later'} revision to restore`,
            });
            return 2;
        }
        ctx.emitResult(design.design, `${name} revision=${design.revision}`);
        return 0;
    });
}

cli
    .command('inspect', 'cli.cmd.inspect')
    .option('--checks <list>', 'cli.opt.checks')
    .option('--severity <pairs>', 'cli.opt.severity')
    .option('--waive <list>', 'cli.opt.waive')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const checks = text(options, 'checks')
            ?.split(',')
            .map((name) => name.trim())
            .filter((name) => name.length > 0) as InspectCheck[] | undefined;
        let severity: Record<string, InspectSeverity> | undefined;
        const rawSeverity = text(options, 'severity');
        if (rawSeverity !== undefined) {
            const parsed = parseSeverity(rawSeverity);
            if (!parsed.ok) {
                ctx.fail('open-eda.cli.arguments-invalid', { message: parsed.message });
                return 2;
            }
            severity = parsed.value;
        }
        let waivers: InspectWaiver[] | undefined;
        const rawWaivers = text(options, 'waive');
        if (rawWaivers !== undefined) {
            const parsed = parseWaivers(rawWaivers);
            if (!parsed.ok) {
                ctx.fail('open-eda.cli.arguments-invalid', { message: parsed.message });
                return 2;
            }
            waivers = parsed.value;
        }
        const inspectOptions: InspectOptions = { checks, severity, waivers };
        const report = await withDesign(file, (design) => design.inspect(inspectOptions));
        ctx.emitResult(report, describeReport(report));
        return report.passed ? 0 : 1;
    });

cli
    .command('export', 'cli.cmd.export')
    .option('--format <format>', 'cli.opt.format')
    .option('--out <file>', 'cli.opt.out')
    .option('--variant <name>', 'cli.opt.variant')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a design document is required' });
            return 2;
        }
        const format = asExportTarget(text(options, 'format'));
        if (format === undefined) {
            ctx.fail('open-eda.cli.format-required', {
                message: '`--format` must be one of kicad_sch, kicad_sym, kicad_pcb, net, net_csv, net_hier, spice, spice_subckt, gbr, pdf, svg, bom, bom_json, bom_html, report, json',
            });
            return 2;
        }
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-eda.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const report = await withDesign(file, (design) =>
            design.export({ format, target: out, variant: text(options, 'variant') }),
        );
        ctx.emitResult(report, `exported ${format} to ${out} revision=${report.revision}`);
        return 0;
    });

cli
    .command('import', 'cli.cmd.import')
    .option('--out <file>', 'cli.opt.out')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-eda.cli.file-required', { message: 'a KiCad schematic is required' });
            return 2;
        }
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-eda.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        if (fs.existsSync(out) && !flag(options, 'overwrite')) {
            ctx.fail('format/invalid', { message: `\`${out}\` already exists; pass --overwrite to replace it` });
            return 2;
        }
        const imported = await app.importKicadSch({ source: file, target: out });
        const design = await withDesign(out, async (opened) => opened.summary);
        ctx.emitResult(
            { path: out, design, losses: imported.losses },
            `imported ${file} -> ${out} revision=${design.revision} components=${design.counts.components} nets=${design.counts.nets} wires=${design.counts.wires} losses=${imported.losses.length}`,
        );
        return 0;
    });

cli.command('gui', 'cli.cmd.gui').actionCtx(async (_options, _args, ctx) => {
    try {
        const status = await runGui({});
        ctx.emitResult(status, `window ${status.state}`);
        return 0;
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.fail('open-eda.cli.gui-failed', { message });
        return 1;
    }
});

const entry = process.argv[1];
if (entry !== undefined && import.meta.url === pathToFileURL(entry).href) {
    try {
        process.exitCode = await cli.parse(process.argv);
    } finally {
        await app.dispose();
    }
}
