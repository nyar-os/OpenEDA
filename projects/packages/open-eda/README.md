# @wae/open-eda

TypeScript authoring surface for OpenEDA: one editable design document behind
schematic capture, board layout, inspection, export and agent authoring. The
session entry point is `new OpenEDA(options?)`.

## Status

The schematic document core is implemented against the native `open-eda-model`
crate. `new OpenEDA(options?)` creates a session synchronously, and the returned
`Design` handle supports:

- `addComponent`, `moveComponent`, `updateComponent`, `setFootprint`, `deleteComponent`
- `connectNet`, `renameNet`, `addWire`, `deleteWire`
- `addJunction`, `addText`
- `addSheet`, `renameSheet`, `deleteSheet`
- `undo`, `redo`, with `canUndo`/`canRedo` and a bounded 32-step history
- `listObjects`, `listComponents`, `listNets`, `listSymbols`, `connectivity`
- `inspect` with `erc`, `nets`, `connectivity`, `footprints` and `power`, plus
  per-code `severity` overrides and `waivers` that move findings to `waived`
- `save` to a native `.oeda` document and reopen it
- `export` to `net`, `net_csv`, `spice`, `svg`, `bom`, `bom_json`, `bom_html`, `pdf`, `report`, `json` or `kicad_sch`
- `OpenEDA.importKicadSch({ source, target? })` from KiCad schematic text or a
  file path, optionally materializing a native document

| Capability | Status | Notes |
| --- | --- | --- |
| `.oeda` create/open/save/edit | supported | sheets, components, nets, buses, wires, undo/redo |
| checks `erc`, `nets`, `connectivity`, `footprints`, `power` | supported | `drc` and `clearance` report `capability/not-implemented` |
| export `net`, `net_csv`, `spice`, `svg`, `bom`, `bom_json`, `bom_html`, `pdf`, `report`, `json` | supported | read-only projections |
| export `kicad_sch` | supported | KiCad 6/7 S-expression, deterministic identifiers |
| import `kicad_sch` | supported | symbols, wires, junctions, no-connect flags, text and labels; other constructs return loss entries |
| export `kicad_pcb`, `gbr` | unsupported | declared, not implemented |
| import `kicad_pcb`, `sch`, `brd`, `gbr`, `step`, ... | unsupported | declared, not implemented |
| interactive renderer | unsupported | the preview is an SVG projection |

The KiCad importer reports everything outside its subset instead of dropping it:
buses, hierarchical sheets, arcs, images and unknown root constructs each add a
`{"code": "kicad/unsupported-element", "detail": "..."}` entry to `losses`.
Global and hierarchical labels are imported as sheet-local net labels and add a
`{"code": "kicad/label-scope", "detail": "..."}` entry because cross-sheet
label scope is not modelled. Junctions, no-connect flags and text annotations
are part of the subset. Duplicate references and symbols without a reference
are reported as `kicad/duplicate-reference` and `kicad/missing-reference`.

A component stores an optional footprint. `setFootprint` assigns or clears it,
the KiCad writer emits the `Footprint` property and the importer reads it back,
`bom` groups by value and footprint with designator ranges, `bom_json`
projects the same rows as JSON and `bom_html` writes them as an HTML table.
The model never picks or validates a footprint. No-connect flags can name a
`ref`/`pin` node, which makes ERC and the connectivity report treat that pin as
deliberately open.

The package also exports the design types, stable error codes and the format
registry (`kicad_sch`/`kicad_pcb`/`sch`/`brd`/`net`/`edif`/`spice`/`cir`/`gbr`/
`drl`/`step`/`svg`/`pdf`/`png`/`bom`).

`drc`, `clearance`, KiCad board/Gerber exchange and the interactive
renderer report `capability/not-implemented`. Classification is not a support
claim: reads, writes and exports are reported by `capabilities()`, never
inferred from an extension or a method name. Undo history belongs to one open
handle and is not written into the document, so opening a file starts a fresh
history. A session created with `new OpenEDA({ history: 'sidecar' })` also keeps
the same bounded 32-step history in `<file>.history.json` beside the document,
so `undo`/`redo` work across separate processes; `save()` to a new target clears
the stale sidecar on that target, and an exhausted direction reports
`history/empty`. The CLI is a thin shell over this session and enables the
sidecar.

## Boundaries

- No board parsing, routing, clearance analysis or interactive rendering in this
  package.
- No second design model: the CLI, the SDK and the studio consume one revision
  from `open-eda-model`.
- Native access goes through the Node-API binding.

## Scripts

```bash
pnpm --filter @wae/open-eda run typecheck
pnpm --filter @wae/open-eda run build
pnpm --filter @wae/open-eda run test
```
