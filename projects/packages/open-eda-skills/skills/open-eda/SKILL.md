---
name: open-eda
description: Create, edit, inspect and deliver schematic design documents with OpenEDA. Use when a task involves schematic sheets, components, nets, netlists, BOM or PDF handoff, ERC review or SVG sheet output.
---

# open-eda

## Status

The schematic document core is executable. The CLI and SDK create, edit and
reopen one `.oeda` document, run ERC, net, connectivity, footprint and power
checks, and write a netlist, SPICE netlist, SVG sheet, BOM CSV, one-page PDF or
Markdown report projection. The SDK keeps a bounded 32-step undo/redo history
per open handle, and the CLI mirrors it in a `<file>.history.json` sidecar so
`undo` and `redo` work across separate command invocations. Sheet
add/rename/delete, component move, update and delete, net derive, disconnect and
delete, and junction/text move and delete work in place. Board layout, the
interactive renderer, DRC and clearance checks, and KiCad board, Gerber and drill
exchange are not implemented and report `capability/not-implemented`.

Only these commands are executable today:

```bash
open-eda capabilities
open-eda create --out <file.oeda> [--title <title>] [--overwrite]
open-eda open <file.oeda>
open-eda list <file.oeda>
open-eda symbols <file.oeda>
open-eda connectivity <file.oeda>
open-eda inspect <file.oeda> [--checks erc,nets,connectivity,footprints,power] [--severity code=level,...] [--waive code[@objectId|@sheetId/objectId],...]
open-eda export <file.oeda> --format net|net_csv|spice|svg|bom|bom_json|pdf|report|kicad_sch --out <file>
open-eda import <file.kicad_sch> --out <file.oeda> [--overwrite]
open-eda undo <file.oeda>
open-eda redo <file.oeda>
open-eda component add <file.oeda> --ref <ref> --value <value> --x <n> --y <n> [--rotation <deg>] [--pins 1,2] [--sheet <sheet>]
open-eda component move <file.oeda> --ref <ref> --x <n> --y <n> [--rotation <deg>] [--sheet <sheet>]
open-eda component update <file.oeda> --ref <ref> --value <value> [--sheet <sheet>]
open-eda component footprint <file.oeda> --ref <ref> [--footprint <footprint>] [--sheet <sheet>]
open-eda component delete <file.oeda> --ref <ref> [--sheet <sheet>]
open-eda net connect <file.oeda> --net <name> --nodes <ref.pin>,<ref.pin> [--sheet <sheet>]
open-eda net derive <file.oeda> [--sheet <sheet>]
open-eda net disconnect <file.oeda> --net <name> [--nodes <ref.pin>,<ref.pin>] [--sheet <sheet>]
open-eda net delete <file.oeda> --net <name> [--sheet <sheet>]
open-eda net rename <file.oeda> --id <net> --name <new-name> [--sheet <sheet>]
open-eda wire delete <file.oeda> --id <wire-id> [--sheet <sheet>]
open-eda junction add <file.oeda> --x <n> --y <n> [--sheet <sheet>]
open-eda junction move <file.oeda> --index <n> --x <n> --y <n> [--sheet <sheet>]
open-eda junction delete <file.oeda> --index <n> [--sheet <sheet>]
open-eda text add <file.oeda> --text <text> --x <n> --y <n> [--sheet <sheet>]
open-eda text move <file.oeda> --index <n> --x <n> --y <n> [--sheet <sheet>]
open-eda text delete <file.oeda> --index <n> [--sheet <sheet>]
open-eda sheet add <file.oeda> [--name <name>]
open-eda sheet rename <file.oeda> --id <sheet-id> --name <name>
open-eda sheet delete <file.oeda> --id <sheet-id>
open-eda gui
```

Every editing command except `capabilities`, `open`, `list`, `inspect` and `gui`
rewrites the input document in place through a temporary file and a rename.
Confirm the path before running them.

`undo` and `redo` restore the newest snapshot from `<file>.history.json`, bump
the restored revision relative to the current file, and move the replaced text to
the opposite stack. Each mutating command records the document it replaces, keeps
the newest 32 steps and clears the redo stack. `create` deletes a stale sidecar.
A direction with no snapshot fails with `history/empty`. The sidecar is not part
of the design and is never written into `.oeda`.

## Preflight

1. Run `open-eda capabilities` and confirm that the check or export target the
   task needs is reported as `supported`. A method or extension name is not a
   capability.
2. Run `open-eda list <file.oeda>` and confirm the components, references, pins
   and sheet ids the task will address actually exist.
3. If the preflight reports a missing capability, stop and report it. Never
   degrade silently or swap in an unrelated tool.

## Executable routes

| Route | Intent | Minimum evidence |
|-------|--------|------------------|
| create | Build a schematic document from a brief | `open-eda list` output, `open-eda inspect` report |
| edit | Change an existing document without losing identity | `open-eda list` before and after, `open-eda inspect` |
| review | Audit references, connectivity and component values | `open-eda inspect` findings with codes and objects |
| deliver | Write a netlist, SVG, BOM or PDF projection | the exported file plus `open-eda inspect` at the same revision |

