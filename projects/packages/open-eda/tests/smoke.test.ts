/**
 * Smoke tests for the authoring surface.
 *
 * They assert the shell semantics (a session that closes idempotently,
 * synchronous argument validation, a complete format registry) and, when a
 * native addon is installed, the implemented document core: editing, undo/redo,
 * sheet edits, inspection, save/open round trip and netlist/SVG/BOM/PDF export.
 */

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { designFormats, formatFromPath, formatInfo, OpenEDA, OpenEDAError } from '../src/index.js';
import type { Design, DesignInput } from '../src/index.js';
import { nativeBinding } from '../src/native.js';

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

        const audit = await design.netAudit();
        assert.equal(audit.report, 'open-eda-net-audit');
        assert.equal(audit.status, 'warn');
        assert.equal(audit.netCount, 2);
        assert.equal(audit.pinCount, 4);
        assert.equal(audit.wireCount, 1);
        assert.equal(audit.totalLength, 30);
        assert.deepEqual(
            audit.nets.map((net) => `${net.name}:${net.nodes}:${net.wires}:${net.connected}`),
            ['VCC:2:1:true', 'GND:2:0:true'],
        );
        assert.deepEqual(
            audit.issues.map((issue) => issue.code),
            ['net/no-wire'],
        );
        assert.equal((await eda.capabilities()).queries.netAudit, 'supported');

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
        const classPath = path.join(directory, 'blinker-classes.csv');
        const netClass = await reopened.export({ format: 'net_class', target: classPath });
        assert.equal(netClass.format, 'net_class');
        assert.equal(netClass.artifact.classes, 0);
        assert.equal(netClass.artifact.rows, 4);
        assert.equal(netClass.artifact.unclassifiedRows, 4);
        const classText = fs.readFileSync(classPath, 'utf8');
        assert.ok(classText.startsWith('Class,Sheet,Net,Ref,Pin,Role\n'));
        assert.ok(classText.includes(',Sheet 1,VCC,R1,1,'));
        const lengthPath = path.join(directory, 'blinker-lengths.csv');
        const netLength = await reopened.export({ format: 'net_length', target: lengthPath });
        assert.equal(netLength.format, 'net_length');
        assert.equal(netLength.artifact.kind, 'net_length');
        assert.equal(netLength.artifact.nets, 2);
        assert.equal(netLength.artifact.wires, 1);
        assert.equal(netLength.artifact.unrouted, 1);
        assert.equal(netLength.artifact.length, '30');
        assert.equal(
            fs.readFileSync(lengthPath, 'utf8'),
            'Sheet,Net,Length,Wires,Unrouted\nSheet 1,VCC,30,1,no\nSheet 1,GND,0,0,yes\n',
        );
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
        assert.ok(bomText.startsWith('Comment,Footprint,Designator,Quantity\n'));
        assert.ok(bomText.includes('100n,,C1,1'));
        assert.ok(bomText.includes('10k,,R1,1'));
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
        assert.equal(renamed.net.name, 'VIN');
        assert.equal(renamed.previousName, 'VCC');
        assert.equal(renamed.changed, true);
        assert.equal((await design.listObjects()).wires[0]?.net, 'VIN');
        const unchangedRename = await design.renameNet({ net: 'VIN', name: 'VIN' });
        assert.equal(unchangedRename.changed, false);
        assert.equal(unchangedRename.revision, renamed.revision, 'an already-matching name adds no revision');
        await assert.rejects(
            design.renameNet({ net: 'NOPE', name: 'X' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'net/not-found',
        );
        const split = await design.splitWire({ id: 'wire-1', x: 20, y: 10 });
        assert.equal(split.first.id, 'wire-1');
        assert.deepEqual(split.first.to, [20, 10]);
        assert.equal(split.second.id, 'wire-2');
        assert.deepEqual(split.second.from, [20, 10]);
        assert.deepEqual(split.second.to, [30, 10]);
        assert.equal(split.second.net, 'VIN');
        assert.equal((await design.listWires()).length, 2);
        const translated = await design.translate({ references: ['C1'], dx: 5, dy: 0 });
        assert.equal(translated.components, 1);
        assert.equal(translated.wires, 0);
        const c1 = (await design.listComponents()).find((component) => component.ref === 'C1');
        assert.equal(c1?.x, 35);
        assert.equal(c1?.y, 10);
        const movedWires = await design.translate({ wires: ['wire-2'], dx: 0, dy: 5 });
        assert.equal(movedWires.wires, 1);
        assert.deepEqual((await design.listWires())[1]?.from, [20, 15]);
        await assert.rejects(
            design.translate({ references: ['R9'], dx: 1, dy: 0 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await design.connectNet({ net: 'GND', nodes: ['R1.2', 'C1.2'] });
        const merged = await design.mergeNets({ target: 'VIN', source: 'GND' });
        assert.equal(merged.net.name, 'VIN');
        assert.equal(merged.source, 'GND');
        assert.equal(merged.movedNodes, 2);
        assert.equal(merged.repointedWires, 0);
        assert.deepEqual(
            merged.net.nodes.map((node) => `${node.ref}.${node.pin}`),
            ['R1.1', 'C1.1', 'R1.2', 'C1.2'],
        );
        assert.equal((await design.listNets()).length, 1);

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

test('pin_types check reports role conflicts the other checks leave open', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'PinTypes' });
        await design.addComponent({ reference: 'U1', value: 'buffer', x: 0, y: 0, pins: ['OUT', 'VCC', 'BUS'] });
        await design.addComponent({ reference: 'U2', value: 'regulator', x: 20, y: 0, pins: ['PWR'] });
        await design.connectNet({ net: 'SUPPLY_IN', nodes: ['U1.OUT', 'U1.VCC'] });
        await design.connectNet({ net: 'BUS_DRIVE', nodes: ['U2.PWR', 'U1.BUS'] });
        await design.connectNet({ net: 'CLEAN', nodes: ['U2.PWR', 'U1.VCC'] });
        await design.setPinType({ reference: 'U1', pin: 'OUT', pinType: 'output' });
        await design.setPinType({ reference: 'U1', pin: 'VCC', pinType: 'power_in' });
        await design.setPinType({ reference: 'U1', pin: 'BUS', pinType: 'tri_state' });
        await design.setPinType({ reference: 'U2', pin: 'PWR', pinType: 'power_out' });

        const report = await design.inspect({ checks: ['pin_types'] });
        const codes = report.issues.map((issue) => issue.code);
        assert.deepEqual(codes, ['erc/output-power-in-conflict', 'erc/power-output-driver-conflict']);
        assert.equal(report.passed, false);
        assert.ok(report.issues.every((issue) => issue.severity === 'error'));
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('drivers check reports undriven inputs and open power pins', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Drivers' });
        await design.addComponent({ reference: 'U1', value: 'gate', x: 0, y: 0, pins: ['1', '2'] });
        await design.addComponent({ reference: 'U2', value: 'gate', x: 20, y: 0, pins: ['1', '2'] });
        await design.connectNet({ net: 'SIGNAL', nodes: ['U1.1', 'U2.1'] });
        await design.connectNet({ net: 'VCC', nodes: ['U1.2', 'U2.2'] });
        await design.setPinType({ reference: 'U1', pin: '1', pinType: 'input' });
        await design.setPinType({ reference: 'U1', pin: '2', pinType: 'power_in' });

        const undriven = await design.inspect({ checks: ['drivers'] });
        assert.ok(undriven.issues.some((issue) => issue.code === 'erc/undriven-input'));
        assert.ok(!undriven.issues.some((issue) => issue.code === 'erc/unconnected-power-pin'));
        assert.equal(undriven.passed, false);

        await design.setPinType({ reference: 'U2', pin: '1', pinType: 'output' });
        const driven = await design.inspect({ checks: ['drivers'] });
        assert.ok(!driven.issues.some((issue) => issue.code === 'erc/undriven-input'));
        assert.equal(driven.passed, true);
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('component custom properties persist and stay addressable', { skip: withoutAddon }, async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-component-properties-'));
    const document = path.join(directory, 'properties.oeda');
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Properties' });
        await design.addComponent({ reference: 'R1', value: '10k', x: 0, y: 0, pins: ['1', '2'] });
        const first = await design.setComponentProperty({ reference: 'R1', key: 'MPN', value: 'ABC-123' });
        assert.equal(first.key, 'MPN');
        assert.equal(first.value, 'ABC-123');
        assert.equal(first.component.properties?.MPN, 'ABC-123');
        const unchanged = await design.setComponentProperty({ reference: 'R1', key: 'MPN', value: 'ABC-123' });
        assert.equal(unchanged.revision, first.revision);
        await design.setComponentProperty({ reference: 'R1', key: 'Description', value: '10k resistor' });
        const listed = await design.listComponentProperties('R1');
        assert.deepEqual(listed.properties, [
            { key: 'Description', value: '10k resistor' },
            { key: 'MPN', value: 'ABC-123' },
        ]);
        await assert.rejects(
            design.setComponentProperty({ reference: 'R1', key: 'Bad Key', value: 'x' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await assert.rejects(
            design.setComponentProperty({ reference: 'R9', key: 'MPN', value: 'x' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await design.save({ target: document, overwrite: true });
        await design.close();

        const reopened = await eda.open({ path: document });
        const persisted = await reopened.listComponentProperties('R1');
        assert.equal(persisted.properties.find((property) => property.key === 'MPN')?.value, 'ABC-123');
        const removed = await reopened.removeComponentProperty({ reference: 'R1', key: 'Description' });
        assert.equal(removed.key, 'Description');
        assert.equal(removed.component.properties?.Description, undefined);
        await assert.rejects(
            reopened.removeComponentProperty({ reference: 'R1', key: 'Description' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await reopened.close();
    } finally {
        fs.rmSync(directory, { recursive: true, force: true });
        await eda.dispose();
    }
});

test('hierarchical sheet symbols and ports persist', { skip: withoutAddon }, async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-hierarchy-'));
    const document = path.join(directory, 'hierarchy.oeda');
    const eda = new OpenEDA();
    try {
        const design = await eda.create({ title: 'Hierarchy' });
        const added = await design.addHierarchy({ name: 'Power', file: 'power.kicad_sch', x: 10, y: 20 });
        assert.equal(added.hierarchy.id, 'hierarchy-1');
        assert.equal(added.hierarchy.width, 60);
        assert.deepEqual(added.hierarchy.ports ?? [], []);
        await assert.rejects(
            design.addHierarchy({ name: 'Power', file: 'other.kicad_sch' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/duplicate',
        );
        const port = await design.addHierarchyPort({
            sheet: 'Power',
            name: 'VIN',
            direction: 'input',
            x: 10,
            y: 24,
        });
        assert.equal(port.port.direction, 'input');
        await assert.rejects(
            design.addHierarchyPort({ sheet: 'Power', name: 'VIN' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/duplicate',
        );
        await assert.rejects(
            design.addHierarchyPort({ sheet: 'Missing', name: 'GND' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await design.save({ target: document, overwrite: true });
        await design.close();

        const reopened = await eda.open({ path: document });
        const listed = await reopened.listHierarchy();
        assert.equal(listed.count, 1);
        assert.equal(listed.hierarchy[0]?.ports?.[0]?.name, 'VIN');
        const removedPort = await reopened.removeHierarchyPort({ sheet: 'hierarchy-1', name: 'VIN' });
        assert.equal(removedPort.removed, 'VIN');
        await reopened.removeHierarchy('Power');
        assert.equal((await reopened.listHierarchy()).count, 0);
        await assert.rejects(
            reopened.removeHierarchy('Power'),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await reopened.close();
    } finally {
        fs.rmSync(directory, { recursive: true, force: true });
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

test('component copy duplicates placement and metadata without net nodes', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create();
        await design.addComponent({ reference: 'R1', value: '10k', x: 10, y: 20, rotation: 90, pins: ['1', '2'] });
        await design.setFootprint({ reference: 'R1', footprint: 'R_0805' });
        await design.setPinType({ reference: 'R1', pin: '1', pinType: 'passive' });
        await design.setComponentProperty({ reference: 'R1', key: 'MPN', value: 'ABC-123' });
        await design.setReferencePrefix({ class: 'resistor', prefix: 'R' });
        await design.setComponentClass({ reference: 'R1', class: 'resistor' });
        await design.connectNet({ net: 'VCC', nodes: ['R1.1'] });

        const revision = design.revision;
        const copy = await design.copyComponent({ reference: 'R1', newReference: 'R2', dx: 15, dy: -5 });
        assert.equal(copy.id, 'component-2');
        assert.equal(copy.ref, 'R2');
        assert.equal(copy.value, '10k');
        assert.equal(copy.footprint, 'R_0805');
        assert.equal(copy.x, 25);
        assert.equal(copy.y, 15);
        assert.equal(copy.rotation, 90);
        assert.deepEqual(copy.pins, ['1', '2']);
        assert.equal(copy.pinTypes?.['1'], 'passive');
        assert.equal(copy.properties?.MPN, 'ABC-123');
        assert.equal(design.revision, revision + 1);
        assert.equal((await design.listNets()).length, 1);
        assert.equal((await design.listNets())[0]?.nodes.length, 1);
        const classes = await design.listComponentClasses();
        assert.ok(classes.some((entry) => entry.reference === 'R2' && entry.class === 'resistor'));

        await assert.rejects(
            design.copyComponent({ reference: 'R1', newReference: 'R2' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/duplicate',
        );
        await assert.rejects(
            design.copyComponent({ reference: 'R9', newReference: 'R3' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await assert.rejects(
            design.copyComponent({ reference: 'R1', newReference: '  ' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await assert.rejects(
            design.copyComponent({ reference: 'R1', newReference: 'R3', dx: Number.NaN }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await design.undo();
        assert.equal((await design.listComponents()).length, 1);
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('text list and update edit in place across undo', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create();
        const first = await design.addText({ text: 'NOTE', x: 5, y: 6 });
        assert.equal(first.text, 'NOTE');
        const second = await design.addText({ text: 'SECOND', x: 7, y: 8 });
        assert.equal(second.text, 'SECOND');
        const listed = await design.listTexts();
        assert.equal(listed.sheetId, 'sheet-1');
        assert.deepEqual(
            listed.texts.map((entry) => entry.index),
            [0, 1],
        );
        assert.deepEqual(
            listed.texts.map((entry) => entry.text),
            ['NOTE', 'SECOND'],
        );

        const textRevision = design.revision;
        const updated = await design.updateText({ index: 0, text: 'UPDATED', x: 11 });
        assert.equal(updated.index, 0);
        assert.equal(updated.text.text, 'UPDATED');
        assert.equal(updated.text.x, 11);
        assert.equal(updated.text.y, 6);
        assert.equal(design.revision, textRevision + 1);
        assert.equal((await design.listTexts()).texts[0]?.text, 'UPDATED');

        await assert.rejects(
            design.updateText({ index: 0, text: '   ' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await assert.rejects(
            design.updateText({ index: 0 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await assert.rejects(
            design.updateText({ index: 0, x: Number.NaN }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await assert.rejects(
            design.updateText({ index: 9, x: 1 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );
        await design.undo();
        assert.equal((await design.listTexts()).texts[0]?.text, 'NOTE');
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('net labels carry scope and stats report deterministic counts', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create();
        const local = await design.addLabel({ text: 'VCC', x: 10, y: 20 });
        assert.equal(local.id, 'label-1');
        assert.equal(local.scope, 'local');
        assert.equal(local.rotation, 0);
        assert.equal(local.justify, 'left');
        await design.addLabel({ text: 'GND', x: 30, y: 40, scope: 'global', rotation: 90, justify: 'right' });

        let listed = await design.listLabels();
        assert.equal(listed.sheetId, 'sheet-1');
        assert.deepEqual(
            listed.labels.map((label) => `${label.id}:${label.text}:${label.scope}`),
            ['label-1:VCC:local', 'label-2:GND:global'],
        );

        const updated = await design.updateLabel({ id: 'label-1', text: 'VDD', scope: 'hierarchical', x: 11 });
        assert.equal(updated.label.text, 'VDD');
        assert.equal(updated.label.scope, 'hierarchical');
        assert.equal(updated.label.x, 11);
        assert.equal(updated.label.y, 20);

        await assert.rejects(
            design.addLabel({ text: '  ', x: 0, y: 0 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'session/argument-invalid',
        );
        await assert.rejects(
            design.updateLabel({ id: 'label-9', text: 'X' }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'object/not-found',
        );

        await design.addJunction({ x: 1, y: 2 });
        await design.addNoConnect({ x: 3, y: 4 });
        const junctions = await design.listJunctions();
        assert.deepEqual(junctions.junctions, [{ index: 0, x: 1, y: 2 }]);
        const noConnects = await design.listNoConnects();
        assert.deepEqual(noConnects.noConnects, [{ index: 0, x: 3, y: 4 }]);
        const sheets = await design.listSheets();
        assert.equal(sheets.sheets[0]?.labels, 2);

        const stats = await design.stats();
        assert.equal(stats.format, 'open-eda-stats');
        assert.equal(stats.totals.sheets, 1);
        assert.equal(stats.totals.labels, 2);
        assert.equal(stats.totals.junctions, 1);
        assert.equal(stats.totals.noConnects, 1);
        assert.equal(stats.totals.buses, 0);
        assert.equal(stats.totals.bomRows, 0);

        await design.removeLabel('label-2');
        listed = await design.listLabels();
        assert.equal(listed.labels.length, 1);
        assert.equal(listed.labels[0]?.id, 'label-1');

        await design.undo();
        assert.equal((await design.listLabels()).labels.length, 2);
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('junction and no-connect edits refuse duplicates and track ERC', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-annotations-'));
    const target = path.join(directory, 'annotations.oeda');
    try {
        const design = await eda.create();
        await design.addComponent({ reference: 'R1', value: '10k', x: 40, y: 30, pins: ['1', '2'] });
        await design.connectNet({ net: 'SIGNAL', nodes: ['R1.1'] });

        const unconnected = async (open: Design): Promise<number> => {
            const report = await open.inspect({ checks: ['erc'] });
            return report.issues.filter((issue) => issue.code === 'erc/unconnected-pin').length;
        };
        assert.equal(await unconnected(design), 1);

        await design.addJunction({ x: 10, y: 20 });
        await assert.rejects(
            design.addJunction({ x: 10, y: 20 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'junction/duplicate',
        );
        await design.addJunction({ x: 30, y: 40 });
        await assert.rejects(
            design.addJunction({ sheet: 'missing', x: 1, y: 2 }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'sheet/not-found',
        );
        await design.deleteJunction(0);
        assert.deepEqual((await design.listJunctions()).junctions, [{ index: 0, x: 30, y: 40 }]);
        await assert.rejects(
            design.deleteJunction(1),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'junction/not-found',
        );

        await design.addNoConnect({ x: 40, y: 30, node: { ref: 'R1', pin: '2' } });
        assert.equal(await unconnected(design), 0);
        await design.addNoConnect({ x: 0, y: 0 });
        assert.equal(await unconnected(design), 0);
        await assert.rejects(
            design.addNoConnect({ x: 40, y: 30, node: { ref: 'R1', pin: '2' } }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'no-connect/duplicate',
        );
        await assert.rejects(
            design.addNoConnect({ x: 40, y: 30, node: { ref: 'R1', pin: '9' } }),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'no-connect/pin-missing',
        );

        await design.save({ target, overwrite: true });
        await design.close();

        const reopened = await eda.open({ path: target });
        assert.deepEqual((await reopened.listJunctions()).junctions, [{ index: 0, x: 30, y: 40 }]);
        const flags = (await reopened.listNoConnects()).noConnects;
        assert.equal(flags.length, 2);
        assert.deepEqual(flags[0]?.node, { ref: 'R1', pin: '2' });
        assert.equal(flags[1]?.node, undefined);
        await reopened.deleteNoConnect(0);
        assert.equal((await reopened.listNoConnects()).noConnects.length, 1);
        await assert.rejects(
            reopened.deleteNoConnect(1),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'no-connect/not-found',
        );
        assert.equal(await unconnected(reopened), 1);
        await reopened.close();
    } finally {
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('junction and no-connect commands edit a document in place', { skip: withoutAddon }, () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-cli-annotations-'));
    const target = path.join(directory, 'cli.oeda');
    const testDirectory = path.dirname(fileURLToPath(import.meta.url));
    const cli = fileURLToPath(new URL('../src/cli.js', import.meta.url));
    // The test build keeps `src` under `dist-test`, so the CLI looks for
    // `dist-test/locales`; mirror the package locale files for this run only.
    const generatedLocales = path.resolve(testDirectory, '../locales');
    const sourceLocales = path.resolve(testDirectory, '../../locales');
    const copiedLocales = !fs.existsSync(generatedLocales);
    if (copiedLocales) {
        fs.cpSync(sourceLocales, generatedLocales, { recursive: true });
    }
    const run = (...args: string[]) => spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8' });
    try {
        assert.equal(run('create', '--out', target).status, 0);

        let result = run('junction', 'add', target, '--x', '55', '--y', '30');
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /added junction at \(55, 30\)/);
        result = run('junction', 'add', target, '--x', '55', '--y', '30');
        assert.equal(result.status, 1);
        assert.match(result.stdout + result.stderr, /a junction already exists at \(55, 30\)/);

        result = run('junction', 'delete', target, '--index', '0');
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /deleted junction 0/);
        result = run('junction', 'list', target);
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /count=0/);

        result = run('component', 'add', target, '--ref', 'R1', '--value', '10k', '--x', '40', '--y', '30', '--pins', '1,2');
        assert.equal(result.status, 0, result.stderr);
        result = run('no-connect', 'add', target, '--x', '40', '--y', '30', '--ref', 'R1', '--pin', '2');
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /added no-connect at \(40, 30\)/);
        result = run('no-connect', 'list', target);
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /R1\.2/);

        result = run('no-connect', 'delete', target, '--index', '0');
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /deleted no-connect 0/);
        result = run('no-connect', 'list', target);
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /count=0/);
    } finally {
        if (copiedLocales) {
            fs.rmSync(generatedLocales, { recursive: true, force: true });
        }
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('hierarchy check reads child documents without writing them', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-hierarchy-'));
    const child = path.join(directory, 'child.oeda');
    const parent = path.join(directory, 'parent.oeda');
    try {
        const childDesign = await eda.create();
        await childDesign.addLabel({ text: 'IN', x: 0, y: 0, scope: 'hierarchical' });
        await childDesign.addLabel({ text: 'EXTRA', x: 1, y: 1, scope: 'hierarchical' });
        await childDesign.save({ target: child, overwrite: true });
        await childDesign.close();

        const parentDesign = await eda.create();
        await parentDesign.addHierarchy({ name: 'Child', file: 'child.oeda' });
        await parentDesign.addHierarchyPort({ sheet: 'Child', name: 'IN', direction: 'input' });
        await parentDesign.addHierarchyPort({ sheet: 'Child', name: 'MISSING', direction: 'input' });
        await parentDesign.save({ target: parent, overwrite: true });
        await parentDesign.close();

        const before = fs.readFileSync(parent, 'utf8');
        const opened = await eda.open({ path: parent });
        const report = await opened.checkHierarchy();
        assert.equal(report.format, 'open-eda-hierarchy');
        assert.equal(report.version, 1);
        assert.equal(report.sheets, 1);
        assert.equal(report.passed, false);
        assert.deepEqual(
            report.issues.map((issue) => issue.code),
            ['hierarchy/port-without-label', 'hierarchy/label-without-port'],
        );
        assert.equal(fs.readFileSync(parent, 'utf8'), before);
        await opened.close();

        const unsaved = await eda.create();
        await assert.rejects(
            unsaved.checkHierarchy(),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'hierarchy/document-path-missing',
        );
        await unsaved.close();
    } finally {
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('hierarchy check classifies children and stops at cycles', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-hierarchy-children-'));
    try {
        const second = await eda.create();
        await second.addHierarchy({ name: 'A', file: 'a.oeda' });
        await second.save({ target: path.join(directory, 'b.oeda'), overwrite: true });
        await second.close();

        const first = await eda.create();
        await first.addHierarchy({ name: 'B', file: 'b.oeda' });
        await first.addHierarchy({ name: 'Text', file: 'notes.txt' });
        await first.addHierarchy({ name: 'Gone', file: 'missing.oeda' });
        await first.save({ target: path.join(directory, 'a.oeda'), overwrite: true });
        await first.close();

        const opened = await eda.open({ path: path.join(directory, 'a.oeda') });
        const report = await opened.checkHierarchy();
        assert.equal(report.sheets, 1);
        assert.deepEqual(
            report.issues.map((issue) => issue.code),
            ['hierarchy/cycle', 'hierarchy/child-format-unsupported', 'hierarchy/file-missing'],
        );
        assert.equal(report.passed, false);
        await opened.close();
    } finally {
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('hierarchy check command reports a document without changing it', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-cli-hierarchy-'));
    const target = path.join(directory, 'parent.oeda');
    const testDirectory = path.dirname(fileURLToPath(import.meta.url));
    const cli = fileURLToPath(new URL('../src/cli.js', import.meta.url));
    const generatedLocales = path.resolve(testDirectory, '../locales');
    const sourceLocales = path.resolve(testDirectory, '../../locales');
    const copiedLocales = !fs.existsSync(generatedLocales);
    if (copiedLocales) {
        fs.cpSync(sourceLocales, generatedLocales, { recursive: true });
    }
    try {
        const childDesign = await eda.create();
        await childDesign.addLabel({ text: 'IN', x: 0, y: 0, scope: 'hierarchical' });
        await childDesign.save({ target: path.join(directory, 'child.oeda'), overwrite: true });
        await childDesign.close();

        const parentDesign = await eda.create();
        await parentDesign.addHierarchy({ name: 'Child', file: 'child.oeda' });
        await parentDesign.addHierarchyPort({ sheet: 'Child', name: 'IN', direction: 'input' });
        await parentDesign.save({ target, overwrite: true });
        await parentDesign.close();

        const before = fs.readFileSync(target, 'utf8');
        const result = spawnSync(process.execPath, [cli, 'hierarchy', 'check', target], { encoding: 'utf8' });
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /hierarchy sheets=1 issues=0 passed=true/);
        assert.equal(fs.readFileSync(target, 'utf8'), before);
    } finally {
        if (copiedLocales) {
            fs.rmSync(generatedLocales, { recursive: true, force: true });
        }
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('library and library-check read and validate a kicad_sym file', { skip: withoutAddon }, () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-library-'));
    const clean = path.join(directory, 'clean.kicad_sym');
    const broken = path.join(directory, 'broken.kicad_sym');
    const testDirectory = path.dirname(fileURLToPath(import.meta.url));
    const cli = fileURLToPath(new URL('../src/cli.js', import.meta.url));
    const generatedLocales = path.resolve(testDirectory, '../locales');
    const sourceLocales = path.resolve(testDirectory, '../../locales');
    const copiedLocales = !fs.existsSync(generatedLocales);
    if (copiedLocales) {
        fs.cpSync(sourceLocales, generatedLocales, { recursive: true });
    }
    const library = (pins: string) => `(kicad_symbol_lib (version 20211014) (generator kicad_symbol_editor)
  (symbol "GATE"
    (symbol "GATE_1_1"
${pins})))
`;
    fs.writeFileSync(
        clean,
        library(`      (pin input line (at 0 0 0) (length 2.54)
        (name "A" (effects (font (size 1.27 1.27))))
        (number "1" (effects (font (size 1.27 1.27)))))
      (pin output line (at 0 0 0) (length 2.54)
        (name "Y" (effects (font (size 1.27 1.27))))
        (number "2" (effects (font (size 1.27 1.27)))))`),
    );
    fs.writeFileSync(
        broken,
        library(`      (pin input line (at 0 0 0) (length 2.54)
        (name "A" (effects (font (size 1.27 1.27))))
        (number "1" (effects (font (size 1.27 1.27)))))
      (pin input line (at 0 0 0) (length 2.54)
        (name "B" (effects (font (size 1.27 1.27))))
        (number "1" (effects (font (size 1.27 1.27)))))`),
    );
    const run = (...args: string[]) => spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8' });
    try {
        let result = run('library', clean, '--pins');
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /unit 1 pin 1 A input/);
        assert.match(result.stdout, /unit 1 pin 2 Y output/);

        result = run('library-check', clean);
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /library-check symbols=1 pins=2 findings=0 passed=true/);

        result = run('library-check', broken);
        assert.equal(result.status, 1);
        assert.match(result.stdout + result.stderr, /library\/duplicate-pin-number/);

        result = run('library-check');
        assert.equal(result.status, 1);
        assert.match(result.stdout + result.stderr, /open-eda\.cli\.file-required/);
    } finally {
        if (copiedLocales) {
            fs.rmSync(generatedLocales, { recursive: true, force: true });
        }
        fs.rmSync(directory, { recursive: true, force: true });
    }
});

test('tracePins follows wires, junctions, labels and global scope', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    try {
        const design = await eda.create();
        await design.addComponent({ reference: 'R1', value: '10k', x: 10, y: 10, pins: ['1', '2'] });
        await design.addComponent({ reference: 'R2', value: '10k', x: 90, y: 10, pins: ['1', '2'] });
        await design.connectNet({ net: 'A', nodes: ['R1.1'] });
        await design.connectNet({ net: 'B', nodes: ['R2.1'] });
        await design.addWire({ net: 'A', from: [0, 50], to: [100, 50] });
        await design.addWire({ net: 'B', from: [50, 0], to: [50, 100] });

        const disconnected = await design.tracePins('R1.1', 'R2.1');
        assert.equal(disconnected.format, 'open-eda-trace');
        assert.equal(disconnected.version, 1);
        assert.equal(disconnected.connected, false);
        assert.equal(disconnected.via, 'none');
        assert.deepEqual(disconnected.hops, []);
        assert.deepEqual(disconnected.sheets, []);

        const direct = await design.tracePins('R1.1', 'R1.1');
        assert.equal(direct.connected, true);
        assert.equal(direct.via, 'direct');
        assert.deepEqual(direct.hops, []);

        await assert.rejects(
            design.tracePins('R9.1', 'R1.1'),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'trace/component-missing',
        );
        await assert.rejects(
            design.tracePins('R1.9', 'R1.1'),
            (error: unknown) => error instanceof OpenEDAError && error.code === 'trace/pin-missing',
        );

        await design.addJunction({ x: 50, y: 50 });
        const joined = await design.tracePins('R1.1', 'R2.1');
        assert.equal(joined.connected, true);
        assert.equal(joined.via, 'wire');
        assert.deepEqual(
            joined.hops.map((hop) => hop.type),
            ['pin', 'wire', 'junction', 'wire', 'pin'],
        );
        assert.deepEqual(joined.sheets, ['Sheet 1']);
        assert.equal(joined.hops[2]?.index, 0);

        const crossSheet = await eda.create();
        await crossSheet.addSheet('Power');
        await crossSheet.addComponent({ reference: 'C1', value: '100n', x: 10, y: 10, pins: ['1', '2'] });
        await crossSheet.addComponent({ sheet: 'Power', reference: 'C2', value: '100n', x: 10, y: 10, pins: ['1', '2'] });
        await crossSheet.connectNet({ net: 'A', nodes: ['C1.1'] });
        await crossSheet.connectNet({ sheet: 'Power', net: 'B', nodes: ['C2.1'] });
        await crossSheet.addWire({ net: 'A', from: [0, 0], to: [20, 0] });
        await crossSheet.addWire({ sheet: 'Power', net: 'B', from: [0, 0], to: [20, 0] });
        await crossSheet.addLabel({ text: 'VCC', x: 0, y: 0, scope: 'local' });
        await crossSheet.addLabel({ sheet: 'Power', text: 'VCC', x: 0, y: 0, scope: 'local' });
        const local = await crossSheet.tracePins('C1.1', 'C2.1');
        assert.equal(local.connected, false, 'a local label stays on its sheet');

        await crossSheet.addLabel({ text: 'VCC', x: 0, y: 0, scope: 'global' });
        await crossSheet.addLabel({ sheet: 'Power', text: 'VCC', x: 0, y: 0, scope: 'global' });
        const global = await crossSheet.tracePins('C1.1', 'C2.1');
        assert.equal(global.connected, true);
        assert.equal(global.via, 'label');
        assert.deepEqual(global.sheets, ['Sheet 1', 'Power']);
        assert.ok(global.hops.some((hop) => hop.type === 'label'));

        await crossSheet.close();
        await design.close();
    } finally {
        await eda.dispose();
    }
});

test('trace command prints a human path and the JSON envelope', { skip: withoutAddon }, async () => {
    const eda = new OpenEDA();
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'open-eda-cli-trace-'));
    const target = path.join(directory, 'trace.oeda');
    const testDirectory = path.dirname(fileURLToPath(import.meta.url));
    const cli = fileURLToPath(new URL('../src/cli.js', import.meta.url));
    const generatedLocales = path.resolve(testDirectory, '../locales');
    const sourceLocales = path.resolve(testDirectory, '../../locales');
    const copiedLocales = !fs.existsSync(generatedLocales);
    if (copiedLocales) {
        fs.cpSync(sourceLocales, generatedLocales, { recursive: true });
    }
    try {
        const design = await eda.create();
        await design.addComponent({ reference: 'R1', value: '10k', x: 10, y: 10, pins: ['1', '2'] });
        await design.addComponent({ reference: 'R2', value: '10k', x: 90, y: 10, pins: ['1', '2'] });
        await design.connectNet({ net: 'VCC', nodes: ['R1.1', 'R2.1'] });
        await design.addWire({ net: 'VCC', from: [0, 0], to: [20, 0] });
        await design.save({ target, overwrite: true });
        await design.close();

        const human = spawnSync(process.execPath, [cli, 'trace', target, 'R1.1', 'R2.1'], { encoding: 'utf8' });
        assert.equal(human.status, 0, human.stderr);
        assert.match(human.stdout, /trace R1\.1 -> R2\.1 connected=true via=wire hops=3/);
        assert.match(human.stdout, /\[Sheet 1\] wire #0/);

        const machine = spawnSync(process.execPath, [cli, 'trace', target, 'R1.1', 'R2.1', '--json'], {
            encoding: 'utf8',
        });
        assert.equal(machine.status, 0, machine.stderr);
        const envelope = JSON.parse(machine.stdout) as { ok: boolean; data: { format: string; via: string } };
        assert.equal(envelope.ok, true);
        assert.equal(envelope.data.format, 'open-eda-trace');
        assert.equal(envelope.data.via, 'wire');

        const missing = spawnSync(process.execPath, [cli, 'trace', target, 'R9.1', 'R2.1'], { encoding: 'utf8' });
        assert.equal(missing.status, 1);
        assert.match(missing.stdout + missing.stderr, /no component with reference `R9`/);
    } finally {
        if (copiedLocales) {
            fs.rmSync(generatedLocales, { recursive: true, force: true });
        }
        await eda.dispose();
        fs.rmSync(directory, { recursive: true, force: true });
    }
});
