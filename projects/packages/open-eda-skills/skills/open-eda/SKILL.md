---
name: open-eda
description: Create, edit, inspect and deliver schematic design documents with OpenEDA. Use when a task involves schematic sheets, components, nets, netlists, BOM or PDF handoff, ERC review or SVG sheet output.
---

# open-eda

## Status

The schematic document core is executable. The CLI and SDK create, edit and
reopen one `.oeda` document, run ERC, electrical, net, connectivity, footprint
and power checks, compare two documents with `diff`, and write a netlist, SPICE netlist,
SVG sheet, BOM CSV/JSON/HTML, one-page PDF or
Markdown report projection. The design carries title-block properties (title,
drawing revision, company and date) that `design set` edits and the KiCad writer
reads and writes. Hierarchical sheet symbols and their ports round-trip as
declarations, and `export --format kicad_sym` writes the placed components' pin
interface as an importable KiCad symbol library. The SDK keeps a bounded 32-step undo/redo history
per open handle, and the CLI mirrors it in a `<file>.history.json` sidecar so
`undo` and `redo` work across separate command invocations. Sheet
add/rename/delete, component move, update and delete, deterministic reference
annotation, BOM variants with do-not-populate lists, no-connect flags that can
mark a pin as deliberately open, net derive, disconnect and
delete, and junction/text move and delete work in place. Board layout, the
interactive renderer, DRC and clearance checks, and KiCad board, Gerber and drill
exchange are not implemented and report `capability/not-implemented`.

Only these commands are executable today:

```bash
open-eda capabilities
open-eda create --out <file.oeda> [--title <title>] [--rev <rev>] [--company <company>] [--date <date>] [--overwrite]
open-eda design set <file.oeda> [--title <title>] [--rev <rev>] [--company <company>] [--date <date>]
open-eda open <file.oeda>
open-eda list <file.oeda>
open-eda symbols <file.oeda>
open-eda connectivity <file.oeda>
open-eda inspect <file.oeda> [--checks erc,electrical,nets,connectivity,footprints,power] [--severity code=level,...] [--waive code[@objectId|@sheetId/objectId],...]
open-eda diff <left.oeda> <right.oeda>
open-eda annotate <file.oeda>
open-eda export <file.oeda> --format net|net_csv|net_hier|spice|spice_subckt|svg|bom|bom_json|bom_html|pdf|report|kicad_sch|kicad_sym --out <file> [--variant <name>]
open-eda variant add <file.oeda> --name <name> [--dnp R1,R2] [--values R1=22k]
open-eda variant list <file.oeda>
open-eda variant remove <file.oeda> --name <name>
open-eda import <file.kicad_sch> --out <file.oeda> [--overwrite]
open-eda library <file.kicad_sym>
open-eda undo <file.oeda>
open-eda redo <file.oeda>
open-eda component add <file.oeda> --ref <ref> --value <value> --x <n> --y <n> [--rotation <deg>] [--pins 1,2] [--sheet <sheet>]
open-eda component move <file.oeda> --ref <ref> --x <n> --y <n> [--rotation <deg>] [--sheet <sheet>]
open-eda component update <file.oeda> --ref <ref> --value <value> [--sheet <sheet>]
open-eda component footprint <file.oeda> --ref <ref> [--footprint <footprint>] [--sheet <sheet>]
open-eda component pin-type <file.oeda> <ref> <pin> [type] [--clear] [--sheet <sheet>]
open-eda component delete <file.oeda> --ref <ref> [--sheet <sheet>]
open-eda net connect <file.oeda> --net <name> --nodes <ref.pin>,<ref.pin> [--sheet <sheet>]
open-eda net derive <file.oeda> [--sheet <sheet>]
open-eda net disconnect <file.oeda> --net <name> [--nodes <ref.pin>,<ref.pin>] [--sheet <sheet>]
open-eda net delete <file.oeda> --net <name> [--sheet <sheet>]
open-eda net rename <file.oeda> --id <net> --name <new-name> [--sheet <sheet>]
open-eda bus add <file.oeda> --name <name> --width <n> [--aliases D,E]
open-eda bus list <file.oeda>
open-eda bus remove <file.oeda> --id <bus>
open-eda wire delete <file.oeda> --id <wire-id> [--sheet <sheet>]
open-eda junction add <file.oeda> --x <n> --y <n> [--sheet <sheet>]
open-eda junction move <file.oeda> --index <n> --x <n> --y <n> [--sheet <sheet>]
open-eda junction delete <file.oeda> --index <n> [--sheet <sheet>]
open-eda no-connect add <file.oeda> --x <n> --y <n> [--ref <ref> --pin <pin>] [--sheet <sheet>]
open-eda no-connect delete <file.oeda> --index <n> [--sheet <sheet>]
open-eda text add <file.oeda> --text <text> --x <n> --y <n> [--sheet <sheet>]
open-eda text move <file.oeda> --index <n> --x <n> --y <n> [--sheet <sheet>]
open-eda text delete <file.oeda> --index <n> [--sheet <sheet>]
open-eda sheet add <file.oeda> [--name <name>]
open-eda sheet rename <file.oeda> --id <sheet-id> --name <name>
open-eda sheet delete <file.oeda> --id <sheet-id>
open-eda gui
```