## Inspection checks

`erc` reports duplicate references, empty references, duplicate net names and net
nodes that point at a missing component or pin, and warns about unconnected pins.
It also rejects names that would break the writer grammar:
`erc/reference-illegal` and `erc/pin-name-illegal` for whitespace or a dot inside
a `REF.PIN` token, and `erc/net-name-illegal` for whitespace or control
characters in a net name.
`nets` reports floating wires, wires that name an undeclared net, single-node
nets and empty sheets. `connectivity` groups wires by coincident endpoints,
junctions and T-junctions, then reports `wire-net-conflict` (one error per group
naming two nets), `wire-net-unassigned` (one warning per unlabeled group) and
`junction-floating`. `footprints` reports components without a value. `power`
detects rails by case-insensitive prefix (`VCC`, `VDD`, `VSS`, `GND`, `VBAT`,
`VIN`, `PWR`, or a leading `+`/`-`) and warns with `power-net-underconnected`
below two nodes and `power-net-duplicate` when one rail name appears on more
than one sheet. Errors fail the report; warnings are reported but do not.

`inspect` also accepts `--severity code=level` overrides (`error`, `warning`,
`info` or `off`) and `--waive code[@objectId|@sheetId/objectId]` entries. A
waiver moves the finding from `issues` to `waived` (shape plus `waived: true`),
and only active errors fail the report. Never use a waiver to hide a finding the
user has not accepted.

`drc` and `clearance` are declared but not implemented; requesting them rejects
with `capability/not-implemented`. Never describe a design as DRC clean or
clearance checked.

## Connectivity report

`open-eda connectivity <file.oeda>` lists what each net actually connects: per
sheet, every net with its node count and `REF.PIN` pin list, the pins no net
reaches, and the wire-group and junction totals (groups, unassigned groups,
conflicting groups, floating junctions). The report is read-only, tagged
`open-eda-connectivity`, and deterministic for one revision; it never assigns
nets or edits the document.

## Symbol library

`open-eda symbols` groups placed components by the library id the KiCad writer
derives from their pins (`open-eda:<n>pin` for the numeric sequence `1..n`, a
hashed suffix otherwise). Each entry reports the pin count, the sorted
references that place the symbol, the distinct assigned footprints and how many
components still have no footprint. It is a read-only report: it never assigns
a footprint and never claims a library part exists outside the document.

## Referential integrity

- Deleting a component removes its net nodes, drops nets that become empty, and
  detaches wires that named a dropped net.
- Renaming a net updates every wire that used the old name, and rejects a name
  already used on that sheet.
- A sheet with components, nets or wires cannot be deleted, and the last sheet is
  always protected (`sheet/not-empty`, `sheet/last-sheet`).

## Export limits

- `net` writes `# OpenEDA netlist v1`, then `NET <name>` and `NODE <ref>.<pin>`.
- `net_csv` writes the same connectivity as `Sheet,Net,Ref,Pin` rows; bus nets
  expand into their member names and fields are quoted when the CSV grammar
  needs it.
- `spice` writes a deterministic `.cir` netlist; open pins become
  `NC_<ref>_<pin>` and are reported with `spice/open-pin`.
- `svg` writes a sheet frame, wires and component boxes with reference and value
  labels; the viewBox is derived from the placed geometry.
- `bom` writes `Comment,Footprint,Designator,Quantity`, grouped by value and
  footprint with sorted designators compressed into inclusive ranges (`R5-R7`
  for three or more consecutive numbers). Footprint text is whatever was
  assigned with `component footprint`; the model never picks one.
- `bom_json` writes the same rows as a deterministic `open-eda-bom` JSON
  projection with `designators` and `designatorRanges` arrays.
- `pdf` writes one A4 landscape vector page with sheet names, component boxes,
  wires and a cross-reference table.
- `report` writes deterministic Markdown with a design header, per-sheet object
  counts, a `Net | Nodes | Pins` table per sheet and the inspection findings
  grouped by severity.
- `kicad_sch` writes the modelled schematic subset; `kicad_pcb` and `gbr` reject
  with `capability/not-implemented`.
- No exchange format can be imported or edited. `.oeda` is the only editable
  document; `import` materializes a `.kicad_sch` into `.oeda`.

## Hard rules

- Never claim a check ran when it did not, and never fabricate ERC or
  connectivity results.
- Never describe a format as supported because an extension is listed; read the
  capability matrix.
- Keep one design model: the netlist, SVG, BOM or PDF must come from the revision
  the inspection saw.
- Do not add ad-hoc geometry, routing or placement outside the editing commands.
- In the SDK, undo history belongs to one open handle. Across CLI invocations it
  lives in `<file>.history.json`; deleting the sidecar or recreating the design
  discards it.

## Safety

- Confirm with the user before any command that writes or overwrites a design
  document, and state the target path first.
- Never overwrite an input design unless the task asks for an in-place edit.
- Fabrication outputs are manufacturing inputs; state exactly what was exported
  and what was not. No Gerber or drill file can be produced by this build.
