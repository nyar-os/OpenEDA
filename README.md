# OpenEDA

One editable schematic document behind capture, inspection, netlist and fabrication
handoff, for people, scripts and agents.

[![Formats](https://img.shields.io/badge/formats-KiCad%20%7C%20Gerber%20%7C%20Netlist-1f6feb)](https://github.com/nyar-os/OpenEDA)
[![Editable](https://img.shields.io/badge/documents-editable-0f766e)](https://github.com/nyar-os/OpenEDA)
[![Agent skills](https://img.shields.io/badge/agents-skills-5b6b78)](https://github.com/nyar-os/OpenEDA)

## 💡 What is OpenEDA?

OpenEDA keeps an electronic design in one revisioned `.oeda` document. Schematic
sheets, components, nets, wires, junctions, labels, no-connect flags, buses,
hierarchy declarations and the title block live in that document instead of
being scattered across a folder of exchange files.

You can edit the document from the `open-eda` command line, from the
`@wae/open-eda` SDK, or through an agent that follows the bundled skill. Every
path goes through the same schematic model, so an edit, a check and an export
always describe the same revision.

OpenEDA is **not** a board editor, a router, a field solver or a simulator. Board
layout, DRC, Gerber and KiCad board exchange report `capability/not-implemented`
instead of pretending to work. The `clearance` check compares schematic
component bodies; it does not analyse copper or board geometry. The SPICE writer
emits a netlist; it does not run a simulation.

## 🚀 Getting started

### 1. Agent path (recommended)

Install the skills package, then point your agent at the skill file:

```bash
pnpm add @wae/open-eda-skills
```

```text
node_modules/@wae/open-eda-skills/skills/open-eda/SKILL.md
```

A realistic prompt:

> Create a schematic called Blinker with a title block. Add R1 and C1, connect
> them to VCC and GND, add a global label, run ERC and export a netlist, a BOM
> and an SVG preview to `./out`.

### 2. Manual install

```bash
pnpm add @wae/open-eda

pnpm exec open-eda create --out blinker.oeda --title Blinker --rev A
pnpm exec open-eda component add blinker.oeda --ref R1 --value 10k --x 40 --y 30 --pins 1,2
pnpm exec open-eda component add blinker.oeda --ref C1 --value 100n --x 70 --y 30 --pins 1,2
pnpm exec open-eda net connect blinker.oeda --net VCC --nodes R1.1,C1.1
pnpm exec open-eda wire add blinker.oeda --net VCC --from 40,30 --to 70,30
pnpm exec open-eda label add blinker.oeda --text VCC --x 40 --y 30 --scope global
pnpm exec open-eda inspect blinker.oeda --checks erc,connectivity
pnpm exec open-eda export blinker.oeda --format net --out blinker.net
pnpm exec open-eda export blinker.oeda --format kicad_sch --out blinker.kicad_sch
```

Every edit rewrites the document through a temporary file and a rename. A
rejected edit leaves the file unchanged, and an unsupported export fails before
any file is written. The CLI keeps sidecar history, so `open-eda undo blinker.oeda`
restores the previous revision.

The SDK is the same engine with a typed API:

```ts
import { OpenEDA } from '@wae/open-eda';

const eda = new OpenEDA({ history: 'sidecar' });
const design = await eda.create({ title: 'Blinker', rev: 'A' });
await design.addComponent({ reference: 'R1', value: '10k', x: 40, y: 30, pins: ['1', '2'] });
await design.connectNet({ net: 'VCC', nodes: ['R1.1'] });
await design.setComponentProperty({ reference: 'R1', key: 'MPN', value: 'ABC-123' });
const report = await design.stats();
await design.save({ target: 'blinker.oeda', overwrite: true });
await eda.dispose();
```

`pnpm exec open-eda capabilities` prints exactly which reads, writes and exports
the installed build supports. Classification is not a support claim: an
extension may be listed while remaining `unsupported`.

## ✨ Highlights

- **One editable schematic document.** Sheets, components, nets, buses, wires,
  junctions, no-connect flags, text, net labels with local/global/hierarchical
  scope and hierarchical sheet symbols share one revision counter.
- **Checks that state their result.** ERC, electrical-role, bus, net,
  connectivity, footprint, component-clearance and power checks return stable
  finding codes; severities and waivers are part of the report.
- **Deterministic exchange.** KiCad schematic and symbol import/export,
  netlist/net CSV/hierarchical netlist, SPICE and subcircuits, SVG, BOM
  (CSV/JSON/HTML), PDF, Markdown and JSON projections, with a loss ledger for
  everything the document cannot hold.
- **Design data that survives.** Reference annotation, BOM variants with DNP
  lists, net classes, custom component properties, pin electrical roles, design
  statistics and a structural `diff` between two documents.
- **Pins you can edit.** Add, remove and rename component pins after placement;
  a rename follows every net node, no-connect flag and electrical role that
  named the old `REF.PIN`, and an in-use pin is never removed silently.
- **Agent-ready.** The skills package records the command surface, preflight
  steps and evidence rules so an assistant can author and review a schematic
  without inventing capabilities.
- **Undo across runs.** The CLI keeps up to 32 snapshots in
  `<file>.history.json`, so `undo` and `redo` work between invocations.
- **A studio window.** `open-eda gui` serves the bundled static studio over the
  same native operations.

## 🧪 Examples

- [Package README](projects/packages/open-eda/README.md) — SDK exports, usage
  and constraints.
- [Schematic test](projects/crates/eda-model/tests/unit/lib.rs) — create, edit,
  inspection, save/open round trip and every writer.
- [KiCad round-trip test](projects/crates/eda-model/tests/unit/kicad.rs) — the
  supported schematic subset and its loss ledger.
- [Agent skill](projects/packages/open-eda-skills/skills/open-eda/SKILL.md) —
  commands, workflows and evidence rules.
- [Maintainer documentation](documentation/README.md) — architecture,
  capabilities, CLI reference and development guide.

## 📚 Documentation

- [Architecture](documentation/architecture.md) — schematic model, module
  boundaries, data flow, determinism and history ownership.
- [Capabilities](documentation/capabilities.md) — capability matrix, check
  semantics, format support, loss codes and unsupported boundaries.
- [CLI reference](documentation/cli.md) — every command, flag and example.
- [Development](documentation/development.md) — build, test, native addon
  staging, CI and release.
- [Package README](projects/packages/open-eda/README.md) — SDK surface for
  `@wae/open-eda`.

## 📜 License

Mozilla Public License 2.0. See [License.md](License.md).