Every editing command except `capabilities`, `open`, `diff`, `list`, `inspect`
and `gui` rewrites the input document in place through a temporary file and a
rename. Confirm the path before running them.

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
| review | Audit references, connectivity, pin roles and component values | `open-eda inspect` findings with codes and objects |
| compare | See what changed between two revisions | `open-eda diff` report with changed objects and `equal` |
| deliver | Write a netlist, SVG, BOM or PDF projection | the exported file plus `open-eda inspect` at the same revision |

## Inspection checks

`erc` reports duplicate references, empty references, duplicate net names and net
nodes that point at a missing component or pin, and warns about unconnected pins.
It also rejects names that would break the writer grammar:
`erc/reference-illegal` and `erc/pin-name-illegal` for whitespace or a dot inside
a `REF.PIN` token, and `erc/net-name-illegal` for whitespace or control
characters in a net name. A no-connect flag with `--ref` and `--pin` (or the SDK
`node`) marks that pin as deliberately open: `erc/unconnected-pin` and
`unreferenced-pin` are skipped for it, and the connectivity report omits it from
`unconnectedPins`. A coordinate-only flag suppresses nothing.
`nets` reports floating wires, wires that name an undeclared net, single-node
nets and empty sheets. A net named with a bus alias resolves to the same bus as
the canonical name; `net-bus-alias-collision` reports two nets on one sheet that
resolve to the same bus. `connectivity` groups wires by coincident endpoints,
junctions and T-junctions, then reports `wire-net-conflict` (one error per group
naming two nets), `wire-net-unassigned` (one warning per unlabeled group) and
`junction-floating`. `footprints` reports components without a value. `power`
detects rails by case-insensitive prefix (`VCC`, `VDD`, `VSS`, `GND`, `VBAT`,
`VIN`, `PWR`, or a leading `+`/`-`) and warns with `power-net-underconnected`
below two nodes and `power-net-duplicate` when one rail name appears on more
than one sheet. It also reports `power/driver-conflict` (error) when more than
one `power_out` pin drives a net and `power/output-driver-conflict` (error) when
a `power_out` pin shares a net with a logic `output` pin. Errors fail the
report; warnings are reported but do not.

`inspect` also accepts `--severity code=level` overrides (`error`, `warning`,
`info` or `off`) and `--waive code[@objectId|@sheetId/objectId]` entries. A
waiver moves the finding from `issues` to `waived` (shape plus `waived: true`),
and only active errors fail the report. Never use a waiver to hide a finding the
user has not accepted.

`drc` and `clearance` are declared but not implemented; requesting them rejects
with `capability/not-implemented`. Never describe a design as DRC clean or
clearance checked.

## Electrical check

Pins may carry a role: `input`, `output`, `bidirectional`, `tri_state`,
`passive`, `power_in`, `power_out`, `open_collector`, `open_emitter` or
`unspecified`. `component pin-type` assigns one, and `--clear` removes it. The
`electrical` check reads only typed pins and reports
`erc/input-input-conflict`, `erc/output-output-conflict` and
`erc/power-in-unpowered`; an untyped pin is never a finding. Power-driver
conflicts between `power_out` pins, or between a `power_out` and a logic
`output`, belong to the `power` check (`power/driver-conflict`,
`power/output-driver-conflict`). Roles are written
into the KiCad library symbols and read back; an unknown imported type is a loss
entry, not a guess.

## Reference annotation

`open-eda annotate <file.oeda>` renumbers every reference designator: sheets in
document order, components top to bottom and left to right, each prefix from 1.
Net nodes are rewritten to the new references, and the command reports every
`from`/`to` change. An already annotated design adds no revision. References
without leading letters join the `U` group; the numerals in `TP_A` are replaced.

## BOM variants and DNP

`open-eda variant add` declares a design-level variant with `--dnp` (reference
designators that are not populated) and `--values` (`ref=value` overrides). It
replaces a variant with the same name. Every designator must already be placed;
a variant never invents a part. `export --variant <name>` applies the variant to
`bom` and `bom_json` and `bom_html` only: DNP rows disappear and overridden
values replace the
component value. Other export targets reject a variant with
`session/argument-invalid`. Variants do not change the schematic, the KiCad
`dnp` flag or any board output.

