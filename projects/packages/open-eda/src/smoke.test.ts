/**
 * Smoke tests for the authoring surface.
 *
 * They assert the shell semantics (a session that closes idempotently,
 * synchronous argument validation, a complete format registry) and, when a
 * native addon is installed, the implemented document core: editing, undo/redo,
 * sheet edits, inspection, save/open round trip and netlist/SVG/BOM/PDF export.
 */

import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { designFormats, formatFromPath, formatInfo, OpenEDA, OpenEDAError } from './index.js';
import type { DesignInput } from './index.js';
import { nativeBinding } from './native.js';

const installed = (() => {
    try {
        return nativeBinding() !== undefined;
    } catch {
        return false;
    }
})();
const withAddon = installed ? 'a built addon is installed' : false;
const withoutAddon = installed ? false : 'a built addon is not installed';

test('session construction is synchronous and side-effect free', async () => {
    const eda = new OpenEDA();
    assert.equal(eda.locale, 'en');
    assert.equal(eda.disposed, false);
    await eda.dispose();
    assert.equal(eda.disposed, true);
});

test('without an installed addon engine calls keep capability/not-implemented', { skip: withAddon }, async () => {
    const eda = new OpenEDA({ locale: 'zh-CN' });
    assert.equal(eda.locale, 'zh-CN');
    const calls: (() => Promise<unknown>)[] = [
        () => eda.capabilities(),
        () => eda.create({ title: 'Blinker', template: 'blank' }),
        () => eda.open({ path: 'blinker.oeda' }),
        () => eda.inspect({ path: 'blinker.oeda' }),
    ];
    for (const call of calls) {
        await assert.rejects(call, (error: unknown) => {
            assert.ok(error instanceof OpenEDAError);
            assert.equal(error.code, 'capability/not-implemented');
            assert.ok(error.details.feature);
            return true;
        });
    }
    await eda.dispose();
});

test('dispose is idempotent and closes the session', async () => {
    const eda = new OpenEDA();
    await eda.dispose();
    await eda.dispose();
    await assert.rejects(eda.capabilities(), (error: unknown) => error instanceof OpenEDAError && error.code === 'session/disposed');
});

test('dispose resolves after a rejected handle acquisition', { skip: withAddon }, async () => {
    const eda = new OpenEDA();
    await assert.rejects(eda.create({ title: 'Blinker', template: 'blank' }));
    await assert.rejects(eda.open({ path: 'blinker.oeda' }));
    await eda.dispose();
    await eda.dispose();
    assert.equal(eda.disposed, true);
});

