# @wae/open-eda

Edit, inspect and export one editable electronics schematic document from
TypeScript or the command line.

```bash
pnpm add @wae/open-eda
```

## What you get

A `.oeda` document holds schematic sheets, components, nets, buses, wires,
junctions, net labels, no-connect flags, text, hierarchical sheet symbols and
the title block in one revisioned file. The SDK opens that file, edits it
in place and writes deterministic netlists, BOMs, previews and KiCad schematics.

The package exposes `OpenEDA` and `Design` handles plus the design types, stable
error codes and the format registry. It requires a matching native addon from
`@wae/open-eda-<platform>-<arch>`; without one, engine calls report
`capability/not-implemented`.

## Quick start

```ts
import { OpenEDA } from '@wae/open-eda';

const eda = new OpenEDA({ history: 'sidecar' });
const design = await eda.create({ title: 'Blinker', rev: 'A', company: 'Acme' });

await design.addComponent({ reference: 'R1', value: '10k', x: 40, y: 30, pins: ['1', '2'] });
await design.addComponent({ reference: 'C1', value: '100n', x: 70, y: 30, pins: ['1', '2'] });
await design.setFootprint({ reference: 'R1', footprint: 'Resistor_SMD:R_0805_2012Metric' });
await design.connectNet({ net: 'VCC', nodes: ['R1.1', 'C1.1'] });
await design.addWire({ net: 'VCC', from: [40, 30], to: [70, 30] });
await design.addJunction({ x: 55, y: 30 });
await design.addLabel({ text: 'VCC', x: 40, y: 30, scope: 'global' });
await design.annotate();

const checks = await design.inspect({ checks: ['erc', 'connectivity'] });
const stats = await design.stats();
await design.save({ target: 'blinker.oeda', overwrite: true });
await design.export({ format: 'net', target: 'blinker.net' });
await design.export({ format: 'kicad_sch', target: 'blinker.kicad_sch' });

console.log(checks.ok, stats.totals.components);
await eda.dispose();
```

On the command line:

```bash
pnpm exec open-eda create --out blinker.oeda --title Blinker
pnpm exec open-eda component add blinker.oeda --ref R1 --value 10k --x 40 --y 30 --pins 1,2
pnpm exec open-eda net connect blinker.oeda --net VCC --nodes R1.1
pnpm exec open-eda inspect blinker.oeda --checks erc,connectivity
pnpm exec open-eda export blinker.oeda --format svg --out blinker.svg
```

## API by task

| Task | Methods |
|------|---------|
| Session | `new OpenEDA({ locale?, history? })`, `create`, `open`, `importKicadSch`, `importKicadSymbol`, `importKicadNet`, `inspect`, `diff`, `capabilities`, `dispose` |
| Components | `addComponent`, `copyComponent`, `moveComponent`, `updateComponent`, `setFootprint`, `deleteComponent`, `setComponentProperty`, `listComponentProperties`, `removeComponentProperty`, `setPinType` |
| Nets and buses | `connectNet`, `disconnectNet`, `deriveNets`, `renameNet`, `mergeNets`, `deleteNet`, `addBus`, `listBuses`, `removeBus`, `addNetClass`, `listNetClasses`, `removeNetClass` |
| Wiring | `addWire`, `updateWire`, `splitWire`, `listWires`, `deleteWire`, `translate`, `addJunction`, `listJunctions`, `moveJunction`, `removeJunction` |
| Annotations | `addLabel`, `listLabels`, `updateLabel`, `removeLabel`, `addText`, `listTexts`, `updateText`, `moveText`, `deleteText`, `addNoConnect`, `listNoConnects`, `removeNoConnect` |
| Sheets | `addSheet`, `listSheets`, `renameSheet`, `deleteSheet`, `addHierarchy`, `listHierarchy`, `addHierarchyPort`, `removeHierarchyPort`, `removeHierarchy` |
| Design data | `annotate`, `setProperties`, `addVariant`, `listVariants`, `removeVariant`, `setReferencePrefix`, `removeReferencePrefix`, `listReferencePrefixes`, `setComponentClass`, `listComponentClasses`, `stats`, `compareNetlist` |
| Reports | `listObjects`, `listComponents`, `listNets`, `listSymbols`, `connectivity`, `inspect` |
| Output | `save`, `export`, `undo`, `redo`, `close` |

`inspect` runs `erc`, `electrical`, `nets`, `connectivity`, `footprints` and
`power`, with per-code `severity` overrides and `waivers`. `export` writes
`net`, `net_csv`, `net_hier`, `spice`, `spice_subckt`, `svg`, `bom`, `bom_json`,
`bom_html`, `pdf`, `report`, `json`, `kicad_sch` and `kicad_sym`.
`importKicadSch` reads the supported KiCad schematic subset and returns a loss
ledger for everything else.

## Constraints

- No board model, routing, DRC or clearance analysis.
- No simulation; the SPICE output is a netlist only.
- No bus wires, arcs or child-sheet content in the KiCad subset.
- No automatic footprint assignment or validation.
- Labels never derive, rename or merge a net; cross-sheet label scope is not
  modelled.
- Undo history is per open handle; `history: 'sidecar'` keeps it beside the file
  so the CLI can undo across invocations.

## Links

- [Product overview](https://github.com/nyar-os/OpenEDA)
- [Capability matrix](https://github.com/nyar-os/OpenEDA/blob/master/documentation/capabilities.md)
- [CLI reference](https://github.com/nyar-os/OpenEDA/blob/master/documentation/cli.md)

## License

Mozilla Public License 2.0. See [License.md](License.md).