## Drawing properties
`title`, `rev`, `company` and `date` are design-wide title-block properties.
`rev` is a text label such as `A`; it is not the edit revision counter. `design
set` changes them in place, an empty string clears a field, and an update that
changes nothing adds no revision. The KiCad writer emits them as a
`title_block`; the importer reads the same four fields back and reports any
other title-block field as a loss.

## Design diff
`open-eda diff <left.oeda> <right.oeda>` compares the modelled objects of two
documents and never writes a file. Sheets match by id, components by reference,
nets by name inside a sheet and buses and BOM variants by name. Only changed
sheets appear; a sheet on one side only lists all its objects as added or
removed. The report, tagged `open-eda-diff`, records field-level changes (value,
footprint, position, rotation, pins, pin roles, net node sets, wire endpoints,
bus width, variant DNP lists and variant value overrides), and `equal` is true
when nothing changed. Net node order is not a change, and the
internal edit counter is ignored, but changed drawing properties (title, rev,
company, date, units) make the report unequal. This is not a textual
or byte diff, and it never merges, applies or repairs a difference.

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

`open-eda library <file.kicad_sym>` reads a KiCad symbol library instead of a
design document. It reports every declared symbol with its unit count and each
pin's number, name, electrical role and unit, plus a
`kicad_sym/unsupported-element` loss entry for any construct outside that
subset. It creates no design handle and writes no file.
`open-eda export --format kicad_sym` is the reverse projection: it writes one
library symbol per derived library id with the pins the schematic declares, and
adds no body, value or footprint.

## Referential integrity

- Deleting a component removes its net nodes, drops nets that become empty, and
  detaches wires that named a dropped net.
- Renaming a net updates every wire that used the old name, and rejects a name
  already used on that sheet.
- A sheet with components, nets or wires cannot be deleted, and the last sheet is
  always protected (`sheet/not-empty`, `sheet/last-sheet`).

## Export limits

- `net` writes `# OpenEDA netlist v1`, then `NET <name>` and `NODE <ref>.<pin>`.
  A declared bus expands into `<bus><index>` members, and a net named with a bus
  alias expands into the canonical members rather than alias-derived names.
- `net_csv` writes the same connectivity as `Sheet,Net,Ref,Pin` rows; bus nets
  expand into their member names and fields are quoted when the CSV grammar
  needs it.
- `net_hier` prefixes each net with its sheet path (`NET Main/VCC`). The segment
  is the sanitized sheet name, a numeric suffix keeps it unique, and a name with
  no usable characters falls back to the sheet id. A `SHEET <path>` line precedes
  that sheet's nets, and declared buses expand like the flat netlist.
- `spice` writes a deterministic `.cir` netlist; open pins become
  `NC_<ref>_<pin>` and are reported with `spice/open-pin`.
- `spice_subckt` writes one `.SUBCKT <name> <ports>` block per sheet, with the
  sheet's nets as ports and a `.ENDS <name>` terminator; names and ports are
  legalized and unique, open pins use the same `NC_<ref>_<pin>` nodes, and the
  file ends with `.END`.
- `svg` writes a sheet frame, wires and component boxes with reference and value
  labels; the viewBox is derived from the placed geometry.
- `bom` writes `Comment,Footprint,Designator,Quantity`, grouped by value and
  footprint with sorted designators compressed into inclusive ranges (`R5-R7`
  for three or more consecutive numbers). Footprint text is whatever was
  assigned with `component footprint`; the model never picks one.
  `--variant <name>` drops that variant's DNP designators and applies its value
  overrides.
- `bom_json` writes the same rows as a deterministic `open-eda-bom` JSON
  projection with `designators` and `designatorRanges` arrays.
- `bom_html` writes one self-contained HTML table with the same rows, escaped
  fields and the design header; `--variant <name>` applies the variant.
- `pdf` writes one A4 landscape vector page with sheet names, component boxes,
  wires and a cross-reference table.
- `report` writes deterministic Markdown with a design header, per-sheet object
  counts, a `Net | Nodes | Pins` table per sheet and the inspection findings
  grouped by severity.
- `kicad_sch` writes the modelled schematic subset plus the title-block title,
  date, revision and company, the library pin electrical roles and hierarchical
  sheet symbols with their ports; `kicad_pcb`
  and `gbr` reject with `capability/not-implemented`.
- `kicad_sym` writes one importable `kicad_symbol_lib` with one symbol per
  derived library id and one `_1_1` unit holding the pins, their names and
  electrical roles. It writes no graphics, value or footprint and invents no
  library part.
- No exchange format can be imported or edited. `.oeda` is the only editable
  document; `import` materializes a `.kicad_sch` into `.oeda`, while `library`
  only reads a `.kicad_sym` symbol declaration. Hierarchical
  `sheet` symbols and their `pin` ports import as declarations; the referenced
  child sheet file is never opened.

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