test('open rejects ambiguous input before reaching the engine', async () => {
    const eda = new OpenEDA();
    const inputs = [
        {} as unknown as DesignInput,
        { path: 'a.oeda', project: '{}' } as unknown as DesignInput,
        { project: 5 } as unknown as DesignInput,
        { path: '' } as unknown as DesignInput,
    ];
    for (const input of inputs) {
        await assert.rejects(
            eda.open(input),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
    }
    await eda.dispose();
});

test('document core: edit, inspect, save/open round trip and projections', { skip: withoutAddon }, async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-sdk-'));
    const document = path.join(directory, 'blinker.oeda');
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Blinker' });
        assert.equal(design.revision, 1);
        assert.equal(design.summary.sheets.length, 1);

        const r1 = await design.addComponent({ reference: 'R1', value: '10k', x: 40, y: 30, pins: ['1', '2'] });
        assert.equal(r1.ref, 'R1');
        assert.equal(r1.id, 'component-1');
        await design.addComponent({ reference: 'C1', value: '100n', x: 70, y: 30, pins: ['1', '2'] });
        const vcc = await design.connectNet({ net: 'VCC', nodes: ['R1.1', 'C1.1'] });
        assert.equal(vcc.name, 'VCC');
        assert.equal(vcc.nodes.length, 2);
        await design.connectNet({ net: 'GND', nodes: ['R1.2', 'C1.2'] });
        await design.addWire({ net: 'VCC', from: [40, 30], to: [70, 30] });

        const report = await design.inspect();
        assert.equal(report.passed, true);
        assert.deepEqual(report.issues, []);
        assert.equal(design.revision, 6);

        const bus = await design.addBus({ name: 'DATA', width: 4 });
        assert.deepEqual(bus, { id: 'bus-1', name: 'DATA', width: 4 });
        assert.equal((await design.listBuses()).length, 1);

        const objects = await design.listObjects();
        assert.equal(objects.components.length, 2);
        assert.equal(objects.nets.length, 2);
        assert.equal(objects.wires.length, 1);
        assert.equal((await design.listComponents()).length, 2);
        assert.equal((await design.listNets()).length, 2);

        const connectivity = await design.connectivity();
        assert.equal(connectivity.format, 'open-eda-connectivity');
        assert.equal(connectivity.totals.nets, 2);
        assert.equal(connectivity.totals.nodes, 4);
        assert.deepEqual(
            connectivity.sheets[0]?.nets.find((net) => net.name === 'VCC')?.pins,
            ['R1.1', 'C1.1'],
        );
        assert.deepEqual(connectivity.sheets[0]?.unconnectedPins, []);

        const saved = await design.save({ target: document, overwrite: true });
        assert.ok(saved.bytes > 0);
        assert.ok(fs.readFileSync(document, 'utf8').includes('"open-eda-design"'));

        const reopened = await eda.open({ path: document });
        assert.equal(reopened.revision, design.revision);
        assert.equal((await reopened.listNets()).length, 2);
        assert.equal((await reopened.listBuses()).length, 1);
        const resaved = await reopened.save();
        assert.equal(resaved.revision, reopened.revision);

        const netlistPath = path.join(directory, 'blinker.net');
        const spicePath = path.join(directory, 'blinker.cir');
        const svgPath = path.join(directory, 'blinker.svg');
        const bomPath = path.join(directory, 'blinker.csv');
        const pdfPath = path.join(directory, 'blinker.pdf');
        const netlist = await reopened.export({ format: 'net', target: netlistPath });
        assert.equal(netlist.format, 'net');
        const netlistText = fs.readFileSync(netlistPath, 'utf8');
        assert.ok(netlistText.includes('NET VCC'));
        assert.ok(netlistText.includes('BUS DATA 4'));
        const csvPath = path.join(directory, 'blinker-nodes.csv');
        const netCsv = await reopened.export({ format: 'net_csv', target: csvPath });
        assert.equal(netCsv.format, 'net_csv');
        assert.equal(netCsv.artifact.rows, 4);
        const csvText = fs.readFileSync(csvPath, 'utf8');
        assert.ok(csvText.startsWith('Sheet,Net,Ref,Pin\n'));
        assert.ok(csvText.includes('Sheet 1,VCC,R1,1'));
        const spice = await reopened.export({ format: 'spice', target: spicePath });
        assert.equal(spice.format, 'spice');
        assert.ok(fs.readFileSync(spicePath, 'utf8').includes('* OpenEDA SPICE netlist v1'));
        const svg = await reopened.export({ format: 'svg', target: svgPath });
        assert.equal(svg.format, 'svg');
        assert.ok(fs.readFileSync(svgPath, 'utf8').includes('<svg'));
        const bom = await reopened.export({ format: 'bom', target: bomPath });
        assert.equal(bom.format, 'bom');
        assert.equal(bom.artifact.rows, 2);
        const bomText = fs.readFileSync(bomPath, 'utf8');
        assert.ok(bomText.startsWith('Comment,Designator,Quantity\n'));
        assert.ok(bomText.includes('100n,C1,1'));
        assert.ok(bomText.includes('10k,R1,1'));
        const pdf = await reopened.export({ format: 'pdf', target: pdfPath });
        assert.equal(pdf.format, 'pdf');
        assert.equal(pdf.artifact.mediaBox, 'a4-landscape');
        const pdfText = fs.readFileSync(pdfPath, 'utf8');
        assert.ok(pdfText.startsWith('%PDF-1.4'));
        assert.ok(pdfText.trimEnd().endsWith('%%EOF'));

        const kicadPath = path.join(directory, 'blinker.kicad_sch');
        const kicad = await reopened.export({ format: 'kicad_sch', target: kicadPath });
        assert.equal(kicad.format, 'kicad_sch');
        const kicadText = fs.readFileSync(kicadPath, 'utf8');
        assert.ok(kicadText.startsWith('(kicad_sch (version 20211123) (generator "open-eda")'));
        assert.ok(kicadText.includes('(label "VCC"'));
        assert.ok(kicadText.includes('(wire (pts (xy 40 30) (xy 70 30))'));

        const importedPath = path.join(directory, 'blinker-imported.oeda');
        const imported = await eda.importKicadSch({ source: kicadText, target: importedPath });
        assert.ok(fs.existsSync(importedPath));
        assert.deepEqual(imported.losses, []);
        assert.equal(imported.design.revision, 1);
        assert.equal(imported.design.units, 'mm');
        const importedSheet = imported.design.sheets[0];
        assert.deepEqual(importedSheet?.components.map((component) => component.ref), ['R1', 'C1']);
        assert.deepEqual(importedSheet?.components.map((component) => component.value), ['10k', '100n']);
        assert.deepEqual(importedSheet?.components[0]?.pins, ['1', '2']);
        assert.deepEqual(importedSheet?.wires[0]?.from, [40, 30]);
        assert.deepEqual(importedSheet?.wires[0]?.to, [70, 30]);
        assert.deepEqual(importedSheet?.nets.map((net) => net.name), ['VCC', 'GND']);

        const junction = await design.addJunction({ x: 55, y: 30 });
        assert.equal(junction.x, 55);
        const annotation = await design.addText({ text: 'Revision A', x: 20, y: 12 });
        assert.equal(annotation.text, 'Revision A');

        await design.deleteComponent('C1');
        assert.equal((await design.listComponents()).length, 1);
        await reopened.close();
        await design.close();
    } finally {
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('history, sheets and deep edits stay referentially sound', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create();
        assert.equal(design.canUndo, false);
        await assert.rejects(
            design.undo(),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'history/empty',
        );

        await design.addComponent({ reference: 'R1', value: '10k', x: 10, y: 10, pins: ['1', '2'] });
        await design.addComponent({ reference: 'C1', value: '100n', x: 30, y: 10, pins: ['1', '2'] });
        await design.connectNet({ net: 'VCC', nodes: ['R1.1', 'C1.1'] });
        await design.addWire({ net: 'VCC', from: [10, 10], to: [30, 10] });
        assert.equal(design.canUndo, true);
        assert.equal(design.summary.history.limit, 32);

        const undone = await design.undo();
        assert.equal(undone.canRedo, true);
        assert.equal(design.summary.counts.wires, 0);
        const redone = await design.redo();
        assert.equal(redone.canRedo, false);
        assert.equal(design.summary.counts.wires, 1);

        const moved = await design.moveComponent({ reference: 'R1', x: 50, y: 60, rotation: 90 });
        assert.equal(moved.x, 50);
        assert.equal(moved.rotation, 90);
        const updated = await design.updateComponent({ reference: 'R1', value: '22k' });
        assert.equal(updated.value, '22k');
        const renamed = await design.renameNet({ net: 'VCC', name: 'VIN' });
        assert.equal(renamed.name, 'VIN');
        assert.equal((await design.listObjects()).wires[0]?.net, 'VIN');
        const split = await design.splitWire({ id: 'wire-1', x: 20, y: 10 });
        assert.equal(split.first.id, 'wire-1');
        assert.deepEqual(split.first.to, [20, 10]);
        assert.equal(split.second.id, 'wire-2');
        assert.deepEqual(split.second.from, [20, 10]);
        assert.deepEqual(split.second.to, [30, 10]);
        assert.equal(split.second.net, 'VIN');
        assert.equal((await design.listWires()).length, 2);

        const bus = await design.addBus({ name: 'DATA', width: 4 });
        assert.deepEqual(bus, { id: 'bus-1', name: 'DATA', width: 4 });
        assert.deepEqual(await design.listBuses(), [bus]);
        await assert.rejects(
            design.addBus({ name: 'DATA', width: 2 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/duplicate',
        );
        await design.removeBus('DATA');
        assert.deepEqual(await design.listBuses(), []);
        await assert.rejects(
            design.removeBus('DATA'),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );

        const sheet = await design.addSheet('Power');
        assert.equal(sheet.id, 'sheet-2');
        assert.equal(design.summary.sheets[1]?.name, 'Power');
        const renamedSheet = await design.renameSheet('sheet-2', 'Main');
        assert.equal(renamedSheet.name, 'Main');
        await assert.rejects(
            design.deleteSheet('sheet-1'),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'sheet/not-empty',
        );
        await design.deleteSheet('sheet-2');
        await assert.rejects(
            design.deleteSheet('sheet-1'),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'sheet/last-sheet',
        );

        await design.deleteWire('wire-2');
        await design.deleteWire('wire-1');
        assert.equal(design.summary.counts.wires, 0);
        await design.deleteComponent('C1');
        assert.equal((await design.inspect()).passed, true);
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('sidecar history persists undo and redo across sessions', { skip: withoutAddon }, async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-sidecar-'));
    const document = path.join(directory, 'sidecar.oeda');
    const history = `${document}.history.json`;
    try {
        const first = new OpenEDA({ history: 'sidecar' });
        assert.equal(first.history, 'sidecar');
        const design = await first.create({ title: 'Sidecar' });
        await design.save({ target: document, overwrite: true });
        assert.equal(fs.existsSync(history), false);
        await design.addComponent({ reference: 'R1', value: '10k', x: 1, y: 1, pins: ['1', '2'] });
        assert.equal(design.summary.counts.components, 1);
        assert.equal(design.canUndo, true);
        assert.ok(fs.existsSync(history));
        assert.ok(fs.readFileSync(document, 'utf8').includes('"R1"'));
        await design.close();
        await first.dispose();

        const second = new OpenEDA({ history: 'sidecar' });
        const reopened = await second.open({ path: document });
        assert.equal(reopened.canUndo, true);
        assert.equal(reopened.summary.counts.components, 1);
        const undone = await reopened.undo();
        assert.equal(undone.design.counts.components, 0);
        assert.equal(undone.canRedo, true);
        assert.equal(reopened.revision, undone.revision);
        assert.equal((await reopened.listComponents()).length, 0);
        assert.equal(fs.readFileSync(document, 'utf8').includes('"R1"'), false);
        const redone = await reopened.redo();
        assert.equal(redone.design.counts.components, 1);
        assert.equal(redone.revision, undone.revision + 1);
        assert.equal((await reopened.listComponents()).length, 1);
        await reopened.close();
        await second.dispose();

        const third = new OpenEDA({ history: 'sidecar' });
        const fresh = await third.create();
        await fresh.save({ target: document, overwrite: true });
        assert.equal(fs.existsSync(history), false);
        await fresh.close();
        await third.dispose();
    } finally {
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('pin electrical types drive the electrical check', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Electrical' });
        await design.addComponent({ reference: 'U1', value: 'gate', x: 0, y: 0, pins: ['1', '2'] });
        await design.addComponent({ reference: 'U2', value: 'gate', x: 20, y: 0, pins: ['1', '2'] });
        await design.connectNet({ net: 'SIGNAL', nodes: ['U1.1', 'U2.1'] });
        await design.connectNet({ net: 'VCC', nodes: ['U1.2', 'U2.2'] });
        const typed = await design.setPinType({ reference: 'U1', pin: '1', pinType: 'output' });
        assert.equal(typed.pinType, 'output');
        assert.equal(typed.design.revision, 6);
        await design.setPinType({ reference: 'U2', pin: '1', pinType: 'output' });
        const conflict = await design.inspect({ checks: ['electrical'] });
        assert.ok(conflict.issues.some((issue) => issue.code === 'erc/output-output-conflict'));
        await design.setPinType({ reference: 'U2', pin: '1', pinType: 'input' });
        await design.setPinType({ reference: 'U1', pin: '2', pinType: 'power_in' });
        const unpowered = await design.inspect({ checks: ['electrical'] });
        assert.ok(unpowered.issues.some((issue) => issue.code === 'erc/power-in-unpowered'));
        await design.setPinType({ reference: 'U2', pin: '2', pinType: 'power_out' });
        await design.setPinType({ reference: 'U1', pin: '1', pinType: 'input' });
        const inputs = await design.inspect({ checks: ['electrical'] });
        assert.ok(inputs.issues.some((issue) => issue.code === 'erc/input-input-conflict'));
        assert.ok(!inputs.issues.some((issue) => issue.code === 'erc/power-in-unpowered'));
        const cleared = await design.setPinType({ reference: 'U1', pin: '1', pinType: null });
        assert.equal(cleared.pinType, null);
        const after = await design.inspect({ checks: ['electrical'] });
        assert.ok(!after.issues.some((issue) => issue.code === 'erc/input-input-conflict'));
        await assert.rejects(
            design.setPinType({ reference: 'U1', pin: '9', pinType: 'output' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await assert.rejects(
            design.setPinType({ reference: 'U1', pin: '1', pinType: 'bogus' as never }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('drawing properties update, persist and reach the KiCad title block', { skip: withoutAddon }, async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-properties-'));
    const document = path.join(directory, 'title.oeda');
    const kicad = path.join(directory, 'title.kicad_sch');
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Blinker', rev: 'A', company: 'Acme', date: '2026-11-22' });
        assert.equal(design.summary.rev, 'A');
        assert.equal(design.summary.company, 'Acme');
        assert.equal(design.summary.date, '2026-11-22');
        const updated = await design.setProperties({ title: 'Blinker II', rev: 'B' });
        assert.equal(updated.title, 'Blinker II');
        assert.equal(updated.rev, 'B');
        assert.equal(updated.company, 'Acme');
        assert.equal(updated.design.revision, 2);
        assert.equal(updated.canUndo, true);
        await design.save({ target: document, overwrite: true });
        const exported = await design.export({ format: 'kicad_sch', target: kicad });
        assert.equal(exported.format, 'kicad_sch');
        const text = fs.readFileSync(kicad, 'utf8');
        assert.ok(text.includes('(title_block'));
        assert.ok(text.includes('(title "Blinker II")'));
        assert.ok(text.includes('(rev "B")'));
        assert.ok(text.includes('(company "Acme")'));
        await design.close();

        const reopened = await eda.open({ path: document });
        assert.equal(reopened.summary.title, 'Blinker II');
        assert.equal(reopened.summary.rev, 'B');
        assert.equal(reopened.summary.company, 'Acme');
        assert.equal(reopened.summary.date, '2026-11-22');
        const cleared = await reopened.setProperties({ title: '', company: '' });
        assert.equal(cleared.title, null);
        assert.equal(cleared.company, '');
        await reopened.close();
    } finally {
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('diff compares two documents without opening them for editing', { skip: withoutAddon }, async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-diff-'));
    const left = path.join(directory, 'left.oeda');
    const right = path.join(directory, 'right.oeda');
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Blinker' });
        await design.addComponent({ reference: 'R1', value: '10k', x: 10, y: 20, pins: ['1', '2'] });
        await design.save({ target: left, overwrite: true });
        await design.updateComponent({ reference: 'R1', value: '22k' });
        await design.addComponent({ reference: 'C1', value: '100n', x: 30, y: 20, pins: ['1', '2'] });
        await design.save({ target: right, overwrite: true });
        await design.close();

        const report = await eda.diff({ left, right });
        assert.equal(report.format, 'open-eda-diff');
        assert.equal(report.equal, false);
        assert.deepEqual(report.summary.components, { added: 1, removed: 0, modified: 1 });
        assert.equal(report.summary.sheets.modified, 1);
        assert.equal(report.sheets[0]?.status, 'modified');
        assert.equal(report.sheets[0]?.components.added[0]?.ref, 'C1');
        assert.equal(report.sheets[0]?.components.modified[0]?.ref, 'R1');
        assert.equal(report.left.path, left);
        assert.equal(report.right.path, right);

        const unchanged = await eda.diff({ left, right: left });
        assert.equal(unchanged.equal, true);
        assert.deepEqual(unchanged.sheets, []);
        await assert.rejects(
            eda.diff({ left: '', right }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
    } finally {
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('inspection reports findings and refuses unsupported checks', { skip: withoutAddon }, async () => {    const eda = new OpenEDA();
    try {
        const design = await eda.create();
        await design.addComponent({ reference: 'R1', value: '', x: 0, y: 0, pins: ['1', '2'] });
        await design.connectNet({ net: 'SIG', nodes: ['R1.1'] });
        const report = await design.inspect();
        const codes = report.issues.map((issue) => issue.code);
        assert.ok(codes.includes('net-fewer-than-two-nodes'));
        assert.ok(codes.includes('component-empty-value'));
        assert.ok(codes.includes('unreferenced-pin'));
        assert.deepEqual(report.waived, []);
        const waived = await design.inspect({ waivers: [{ code: 'unreferenced-pin', objectId: 'component-1' }] });
        assert.ok(waived.waived.some((issue) => issue.code === 'unreferenced-pin'));
        assert.equal(waived.waived[0]?.waived, true);
        assert.ok(!waived.issues.some((issue) => issue.code === 'unreferenced-pin'));
        assert.equal(waived.counts.waived, 1);
        const silenced = await design.inspect({ severity: { 'component-empty-value': 'off' } });
        assert.ok(!silenced.issues.some((issue) => issue.code === 'component-empty-value'));
        const promoted = await design.inspect({ severity: { 'component-empty-value': 'error' } });
        assert.equal(promoted.issues.find((issue) => issue.code === 'component-empty-value')?.severity, 'error');
        await assert.rejects(
            design.inspect({ checks: ['drc'] }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'capability/not-implemented',
        );
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('format registry classifies the design format family', () => {
    const expected = [
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
    ];
    assert.deepEqual([...designFormats].sort(), [...expected].sort());

    assert.equal(formatInfo('kicad_sch').family, 'schematic');
    assert.equal(formatInfo('kicad_sch').opensAs, 'design');
    assert.equal(formatInfo('kicad_pcb').family, 'board');
    assert.equal(formatInfo('kicad_pcb').defaultIntent, 'edit');
    assert.equal(formatInfo('sch').opensAs, 'none');
    assert.equal(formatInfo('sch').defaultIntent, null);
    assert.equal(formatInfo('net').opensAs, 'netlist');
    assert.equal(formatInfo('net').defaultIntent, 'exchange');
    assert.equal(formatInfo('edif').markup, 'sexpr');
    assert.equal(formatInfo('spice').family, 'simulation');
    assert.equal(formatInfo('gbr').family, 'fabrication');
    assert.equal(formatInfo('step').markup, 'step');
    assert.equal(formatInfo('png').macroEnabled, false);

    assert.equal(formatFromPath('BLINKER.KICAD_PCB'), 'kicad_pcb');
    assert.equal(formatFromPath('blinker.oeda'), undefined);
    assert.equal(formatFromPath('no-extension'), undefined);
});
